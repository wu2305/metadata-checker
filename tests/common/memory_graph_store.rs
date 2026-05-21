use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_store::{
    GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreResult, GraphWriteStore, IndexCommit,
    IndexReport, IndexStateStore,
};
use std::collections::HashMap;

/// 内存图存储测试替身
///
/// M39.6：手工构造节点和边，实现 GraphReadStore / GraphWriteStore，
/// 验证 query 代码依赖 GraphReadStore 而不是 GraphDB / redb。
pub struct MemoryGraphStore {
    nodes: HashMap<String, Node>,
    outgoing: HashMap<String, Vec<(Node, Edge)>>,
    incoming: HashMap<String, Vec<(Node, Edge)>>,
}

impl MemoryGraphStore {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
        }
    }

    pub fn add_test_node(&mut self, id: &str, name: &str, node_type: NodeType, path: &str) {
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
        edge_type: EdgeType,
        field_path: Option<&str>,
    ) {
        let edge = Edge {
            from: from.to_string(),
            to: to.to_string(),
            edge_type: edge_type.clone(),
            field_path: field_path.map(|s| s.to_string()),
            meta: None,
        };
        // outgoing[from] contains the TARGET node (same as GraphDB semantics)
        if let Some(to_node) = self.nodes.get(to).cloned() {
            self.outgoing
                .entry(from.to_string())
                .or_default()
                .push((to_node, edge.clone()));
        }
        // incoming[to] contains the SOURCE node (same as GraphDB semantics)
        if let Some(from_node) = self.nodes.get(from).cloned() {
            self.incoming
                .entry(to.to_string())
                .or_default()
                .push((from_node, edge));
        }
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
        if let Some(from_node) = self.nodes.get(&edge.from).cloned() {
            self.outgoing
                .entry(edge.from.clone())
                .or_default()
                .push((from_node.clone(), edge.clone()));
            if let Some(to_node) = self.nodes.get(&edge.to).cloned() {
                self.incoming
                    .entry(edge.to.clone())
                    .or_default()
                    .push((to_node, edge));
            }
        }
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        for id in node_ids {
            self.nodes.remove(id);
            self.outgoing.remove(id);
            self.incoming.remove(id);
        }
        Ok(())
    }
}

impl IndexStateStore for MemoryGraphStore {
    fn load_file_states(
        &self,
    ) -> GraphStoreResult<std::collections::HashMap<String, metadata_checker::graph::FileState>>
    {
        Ok(std::collections::HashMap::new())
    }

    fn persist_index(&mut self, _commit: IndexCommit) -> GraphStoreResult<IndexReport> {
        Ok(IndexReport {
            indexed: 0,
            unchanged: 0,
            dirty: 0,
            deleted: 0,
        })
    }
}
