#![cfg(feature = "cli-local")]
//! M56 Task 15：PersistReport 与规模曲线
//!
//! 断言小 topology dirty 走 delta（full_rewrite=false、v2=Stale）、
//! 显式 rebuild 走全量（full_rewrite=true、v2=Current），
//! 并输出 10/100/1000/10000 dirty nodes 的 report 快照（--nocapture 取数）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use metadata_checker::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
use metadata_checker::graph_redb::edge_storage_key;
use metadata_checker::graph_store::{
    GraphReadStore, GraphWriteStore, IndexCommit, IndexDelta, V2ShadowState,
};
use metadata_checker::scanner::indexer::ProjectIndexer;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m56-report-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn page_spg(model_id: &str, extra_component: bool) -> String {
    let mut components = vec![
        serde_json::json!({"id": "input1", "type": "input", "submitField": format!("{model_id}.name")}),
    ];
    if extra_component {
        components.push(serde_json::json!({"id": "button1", "type": "button"}));
    }
    serde_json::json!({
        "version": "4.19.7",
        "sources": [{"id": model_id, "modelType": "dwtable", "path": "data/table1.tbl"}],
        "canvas": {"id": "canvas", "type": "canvas", "components": components}
    })
    .to_string()
}

/// 小 topology dirty 走 delta：full_rewrite=false、v2_shadow_state=Stale。
#[test]
fn m56_persist_report_small_topology_dirty_uses_delta() {
    let project_dir = test_root("small-delta");
    std::fs::create_dir_all(project_dir.join("app")).expect("create app dir");
    std::fs::create_dir_all(project_dir.join("data")).expect("create data dir");
    std::fs::write(
        project_dir.join("app/page_a.spg"),
        page_spg("model_a", false),
    )
    .expect("write page_a");
    std::fs::write(
        project_dir.join("data/table1.tbl"),
        r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#,
    )
    .expect("write tbl");
    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("initial scan");

    // 单 SPG topology change
    std::fs::write(
        project_dir.join("app/page_a.spg"),
        page_spg("model_a", true),
    )
    .expect("write topology change");
    let prepared = ProjectIndexer::prepare(&project_dir, &db_path).expect("prepare");

    let mut candidate = prepared.graph;
    let report = candidate
        .persist_commit(&prepared.commit)
        .expect("persist commit");

    assert_eq!(
        report.full_rewrite, false,
        "delta path must not full-rewrite"
    );
    assert_eq!(report.v2_shadow_state, V2ShadowState::Stale);
    assert!(report.dirty_nodes > 0);
    assert!(report.dirty_edges > 0);
    assert_eq!(report.changed_file_states, 1, "one file state changed");
    assert!(report.bytes_written > 0);

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 显式 rebuild 走全量：full_rewrite=true、v2_shadow_state=Current。
#[test]
fn m56_persist_report_explicit_rebuild_uses_full_path() {
    let project_dir = test_root("explicit-full");
    std::fs::create_dir_all(project_dir.join("app")).expect("create app dir");
    std::fs::create_dir_all(project_dir.join("data")).expect("create data dir");
    std::fs::write(
        project_dir.join("app/page_a.spg"),
        page_spg("model_a", false),
    )
    .expect("write page_a");
    std::fs::write(
        project_dir.join("data/table1.tbl"),
        r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#,
    )
    .expect("write tbl");
    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("initial scan");

    // delta 提交先置 Stale
    std::fs::write(
        project_dir.join("app/page_a.spg"),
        page_spg("model_a", true),
    )
    .expect("write topology change");
    ProjectIndexer::scan(&project_dir, &db_path).expect("delta scan");

    // 显式全量路径（delta=None，checkpoint 强制推进）
    let mut graph = GraphDB::open(&db_path).expect("open graph");
    let states = graph.load_file_states().expect("load states");
    let checkpoint = metadata_checker::diff_refresh::DiffRefreshCheckpoint {
        active: metadata_checker::diff_refresh::SourceCursor::new(1000, Vec::new()),
        deleted: metadata_checker::diff_refresh::SourceCursor::new(0, Vec::new()),
    };
    let report = graph
        .persist_with_checkpoint(&states, Some(&checkpoint))
        .expect("explicit full persist");

    assert_eq!(
        report.full_rewrite, true,
        "explicit rebuild must be full path"
    );
    assert_eq!(report.v2_shadow_state, V2ShadowState::Current);
    assert!(
        report.bytes_written > 0,
        "full path rewrites edges/states/v2 blobs"
    );

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 规模曲线快照：dirty nodes 10/100/1000/10000 的 delta commit report。
///
/// 链式图（node_i -Reads→ node_{i+1}），baseline 为全量 Current；
/// 每档：restore → open → remove+re-add 前 k 个节点 → persist_commit。
/// 数字供 journal 采集（--nocapture），断言只保基本不变式。
#[test]
fn m56_persist_report_curve_snapshot() {
    const TOTAL_NODES: usize = 12_000;
    let root = test_root("curve");
    std::fs::create_dir_all(&root).expect("create curve root");
    let db_path = root.join("graph.redb");

    // 基线：链式图 + 全量 Current persist
    let node_ids: Vec<String> = (0..TOTAL_NODES).map(|i| format!("node:{i}")).collect();
    {
        let mut graph = GraphDB::open(&db_path).expect("create graph");
        for id in &node_ids {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: id.clone(),
                    node_type: NodeType::Component,
                    path: "bench/curve.spg".to_string(),
                    name: id.clone(),
                    meta: None,
                    origin_file: None,
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

    for &dirty_count in &[10_usize, 100, 1000, 10000] {
        std::fs::write(&db_path, &baseline_db).expect("restore baseline");
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        let dirty_ids: Vec<String> = node_ids[..dirty_count].to_vec();

        // apply 前：收集被删节点 incident edge keys 与节点/边快照
        let mut removed_edge_keys = HashSet::new();
        let mut removed_nodes = Vec::new();
        let mut removed_edges: Vec<Edge> = Vec::new();
        for id in &dirty_ids {
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

        // remove + re-add（模拟 scanner dirty 应用）
        graph.remove_nodes_by_ids(&dirty_ids);
        for node in removed_nodes {
            GraphWriteStore::upsert_node(&mut graph, node).expect("re-add node");
        }
        let mut seen_edges = HashSet::new();
        let mut dirty_edges = Vec::new();
        for edge in removed_edges {
            graph.add_edge_with_meta(
                &edge.from,
                &edge.to,
                edge.edge_type.clone(),
                edge.field_path.clone(),
                edge.meta.clone(),
            );
            if seen_edges.insert(edge_storage_key(&edge)) {
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
        let report = graph.persist_commit(&commit).expect("delta persist");

        assert_eq!(report.full_rewrite, false);
        // M56 P1 阈值策略：累计受影响节点 keys >= 1024 时本轮同事务重建 v2 置 Current；
        // 本测试每档 restore baseline（计数清零），累计即当档 dirty_count
        let expected_state = if dirty_count >= 1024 {
            V2ShadowState::Current
        } else {
            V2ShadowState::Stale
        };
        assert_eq!(report.v2_shadow_state, expected_state);
        assert_eq!(
            report.dirty_nodes, dirty_count,
            "remove+re-add 后 removed 被抵消，净写入 k 个节点 keys"
        );
        eprintln!(
            "[m56-curve] dirty_nodes={} report: dirty_nodes={} dirty_edges={} changed_file_states={} bytes_written={} commit_ms={} full_rewrite={} v2={:?}",
            dirty_count,
            report.dirty_nodes,
            report.dirty_edges,
            report.changed_file_states,
            report.bytes_written,
            report.commit_ms,
            report.full_rewrite,
            report.v2_shadow_state,
        );
    }

    // reopen hydrate 代价：Stale（v1） vs Current（v2）
    std::fs::write(&db_path, &baseline_db).expect("restore baseline");
    {
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        graph.remove_nodes_by_ids(&node_ids[..10].to_vec());
        let states = HashMap::new();
        let commit = IndexCommit {
            file_states: states,
            dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
            deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
            checkpoint: None,
            delta: Some(IndexDelta {
                dirty_edges: Vec::new(),
                removed_edge_keys: Vec::new(),
                changed_file_states: Vec::new(),
                removed_file_paths: Vec::new(),
            }),
            scanner_entries: Vec::new(),
            scanner_deleted_paths: Vec::new(),
        };
        graph.persist_commit(&commit).expect("mark stale");
    }
    let stale_start = std::time::Instant::now();
    let graph = GraphDB::open(&db_path).expect("open stale db");
    let stale_open_ms = stale_start.elapsed().as_millis();
    drop(graph);

    {
        let mut graph = GraphDB::open(&db_path).expect("open for full persist");
        graph.persist(&HashMap::new()).expect("restore current");
    }
    let current_start = std::time::Instant::now();
    let graph = GraphDB::open(&db_path).expect("open current db");
    let current_open_ms = current_start.elapsed().as_millis();
    drop(graph);
    eprintln!(
        "[m56-curve] reopen hydrate: stale_v1={}ms current_v2={}ms (total_nodes={})",
        stale_open_ms, current_open_ms, TOTAL_NODES
    );

    let _ = std::fs::remove_dir_all(root);
}
