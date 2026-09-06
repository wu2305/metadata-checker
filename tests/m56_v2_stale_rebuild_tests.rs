#![cfg(feature = "cli-local")]
//! M56 P1：v2 stale 累计阈值重建策略
//!
//! 覆盖：小 dirty 不重建（现状一致）、累计超阈值同事务重建 v2 置 Current、
//! 重建后 reopen 走 v2 hydrate 且内容与 v1 相等、提交失败保持 Stale 降级。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use metadata_checker::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
use metadata_checker::graph_redb::edge_storage_key;
use metadata_checker::graph_redb_v2::{
    V2ShadowState, read_v2_shadow_state, read_v2_stale_dirty_nodes,
};
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore, IndexCommit, IndexDelta};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

const TOTAL_NODES: usize = 1200;

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m56-stale-rebuild-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create test root");
    path
}

/// 链式图基线（node_i -Reads→ node_{i+1}），全量 persist 后保存 db 字节。
fn build_chain_baseline(name: &str) -> (PathBuf, PathBuf, Vec<String>, Vec<u8>) {
    let root = test_root(name);
    let db_path = root.join("graph.redb");
    let node_ids: Vec<String> = (0..TOTAL_NODES).map(|i| format!("node:{i}")).collect();
    {
        let mut graph = GraphDB::open(&db_path).expect("create graph");
        for id in &node_ids {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: id.clone(),
                    node_type: NodeType::Component,
                    path: "bench/chain.spg".to_string(),
                    name: id.clone(),
                    meta: None,
                },
            )
            .expect("add node");
        }
        for pair in node_ids.windows(2) {
            graph.add_edge(&pair[0], &pair[1], EdgeType::Reads, None);
        }
        graph
            .persist(&HashMap::new())
            .expect("baseline full persist");
    }
    let baseline_db = std::fs::read(&db_path).expect("read baseline db");
    (root, db_path, node_ids, baseline_db)
}

/// restore → open → remove+re-add 前 k 个节点并构造 delta commit。
fn prepare_delta_commit(
    db_path: &PathBuf,
    baseline_db: &[u8],
    node_ids: &[String],
    dirty_count: usize,
) -> (GraphDB, IndexCommit) {
    std::fs::write(db_path, baseline_db).expect("restore baseline");
    let graph = GraphDB::open(db_path).expect("open graph");
    mutate_and_build_commit(graph, &node_ids[..dirty_count])
}

/// 在已打开的 graph 上 remove+re-add 指定节点并构造 delta commit（不 restore）。
fn mutate_and_build_commit(mut graph: GraphDB, dirty_ids: &[String]) -> (GraphDB, IndexCommit) {
    let mut removed_edge_keys = std::collections::HashSet::new();
    let mut removed_nodes = Vec::new();
    let mut removed_edges: Vec<Edge> = Vec::new();
    for id in dirty_ids {
        let node = GraphReadStore::get_node(&graph, id)
            .expect("get node")
            .expect("node exists");
        removed_nodes.push(node);
        if let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("edges") {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                removed_edge_keys.insert(edge_storage_key(&view.edge));
                removed_edges.push(view.edge.clone());
            }
        }
    }

    graph.remove_nodes_by_ids(dirty_ids);
    for node in removed_nodes {
        GraphWriteStore::upsert_node(&mut graph, node).expect("re-add node");
    }
    let mut seen = std::collections::HashSet::new();
    let mut dirty_edges = Vec::new();
    for edge in removed_edges {
        graph.add_edge_with_meta(
            &edge.from,
            &edge.to,
            edge.edge_type.clone(),
            edge.field_path.clone(),
            edge.meta.clone(),
        );
        if seen.insert(edge_storage_key(&edge)) {
            dirty_edges.push(edge);
        }
    }

    let commit = IndexCommit {
        file_states: HashMap::new(),
        dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
        deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
        checkpoint: None,
        delta: Some(IndexDelta {
            dirty_edges,
            removed_edge_keys: removed_edge_keys.into_iter().collect(),
            changed_file_states: Vec::new(),
            removed_file_paths: Vec::new(),
        }),
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    (graph, commit)
}

fn node_id_set(graph: &GraphDB) -> std::collections::HashSet<String> {
    GraphReadStore::iter_nodes(graph)
        .expect("iter nodes")
        .map(|node| node.id)
        .collect()
}

/// 小 dirty 不触发重建：连续两次小 delta，Stale 保持、计数累计、reopen 走 v1。
#[test]
fn m56_v2_stale_rebuild_small_dirty_does_not_rebuild() {
    let (root, db_path, node_ids, baseline_db) = build_chain_baseline("small");

    let (mut graph, commit) = prepare_delta_commit(&db_path, &baseline_db, &node_ids, 10);
    let report = graph.persist_commit(&commit).expect("first delta");
    assert_eq!(report.v2_shadow_state, V2ShadowState::Stale);
    assert_eq!(read_v2_stale_dirty_nodes(&db_path).expect("counter"), 10);

    // 不 restore（restore 会把 Stale 计数清零），在当前 db 上继续累计
    let graph = GraphDB::open(&db_path).expect("open graph for second delta");
    let (mut graph, commit) = mutate_and_build_commit(graph, &node_ids[..10]);
    let report = graph.persist_commit(&commit).expect("second delta");
    assert_eq!(report.v2_shadow_state, V2ShadowState::Stale);
    assert_eq!(
        read_v2_stale_dirty_nodes(&db_path).expect("counter"),
        20,
        "small deltas must accumulate without rebuild"
    );
    assert_eq!(
        read_v2_shadow_state(&db_path).expect("shadow"),
        V2ShadowState::Stale
    );

    // reopen 走 v1 hydrate（Stale），内容正确
    let reopened = GraphDB::open(&db_path).expect("reopen stale db");
    assert_eq!(node_id_set(&reopened).len(), TOTAL_NODES);

    let _ = std::fs::remove_dir_all(root);
}

/// 累计超阈值：本轮同事务重建 v2 置 Current、计数清零，
/// reopen 走 v2 hydrate 且内容与 v1 相等。
#[test]
fn m56_v2_stale_rebuild_threshold_triggers_rebuild_and_current() {
    let (root, db_path, node_ids, baseline_db) = build_chain_baseline("threshold");

    // 先累计 30（低于 1024）
    let (mut graph, commit) = prepare_delta_commit(&db_path, &baseline_db, &node_ids, 30);
    let report = graph.persist_commit(&commit).expect("small delta");
    assert_eq!(report.v2_shadow_state, V2ShadowState::Stale);

    // 再 dirty 1100：累计 1130 >= 1024 → 触发重建
    // 注意不 restore：baseline restore 会把 Stale 状态/计数重置，
    // 必须在当前（Stale）db 上继续，保持累计语义。
    let graph = GraphDB::open(&db_path).expect("open graph");
    let (mut graph, commit) = mutate_and_build_commit(graph, &node_ids[..1100]);
    let report = graph.persist_commit(&commit).expect("threshold delta");

    assert_eq!(
        report.v2_shadow_state,
        V2ShadowState::Current,
        "accumulated stale over threshold must rebuild v2"
    );
    assert_eq!(read_v2_stale_dirty_nodes(&db_path).expect("counter"), 0);
    assert_eq!(
        read_v2_shadow_state(&db_path).expect("shadow"),
        V2ShadowState::Current
    );

    // reopen 走 v2 hydrate（Current），内容与 v1 hydrate 相等
    let reopened_v2 = GraphDB::open(&db_path).expect("reopen current db");
    let v2_nodes = node_id_set(&reopened_v2);
    assert_eq!(v2_nodes.len(), TOTAL_NODES);

    // 对照：Stale 的 v1 hydrate（把 shadow 标回 Stale 不影响 v1 内容）
    let v1_graph = {
        // 直接从 v1 表 hydrate：open_inner_v1 无公开入口，
        // 用「v2 置 Stale 后 open」走 v1 分支验证等价
        {
            let db = redb::Database::open(&db_path).expect("open redb");
            let txn = db.begin_write().expect("write txn");
            metadata_checker::graph_redb_v2::write_v2_shadow_state(&txn, V2ShadowState::Stale)
                .expect("mark stale for v1 comparison");
            txn.commit().expect("commit");
        }
        GraphDB::open(&db_path).expect("open as v1 hydrate")
    };
    assert_eq!(
        v2_nodes,
        node_id_set(&v1_graph),
        "v2 hydrate must equal v1 hydrate"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// 提交失败（外部持有 redb 阻塞）：保持 Stale 与计数，reopen 走 v1 图仍正确。
#[test]
fn m56_v2_stale_rebuild_failure_keeps_stale() {
    let (root, db_path, node_ids, baseline_db) = build_chain_baseline("failure");

    // 先一次小 delta 置 Stale + 计数 10
    let (mut graph, commit) = prepare_delta_commit(&db_path, &baseline_db, &node_ids, 10);
    graph.persist_commit(&commit).expect("first delta");
    assert_eq!(read_v2_stale_dirty_nodes(&db_path).expect("counter"), 10);

    // 第二次 delta（足以超阈值）被外部 redb 阻塞 → 整次提交失败。
    // 不 restore：在当前（Stale、计数 10）db 上构造，验证失败后状态不变。
    let graph = GraphDB::open(&db_path).expect("open graph for second delta");
    let (mut graph, commit) = mutate_and_build_commit(graph, &node_ids[..1100]);
    {
        let _blocker = redb::Database::create(&db_path).expect("hold redb blocker");
        let result = graph.persist_commit(&commit);
        assert!(result.is_err(), "persist should fail while redb is blocked");
    }

    // 失败降级：Stale 与累计计数保持，reopen 走 v1 且图正确
    assert_eq!(
        read_v2_shadow_state(&db_path).expect("shadow"),
        V2ShadowState::Stale
    );
    assert_eq!(read_v2_stale_dirty_nodes(&db_path).expect("counter"), 10);
    let reopened = GraphDB::open(&db_path).expect("reopen after failed commit");
    assert_eq!(node_id_set(&reopened).len(), TOTAL_NODES);

    let _ = std::fs::remove_dir_all(root);
}
