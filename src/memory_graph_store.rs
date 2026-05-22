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
pub struct MemoryGraphStore {
    nodes: HashMap<String, Node>,
    outgoing: HashMap<String, Vec<(Node, Edge)>>,
    incoming: HashMap<String, Vec<(Node, Edge)>>,
    file_states: HashMap<String, FileState>,
}

impl MemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            file_states: HashMap::new(),
        }
    }

    pub fn add_test_node(&mut self, id: &str, name: &str, node_type: crate::graph::NodeType, path: &str) {
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
            edge_type: edge_type.clone(),
            field_path: field_path.map(|s| s.to_string()),
            meta: None,
        };
        if let Some(to_node) = self.nodes.get(to).cloned() {
            self.outgoing
                .entry(from.to_string())
                .or_default()
                .push((to_node, edge.clone()));
        }
        if let Some(from_node) = self.nodes.get(from).cloned() {
            self.incoming
                .entry(to.to_string())
                .or_default()
                .push((from_node, edge));
        }
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
        let out: Vec<GraphEdgeView> = self
            .outgoing
            .get(node_id)
            .map(|v| {
                v.iter()
                    .map(|(n, e)| GraphEdgeView {
                        node: n.clone(),
                        edge: e.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let inc: Vec<GraphEdgeView> = self
            .incoming
            .get(node_id)
            .map(|v| {
                v.iter()
                    .map(|(n, e)| GraphEdgeView {
                        node: n.clone(),
                        edge: e.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if out.is_empty() && inc.is_empty() && !self.nodes.contains_key(node_id) {
            return Ok(None);
        }
        Ok(Some(GraphNeighbors {
            outgoing: out,
            incoming: inc,
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
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        let Some(from_node) = self.nodes.get(&edge.from).cloned() else {
            return Ok(());
        };
        let Some(to_node) = self.nodes.get(&edge.to).cloned() else {
            return Ok(());
        };
        self.outgoing
            .entry(edge.from.clone())
            .or_default()
            .push((to_node, edge.clone()));
        self.incoming
            .entry(edge.to.clone())
            .or_default()
            .push((from_node, edge));
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
            edges.retain(|(_, e)| !removed_ids.contains(e.to.as_str()));
        }

        for edges in self.incoming.values_mut() {
            edges.retain(|(_, e)| !removed_ids.contains(e.from.as_str()));
        }
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
        Ok(IndexReport {
            indexed,
            unchanged,
            dirty,
            deleted,
        })
    }
}
