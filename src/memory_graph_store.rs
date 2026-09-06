//! 内存图存储实现
//!
//! M40.1：从 tests/common/memory_graph_store.rs 迁入生产模块。
//! 不依赖 redb / 文件系统，在 browser-wasm 和 native 测试中共用。

use crate::graph::{Edge, FileState, Node};
use crate::graph_store::{
    GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreResult, GraphWriteStore, IndexCommit,
    IndexReport, IndexStateStore,
};
use std::collections::{HashMap, HashSet};

/// 内存图存储
///
/// 手工构造节点和边，实现 GraphReadStore / GraphWriteStore / IndexStateStore。
/// 用于 browser-wasm 场景和测试替身。
#[derive(Debug)]
pub struct MemoryGraphStore {
    nodes: HashMap<String, Node>,
    /// M59-B1：邻接表只存**边**，不存节点副本。
    ///
    /// 原先存的是 `(Node, Edge)`——建边时把端点 clone 进来。之后
    /// `upsert_node` 更新节点，`get_node` 返回新值而邻边里仍是旧副本，
    /// 同一个节点从两条路径读出两种内容。redb 侧走 petgraph 的
    /// `node_weight` 实时查，从来没有这个问题。现在读时按 id 现查
    /// （见 `neighbors_of`），结构上消除了不一致的可能。
    outgoing: HashMap<String, Vec<Edge>>,
    incoming: HashMap<String, Vec<Edge>>,
    /// M59-B1：边去重键集合，与 `graph_redb` 的 `seen_edges` 同口径。
    seen_edges: HashSet<(String, String, crate::graph::EdgeType, Option<String>)>,
    file_states: HashMap<String, FileState>,
    checkpoint: Option<crate::diff_refresh::DiffRefreshCheckpoint>,
}

impl MemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            seen_edges: HashSet::new(),
            file_states: HashMap::new(),
            checkpoint: None,
        }
    }

    /// 读取最近一次 persist_index 提交的 diff-refresh checkpoint。
    pub fn diff_refresh_checkpoint(&self) -> Option<&crate::diff_refresh::DiffRefreshCheckpoint> {
        self.checkpoint.as_ref()
    }

    pub fn add_test_node(
        &mut self,
        id: &str,
        name: &str,
        node_type: crate::graph::NodeType,
        path: &str,
    ) {
        self.nodes.insert(
            id.to_string(),
            Node {
                id: id.to_string(),
                node_type,
                path: path.to_string(),
                name: name.to_string(),
                meta: None,
            },
        );
    }

    /// M59-B1：改为直接走 `GraphWriteStore::add_edge`，测试替身与生产写入路径
    /// 共用同一套去重与端点校验。原实现两个 `if let` 各判一次端点，
    /// 只有一端存在时会留下**半悬挂**的单向边——这种状态 redb 侧构造不出来，
    /// 用它搭出来的测试期望是无效基准。
    pub fn add_test_edge(
        &mut self,
        from: &str,
        to: &str,
        edge_type: crate::graph::EdgeType,
        field_path: Option<&str>,
    ) {
        let edge = Edge {
            from: from.to_string(),
            to: to.to_string(),
            edge_type,
            field_path: field_path.map(|s| s.to_string()),
            meta: None,
        };
        let _ = <Self as GraphWriteStore>::add_edge(self, edge);
    }

    /// 按 id 现查端点，拼出邻居视图。端点必然存在（建边时校验过、
    /// 删节点时连带删边），缺失只可能是内部不变量被破坏，故跳过而非 panic。
    fn neighbors_of(&self, edges: Option<&Vec<Edge>>, take_target: bool) -> Vec<GraphEdgeView> {
        let Some(edges) = edges else {
            return Vec::new();
        };
        edges
            .iter()
            .filter_map(|e| {
                let other = if take_target { &e.to } else { &e.from };
                self.nodes.get(other).map(|n| GraphEdgeView {
                    node: n.clone(),
                    edge: e.clone(),
                })
            })
            .collect()
    }
}

impl Default for MemoryGraphStore {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphReadStore for MemoryGraphStore {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        Ok(self.nodes.get(node_id).cloned())
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        // 契约：`None` 表示**节点不存在**，`Some(空邻居)` 表示节点存在但无边。
        // 两者对查询方含义不同，不能混。redb 侧按 node_indices 是否命中判定，
        // 这里按 nodes 是否命中判定，口径一致。
        if !self.nodes.contains_key(node_id) {
            return Ok(None);
        }
        Ok(Some(GraphNeighbors {
            outgoing: self.neighbors_of(self.outgoing.get(node_id), true),
            incoming: self.neighbors_of(self.incoming.get(node_id), false),
        }))
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        Ok(self.nodes.len())
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        Ok(self.outgoing.values().map(|v| v.len()).sum())
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        Ok(Box::new(self.nodes.values().cloned()))
    }
}

impl GraphWriteStore for MemoryGraphStore {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        // M59-B1：走共享的 meta 合并规则（占位节点不被降级、无 meta 的写入
        // 不抹掉既有 meta），与 redb 侧同一份实现。
        let merged = crate::graph_store::merge_upsert_meta(
            self.nodes.get(&node.id).and_then(|n| n.meta.clone()),
            node.meta.clone(),
        );
        self.nodes.insert(
            node.id.clone(),
            Node {
                meta: merged,
                ..node
            },
        );
        Ok(())
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        // 端点缺失则静默忽略（与 redb 的 add_edge_with_meta 一致）：
        // 扫描顺序会让引用先于定义出现，这里不是错误。
        if !self.nodes.contains_key(&edge.from) || !self.nodes.contains_key(&edge.to) {
            return Ok(());
        }
        // M59-B1：按 (from, to, type, field_path) 去重，与 redb 的 seen_edges 同口径。
        if !self
            .seen_edges
            .insert(crate::graph_store::edge_dedup_key(&edge))
        {
            return Ok(());
        }
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .push(edge.clone());
        self.incoming.entry(edge.to.clone()).or_default().push(edge);
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        let removed_ids: HashSet<&str> = node_ids.iter().map(String::as_str).collect();

        for id in node_ids {
            self.nodes.remove(id);
            self.outgoing.remove(id);
            self.incoming.remove(id);
        }

        for edges in self.outgoing.values_mut() {
            edges.retain(|e| !removed_ids.contains(e.to.as_str()));
        }

        for edges in self.incoming.values_mut() {
            edges.retain(|e| !removed_ids.contains(e.from.as_str()));
        }

        // M59-B1：去重键必须一起清，否则节点被删又重建后，原来那条边会被
        // 当成重复边丢弃——图里永久缺一条边。redb 侧对 seen_edges 做同样的 retain。
        self.seen_edges.retain(|(from, to, _, _)| {
            !removed_ids.contains(from.as_str()) && !removed_ids.contains(to.as_str())
        });
        Ok(())
    }
}

impl IndexStateStore for MemoryGraphStore {
    fn load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>> {
        Ok(self.file_states.clone())
    }

    fn persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport> {
        let indexed = commit.file_states.len();
        let dirty = commit.dirty_nodes.len();
        let deleted = commit.deleted_nodes.len();
        let unchanged = indexed.saturating_sub(dirty);
        self.file_states = commit.file_states;
        self.checkpoint = commit.checkpoint;
        Ok(IndexReport {
            indexed,
            unchanged,
            dirty,
            deleted,
        })
    }
}
