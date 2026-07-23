#![cfg(feature = "cli-local")]
//! M56 Task 14：v1 edge/file-state delta 与 v2 shadow 失效
//!
//! 覆盖：单 SPG topology change 只写受影响 node/edge/file-state keys、
//! v2 shadow 置 Stale 后 open 跳过 v2 走 v1 hydrate 且与完整 rebuild 相等、
//! 旧库无 shadow state 记录视为 Current（向后兼容）、
//! delta+checkpoint 单事务、显式 full rebuild 路径恢复 Current。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use metadata_checker::diff_refresh::{DiffRefreshCheckpoint, SourceCursor};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, IndexStateStore};
use metadata_checker::scanner::indexer::ProjectIndexer;
use redb::{ReadableDatabase, ReadableTable};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

const NODES_TABLE: redb::TableDefinition<&str, Vec<u8>> = redb::TableDefinition::new("nodes");
const EDGES_TABLE: redb::TableDefinition<&str, Vec<u8>> = redb::TableDefinition::new("edges");
const FILE_STATES_TABLE: redb::TableDefinition<&str, Vec<u8>> =
    redb::TableDefinition::new("file_states");
const V2_META_TABLE: redb::TableDefinition<&str, Vec<u8>> = redb::TableDefinition::new("v2_meta");

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m56-incr-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// 引用共享 tbl 的最小页面（两个组件 + input 绑定）。
fn page_spg(model_id: &str, extra_component: bool) -> String {
    let mut components = vec![
        serde_json::json!({"id": "input1", "type": "input", "submitField": format!("{model_id}.name")}),
        serde_json::json!({"id": "text1", "type": "text", "value": format!("${{{model_id}.name}}")}),
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

/// 构建 5 页 + 共享 tbl 的合成项目并完成初始 scan。
fn build_project(name: &str) -> (PathBuf, PathBuf) {
    let project_dir = test_root(name);
    std::fs::create_dir_all(project_dir.join("app")).expect("create app dir");
    std::fs::create_dir_all(project_dir.join("data")).expect("create data dir");
    for index in 0..5 {
        std::fs::write(
            project_dir.join(format!("app/page_{index:03}.spg")),
            page_spg(&format!("model_p{index}"), false),
        )
        .expect("write page");
    }
    std::fs::write(
        project_dir.join("data/table1.tbl"),
        r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#,
    )
    .expect("write tbl");
    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path).expect("initial scan");
    (project_dir, db_path)
}

/// 快照整张 redb 表为 owned map（句柄随函数返回释放，不阻塞后续写）。
fn snapshot_table(
    db_path: &Path,
    definition: redb::TableDefinition<&str, Vec<u8>>,
) -> BTreeMap<String, Vec<u8>> {
    let db = redb::Database::open(db_path).expect("open redb for snapshot");
    let txn = db.begin_read().expect("read txn");
    let table = txn.open_table(definition).expect("open table");
    let mut map = BTreeMap::new();
    for item in table.iter().expect("iter table") {
        let (key, value) = item.expect("table item");
        map.insert(key.value().to_string(), value.value().to_vec());
    }
    map
}

fn v2_shadow_state(db_path: &Path) -> Option<String> {
    let db = redb::Database::open(db_path).expect("open redb for v2 meta");
    let txn = db.begin_read().expect("read txn");
    let table = txn.open_table(V2_META_TABLE).expect("open v2 meta");
    let value = table.get("shadow_state").expect("get shadow_state")?;
    let state: String = serde_json::from_slice(value.value().as_slice()).expect("parse state");
    Some(state)
}

fn node_id_set(graph: &GraphDB) -> HashSet<String> {
    GraphReadStore::iter_nodes(graph)
        .expect("iter nodes")
        .map(|node| node.id)
        .collect()
}

fn edge_key_set(db_path: &Path) -> HashSet<String> {
    snapshot_table(db_path, EDGES_TABLE).into_keys().collect()
}

/// 单 SPG topology change：只更新受影响 node/edge/file-state keys，未触碰无关 keys。
#[test]
fn m56_incremental_persist_updates_only_affected_keys() {
    let (project_dir, db_path) = build_project("affected-keys");
    let edges_before = snapshot_table(&db_path, EDGES_TABLE);
    let states_before = snapshot_table(&db_path, FILE_STATES_TABLE);
    let nodes_before = snapshot_table(&db_path, NODES_TABLE);

    // 单 SPG topology change：page_000 增加一个组件
    std::fs::write(
        project_dir.join("app/page_000.spg"),
        page_spg("model_p0", true),
    )
    .expect("write topology change");
    ProjectIndexer::scan(&project_dir, &db_path).expect("delta scan");

    let edges_after = snapshot_table(&db_path, EDGES_TABLE);
    let states_after = snapshot_table(&db_path, FILE_STATES_TABLE);
    let nodes_after = snapshot_table(&db_path, NODES_TABLE);

    // edges：与 page_000 无关的 key 全部保留且字节一致
    for (key, bytes) in &edges_before {
        if key.contains("page_000.spg") {
            continue;
        }
        assert_eq!(
            edges_after.get(key),
            Some(bytes),
            "unrelated edge key must be untouched: {key}"
        );
    }
    // edges：page_000 旧 incident keys 被移除（旧页无 button1）
    assert!(
        edges_before.keys().any(|key| key.contains("page_000.spg")),
        "baseline should contain page_000 edges"
    );
    assert!(
        edges_after
            .keys()
            .any(|key| key.contains("page_000.spg|button1")),
        "delta should insert new button1 edges"
    );
    // file_states：只有 page_000 变化，其余字节一致
    assert_eq!(states_before.len(), states_after.len());
    for (path, bytes) in &states_before {
        if path == "app/page_000.spg" {
            assert_ne!(
                states_after.get(path),
                Some(bytes),
                "changed file state must be rewritten"
            );
        } else {
            assert_eq!(
                states_after.get(path),
                Some(bytes),
                "unrelated file state must be untouched: {path}"
            );
        }
    }
    // nodes：无关节点字节一致；page_000 新增 button1 节点
    for (id, bytes) in &nodes_before {
        if id.contains("page_000.spg") {
            continue;
        }
        assert_eq!(
            nodes_after.get(id),
            Some(bytes),
            "unrelated node must be untouched: {id}"
        );
    }
    assert!(nodes_after.contains_key("comp:app/page_000.spg|button1"));
    // v2 shadow 被标记 Stale
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("stale"));

    let _ = std::fs::remove_dir_all(project_dir);
}

/// reopen 等价：Stale 后 open 跳过 v2、从 v1 hydrate，结果与完整 rebuild 相等。
#[test]
fn m56_incremental_persist_reopen_hydrates_v1_and_matches_full_rebuild() {
    let (project_dir, db_path) = build_project("reopen-equiv");
    std::fs::write(
        project_dir.join("app/page_000.spg"),
        page_spg("model_p0", true),
    )
    .expect("write topology change");
    ProjectIndexer::scan(&project_dir, &db_path).expect("delta scan");
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("stale"));

    // Stale v2 仍含旧 layout（无 button1）；若错误地走 v2 hydrate 会丢失新节点
    let reopened = GraphDB::open(&db_path).expect("reopen after delta commit");
    let reopened_nodes = node_id_set(&reopened);
    assert!(
        reopened_nodes.contains("comp:app/page_000.spg|button1"),
        "reopen must hydrate from v1 (stale v2 lacks new node)"
    );

    // 与完整 rebuild 等价
    let rebuild_db = project_dir.join("rebuild.graphdb");
    ProjectIndexer::scan(&project_dir, &rebuild_db).expect("full rebuild scan");
    let rebuilt = GraphDB::open(&rebuild_db).expect("open full rebuild");
    assert_eq!(
        reopened_nodes,
        node_id_set(&rebuilt),
        "node sets must match"
    );
    assert_eq!(
        edge_key_set(&db_path),
        edge_key_set(&rebuild_db),
        "edge key sets must match"
    );

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 向后兼容：旧库无 shadow state 记录时视为 Current（维持 v2 优先 hydrate）。
#[test]
fn m56_incremental_persist_missing_shadow_state_treated_as_current() {
    let (project_dir, db_path) = build_project("legacy-current");
    // 初始全量构建 = 显式 full rebuild 路径：v2 Current
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("current"));

    // 模拟旧库：删除 shadow_state 记录
    {
        let db = redb::Database::open(&db_path).expect("open redb");
        let txn = db.begin_write().expect("write txn");
        {
            let mut table = txn.open_table(V2_META_TABLE).expect("open v2 meta");
            table.remove("shadow_state").expect("remove shadow_state");
        }
        txn.commit().expect("commit");
    }

    // 缺记录视为 Current：open 正常工作（v2 优先，内容与 v1 一致）
    let graph = GraphDB::open(&db_path).expect("open legacy db");
    assert!(
        node_id_set(&graph).contains("comp:app/page_000.spg|input1"),
        "legacy db should hydrate successfully"
    );

    let _ = std::fs::remove_dir_all(project_dir);
}

/// delta + checkpoint 单事务：reopen 同时看到新节点与新 checkpoint。
#[test]
fn m56_incremental_persist_checkpoint_and_delta_commit_atomically() {
    let (project_dir, db_path) = build_project("delta-checkpoint");
    std::fs::write(
        project_dir.join("app/page_000.spg"),
        page_spg("model_p0", true),
    )
    .expect("write topology change");

    let prepared = ProjectIndexer::prepare(&project_dir, &db_path).expect("prepare");
    let mut commit = prepared.commit;
    let checkpoint = DiffRefreshCheckpoint {
        active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    commit.checkpoint = Some(checkpoint.clone());
    let mut candidate = prepared.graph;
    IndexStateStore::persist_index(&mut candidate, commit).expect("persist delta+checkpoint");

    let reopened = GraphDB::open(&db_path).expect("reopen");
    assert!(
        node_id_set(&reopened).contains("comp:app/page_000.spg|button1"),
        "delta should be committed"
    );
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint"),
        Some(checkpoint),
        "checkpoint should be committed in the same txn"
    );
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("stale"));

    let _ = std::fs::remove_dir_all(project_dir);
}

/// 显式 full rebuild 路径：delta 提交后走全量 persist，v2 shadow 恢复 Current。
#[test]
fn m56_incremental_persist_explicit_full_path_restores_current() {
    let (project_dir, db_path) = build_project("restore-current");
    std::fs::write(
        project_dir.join("app/page_000.spg"),
        page_spg("model_p0", true),
    )
    .expect("write topology change");
    ProjectIndexer::scan(&project_dir, &db_path).expect("delta scan");
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("stale"));

    // 显式全量路径（delta=None）：checkpoint 强制 persist 推进，v2 重建并置 Current
    let mut graph = GraphDB::open(&db_path).expect("open for full persist");
    let states = graph.load_file_states().expect("load states");
    let checkpoint = DiffRefreshCheckpoint {
        active: SourceCursor::new(2000, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    graph
        .persist_with_checkpoint(&states, Some(&checkpoint))
        .expect("explicit full persist");
    assert_eq!(v2_shadow_state(&db_path).as_deref(), Some("current"));

    // Current 后 reopen 走 v2 优先，内容完整
    let reopened = GraphDB::open(&db_path).expect("reopen after full persist");
    assert!(node_id_set(&reopened).contains("comp:app/page_000.spg|button1"));

    let _ = std::fs::remove_dir_all(project_dir);
}
