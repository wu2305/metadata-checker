#![cfg(feature = "cli-local")]
//! M54 Task 6：候选图/read model 与 graph+watermark 原子提交测试
//!
//! 覆盖：prepare 不写盘、replacement read model 重建与 warm cache 保留、
//! checkpoint 与 graph 同一事务提交、migration 与故障语义。

use std::collections::HashSet;

use metadata_checker::diff_refresh::{DiffRefreshCheckpoint, SourceCursor};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, IndexCommit, IndexStateStore};
use metadata_checker::runtime::{
    GraphRuntime, RuntimeMode, RuntimeQueryCommand, RuntimeQueryRequest,
};
use metadata_checker::scanner::indexer::ProjectIndexer;

mod common;

/// 在 fixture spg 中注入一个无害顶层字段使其 dirty（保持合法 JSON）。
fn dirty_spg_file(project_dir: &std::path::Path, rel_path: &str, marker: &str) {
    let path = project_dir.join(rel_path);
    let text = std::fs::read_to_string(&path).expect("read spg");
    let mut value: serde_json::Value = serde_json::from_str(&text).expect("parse spg json");
    value
        .as_object_mut()
        .expect("spg root must be object")
        .insert("x-m54-marker".to_string(), serde_json::Value::from(marker));
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&value).expect("serialize spg"),
    )
    .expect("write dirty spg");
}

fn test_checkpoint(active_ms: u64) -> DiffRefreshCheckpoint {
    DiffRefreshCheckpoint {
        active: SourceCursor::new(active_ms, vec![format!("active:file-a:{active_ms}")]),
        deleted: SourceCursor::new(0, Vec::new()),
    }
}

/// prepare 返回完整 dirty/deleted IDs，但不持久化候选变更（不写盘）。
///
/// 注意：`GraphDB::open` 的 `ensure_redb_tables` 每次都会提交一个建表
/// write txn，redb 文件字节必然变化，因此「不写盘」以逻辑状态等价断言：
/// file states 与节点/边数在 prepare 前后保持一致。
#[test]
fn m54_diff_refresh_prepare_returns_full_ids_without_writing_db() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let (baseline_states, baseline_nodes, baseline_edges, baseline_deleted_node_ids) = {
        let graph = GraphDB::open(&db_path).expect("open baseline graphdb");
        let states = graph.load_file_states().expect("load file states");
        let deleted_node_ids = states
            .get("df_a.tbl")
            .expect("df_a.tbl file state")
            .node_ids
            .clone();
        (
            states,
            GraphReadStore::node_count(&graph).expect("node count"),
            GraphReadStore::edge_count(&graph).expect("edge count"),
            deleted_node_ids,
        )
    };
    assert!(
        !baseline_deleted_node_ids.is_empty(),
        "df_a.tbl should have indexed nodes"
    );

    // 一个 dirty（修改 spg）+ 一个 deleted（移除 tbl）
    dirty_spg_file(&temp_dir, "app/actions_test.spg", "prepare-1");
    std::fs::remove_file(temp_dir.join("df_a.tbl")).expect("remove df_a.tbl");

    let prepared = ProjectIndexer::prepare(&temp_dir, &db_path).expect("prepare");

    // prepare 不持久化候选变更：重开后 file states 与节点/边数保持基线
    let reopened = GraphDB::open(&db_path).expect("reopen graphdb after prepare");
    let states_after = reopened
        .load_file_states()
        .expect("load states after prepare");
    assert_eq!(
        baseline_states, states_after,
        "prepare must not persist candidate file states"
    );
    assert_eq!(
        GraphReadStore::node_count(&reopened).expect("node count after prepare"),
        baseline_nodes,
        "prepare must not persist candidate nodes"
    );
    assert_eq!(
        GraphReadStore::edge_count(&reopened).expect("edge count after prepare"),
        baseline_edges,
        "prepare must not persist candidate edges"
    );

    // 完整 dirty IDs：包含被修改页面的旧节点与新增节点
    assert!(
        prepared
            .dirty_node_ids
            .iter()
            .any(|id| id == "page:app/actions_test.spg"),
        "dirty ids should contain modified page node"
    );
    // 完整 deleted IDs：等于被删文件的全部旧节点
    let deleted_set: HashSet<&String> = prepared.deleted_node_ids.iter().collect();
    let expected_set: HashSet<&String> = baseline_deleted_node_ids.iter().collect();
    assert_eq!(deleted_set, expected_set);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// replacement read model 重建 dense/facts/PageDependencyIndex，
/// 受影响页面取并集，warm cache 只保留未受影响页面。
#[test]
fn m54_diff_refresh_runtime_replacement_preserves_unaffected_warm_cache() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let page_a = "page:app/actions_test.spg";
    let page_b = "page:app/action_category_contract.spg";
    let budget = "normal";

    // M55 起 PageDependencyIndex 覆盖 Model/Field 数据面闭包：fixture 各页
    // 共用本地模型 id `model1`（图中节点合并），page_b 必须换成无共享模型的
    // 独立页才能表达「未受影响页」语义。覆盖后增量重扫。
    std::fs::write(
        temp_dir.join("app/action_category_contract.spg"),
        r#"{
  "version": "4.19.7",
  "canvas": {"id": "canvas", "type": "canvas", "components": [
    {"id": "textB", "type": "text", "value": "independent page"}
  ]}
}"#,
    )
    .expect("overwrite page_b with independent spg");
    metadata_checker::scanner::scan_project(&temp_dir, &db_path).expect("rescan fixture project");

    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )
    .expect("long-lived runtime load");
    runtime
        .warm_page_logic_availability(page_a, budget)
        .expect("warm page_a");
    runtime
        .warm_page_logic_availability(page_b, budget)
        .expect("warm page_b");

    dirty_spg_file(&temp_dir, "app/actions_test.spg", "replacement-1");
    let prepared = ProjectIndexer::prepare(&temp_dir, &db_path).expect("prepare candidate");

    let replacement = runtime
        .prepare_replacement(&prepared.graph, &prepared.dirty_node_ids)
        .expect("prepare replacement read model");

    // 受影响页面：page_a 在内、page_b 不在
    assert!(
        replacement
            .invalidated_pages
            .iter()
            .any(|page| page == page_a),
        "invalidated pages should contain page_a"
    );
    assert!(
        !replacement
            .invalidated_pages
            .iter()
            .any(|page| page == page_b),
        "invalidated pages should not contain page_b"
    );
    // 候选 read model 已重建 dense/facts/dependency index
    assert!(replacement.read_model.dense_graph.is_some());
    assert_eq!(replacement.dense_snapshot_enabled, true);

    // 原子提交 graph（本测试不涉 checkpoint）后安装 replacement
    let mut candidate = prepared.graph;
    IndexStateStore::persist_index(&mut candidate, prepared.commit).expect("persist candidate");
    runtime.install_replacement(candidate, replacement);

    let read_model = runtime
        .read_model
        .as_ref()
        .expect("read model after install");
    assert!(
        !read_model.has_page_logic_availability(page_a, budget),
        "page_a warm cache should be invalidated"
    );
    assert!(
        read_model.has_page_logic_availability(page_b, budget),
        "page_b warm cache should be preserved"
    );

    // 安装后 page_a 冷查询仍正确
    let response = runtime
        .query(RuntimeQueryRequest {
            command: RuntimeQueryCommand::QueryPageLogic,
            target: page_a.to_string(),
            budget: budget.to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        })
        .expect("cold query page_a after install");
    assert!(
        !response.result.is_null(),
        "cold query page_a should return non-null result"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// migration：旧 graph 无 checkpoint 返回 None；
/// checkpoint-only commit（graph 无 dirty）也写入 META_TABLE 且图不变。
#[test]
fn m54_diff_refresh_checkpoint_migration_and_checkpoint_only_commit() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();

    // migration：旧 graph 无 checkpoint
    let graph = GraphDB::open(&db_path).expect("open graphdb");
    assert_eq!(
        graph
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint"),
        None,
        "legacy graph should have no diff refresh checkpoint"
    );
    let baseline_nodes = GraphReadStore::node_count(&graph).expect("node count");

    // checkpoint-only commit：无 dirty/deleted，仅推进 checkpoint
    let checkpoint = test_checkpoint(1000);
    let commit = IndexCommit {
        file_states: graph.load_file_states().expect("load file states"),
        dirty_nodes: Vec::new(),
        deleted_nodes: Vec::new(),
        checkpoint: Some(checkpoint.clone()),
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    let mut graph = graph;
    IndexStateStore::persist_index(&mut graph, commit).expect("persist checkpoint-only commit");

    // reopen 后同时看到 checkpoint 与未变化的图
    let reopened = GraphDB::open(&db_path).expect("reopen graphdb");
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint after commit"),
        Some(checkpoint)
    );
    assert_eq!(
        GraphReadStore::node_count(&reopened).expect("node count after commit"),
        baseline_nodes,
        "checkpoint-only commit must not change graph"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 故障语义：提交失败时 graph 与 checkpoint 均保持旧值；
/// 成功提交后 reopen 同时看到新图与新 checkpoint。
#[test]
fn m54_diff_refresh_commit_failure_keeps_old_graph_and_checkpoint() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let (baseline_nodes, baseline_page_a_hash) = {
        let graph = GraphDB::open(&db_path).expect("open baseline graphdb");
        let states = graph.load_file_states().expect("load baseline states");
        (
            GraphReadStore::node_count(&graph).expect("baseline node count"),
            states
                .get("app/actions_test.spg")
                .expect("page_a file state")
                .file_hash
                .clone(),
        )
    };

    dirty_spg_file(&temp_dir, "app/actions_test.spg", "fault-1");
    let prepared = ProjectIndexer::prepare(&temp_dir, &db_path).expect("prepare candidate");
    let mut commit = prepared.commit;
    commit.checkpoint = Some(test_checkpoint(2000));
    let mut candidate = prepared.graph;

    // 注入失败：外部持有 redb Database，使 persist 的 Database::create 失败
    {
        let _blocker = redb::Database::create(&db_path).expect("hold redb blocker");
        let result = IndexStateStore::persist_index(&mut candidate, commit.clone());
        assert!(result.is_err(), "persist should fail while redb is blocked");
    }

    // 失败后 graph 与 checkpoint 均保持旧值
    let graph_after_failure = GraphDB::open(&db_path).expect("open after failed commit");
    assert_eq!(
        graph_after_failure
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint after failure"),
        None,
        "failed commit must not advance checkpoint"
    );
    assert_eq!(
        GraphReadStore::node_count(&graph_after_failure).expect("node count after failure"),
        baseline_nodes,
        "failed commit must keep old graph"
    );
    drop(graph_after_failure);

    // 解除阻塞后同一 candidate 提交成功，reopen 同时看到新图与新 checkpoint
    IndexStateStore::persist_index(&mut candidate, commit).expect("persist after unblock");
    let reopened = GraphDB::open(&db_path).expect("reopen after successful commit");
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint after success"),
        Some(test_checkpoint(2000))
    );
    let states_after_success = reopened
        .load_file_states()
        .expect("load states after success");
    assert_ne!(
        states_after_success
            .get("app/actions_test.spg")
            .expect("page_a file state after success")
            .file_hash,
        baseline_page_a_hash,
        "successful commit should install new graph file states"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}
