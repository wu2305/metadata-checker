#![cfg(feature = "cli-local")]

use metadata_checker::graph::{EdgeType, GraphDB, NodeType};
use metadata_checker::graph_redb_v2::{read_v2_layout, shadow_compare_v1_v2, write_v2_shadow};
use metadata_checker::scanner::scan_project;
use redb::{Database, TableDefinition};
use std::time::{SystemTime, UNIX_EPOCH};

const NODES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("nodes");
const V2_OUT_ADJ_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_out_adj");

fn fixture_graph(test_name: &str) -> anyhow::Result<(GraphDB, std::path::PathBuf)> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-redb-v2-{test_name}-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    let graph = GraphDB::open(&db_path)?;
    Ok((graph, db_path))
}

/// `GraphDB::open` 在 v2 shadow 有效时应优先 hydrate v2，v1 节点表损坏仍可打开。
#[test]
fn m53_open_prefers_v2_hydrate_when_v1_nodes_corrupted() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("open-v2-prefer")?;
    let node_count = graph.node_indices.len();
    drop(graph);

    {
        let db = Database::create(&db_path)?;
        let write_txn = db.begin_write()?;
        {
            let mut nodes_table = write_txn.open_table(NODES_TABLE)?;
            nodes_table.retain(|_, _| false)?;
        }
        write_txn.commit()?;
    }

    let reopened = GraphDB::open(&db_path)?;
    let layout =
        read_v2_layout(&db_path)?.expect("scan/persist must write readable v2 shadow layout");
    let report = shadow_compare_v1_v2(&reopened, &layout)?;
    assert!(
        report.equivalent,
        "v2 hydrate open must match shadow layout after v1 nodes wiped: {:?}",
        report
    );
    assert_eq!(
        reopened.node_indices.len(),
        node_count,
        "hydrated graph must preserve node count"
    );
    Ok(())
}

/// 小批量 dirty node meta 更新应走 v2 incremental patch 并保持 shadow 等价。
#[test]
fn m53_incremental_v2_patch_after_node_meta_update() -> anyhow::Result<()> {
    let (mut graph, db_path) = fixture_graph("incremental-patch")?;
    let node_id = graph
        .node_indices
        .keys()
        .next()
        .cloned()
        .expect("fixture graph must contain at least one node");
    let Some(idx) = graph.node_indices.get(&node_id).copied() else {
        anyhow::bail!("node index missing");
    };
    let node = graph
        .graph
        .node_weight(idx)
        .expect("node weight must exist")
        .clone();
    graph.add_node(
        node.id.clone(),
        node.node_type.clone(),
        node.path.clone(),
        node.name.clone(),
        Some(serde_json::json!({"m53_incremental_patch": true})),
    );
    let file_states = graph.load_file_states()?;
    graph.persist(&file_states)?;

    let persisted_layout =
        read_v2_layout(&db_path)?.expect("persist must keep v2 layout readable on disk");
    let report = shadow_compare_v1_v2(&graph, &persisted_layout)?;
    assert!(
        report.equivalent,
        "incremental v2 patch must remain equivalent to memory graph: {:?}",
        report
    );
    let node_count = graph.node_indices.len();
    drop(graph);

    let reopened = GraphDB::open(&db_path)?;
    let reopened_layout = read_v2_layout(&db_path)?.expect("reopened db must expose v2 layout");
    let reopened_report = shadow_compare_v1_v2(&reopened, &reopened_layout)?;
    assert!(
        reopened_report.equivalent,
        "reopened graph must match persisted v2 layout: {:?}",
        reopened_report
    );
    assert_eq!(
        reopened.node_indices.len(),
        node_count,
        "reopen after incremental patch must preserve node count"
    );
    Ok(())
}

/// v2 不可读时 open 仍 fallback v1。
#[test]
fn m53_open_falls_back_to_v1_when_v2_unreadable() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("open-v1-fallback")?;
    let node_count = graph.node_indices.len();
    drop(graph);
    let mut layout = read_v2_layout(&db_path)?.expect("v2 layout must exist");
    layout.meta.content_fingerprint ^= 1;
    write_v2_shadow(&db_path, &layout)?;

    let reopened = GraphDB::open(&db_path)?;
    assert_eq!(
        reopened.node_indices.len(),
        node_count,
        "fingerprint-miss must fallback to v1 open"
    );
    Ok(())
}

/// 新增节点不在既有 v2 layout 中时必须全量 rebuild，persist 不得失败。
#[test]
fn m53_persist_full_rebuild_when_dirty_nodes_include_new_ids() -> anyhow::Result<()> {
    let (mut graph, db_path) = fixture_graph("new-node-full-rebuild")?;
    let file_states = graph.load_file_states()?;
    let new_node_id = "comp:app/actions_test.spg|m53_new_component".to_string();
    graph.add_node(
        new_node_id.clone(),
        metadata_checker::graph::NodeType::Component,
        "app/actions_test.spg".to_string(),
        "m53_new_component".to_string(),
        None,
    );
    graph.persist(&file_states)?;

    let layout = read_v2_layout(&db_path)?.expect("persist must write readable v2 layout");
    assert!(
        layout.node_ids.iter().any(|id| id == &new_node_id),
        "full rebuild must include newly added node in v2 layout"
    );
    let report = shadow_compare_v1_v2(&graph, &layout)?;
    assert!(
        report.equivalent,
        "graph after new node persist must match v2 layout: {:?}",
        report
    );
    Ok(())
}

/// scanner 式 remove/re-add 同 ID 改边后，v2 必须重建拓扑并在 reopen 后保持新边。
#[test]
fn m53_scanner_style_readd_same_ids_rebuilds_v2_topology() -> anyhow::Result<()> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-redb-v2-readd-topology-{}-{nanos}.db",
        std::process::id(),
    ));
    let mut graph = GraphDB::open(&db_path)?;
    for id in ["node:a", "node:b", "node:c"] {
        graph.add_node(
            id.to_string(),
            NodeType::Component,
            "app/test.spg".to_string(),
            id.to_string(),
            None,
        );
    }
    graph.add_edge("node:a", "node:b", EdgeType::Contains, None);
    graph.persist(&std::collections::HashMap::new())?;

    graph.remove_nodes_by_ids(&["node:a".to_string()]);
    graph.add_node(
        "node:a".to_string(),
        NodeType::Component,
        "app/test.spg".to_string(),
        "node:a".to_string(),
        None,
    );
    graph.add_edge("node:a", "node:c", EdgeType::Contains, None);
    graph.persist(&std::collections::HashMap::new())?;

    let layout = read_v2_layout(&db_path)?.expect("updated v2 layout must remain readable");
    let report = shadow_compare_v1_v2(&graph, &layout)?;
    assert_eq!(report.equivalent, true, "updated v2 topology must match v1");
    drop(graph);

    let reopened = GraphDB::open(&db_path)?;
    let edge_targets: Vec<&str> = reopened
        .graph
        .raw_edges()
        .iter()
        .filter(|edge| edge.weight.from == "node:a")
        .map(|edge| edge.weight.to.as_str())
        .collect();
    assert_eq!(edge_targets, vec!["node:c"]);
    Ok(())
}

/// v2 blob 无法反序列化时，open 必须忽略 v2 并读取有效 v1。
#[test]
fn m53_open_falls_back_to_v1_when_v2_blob_is_malformed() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("malformed-v2-blob")?;
    let node_count = graph.node_indices.len();
    let edge_count = graph.graph.edge_count();
    drop(graph);

    let db = Database::create(&db_path)?;
    let write_txn = db.begin_write()?;
    {
        let mut table = write_txn.open_table(V2_OUT_ADJ_TABLE)?;
        table.insert("bundle", b"not-json".to_vec())?;
    }
    write_txn.commit()?;
    drop(db);

    assert_eq!(read_v2_layout(&db_path).is_err(), true);
    let reopened = GraphDB::open(&db_path)?;
    assert_eq!(reopened.node_indices.len(), node_count);
    assert_eq!(reopened.graph.edge_count(), edge_count);
    Ok(())
}

/// 邻接内容被篡改但仍结构合法时，完整 fingerprint 必须拒绝该 v2 layout。
#[test]
fn m53_v2_fingerprint_covers_adjacency_payload() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("adjacency-fingerprint")?;
    let node_count = graph.node_indices.len();
    drop(graph);
    let mut layout = read_v2_layout(&db_path)?.expect("v2 layout must exist");
    let entry = layout
        .out_adjacency
        .entries
        .first_mut()
        .expect("fixture graph must contain an edge");
    entry.adjacent_dense_id = (entry.adjacent_dense_id + 1) % node_count as u32;
    write_v2_shadow(&db_path, &layout)?;

    assert_eq!(read_v2_layout(&db_path)?.is_none(), true);
    let reopened = GraphDB::open(&db_path)?;
    assert_eq!(reopened.node_indices.len(), node_count);
    Ok(())
}

/// CSR offsets 损坏时必须在切片前被拒绝，并 fallback 到 v1。
#[test]
fn m53_open_falls_back_to_v1_when_v2_offsets_are_invalid() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("invalid-v2-offsets")?;
    let edge_count = graph.graph.edge_count();
    drop(graph);
    let mut layout = read_v2_layout(&db_path)?.expect("v2 layout must exist");
    layout.out_adjacency.offsets.pop();
    write_v2_shadow(&db_path, &layout)?;

    assert_eq!(read_v2_layout(&db_path).is_err(), true);
    let reopened = GraphDB::open(&db_path)?;
    assert_eq!(reopened.graph.edge_count(), edge_count);
    Ok(())
}
