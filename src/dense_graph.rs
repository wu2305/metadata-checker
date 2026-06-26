//! 稠密只读图快照。
//!
//! 该模块把通用 `GraphReadStore` 转换为适合高 fanout 遍历的 CSR 风格只读结构。
//! 它不替代事实图，也不持久化；任何时候都可以从底层图重新构建。

use crate::graph::{Edge, EdgeType, Node};
use crate::graph_store::{
    GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
};
use std::collections::HashMap;

/// 稠密节点 ID，用于数组下标访问。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DenseNodeId(u32);

impl DenseNodeId {
    /// 返回可用于 Vec 下标的 usize。
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// CSR 邻接项，保存邻接节点和边负载。
#[derive(Debug, Clone)]
struct DenseAdjacencyEntry {
    adjacent: DenseNodeId,
    edge: Edge,
}

/// 稠密只读图快照。
#[derive(Debug, Clone)]
pub struct DenseGraphSnapshot {
    node_ids: HashMap<String, DenseNodeId>,
    nodes: Vec<Node>,
    out_offsets: Vec<usize>,
    out_edges: Vec<DenseAdjacencyEntry>,
    in_offsets: Vec<usize>,
    in_edges: Vec<DenseAdjacencyEntry>,
    edge_types: Vec<EdgeType>,
    edge_payloads: Vec<Edge>,
}

impl DenseGraphSnapshot {
    /// 从只读图构建稠密快照。
    pub fn from_graph(graph: &dyn GraphReadStore) -> GraphStoreResult<Self> {
        let nodes: Vec<Node> = graph.iter_nodes()?.collect();
        if nodes.len() > u32::MAX as usize {
            return Err(GraphStoreError::InvalidArgument {
                message: format!("dense graph supports at most {} nodes", u32::MAX),
            });
        }

        let node_ids: HashMap<String, DenseNodeId> = nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| (node.id.clone(), DenseNodeId(idx as u32)))
            .collect();
        let mut outgoing_by_node: Vec<Vec<DenseAdjacencyEntry>> = vec![Vec::new(); nodes.len()];
        let mut incoming_by_node: Vec<Vec<DenseAdjacencyEntry>> = vec![Vec::new(); nodes.len()];
        let mut edge_types = Vec::new();
        let mut edge_payloads = Vec::new();

        for node in &nodes {
            let from_dense =
                node_ids
                    .get(&node.id)
                    .copied()
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: format!("dense id missing for node {}", node.id),
                    })?;
            let Some(neighbors) = graph.get_node_edges(&node.id)? else {
                continue;
            };
            for edge_view in neighbors.outgoing {
                let to_dense = node_ids.get(&edge_view.node.id).copied().ok_or_else(|| {
                    GraphStoreError::Corrupted {
                        reason: format!(
                            "edge target {} missing while building dense graph",
                            edge_view.node.id
                        ),
                    }
                })?;
                edge_types.push(edge_view.edge.edge_type.clone());
                edge_payloads.push(edge_view.edge.clone());
                outgoing_by_node[from_dense.index()].push(DenseAdjacencyEntry {
                    adjacent: to_dense,
                    edge: edge_view.edge,
                });
            }
        }

        for node in &nodes {
            let to_dense =
                node_ids
                    .get(&node.id)
                    .copied()
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: format!("dense id missing for node {}", node.id),
                    })?;
            let Some(neighbors) = graph.get_node_edges(&node.id)? else {
                continue;
            };
            for edge_view in neighbors.incoming {
                let from_dense = node_ids.get(&edge_view.node.id).copied().ok_or_else(|| {
                    GraphStoreError::Corrupted {
                        reason: format!(
                            "edge source {} missing while building dense graph",
                            edge_view.node.id
                        ),
                    }
                })?;
                incoming_by_node[to_dense.index()].push(DenseAdjacencyEntry {
                    adjacent: from_dense,
                    edge: edge_view.edge,
                });
            }
        }

        let (out_offsets, out_edges) = flatten_adjacency(outgoing_by_node);
        let (in_offsets, in_edges) = flatten_adjacency(incoming_by_node);

        Ok(Self {
            node_ids,
            nodes,
            out_offsets,
            out_edges,
            in_offsets,
            in_edges,
            edge_types,
            edge_payloads,
        })
    }

    /// 稠密节点数。
    pub fn dense_node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 稠密边数。
    pub fn dense_edge_count(&self) -> usize {
        debug_assert_eq!(
            self.edge_types.len(),
            self.edge_payloads.len(),
            "dense edge type column must mirror edge payload count"
        );
        self.edge_payloads.len()
    }

    /// 查询节点的稠密 ID。
    pub fn dense_id(&self, node_id: &str) -> Option<DenseNodeId> {
        self.node_ids.get(node_id).copied()
    }

    fn edge_view(&self, entry: &DenseAdjacencyEntry) -> GraphEdgeView {
        GraphEdgeView {
            node: self.nodes[entry.adjacent.index()].clone(),
            edge: entry.edge.clone(),
        }
    }
}

impl GraphReadStore for DenseGraphSnapshot {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        Ok(self
            .dense_id(node_id)
            .map(|dense_id| self.nodes[dense_id.index()].clone()))
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        let Some(dense_id) = self.dense_id(node_id) else {
            return Ok(None);
        };
        let idx = dense_id.index();

        let outgoing = self.out_edges[self.out_offsets[idx]..self.out_offsets[idx + 1]]
            .iter()
            .map(|entry| self.edge_view(entry))
            .collect();
        let incoming = self.in_edges[self.in_offsets[idx]..self.in_offsets[idx + 1]]
            .iter()
            .map(|entry| self.edge_view(entry))
            .collect();

        Ok(Some(GraphNeighbors { outgoing, incoming }))
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        Ok(self.nodes.len())
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        Ok(self.edge_payloads.len())
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        Ok(Box::new(self.nodes.clone().into_iter()))
    }
}

fn flatten_adjacency(
    adjacency_by_node: Vec<Vec<DenseAdjacencyEntry>>,
) -> (Vec<usize>, Vec<DenseAdjacencyEntry>) {
    let mut offsets = Vec::with_capacity(adjacency_by_node.len() + 1);
    let total_edges = adjacency_by_node.iter().map(Vec::len).sum();
    let mut edges = Vec::with_capacity(total_edges);
    offsets.push(0);
    for mut adjacency in adjacency_by_node {
        edges.append(&mut adjacency);
        offsets.push(edges.len());
    }
    (offsets, edges)
}
