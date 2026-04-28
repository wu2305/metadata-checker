use anyhow::{Context, Result};
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 项目级图数据库模块
///
/// 使用 petgraph 构建内存中的有向图，并通过 redb 持久化到磁盘：
/// - 节点：Page、Component、Model、Field、Action
/// - 边：Reads、Writes、Triggers、Contains、EmbedsPage、OpensPage 等
///
/// 支持增量更新：基于文件 mtime+size+hash 检测变更，只处理脏文件。

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum NodeType {
    Page,
    Component,
    Model,
    Field,
    Action,
}

/// 图中边类型（关系语义）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EdgeType {
    Reads,
    Writes,
    Triggers,
    Contains,
    DataflowInput,
    ActionWrites,
    EmbedsPage,
    OpensPage,
    PassesParam,
    SetsParam,
    OutputsTo,
    DataflowInternal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 图节点
pub struct Node {
    pub id: String,
    pub node_type: NodeType,
    pub path: String,
    pub name: String,
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 图边
pub struct Edge {
    pub from: String,
    pub to: String,
    pub edge_type: EdgeType,
    pub field_path: Option<String>,
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// 文件状态（用于增量更新）
pub struct FileState {
    pub file_path: String,
    pub file_hash: String,
    pub mtime: u64,
    pub size: u64,
    pub node_ids: Vec<String>,
}

const NODES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("nodes");
const EDGES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("edges");
const FILE_STATES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("file_states");
const META_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("meta");

/// 图数据库（内存图 + redb 持久化）
pub struct GraphDB {
    pub graph: DiGraph<Node, Edge>,
    pub node_indices: HashMap<String, NodeIndex>,
    pub db_path: String,
    is_dirty: bool,
    seen_edges: HashSet<(String, String, EdgeType, Option<String>)>,
    dirty_nodes: HashSet<String>,
    removed_nodes: HashSet<String>,
}

type NodeEdgePair<'a> = (&'a Node, &'a Edge);
impl GraphDB {
    /// 打开或创建图数据库
    pub fn open(db_path: &Path) -> Result<Self> {
        let db = Database::create(db_path)
            .with_context(|| format!("Failed to create/open database at {:?}", db_path))?;

        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(NODES_TABLE)?;
            let _ = write_txn.open_table(EDGES_TABLE)?;
            let _ = write_txn.open_table(FILE_STATES_TABLE)?;
            let _ = write_txn.open_table(META_TABLE)?;
        }
        write_txn.commit()?;

        let mut graph = DiGraph::new();
        let mut node_indices = HashMap::new();

        let read_txn = db.begin_read()?;
        let nodes_table = read_txn.open_table(NODES_TABLE)?;
        for item in nodes_table.iter()? {
            let (key, value) = item?;
            let id = key.value();
            if let Ok(node) = serde_json::from_slice::<Node>(value.value().as_slice()) {
                let idx = graph.add_node(node);
                node_indices.insert(id.to_string(), idx);
            }
        }

        let mut seen_edges: HashSet<(String, String, EdgeType, Option<String>)> = HashSet::new();
        let edges_table = read_txn.open_table(EDGES_TABLE)?;
        for item in edges_table.iter()? {
            let (_, value) = item?;
            if let Ok(edge) = serde_json::from_slice::<Edge>(value.value().as_slice())
                && let (Some(&from_idx), Some(&to_idx)) =
                    (node_indices.get(&edge.from), node_indices.get(&edge.to))
            {
                graph.add_edge(from_idx, to_idx, edge.clone());
                seen_edges.insert((
                    edge.from.clone(),
                    edge.to.clone(),
                    edge.edge_type.clone(),
                    edge.field_path.clone(),
                ));
            }
        }

        Ok(GraphDB {
            graph,
            node_indices,
            db_path: db_path.to_string_lossy().to_string(),
            is_dirty: false,
            seen_edges,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
        })
    }

    /// 以只读模式打开图数据库（不创建表，不持有写锁）
    pub fn open_readonly(db_path: &Path) -> Result<Self> {
        if !db_path.exists() {
            anyhow::bail!("Graph database not found at {:?}", db_path);
        }
        let db = Database::open(db_path)
            .with_context(|| format!("Failed to open database at {:?}", db_path))?;

        let mut graph = DiGraph::new();
        let mut node_indices = HashMap::new();

        let read_txn = db.begin_read()?;
        let nodes_table = read_txn.open_table(NODES_TABLE)?;
        for item in nodes_table.iter()? {
            let (key, value) = item?;
            let id = key.value();
            if let Ok(node) = serde_json::from_slice::<Node>(value.value().as_slice()) {
                let idx = graph.add_node(node);
                node_indices.insert(id.to_string(), idx);
            }
        }

        let mut seen_edges: HashSet<(String, String, EdgeType, Option<String>)> = HashSet::new();
        let edges_table = read_txn.open_table(EDGES_TABLE)?;
        for item in edges_table.iter()? {
            let (_, value) = item?;
            if let Ok(edge) = serde_json::from_slice::<Edge>(value.value().as_slice())
                && let (Some(&from_idx), Some(&to_idx)) =
                    (node_indices.get(&edge.from), node_indices.get(&edge.to))
            {
                graph.add_edge(from_idx, to_idx, edge.clone());
                seen_edges.insert((
                    edge.from.clone(),
                    edge.to.clone(),
                    edge.edge_type.clone(),
                    edge.field_path.clone(),
                ));
            }
        }

        Ok(GraphDB {
            graph,
            node_indices,
            db_path: db_path.to_string_lossy().to_string(),
            is_dirty: false,
            seen_edges,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
        })
    }
    /// 添加节点（如已存在则更新）
    pub fn add_node(
        &mut self,
        id: String,
        node_type: NodeType,
        path: String,
        name: String,
        meta: Option<serde_json::Value>,
    ) -> NodeIndex {
        if let Some(&idx) = self.node_indices.get(&id) {
            // Update existing node meta if new meta is provided
            if let Some(existing) = self.graph.node_weight_mut(idx) {
                if meta.is_some() {
                    existing.meta = meta;
                }
                existing.node_type = node_type;
                existing.path = path;
                existing.name = name;
            }
            self.is_dirty = true;
            self.dirty_nodes.insert(id.clone());
            self.removed_nodes.remove(&id);
            return idx;
        }
        let node = Node {
            id: id.clone(),
            node_type,
            path,
            name,
            meta,
        };
        let idx = self.graph.add_node(node);
        self.is_dirty = true;
        self.dirty_nodes.insert(id.clone());
        self.removed_nodes.remove(&id);
        self.node_indices.insert(id, idx);
        idx
    }

    /// 添加有向边
    pub fn add_edge(
        &mut self,
        from: &str,
        to: &str,
        edge_type: EdgeType,
        field_path: Option<String>,
    ) {
        self.add_edge_with_meta(from, to, edge_type, field_path, None)
    }

    pub fn add_edge_with_meta(
        &mut self,
        from: &str,
        to: &str,
        edge_type: EdgeType,
        field_path: Option<String>,
        meta: Option<serde_json::Value>,
    ) {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(from), self.node_indices.get(to))
        {
            let key = (
                from.to_string(),
                to.to_string(),
                edge_type.clone(),
                field_path.clone(),
            );
            if !self.seen_edges.insert(key) {
                return;
            }
            let edge = Edge {
                from: from.to_string(),
                to: to.to_string(),
                edge_type,
                field_path,
                meta,
            };
            self.is_dirty = true;
            self.graph.add_edge(from_idx, to_idx, edge);
        }
    }

    /// 批量删除节点并重建索引，避免 petgraph 删除节点后 NodeIndex 失效
    pub fn remove_nodes_by_ids(&mut self, node_ids: &[String]) {
        if node_ids.is_empty() {
            return;
        }
        self.is_dirty = true;
        for id in node_ids {
            self.removed_nodes.insert(id.clone());
            self.dirty_nodes.remove(id);
        }

        let remove_ids: HashSet<&str> = node_ids.iter().map(String::as_str).collect();
        let mut new_graph = DiGraph::new();
        let mut new_indices = HashMap::new();
        let mut index_map = HashMap::new();

        for old_idx in self.graph.node_indices() {
            let Some(node) = self.graph.node_weight(old_idx) else {
                continue;
            };
            if remove_ids.contains(node.id.as_str()) {
                continue;
            }

            let new_idx = new_graph.add_node(node.clone());
            new_indices.insert(node.id.clone(), new_idx);
            index_map.insert(old_idx, new_idx);
        }

        for edge_ref in self.graph.edge_references() {
            let Some(&from_idx) = index_map.get(&edge_ref.source()) else {
                continue;
            };
            let Some(&to_idx) = index_map.get(&edge_ref.target()) else {
                continue;
            };
            new_graph.add_edge(from_idx, to_idx, edge_ref.weight().clone());
        }

        self.graph = new_graph;
        self.node_indices = new_indices;
        // Rebuild seen_edges to match the new graph
        self.seen_edges.clear();
        for edge_ref in self.graph.edge_references() {
            let edge = edge_ref.weight();
            self.seen_edges.insert((
                edge.from.clone(),
                edge.to.clone(),
                edge.edge_type.clone(),
                edge.field_path.clone(),
            ));
        }
    }

    pub fn persist(&mut self, file_states: &HashMap<String, FileState>) -> Result<()> {
        if !self.is_dirty {
            return Ok(());
        }
        let db = Database::create(&self.db_path)?;
        let write_txn = db.begin_write()?;

        // 增量更新节点：删除已移除节点，upsert 脏节点
        if !self.removed_nodes.is_empty() {
            let mut nodes_table = write_txn.open_table(NODES_TABLE)?;
            for id in &self.removed_nodes {
                nodes_table.remove(id.as_str())?;
            }
        }
        if !self.dirty_nodes.is_empty() {
            let mut nodes_table = write_txn.open_table(NODES_TABLE)?;
            for id in &self.dirty_nodes {
                if let Some(&idx) = self.node_indices.get(id)
                    && let Some(node) = self.graph.node_weight(idx)
                {
                    let bytes = serde_json::to_vec(node)
                        .with_context(|| format!("Failed to serialize node {}", id))?;
                    nodes_table.insert(id.as_str(), bytes)?;
                }
            }
        }

        // 边使用稳定 key 全量重写（数量通常远小于节点，且 key 需保持一致性）
        {
            let mut edges_table = write_txn.open_table(EDGES_TABLE)?;
            edges_table.retain(|_, _| false)?;
            for edge_ref in self.graph.edge_references() {
                let edge = edge_ref.weight();
                let type_str = serde_json::to_string(&edge.edge_type).unwrap_or_default();
                let key = format!(
                    "{}|{}|{}|{}",
                    edge.from,
                    edge.to,
                    type_str,
                    edge.field_path.as_deref().unwrap_or("")
                );
                let bytes = serde_json::to_vec(edge).with_context(|| "Failed to serialize edge")?;
                edges_table.insert(key.as_str(), bytes)?;
            }
        }

        // 文件状态全量重写（数据量小）
        {
            let mut states_table = write_txn.open_table(FILE_STATES_TABLE)?;
            states_table.retain(|_, _| false)?;
            for (path, state) in file_states {
                let bytes = serde_json::to_vec(state)
                    .with_context(|| format!("Failed to serialize file state {}", path))?;
                states_table.insert(path.as_str(), bytes)?;
            }
        }

        write_txn.commit()?;
        self.is_dirty = false;
        self.dirty_nodes.clear();
        self.removed_nodes.clear();
        Ok(())
    }

    /// 从 redb 加载图到内存
    pub fn load_file_states(&self) -> Result<HashMap<String, FileState>> {
        let db = Database::create(&self.db_path)?;
        let read_txn = db.begin_read()?;
        let table = read_txn.open_table(FILE_STATES_TABLE)?;
        let mut states = HashMap::new();
        for item in table.iter()? {
            let (key, value) = item?;
            if let Ok(state) = serde_json::from_slice::<FileState>(value.value().as_slice()) {
                states.insert(key.value().to_string(), state);
            }
        }
        Ok(states)
    }
    /// 查找读取指定模型的所有节点
    pub fn find_readers<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&model_idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(model_idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::Reads)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查找写入指定模型的所有节点
    pub fn find_writers<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&model_idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(model_idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::Writes | EdgeType::ActionWrites)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查找两个页面之间的共同依赖路径
    pub fn find_cross_relations<'a>(
        &'a self,
        page_a: &str,
        page_b: &str,
    ) -> Vec<Vec<NodeEdgePair<'a>>> {
        let mut paths = Vec::new();
        if let (Some(&a_idx), Some(&b_idx)) =
            (self.node_indices.get(page_a), self.node_indices.get(page_b))
        {
            let a_neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(a_idx, petgraph::Direction::Outgoing)
                .collect();
            let b_neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(b_idx, petgraph::Direction::Outgoing)
                .collect();

            for a_n in &a_neighbors {
                if b_neighbors.contains(a_n)
                    && let (Some(a_node), Some(mid_node)) =
                        (self.graph.node_weight(a_idx), self.graph.node_weight(*a_n))
                    && let Some(edge_a) = self.graph.edges_connecting(a_idx, *a_n).next()
                    && let Some(edge_b) = self.graph.edges_connecting(b_idx, *a_n).next()
                {
                    paths.push(vec![(a_node, edge_a.weight()), (mid_node, edge_b.weight())]);
                }
            }
        }
        paths
    }

    /// 查询指定模型的上游依赖（DataflowInput 边）
    pub fn find_upstream_dependencies<'a>(&'a self,
        model_id: &str,
    ) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::DataflowInput)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查询指定模型的下游输出（OutputsTo 边）
    pub fn find_downstream_outputs<'a>(
        &'a self,
        model_id: &str,
    ) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Outgoing)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::OutputsTo)
                    && let Some(node) = self.graph.node_weight(edge_ref.target())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 按 ID 获取节点
    pub fn get_node(&self, node_id: &str) -> Option<Node> {
        self.node_indices
            .get(node_id)
            .and_then(|&idx| self.graph.node_weight(idx))
            .cloned()
    }

    /// 按 ID 获取节点
    pub fn get_node_edges<'a>(
        &'a self,
        node_id: &str,
    ) -> Option<(Vec<NodeEdgePair<'a>>, Vec<NodeEdgePair<'a>>)> {
        let idx = self.node_indices.get(node_id)?;

        let outgoing: Vec<NodeEdgePair<'a>> = self
            .graph
            .edges_directed(*idx, petgraph::Direction::Outgoing)
            .filter_map(|e| self.graph.node_weight(e.target()).map(|n| (n, e.weight())))
            .collect();

        let incoming: Vec<NodeEdgePair<'a>> = self
            .graph
            .edges_directed(*idx, petgraph::Direction::Incoming)
            .filter_map(|e| self.graph.node_weight(e.source()).map(|n| (n, e.weight())))
            .collect();

        Some((outgoing, incoming))
    }
}
