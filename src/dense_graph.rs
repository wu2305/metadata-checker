//! 稠密只读图快照。
//!
//! 该模块把通用 `GraphReadStore` 转换为适合高 fanout 遍历的 CSR 风格只读结构。
//! 它不替代事实图，也不持久化；任何时候都可以从底层图重新构建。

use crate::graph::{Edge, EdgeType, Node};
use crate::graph_store::{
    GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
};
use std::collections::{HashMap, HashSet};

/// 稠密快照沿用 GraphDB 的逻辑边去重键。
type DenseEdgeKey = (String, String, EdgeType, Option<String>);

/// 稠密节点 ID，用于数组下标访问。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DenseNodeId(u32);

impl DenseNodeId {
    /// 返回可用于 Vec 下标的 usize。
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// CSR 邻接切片，避免 path BFS 为每次扩展分配 `GraphNeighbors`。
pub(crate) struct DenseNeighborSlice<'a> {
    nodes: &'a [Node],
    edge_payloads: &'a [Edge],
    entries: &'a [DenseAdjacencyEntry],
}

impl<'a> DenseNeighborSlice<'a> {
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn get(&self, index: usize) -> (&'a Node, &'a Edge) {
        let entry = &self.entries[index];
        (
            &self.nodes[entry.adjacent.index()],
            &self.edge_payloads[entry.edge_idx as usize],
        )
    }
}

/// CSR 邻接项：邻接节点 + 共享 edge payload 下标。
#[derive(Debug, Clone)]
struct DenseAdjacencyEntry {
    adjacent: DenseNodeId,
    edge_idx: u32,
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
    /// 每条逻辑边只存一份 payload；CSR 邻接通过 `edge_idx` 引用。
    edge_payloads: Vec<Edge>,
}

impl DenseGraphSnapshot {
    /// 从只读图构建稠密快照。
    ///
    /// 单次遍历各节点出边：写入一份 edge payload，同时登记 outgoing / incoming 邻接索引。
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
        let mut edge_payloads = Vec::new();
        let mut edge_indices = HashMap::new();

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
                if edge_payloads.len() >= u32::MAX as usize {
                    return Err(GraphStoreError::InvalidArgument {
                        message: format!("dense graph supports at most {} edges", u32::MAX),
                    });
                }
                let edge_idx = edge_payloads.len() as u32;
                edge_indices.insert(dense_edge_key(&edge_view.edge), edge_idx);
                edge_payloads.push(edge_view.edge);
                outgoing_by_node[from_dense.index()].push(DenseAdjacencyEntry {
                    adjacent: to_dense,
                    edge_idx,
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
                let edge_idx = edge_indices
                    .get(&dense_edge_key(&edge_view.edge))
                    .copied()
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: format!(
                            "incoming edge {} -> {} missing outgoing payload",
                            edge_view.edge.from, edge_view.edge.to
                        ),
                    })?;
                incoming_by_node[to_dense.index()].push(DenseAdjacencyEntry {
                    adjacent: from_dense,
                    edge_idx,
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
            edge_payloads,
        })
    }

    /// 稠密节点数。
    pub fn dense_node_count(&self) -> usize {
        self.nodes.len()
    }

    /// 稠密边数。
    pub fn dense_edge_count(&self) -> usize {
        self.edge_payloads.len()
    }

    /// 查询节点的稠密 ID。
    pub fn dense_id(&self, node_id: &str) -> Option<DenseNodeId> {
        self.node_ids.get(node_id).copied()
    }

    /// 按 node id 返回只读节点引用。
    pub(crate) fn node_by_id(&self, node_id: &str) -> Option<&Node> {
        self.dense_id(node_id)
            .map(|dense_id| &self.nodes[dense_id.index()])
    }

    /// 尝试基于 dirty 节点进行稠密快照增量更新。
    ///
    /// 若节点集合稳定且 dirty 节点均在新旧图中存在，则重建受影响 source 的出边并补齐
    /// incoming 视图；否则返回 `Ok(None)`，交给 runtime 执行 full fallback。
    pub fn try_update_incremental(
        &self,
        candidate: &dyn GraphReadStore,
        dirty_node_ids: &[String],
    ) -> GraphStoreResult<Option<Self>> {
        if dirty_node_ids.is_empty() {
            return Ok(Some(self.clone()));
        }

        if candidate.node_count()? != self.dense_node_count() {
            return Ok(None);
        }

        for node in &self.nodes {
            if candidate.get_node(&node.id)?.is_none() {
                return Ok(None);
            }
        }

        let mut dirty_set: HashSet<String> = HashSet::new();
        let mut affected_sources: HashSet<String> = HashSet::new();
        let mut next_nodes = self.nodes.clone();
        let mut dirty_outgoing_cache: HashMap<String, Vec<GraphEdgeView>> = HashMap::new();

        for dirty_node_id in dirty_node_ids {
            if !dirty_set.insert(dirty_node_id.clone()) {
                continue;
            }

            let Some(current_node) = self.get_node(dirty_node_id)? else {
                return Ok(None);
            };
            let Some(candidate_node) = candidate.get_node(dirty_node_id)? else {
                return Ok(None);
            };

            let Some(dense_id) = self.dense_id(dirty_node_id) else {
                return Ok(None);
            };

            let current_neighbors =
                self.get_node_edges(dirty_node_id)?
                    .unwrap_or_else(|| GraphNeighbors {
                        outgoing: Vec::new(),
                        incoming: Vec::new(),
                    });
            let candidate_neighbors =
                candidate
                    .get_node_edges(dirty_node_id)?
                    .unwrap_or_else(|| GraphNeighbors {
                        outgoing: Vec::new(),
                        incoming: Vec::new(),
                    });

            if current_node != candidate_node {
                affected_sources.insert(dirty_node_id.clone());
            }
            next_nodes[dense_id.index()] = candidate_node;

            if !same_edge_multiset(&current_neighbors.outgoing, &candidate_neighbors.outgoing)? {
                affected_sources.insert(dirty_node_id.clone());
            }

            if !same_edge_multiset(&current_neighbors.incoming, &candidate_neighbors.incoming)? {
                let changed_sources = collect_changed_incoming_sources(
                    &current_neighbors.incoming,
                    &candidate_neighbors.incoming,
                )?;
                affected_sources.extend(changed_sources);
            }

            if !candidate_neighbors.outgoing.is_empty() {
                dirty_outgoing_cache.insert(dirty_node_id.clone(), candidate_neighbors.outgoing);
            }
        }

        if affected_sources.is_empty() {
            return Ok(Some(Self {
                node_ids: self.node_ids.clone(),
                nodes: next_nodes,
                out_offsets: self.out_offsets.clone(),
                out_edges: self.out_edges.clone(),
                in_offsets: self.in_offsets.clone(),
                in_edges: self.in_edges.clone(),
                edge_payloads: self.edge_payloads.clone(),
            }));
        }

        let mut outgoing_by_node: Vec<Vec<DenseAdjacencyEntry>> =
            Vec::with_capacity(self.nodes.len());
        outgoing_by_node.resize_with(self.nodes.len(), Vec::new);
        let mut incoming_by_node: Vec<Vec<DenseAdjacencyEntry>> =
            Vec::with_capacity(self.nodes.len());
        incoming_by_node.resize_with(self.nodes.len(), Vec::new);
        let mut edge_payloads: Vec<Edge> = Vec::new();
        let mut edge_indices: HashMap<DenseEdgeKey, u32> = HashMap::new();

        for source_node in &self.nodes {
            let Some(source_dense) = self.dense_id(&source_node.id) else {
                return Ok(None);
            };
            let outgoing = if affected_sources.contains(&source_node.id) {
                if let Some(cached) = dirty_outgoing_cache.remove(&source_node.id) {
                    cached
                } else {
                    candidate
                        .get_node_edges(&source_node.id)?
                        .map(|neighbors| neighbors.outgoing)
                        .unwrap_or_default()
                }
            } else {
                self.out_edges[self.out_offsets[source_dense.index()]
                    ..self.out_offsets[source_dense.index() + 1]]
                    .iter()
                    .map(|entry| self.edge_view(entry))
                    .collect()
            };

            for edge_view in outgoing {
                let Some(target_dense) = self.node_ids.get(&edge_view.edge.to).copied() else {
                    return Ok(None);
                };
                if edge_payloads.len() >= u32::MAX as usize {
                    return Err(GraphStoreError::InvalidArgument {
                        message: format!("dense graph supports at most {} edges", u32::MAX),
                    });
                }
                let edge_idx = edge_payloads.len() as u32;
                edge_payloads.push(edge_view.edge);
                edge_indices.insert(dense_edge_key(&edge_payloads[edge_idx as usize]), edge_idx);
                outgoing_by_node[source_dense.index()].push(DenseAdjacencyEntry {
                    adjacent: target_dense,
                    edge_idx,
                });
            }
        }

        for (source_id, outgoing) in outgoing_by_node.iter().enumerate() {
            let source_dense = DenseNodeId(source_id as u32);
            for outgoing_edge in outgoing {
                let edge = edge_payloads
                    .get(outgoing_edge.edge_idx as usize)
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: "dense snapshot edge payload missing".to_string(),
                    })?;
                let edge_idx = edge_indices
                    .get(&dense_edge_key(edge))
                    .copied()
                    .ok_or_else(|| GraphStoreError::Corrupted {
                        reason: format!(
                            "incoming edge {} -> {} missing outgoing payload",
                            edge.from, edge.to
                        ),
                    })?;
                incoming_by_node[outgoing_edge.adjacent.index()].push(DenseAdjacencyEntry {
                    adjacent: source_dense,
                    edge_idx,
                });
            }
        }

        let (out_offsets, out_edges) = flatten_adjacency(outgoing_by_node);
        let (in_offsets, in_edges) = flatten_adjacency(incoming_by_node);

        Ok(Some(Self {
            node_ids: self.node_ids.clone(),
            nodes: next_nodes,
            out_offsets,
            out_edges,
            in_offsets,
            in_edges,
            edge_payloads,
        }))
    }

    /// CSR 出边切片，供 path finder 直接遍历而不构造 `GraphNeighbors`。
    pub(crate) fn outgoing_neighbors(&self, node_id: &str) -> Option<DenseNeighborSlice<'_>> {
        let dense_id = self.dense_id(node_id)?;
        let idx = dense_id.index();
        Some(DenseNeighborSlice {
            nodes: &self.nodes,
            edge_payloads: &self.edge_payloads,
            entries: &self.out_edges[self.out_offsets[idx]..self.out_offsets[idx + 1]],
        })
    }

    /// CSR 入边切片，供 path finder 直接遍历。
    pub(crate) fn incoming_neighbors(&self, node_id: &str) -> Option<DenseNeighborSlice<'_>> {
        let dense_id = self.dense_id(node_id)?;
        let idx = dense_id.index();
        Some(DenseNeighborSlice {
            nodes: &self.nodes,
            edge_payloads: &self.edge_payloads,
            entries: &self.in_edges[self.in_offsets[idx]..self.in_offsets[idx + 1]],
        })
    }

    fn edge_view(&self, entry: &DenseAdjacencyEntry) -> GraphEdgeView {
        GraphEdgeView {
            node: self.nodes[entry.adjacent.index()].clone(),
            edge: self.edge_payloads[entry.edge_idx as usize].clone(),
        }
    }
}

/// 构造与 GraphDB 去重语义一致的边键。
fn dense_edge_key(edge: &Edge) -> DenseEdgeKey {
    (
        edge.from.clone(),
        edge.to.clone(),
        edge.edge_type.clone(),
        edge.field_path.clone(),
    )
}

/// 比较两组边是否具有相同的逻辑键与边内容多重集。
fn same_edge_multiset(lhs: &[GraphEdgeView], rhs: &[GraphEdgeView]) -> GraphStoreResult<bool> {
    let lhs_buckets = bucket_edges_by_key(lhs)?;
    let rhs_buckets = bucket_edges_by_key(rhs)?;
    if lhs_buckets.len() != rhs_buckets.len() {
        return Ok(false);
    }

    for (key, lhs_edges) in lhs_buckets {
        let Some(rhs_edges) = rhs_buckets.get(&key) else {
            return Ok(false);
        };
        if lhs_edges.len() != rhs_edges.len() {
            return Ok(false);
        }
        if !edge_group_equal(&lhs_edges, rhs_edges)? {
            return Ok(false);
        }
    }

    Ok(true)
}

/// 收集一个 dirty target 的入边变化所影响的 source 节点。
fn collect_changed_incoming_sources(
    old_incoming: &[GraphEdgeView],
    candidate_incoming: &[GraphEdgeView],
) -> GraphStoreResult<HashSet<String>> {
    let mut changed_sources = HashSet::new();
    let old_buckets = bucket_edges_by_key(old_incoming)?;
    let candidate_buckets = bucket_edges_by_key(candidate_incoming)?;
    let mut changed = false;

    for (key, old_edges) in &old_buckets {
        match candidate_buckets.get(key) {
            Some(candidate_edges) => {
                if old_edges.len() != candidate_edges.len()
                    || !edge_group_equal(old_edges, candidate_edges)?
                {
                    changed = true;
                }
            }
            None => {
                changed = true;
            }
        }
    }

    if !changed {
        for key in candidate_buckets.keys() {
            if !old_buckets.contains_key(key) {
                changed = true;
                break;
            }
        }
    }

    if !changed {
        return Ok(changed_sources);
    }

    for edges in old_buckets.values() {
        for edge in edges {
            changed_sources.insert(edge.from.clone());
        }
    }
    for edges in candidate_buckets.values() {
        for edge in edges {
            changed_sources.insert(edge.from.clone());
        }
    }

    Ok(changed_sources)
}

/// 按逻辑边键分桶，保留同键边的完整 payload 以比较 meta 变化。
fn bucket_edges_by_key(
    edges: &[GraphEdgeView],
) -> GraphStoreResult<HashMap<DenseEdgeKey, Vec<Edge>>> {
    let mut buckets: HashMap<DenseEdgeKey, Vec<Edge>> = HashMap::new();
    for edge_view in edges {
        buckets
            .entry(dense_edge_key(&edge_view.edge))
            .or_default()
            .push(edge_view.edge.clone());
    }
    Ok(buckets)
}

/// 比较同一逻辑边键分组中的完整边 payload。
fn edge_group_equal(lhs: &[Edge], rhs: &[Edge]) -> GraphStoreResult<bool> {
    if lhs.len() != rhs.len() {
        return Ok(false);
    }

    let mut matched = vec![false; rhs.len()];
    for lhs_edge in lhs {
        let mut found = false;
        for (idx, rhs_edge) in rhs.iter().enumerate() {
            if matched[idx] {
                continue;
            }
            if edge_content_equal(lhs_edge, rhs_edge) {
                matched[idx] = true;
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

/// 比较边的全部可观测字段。
fn edge_content_equal(lhs: &Edge, rhs: &Edge) -> bool {
    lhs.from == rhs.from
        && lhs.to == rhs.to
        && lhs.edge_type == rhs.edge_type
        && lhs.field_path == rhs.field_path
        && lhs.meta == rhs.meta
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

/// 只读 CSR 遍历适配器，供 path finder 直接消费稠密邻接。
pub(crate) struct DensePathTraversal<'a> {
    snapshot: &'a DenseGraphSnapshot,
    adjacency_hits: std::cell::Cell<usize>,
}

impl<'a> DensePathTraversal<'a> {
    pub(crate) fn new(snapshot: &'a DenseGraphSnapshot) -> Self {
        Self {
            snapshot,
            adjacency_hits: std::cell::Cell::new(0),
        }
    }

    pub(crate) fn adjacency_hits(&self) -> usize {
        self.adjacency_hits.get()
    }
}

impl GraphReadStore for DensePathTraversal<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        Ok(self.snapshot.node_by_id(node_id).cloned())
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        let outgoing: Vec<GraphEdgeView> = self
            .snapshot
            .outgoing_neighbors(node_id)
            .map(|slice| {
                self.adjacency_hits
                    .set(self.adjacency_hits.get() + slice.len());
                (0..slice.len())
                    .map(|idx| {
                        let (node, edge) = slice.get(idx);
                        GraphEdgeView {
                            node: node.clone(),
                            edge: edge.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        let incoming: Vec<GraphEdgeView> = self
            .snapshot
            .incoming_neighbors(node_id)
            .map(|slice| {
                self.adjacency_hits
                    .set(self.adjacency_hits.get() + slice.len());
                (0..slice.len())
                    .map(|idx| {
                        let (node, edge) = slice.get(idx);
                        GraphEdgeView {
                            node: node.clone(),
                            edge: edge.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        if outgoing.is_empty() && incoming.is_empty() {
            return Ok(None);
        }
        Ok(Some(GraphNeighbors { outgoing, incoming }))
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        self.snapshot.node_count()
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        self.snapshot.edge_count()
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        self.snapshot.iter_nodes()
    }
}
