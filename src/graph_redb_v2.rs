//! redb v2 图布局 shadow build。
//!
//! M52 只设计和验证 v2 存储布局，不切换默认读写路径。v1 仍是主路径，
//! v2 作为 shadow 写入并在测试中做等价校验；M53 再切入 hydrate / incremental persist。

use crate::graph::{Edge, EdgeType, FileState, Node};
use crate::graph_redb::GraphDB;
use anyhow::{Context, Result};
use petgraph::graph::DiGraph;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hasher;
use std::path::Path;
use twox_hash::XxHash64;

/// v2 shadow layout schema 版本。
pub const REDB_V2_SCHEMA_VERSION: &str = "m52-redb-v2-shadow";

const V2_META_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_meta");
const V2_NODE_IDS_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_node_ids");
const V2_NODE_META_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_node_meta");
const V2_OUT_ADJ_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_out_adj");
const V2_IN_ADJ_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_in_adj");
const V2_FIELD_PATHS_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_field_paths");
const V2_FILE_STATES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_file_states");

const V2_SINGLE_KEY: &str = "bundle";

/// v2 布局元信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RedbV2Meta {
    pub schema_version: String,
    pub node_count: u32,
    pub edge_count: u32,
    pub field_path_count: u32,
    pub file_state_count: u32,
    pub content_fingerprint: u64,
}

/// 稠密节点元数据（node id 单独存放在 `node_ids`）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RedbV2NodeRecord {
    pub node_type: crate::graph::NodeType,
    pub path: String,
    pub name: String,
    pub meta: Option<serde_json::Value>,
}

/// CSR 邻接切片。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RedbV2Adjacency {
    pub offsets: Vec<u32>,
    pub entries: Vec<RedbV2AdjacencyEntry>,
}

/// 邻接项，使用稠密 ID 和 field path 字典索引。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RedbV2AdjacencyEntry {
    pub adjacent_dense_id: u32,
    pub edge_type: EdgeType,
    pub field_path_id: Option<u32>,
    pub meta: Option<serde_json::Value>,
}

/// v2 shadow 布局的完整内存表示。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RedbV2Layout {
    pub meta: RedbV2Meta,
    pub node_ids: Vec<String>,
    pub nodes: Vec<RedbV2NodeRecord>,
    pub out_adjacency: RedbV2Adjacency,
    pub in_adjacency: RedbV2Adjacency,
    pub field_paths: Vec<String>,
    pub file_states: HashMap<String, FileState>,
}

/// shadow compare 结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedbV2ShadowReport {
    pub equivalent: bool,
    pub v1_node_count: usize,
    pub v2_node_count: usize,
    pub v1_edge_count: usize,
    pub v2_edge_count: usize,
    pub missing_nodes_in_v2: Vec<String>,
    pub extra_nodes_in_v2: Vec<String>,
    pub missing_edges_in_v2: usize,
    pub extra_edges_in_v2: usize,
    pub file_state_mismatch: bool,
}

impl RedbV2NodeRecord {
    fn from_node(node: &Node) -> Self {
        Self {
            node_type: node.node_type.clone(),
            path: node.path.clone(),
            name: node.name.clone(),
            meta: node.meta.clone(),
        }
    }

    fn to_node(&self, id: &str) -> Node {
        Node {
            id: id.to_string(),
            node_type: self.node_type.clone(),
            path: self.path.clone(),
            name: self.name.clone(),
            meta: self.meta.clone(),
        }
    }
}

/// 从 v1 内存图构建 v2 shadow layout。
pub fn build_v2_layout(
    graph: &GraphDB,
    file_states: &HashMap<String, FileState>,
) -> Result<RedbV2Layout> {
    let mut node_ids: Vec<String> = graph.node_indices.keys().cloned().collect();
    node_ids.sort();

    let dense_by_id: HashMap<&str, u32> = node_ids
        .iter()
        .enumerate()
        .map(|(idx, id)| (id.as_str(), idx as u32))
        .collect();
    let nodes: Vec<RedbV2NodeRecord> = node_ids
        .iter()
        .map(|id| {
            let idx = graph.node_indices[id];
            RedbV2NodeRecord::from_node(graph.graph.node_weight(idx).expect("node must exist"))
        })
        .collect();

    let mut field_path_ids: HashMap<String, u32> = HashMap::new();
    let mut field_paths: Vec<String> = Vec::new();
    let mut intern_field_path = |field_path: Option<&str>| -> Option<u32> {
        let Some(path) = field_path.filter(|value| !value.is_empty()) else {
            return None;
        };
        if let Some(id) = field_path_ids.get(path) {
            return Some(*id);
        }
        let id = field_paths.len() as u32;
        field_path_ids.insert(path.to_string(), id);
        field_paths.push(path.to_string());
        Some(id)
    };

    let mut outgoing_by_node = vec![Vec::<RedbV2AdjacencyEntry>::new(); node_ids.len()];
    let mut incoming_by_node = vec![Vec::<RedbV2AdjacencyEntry>::new(); node_ids.len()];
    for edge_ref in graph.graph.edge_references() {
        let edge = edge_ref.weight();
        let Some(&from_dense) = dense_by_id.get(edge.from.as_str()) else {
            continue;
        };
        let Some(&to_dense) = dense_by_id.get(edge.to.as_str()) else {
            continue;
        };
        let field_path_id = intern_field_path(edge.field_path.as_deref());
        outgoing_by_node[from_dense as usize].push(RedbV2AdjacencyEntry {
            adjacent_dense_id: to_dense,
            edge_type: edge.edge_type.clone(),
            field_path_id,
            meta: edge.meta.clone(),
        });
        incoming_by_node[to_dense as usize].push(RedbV2AdjacencyEntry {
            adjacent_dense_id: from_dense,
            edge_type: edge.edge_type.clone(),
            field_path_id,
            meta: edge.meta.clone(),
        });
    }

    let out_adjacency = flatten_adjacency(outgoing_by_node);
    let in_adjacency = flatten_adjacency(incoming_by_node);
    let edge_count = out_adjacency.entries.len() as u32;

    let mut layout = RedbV2Layout {
        meta: RedbV2Meta {
            schema_version: REDB_V2_SCHEMA_VERSION.to_string(),
            node_count: node_ids.len() as u32,
            edge_count,
            field_path_count: field_paths.len() as u32,
            file_state_count: file_states.len() as u32,
            content_fingerprint: 0,
        },
        node_ids,
        nodes,
        out_adjacency,
        in_adjacency,
        field_paths,
        file_states: file_states.clone(),
    };
    layout.meta.content_fingerprint = fingerprint_layout(&layout);
    Ok(layout)
}

/// 将 v2 shadow layout 写入已有 write transaction，与 v1 persist 共用单次 commit。
pub fn write_v2_shadow_tables(
    write_txn: &redb::WriteTransaction,
    layout: &RedbV2Layout,
) -> Result<()> {
    {
        let mut table = write_txn.open_table(V2_META_TABLE)?;
        let bytes = serde_json::to_vec(&layout.meta).context("serialize v2 meta")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_NODE_IDS_TABLE)?;
        let bytes = serde_json::to_vec(&layout.node_ids).context("serialize v2 node ids")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_NODE_META_TABLE)?;
        let bytes = serde_json::to_vec(&layout.nodes).context("serialize v2 node meta")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_OUT_ADJ_TABLE)?;
        let bytes =
            serde_json::to_vec(&layout.out_adjacency).context("serialize v2 out adjacency")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_IN_ADJ_TABLE)?;
        let bytes =
            serde_json::to_vec(&layout.in_adjacency).context("serialize v2 in adjacency")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_FIELD_PATHS_TABLE)?;
        let bytes = serde_json::to_vec(&layout.field_paths).context("serialize v2 field paths")?;
        table.insert(V2_SINGLE_KEY, bytes)?;
    }
    {
        let mut table = write_txn.open_table(V2_FILE_STATES_TABLE)?;
        table.retain(|_, _| false)?;
        for (path, state) in &layout.file_states {
            let bytes = serde_json::to_vec(state)
                .with_context(|| format!("serialize v2 file state {}", path))?;
            table.insert(path.as_str(), bytes)?;
        }
    }
    Ok(())
}

/// 将 v2 shadow layout 写入 redb，不改动 v1 表。
pub fn write_v2_shadow(db_path: &Path, layout: &RedbV2Layout) -> Result<()> {
    let db = Database::create(db_path)
        .with_context(|| format!("open redb for v2 shadow write at {:?}", db_path))?;
    let write_txn = db.begin_write()?;
    write_v2_shadow_tables(&write_txn, layout)?;
    write_txn.commit()?;
    Ok(())
}

/// 读取 v2 shadow layout；表缺失或 schema 不匹配时返回 `None`。
pub fn read_v2_layout(db_path: &Path) -> Result<Option<RedbV2Layout>> {
    if !db_path.exists() {
        return Ok(None);
    }
    let db = Database::open(db_path)
        .with_context(|| format!("open redb for v2 shadow read at {:?}", db_path))?;
    let read_txn = db.begin_read()?;
    let meta_table = match read_txn.open_table(V2_META_TABLE) {
        Ok(table) => table,
        Err(_) => return Ok(None),
    };
    let Some(meta_bytes) = meta_table
        .get(V2_SINGLE_KEY)?
        .map(|value| value.value().to_vec())
    else {
        return Ok(None);
    };
    let meta: RedbV2Meta = serde_json::from_slice(&meta_bytes).context("parse v2 meta")?;
    if meta.schema_version != REDB_V2_SCHEMA_VERSION {
        return Ok(None);
    }

    let node_ids = read_single_blob(&read_txn, V2_NODE_IDS_TABLE)?.unwrap_or_default();
    let nodes = read_single_blob(&read_txn, V2_NODE_META_TABLE)?.unwrap_or_default();
    let out_adjacency =
        read_single_blob(&read_txn, V2_OUT_ADJ_TABLE)?.context("v2 out adjacency missing")?;
    let in_adjacency =
        read_single_blob(&read_txn, V2_IN_ADJ_TABLE)?.context("v2 in adjacency missing")?;
    let field_paths =
        read_single_blob(&read_txn, V2_FIELD_PATHS_TABLE)?.context("v2 field paths missing")?;

    let mut file_states = HashMap::new();
    if let Ok(table) = read_txn.open_table(V2_FILE_STATES_TABLE) {
        for item in table.iter()? {
            let (key, value) = item?;
            if let Ok(state) = serde_json::from_slice::<FileState>(value.value().as_slice()) {
                file_states.insert(key.value().to_string(), state);
            }
        }
    }

    let layout = RedbV2Layout {
        meta,
        node_ids,
        nodes,
        out_adjacency,
        in_adjacency,
        field_paths,
        file_states,
    };
    if layout.meta.content_fingerprint != fingerprint_layout(&layout) {
        return Ok(None);
    }
    Ok(Some(layout))
}

/// 从 v2 layout hydrate 内存图，供 shadow compare 和 M53 预研。
pub fn hydrate_graph_from_v2(layout: &RedbV2Layout, db_path: &str) -> Result<GraphDB> {
    if layout.node_ids.len() != layout.nodes.len() {
        anyhow::bail!(
            "v2 layout node id/meta length mismatch: {} vs {}",
            layout.node_ids.len(),
            layout.nodes.len()
        );
    }

    let mut graph = DiGraph::new();
    let mut node_indices = HashMap::new();
    for (dense_id, node_record) in layout.nodes.iter().enumerate() {
        let id = layout
            .node_ids
            .get(dense_id)
            .context("v2 node id missing for dense index")?;
        let idx = graph.add_node(node_record.to_node(id));
        node_indices.insert(id.clone(), idx);
    }

    let edges = materialize_edges_from_out_adjacency(layout)?;
    let mut seen_edges = HashSet::new();
    for edge in edges {
        let key = edge_key(&edge);
        if !seen_edges.insert(key) {
            continue;
        }
        if let (Some(&from_idx), Some(&to_idx)) =
            (node_indices.get(&edge.from), node_indices.get(&edge.to))
        {
            graph.add_edge(from_idx, to_idx, edge);
        }
    }

    Ok(GraphDB::from_memory_graph(
        graph,
        node_indices,
        db_path.to_string(),
    ))
}

/// 对比 v1 内存图与 v2 layout hydrate 结果。
pub fn shadow_compare_v1_v2(v1: &GraphDB, layout: &RedbV2Layout) -> Result<RedbV2ShadowReport> {
    let v2 = hydrate_graph_from_v2(layout, &v1.db_path)?;
    Ok(compare_graph_dbs(
        v1,
        &v2,
        &v1.load_file_states()?,
        &layout.file_states,
    ))
}

fn compare_graph_dbs(
    v1: &GraphDB,
    v2: &GraphDB,
    v1_file_states: &HashMap<String, FileState>,
    v2_file_states: &HashMap<String, FileState>,
) -> RedbV2ShadowReport {
    let v1_nodes: HashSet<String> = v1.node_indices.keys().cloned().collect();
    let v2_nodes: HashSet<String> = v2.node_indices.keys().cloned().collect();
    let missing_nodes_in_v2: Vec<String> = v1_nodes.difference(&v2_nodes).cloned().collect();
    let extra_nodes_in_v2: Vec<String> = v2_nodes.difference(&v1_nodes).cloned().collect();

    let v1_edges = collect_edge_keys(v1);
    let v2_edges = collect_edge_keys(v2);
    let missing_edges_in_v2 = v1_edges.difference(&v2_edges).count();
    let extra_edges_in_v2 = v2_edges.difference(&v1_edges).count();

    let equivalent = missing_nodes_in_v2.is_empty()
        && extra_nodes_in_v2.is_empty()
        && missing_edges_in_v2 == 0
        && extra_edges_in_v2 == 0
        && v1_file_states == v2_file_states
        && nodes_semantically_equal(v1, v2, &v1_nodes);

    RedbV2ShadowReport {
        equivalent,
        v1_node_count: v1_nodes.len(),
        v2_node_count: v2_nodes.len(),
        v1_edge_count: v1_edges.len(),
        v2_edge_count: v2_edges.len(),
        missing_nodes_in_v2,
        extra_nodes_in_v2,
        missing_edges_in_v2,
        extra_edges_in_v2,
        file_state_mismatch: v1_file_states != v2_file_states,
    }
}

fn nodes_semantically_equal(v1: &GraphDB, v2: &GraphDB, node_ids: &HashSet<String>) -> bool {
    node_ids
        .iter()
        .all(|id| match (v1.get_node(id), v2.get_node(id)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        })
}

fn collect_edge_keys(graph: &GraphDB) -> HashSet<EdgeKey> {
    let mut keys = HashSet::new();
    for edge_ref in graph.graph.edge_references() {
        keys.insert(edge_key(edge_ref.weight()));
    }
    keys
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EdgeKey {
    from: String,
    to: String,
    edge_type: EdgeType,
    field_path: Option<String>,
    meta: Option<serde_json::Value>,
}

fn edge_key(edge: &Edge) -> EdgeKey {
    EdgeKey {
        from: edge.from.clone(),
        to: edge.to.clone(),
        edge_type: edge.edge_type.clone(),
        field_path: edge.field_path.clone(),
        meta: edge.meta.clone(),
    }
}

fn materialize_edges_from_out_adjacency(layout: &RedbV2Layout) -> Result<Vec<Edge>> {
    let mut edges = Vec::new();
    for dense_id in 0..layout.node_ids.len() {
        let from = layout.node_ids[dense_id].clone();
        let start = layout.out_adjacency.offsets[dense_id] as usize;
        let end = layout.out_adjacency.offsets[dense_id + 1] as usize;
        for entry in &layout.out_adjacency.entries[start..end] {
            let to = layout
                .node_ids
                .get(entry.adjacent_dense_id as usize)
                .context("v2 adjacent dense id out of range")?
                .clone();
            let field_path = entry
                .field_path_id
                .map(|id| {
                    layout
                        .field_paths
                        .get(id as usize)
                        .cloned()
                        .context("v2 field path id out of range")
                })
                .transpose()?;
            edges.push(Edge {
                from: from.clone(),
                to,
                edge_type: entry.edge_type.clone(),
                field_path,
                meta: entry.meta.clone(),
            });
        }
    }
    Ok(edges)
}

fn flatten_adjacency(adjacency_by_node: Vec<Vec<RedbV2AdjacencyEntry>>) -> RedbV2Adjacency {
    let mut offsets = Vec::with_capacity(adjacency_by_node.len() + 1);
    let total_edges = adjacency_by_node.iter().map(Vec::len).sum();
    let mut entries = Vec::with_capacity(total_edges);
    let mut cursor = 0u32;
    for mut adjacency in adjacency_by_node {
        offsets.push(cursor);
        for entry in adjacency.drain(..) {
            entries.push(entry);
            cursor += 1;
        }
    }
    offsets.push(cursor);
    RedbV2Adjacency { offsets, entries }
}

fn read_single_blob<T: for<'de> Deserialize<'de>>(
    read_txn: &redb::ReadTransaction,
    table_def: TableDefinition<&str, Vec<u8>>,
) -> Result<Option<T>> {
    let table = match read_txn.open_table(table_def) {
        Ok(table) => table,
        Err(_) => return Ok(None),
    };
    let Some(bytes) = table
        .get(V2_SINGLE_KEY)?
        .map(|value| value.value().to_vec())
    else {
        return Ok(None);
    };
    Ok(Some(
        serde_json::from_slice(&bytes).context("parse v2 blob")?,
    ))
}

fn fingerprint_layout(layout: &RedbV2Layout) -> u64 {
    let mut hasher = XxHash64::default();
    hasher.write(REDB_V2_SCHEMA_VERSION.as_bytes());
    hasher.write_u64(layout.node_ids.len() as u64);
    hasher.write_u64(layout.out_adjacency.entries.len() as u64);
    hasher.write_u64(layout.field_paths.len() as u64);
    hasher.write_u64(layout.file_states.len() as u64);
    for id in &layout.node_ids {
        hasher.write(id.as_bytes());
    }
    for path in &layout.field_paths {
        hasher.write(path.as_bytes());
    }
    let mut file_state_paths: Vec<&String> = layout.file_states.keys().collect();
    file_state_paths.sort();
    for path in file_state_paths {
        let state = &layout.file_states[path];
        hasher.write(path.as_bytes());
        hasher.write(state.file_hash.as_bytes());
    }
    hasher.finish()
}
