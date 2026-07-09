#![cfg(feature = "cli-local")]

use metadata_checker::runtime::{
    GraphRuntime, RuntimeMode, RuntimeQueryCommand, RuntimeQueryRequest,
};
use metadata_checker::scanner;

mod common;

/// M23 核心验收：同一个 GraphRuntime 连续执行两次 ExplainCondition，
/// 第二次不再重复加载 graphdb
#[test]
fn test_graph_runtime_reuses_loaded_graph_for_explain_condition() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();

    let mut runtime = GraphRuntime::load(&db_path).expect("GraphRuntime::load must succeed");
    assert_eq!(runtime.load_count, 1, "首次加载后 load_count 应为 1");
    assert!(
        runtime.dense_snapshot.is_none(),
        "default runtime load must not build dense snapshot"
    );
    assert_eq!(runtime.dense_snapshot_build_ms, 0);
    assert_eq!(runtime.availability_facts_build_ms, 0);
    assert_eq!(runtime.page_dependency_index_build_ms, 0);
    assert!(
        runtime.graph_load_ms > 0,
        "首次加载 graph_load_ms 应大于 0，实际 {}",
        runtime.graph_load_ms
    );

    let request = RuntimeQueryRequest {
        command: RuntimeQueryCommand::ExplainCondition,
        target: "comp:app/actions_test.spg|input1".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        depth: None,
        check_reload: false,
        page_scope: None,
    };

    // 第一次查询
    let resp1 = runtime
        .query(request.clone())
        .expect("第一次 query 必须成功");
    let result1 = resp1.result.clone();
    assert!(
        result1.get("summary").is_some(),
        "第一次查询结果必须有 summary"
    );
    assert!(
        result1.get("details").is_some(),
        "第一次查询结果必须有 details"
    );
    assert_eq!(
        result1.get("kind").and_then(|v| v.as_str()),
        Some("Explain"),
        "第一次查询结果 kind 必须是 Explain"
    );

    // 第二次查询（复用同一 runtime）
    let resp2 = runtime.query(request).expect("第二次 query 必须成功");
    let result2 = resp2.result;
    assert_eq!(
        result1.get("query_target"),
        result2.get("query_target"),
        "两次查询的 query_target 必须一致"
    );
    assert_eq!(
        result1
            .get("summary")
            .unwrap()
            .get("blocking_conditions_count"),
        result2
            .get("summary")
            .unwrap()
            .get("blocking_conditions_count"),
        "两次查询的 blocking_conditions_count 必须一致"
    );

    // 第二次 graph_load_ms 必须为 0（热查询复用）
    assert_eq!(
        resp2.timing.graph_load_ms, 0,
        "第二次查询 graph_load_ms 应为 0，因为 graph 已在内存中"
    );

    // load_count 仍为 1
    assert_eq!(runtime.load_count, 1, "连续两次查询后 load_count 仍为 1");
}

/// M23 验收：真实项目 input3  explain-condition
/// 通过 runtime 复用 graphdb，结果仍包含 model22.phoneNumber / fact_qwSidebar.phoneNumber
#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_runtime_input3_explain_condition_reuses_graph() {
    let project_dir = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
    let db_path = std::path::PathBuf::from("/tmp/m23_runtime_test.graphdb");
    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(std::path::Path::new(project_dir), &db_path)
        .expect("scan real project must succeed");

    let mut runtime = GraphRuntime::load(&db_path).expect("load real project graph must succeed");
    assert_eq!(runtime.load_count, 1);
    assert!(
        runtime.graph_load_ms > 0,
        "真实项目首次加载 graph_load_ms 应大于 0，实际 {}",
        runtime.graph_load_ms
    );

    let request = RuntimeQueryRequest {
        command: RuntimeQueryCommand::ExplainCondition,
        target: "comp:app/销售.app/销售/合同协议.spg|input3".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: Some("writer".to_string()),
        depth: None,
        check_reload: false,
        page_scope: None,
    };

    // 第一次查询
    let resp1 = runtime
        .query(request.clone())
        .expect("第一次 query 必须成功");
    let result1 = resp1.result;

    // 第二次查询（复用 graph）
    let resp2 = runtime.query(request).expect("第二次 query 必须成功");
    let result2 = resp2.result;

    // 两次结果关键字段一致
    assert_eq!(
        result1.get("kind").and_then(|v| v.as_str()),
        Some("Explain"),
        "kind must be Explain"
    );
    assert_eq!(
        result1.get("query_target"),
        result2.get("query_target"),
        "两次查询 query_target 必须一致"
    );

    // compact 模式隐藏 primary_path，writer_facts 保留短证据
    let writer_paths = result1
        .get("details")
        .and_then(|d| d.get("answer_facts"))
        .and_then(|f| f.get("writer_facts"))
        .and_then(|f| f.get("paths"))
        .and_then(|v| v.as_array())
        .expect("writer_facts.paths must be array");
    assert!(
        !writer_paths.is_empty(),
        "writer_facts.paths must not be empty"
    );

    // 必须包含 model22.phoneNumber
    let has_model22 = writer_paths.iter().any(|p| {
        p.get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("model22.phoneNumber"))
            .unwrap_or(false)
    });
    assert!(has_model22, "writer_facts must contain model22.phoneNumber");

    // 必须包含 fact_qwSidebar.phoneNumber
    let has_fact = writer_paths.iter().any(|p| {
        p.get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("fact_qwSidebar.phoneNumber"))
            .unwrap_or(false)
    });
    assert!(
        has_fact,
        "writer_facts must contain fact_qwSidebar.phoneNumber"
    );

    // 必须包含 action1 或 action4
    let has_writer = writer_paths.iter().any(|p| {
        p.get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("action1") || s.contains("action4"))
            .unwrap_or(false)
    });
    assert!(has_writer, "writer_facts must contain action1 or action4");

    // 第二次 graph_load_ms == 0（热查询复用）
    assert_eq!(
        resp2.timing.graph_load_ms, 0,
        "第二次查询 graph_load_ms 必须为 0"
    );

    // load_count 仍为 1
    assert_eq!(runtime.load_count, 1, "两次查询后 load_count 仍为 1");
}

/// M25 验收：status() 返回正确的节点和边数量
#[test]
fn test_runtime_status_returns_counts() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let runtime = GraphRuntime::load(&db_path).expect("load must succeed");

    let status = runtime.status();
    assert_eq!(status.load_count, 1);
    assert_eq!(status.reload_count, 0);
    assert!(status.node_count > 0, "node_count must be > 0");
    assert!(status.graph_db_path.ends_with(".metadata-checker.graphdb"));
    assert!(status.last_reload_error.is_none());
}

/// M25 验收：graphdb 未变更时 reload_if_changed 返回 false
#[test]
fn test_runtime_reload_if_changed_no_change() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");

    let result = runtime
        .reload_if_changed()
        .expect("reload_if_changed must succeed");
    assert_eq!(
        result,
        metadata_checker::runtime::ReloadResult::Unchanged,
        "未变更时应返回 Unchanged"
    );
    assert_eq!(runtime.reload_count, 0, "未变更时 reload_count 仍为 0");
}

/// M25 验收：graphdb 替换后 reload_if_changed 返回 true
#[test]
fn test_runtime_reload_if_changed_after_replace() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");
    let old_node_count = runtime.status().node_count;

    // 重建 graphdb（模拟外部更新）
    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(&temp_dir, &db_path).expect("re-scan must succeed");

    let result = runtime
        .reload_if_changed()
        .expect("reload_if_changed must succeed");
    assert_eq!(
        result,
        metadata_checker::runtime::ReloadResult::Reloaded,
        "文件变更后应返回 Reloaded"
    );
    assert_eq!(runtime.reload_count, 1, "reload_count 应增加到 1");
    assert_eq!(
        runtime.status().node_count,
        old_node_count,
        "reload 后 node_count 应一致"
    );
    assert!(
        runtime.dense_snapshot.is_none(),
        "default runtime reload must not build dense snapshot"
    );
    assert!(runtime.last_reload_error.is_none());
}

#[test]
fn test_graph_runtime_can_opt_into_dense_snapshot() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime =
        GraphRuntime::load_with_project_dir_and_dense_snapshot(&db_path, Some(&temp_dir))
            .expect("opt-in dense runtime load must succeed");
    let dense_snapshot = runtime
        .dense_snapshot
        .as_ref()
        .expect("opt-in runtime load must build dense snapshot");
    assert_eq!(
        dense_snapshot.dense_node_count(),
        runtime.status().node_count
    );
    assert_eq!(
        dense_snapshot.dense_edge_count(),
        runtime.status().edge_count
    );

    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(&temp_dir, &db_path).expect("re-scan must succeed");
    let result = runtime
        .reload_if_changed()
        .expect("opt-in runtime reload_if_changed must succeed");
    assert_eq!(result, metadata_checker::runtime::ReloadResult::Reloaded);
    assert!(
        runtime.dense_snapshot.is_some(),
        "opt-in runtime reload must rebuild dense snapshot"
    );
}

#[test]
fn test_graph_runtime_long_lived_mode_builds_read_model() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )
    .expect("long-lived runtime load must succeed");

    let read_model = runtime
        .read_model
        .as_ref()
        .expect("long-lived runtime must build read model during init");
    let dense_graph = read_model
        .dense_graph
        .as_ref()
        .expect("long-lived runtime must build dense snapshot during init");
    assert_eq!(dense_graph.dense_node_count(), runtime.status().node_count);
    assert_eq!(dense_graph.dense_edge_count(), runtime.status().edge_count);
    assert!(
        runtime.read_model_build_ms > 0,
        "long-lived runtime should record read model build cost"
    );
    assert_eq!(
        runtime.dense_snapshot_build_ms
            + runtime.availability_facts_build_ms
            + runtime.page_dependency_index_build_ms,
        runtime.read_model_build_ms,
        "read model build ms should equal sum of three stage timings"
    );

    let previous_read_model = std::sync::Arc::clone(read_model);
    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(&temp_dir, &db_path).expect("re-scan must succeed");
    let result = runtime
        .reload_if_changed()
        .expect("long-lived runtime reload_if_changed must succeed");
    assert_eq!(result, metadata_checker::runtime::ReloadResult::Reloaded);
    let reloaded_read_model = runtime
        .read_model
        .as_ref()
        .expect("long-lived runtime reload must rebuild read model");
    assert!(
        !std::sync::Arc::ptr_eq(&previous_read_model, reloaded_read_model),
        "reload should replace read model atomically"
    );
}

#[test]
fn test_graph_runtime_one_shot_mode_does_not_build_read_model() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::OneShot,
    )
    .expect("one-shot runtime load must succeed");

    assert!(
        runtime.read_model.is_none(),
        "one-shot runtime must avoid init-heavy read model"
    );
    assert_eq!(runtime.read_model_build_ms, 0);
    assert_eq!(runtime.availability_facts_build_ms, 0);
    assert_eq!(runtime.page_dependency_index_build_ms, 0);
}

#[test]
fn test_graph_runtime_warms_page_logic_availability_cache() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let mut baseline_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::OneShot,
    )
    .expect("baseline runtime load must succeed");
    let mut long_lived_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )
    .expect("long-lived runtime load must succeed");

    let request = RuntimeQueryRequest {
        command: RuntimeQueryCommand::QueryPageLogic,
        target: "page:app/actions_test.spg".to_string(),
        budget: "normal".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };

    let baseline = baseline_runtime
        .query(request.clone())
        .expect("baseline query_page_logic must succeed");
    let warm_ms = long_lived_runtime
        .warm_page_logic_availability("page:app/actions_test.spg", "normal")
        .expect("warm page logic availability must succeed");
    assert!(warm_ms > 0, "warm should record non-zero init cost");
    assert!(
        long_lived_runtime
            .read_model
            .as_ref()
            .expect("long-lived runtime should have read model")
            .has_page_logic_availability("page:app/actions_test.spg", "normal"),
        "read model should contain warmed page availability"
    );

    let warmed = long_lived_runtime
        .query(request)
        .expect("warmed query_page_logic must succeed");
    assert_eq!(
        warmed.result, baseline.result,
        "warmed availability cache must preserve runtime query output"
    );
}

#[test]
fn test_graph_runtime_page_logic_profiled_collects_b1_b2_counters() {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let mut long_lived_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )
    .expect("long-lived runtime load must succeed");
    let page_id = "page:app/actions_test.spg";
    let budget = "normal";

    let (_cold, cold_profile) = long_lived_runtime
        .query_page_logic_profiled(page_id, budget)
        .expect("cold profiled query_page_logic must succeed");
    assert_eq!(
        cold_profile.counter("path_dense_graph_used"),
        1,
        "long-lived cold query must use dense snapshot for path_summary"
    );
    assert!(
        cold_profile.counter("path_dense_adjacency_hits") > 0,
        "long-lived cold query must traverse dense adjacency during path search"
    );

    long_lived_runtime
        .warm_page_logic_availability(page_id, budget)
        .expect("warm page logic availability must succeed");

    let (_response, profile) = long_lived_runtime
        .query_page_logic_profiled(page_id, budget)
        .expect("profiled query_page_logic must succeed");

    assert_eq!(
        profile.counter("availability_read_model_used"),
        1,
        "warmed long-lived query must consume availability read model"
    );
    assert!(
        profile.counter("availability_materialized_hits") > 0,
        "long-lived warmed query must report materialized availability hits from cache batch"
    );
    assert_eq!(
        profile.counter("prerequisites_read_model_used"),
        1,
        "warmed long-lived query must consume prerequisites read model"
    );
    assert_eq!(
        profile.counter("path_read_model_used"),
        1,
        "warmed long-lived query must consume path_summary read model"
    );
    assert_eq!(
        profile.counter("path_dense_graph_used"),
        0,
        "path cache hit should skip query-time dense path_summary rebuild"
    );
}

/// M25 验收：reload 失败时保留旧 graph，仍可查询
#[test]
fn test_runtime_reload_failure_preserves_old_graph() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");
    let old_node_count = runtime.status().node_count;

    // 破坏 graphdb 文件
    std::fs::write(&db_path, b"not a valid graphdb").unwrap();

    let result = runtime.reload();
    assert!(result.is_err(), "损坏的 graphdb reload 必须失败");
    assert!(
        runtime.last_reload_error.is_some(),
        "失败时应记录 last_reload_error"
    );
    assert_eq!(
        runtime.status().node_count,
        old_node_count,
        "失败时应保留旧 graph"
    );
    assert_eq!(runtime.reload_count, 0, "失败时 reload_count 不应增加");

    // 旧 graph 仍可查询
    let request = RuntimeQueryRequest {
        command: RuntimeQueryCommand::ExplainCondition,
        target: "comp:app/actions_test.spg|input1".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        depth: None,
        check_reload: false,
        page_scope: None,
    };
    let resp = runtime.query(request).expect("旧 graph 仍可查询");
    assert_eq!(
        resp.result.get("kind").and_then(|v| v.as_str()),
        Some("Explain")
    );
}

#[test]
fn test_runtime_query_page_returns_real_value() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).unwrap();
    let req = RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryPage,
        target: "page:app/actions_test.spg".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };
    let resp = runtime.query(req).unwrap();
    let result = resp.result;
    assert!(
        !result.get("note").is_some(),
        "query_page should not return placeholder note, got: {:?}",
        result
    );
    assert!(
        result.get("summary").is_some(),
        "query_page should return summary, got: {:?}",
        result
    );
}

#[test]
fn test_runtime_query_cross_returns_real_value() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).unwrap();
    let req = RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryCross,
        target: "page:app/actions_test.spg,page:app/page_relations.spg".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };
    let resp = runtime.query(req).unwrap();
    let result = resp.result;
    assert!(
        !result.get("note").is_some(),
        "query_cross should not return placeholder note, got: {:?}",
        result
    );
    assert_eq!(
        result.get("kind").and_then(|v| v.as_str()),
        Some("CrossPageQuery"),
        "query_cross must return CrossPageQuery, got: {:?}",
        result
    );
    assert!(
        result.get("summary").is_some(),
        "query_cross should return summary, got: {:?}",
        result
    );
}

#[test]
fn test_runtime_query_cross_invalid_target_returns_error() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).unwrap();
    let req = RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryCross,
        target: "page:app/actions_test.spg".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };
    let err = runtime
        .query(req)
        .expect_err("invalid query_cross target must fail");
    assert!(
        err.to_string().contains("INVALID_ARGUMENT"),
        "runtime query_cross invalid target must surface INVALID_ARGUMENT, got: {}",
        err
    );
}

#[test]
fn test_runtime_query_dataflow_returns_real_value() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).unwrap();
    let req = RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryDataflow,
        target: "model:dataflow_a".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };
    let resp = runtime.query(req).unwrap();
    let result = resp.result;
    assert!(
        !result.get("note").is_some(),
        "query_dataflow should not return placeholder note, got: {:?}",
        result
    );
    assert!(
        result.get("summary").is_some(),
        "query_dataflow should return summary, got: {:?}",
        result
    );
}

#[cfg(feature = "cli-local")]
mod page_dependency_index_degradation {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::graph_store::{
        GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
    };
    use metadata_checker::query::PageDependencyIndex;
    use metadata_checker::runtime::{GraphRuntime, RuntimeMode};
    use std::cell::Cell;
    use std::sync::Arc;
    use std::time::Instant;

    use super::common;

    /// 在 `get_node_edges` 调用时注入读取失败，用于模拟 PageDependencyIndex 构建降级。
    struct FailingPageDependencyGraphStore<'a> {
        inner: &'a GraphDB,
        fail_on_get_node_edges: bool,
        get_node_edges_calls: Cell<usize>,
    }

    impl<'a> FailingPageDependencyGraphStore<'a> {
        fn fail_on_get_node_edges(inner: &'a GraphDB) -> Self {
            Self {
                inner,
                fail_on_get_node_edges: true,
                get_node_edges_calls: Cell::new(0),
            }
        }
    }

    impl GraphReadStore for FailingPageDependencyGraphStore<'_> {
        fn get_node(
            &self,
            node_id: &str,
        ) -> GraphStoreResult<Option<metadata_checker::graph::Node>> {
            GraphReadStore::get_node(self.inner, node_id)
        }

        fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
            self.get_node_edges_calls
                .set(self.get_node_edges_calls.get() + 1);
            if self.fail_on_get_node_edges {
                return Err(GraphStoreError::ReadFailed {
                    reason: format!("injected failure while reading edges for {node_id}"),
                });
            }
            GraphReadStore::get_node_edges(self.inner, node_id)
        }

        fn node_count(&self) -> GraphStoreResult<usize> {
            GraphReadStore::node_count(self.inner)
        }

        fn edge_count(&self) -> GraphStoreResult<usize> {
            GraphReadStore::edge_count(self.inner)
        }

        fn iter_nodes(
            &self,
        ) -> GraphStoreResult<Box<dyn Iterator<Item = metadata_checker::graph::Node> + '_>>
        {
            GraphReadStore::iter_nodes(self.inner)
        }
    }

    /// PageDependencyIndex::build 失败时应降级为空索引，并记录非零构建耗时。
    #[test]
    fn test_page_dependency_index_build_failure_falls_back_to_empty_index() -> anyhow::Result<()> {
        let (_temp_dir, db_path) = common::build_fixture_graphdb();
        let graph = GraphDB::open(&db_path)?;
        let failing = FailingPageDependencyGraphStore::fail_on_get_node_edges(&graph);

        let started = Instant::now();
        let (index, diagnostic) = match PageDependencyIndex::build(&failing) {
            Ok(index) => (Arc::new(index), None),
            Err(error) => (
                Arc::new(PageDependencyIndex::empty()),
                Some(format!(
                    "PageDependencyIndex build failed: {error}; using empty index"
                )),
            ),
        };
        let build_ms = started.elapsed().as_millis();

        let diagnostic = diagnostic.expect("build should fail under injected graph read error");
        assert!(
            diagnostic.contains("PageDependencyIndex build failed"),
            "diagnostic must mention build failure: {diagnostic}"
        );
        assert!(
            diagnostic.contains("using empty index"),
            "diagnostic must mention empty index fallback: {diagnostic}"
        );
        assert_eq!(index.indexed_node_count(), 0);
        assert!(
            index
                .affected_pages(&["comp:app/actions_test.spg|input1".to_string()])
                .is_empty()
        );
        assert!(
            failing.get_node_edges_calls.get() > 0,
            "build attempt should reach graph edge reads before failing"
        );
        let _ = build_ms;
        Ok(())
    }

    /// 空 page dependency 索引下，invalidate 应返回空列表且不报错。
    #[test]
    fn test_graph_runtime_invalidate_with_empty_page_dependency_index() -> anyhow::Result<()> {
        let (temp_dir, db_path) = common::build_fixture_graphdb();
        let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
            &db_path,
            Some(&temp_dir),
            RuntimeMode::LongLived,
        )?;

        let read_model = runtime
            .read_model
            .as_mut()
            .expect("long-lived runtime must have read model");
        let model = Arc::get_mut(read_model).expect("fresh runtime read model must be unique");
        model.page_dependency_index = Arc::new(PageDependencyIndex::empty());

        runtime.warm_page_logic_availability("page:app/actions_test.spg", "normal")?;

        let removed = runtime
            .invalidate_pages_for_dirty_nodes(&["comp:app/actions_test.spg|input1".to_string()])?;
        assert!(
            removed.is_empty(),
            "empty index must not invalidate any page"
        );

        assert!(
            runtime
                .read_model
                .as_ref()
                .expect("read model must remain available")
                .has_page_logic_availability("page:app/actions_test.spg", "normal"),
            "empty index invalidation must not remove warmed cache entries"
        );
        Ok(())
    }

    /// PageDependencyIndex::empty 应返回空 affected_pages。
    #[test]
    fn test_page_dependency_index_empty_returns_no_affected_pages() {
        let index = PageDependencyIndex::empty();
        assert_eq!(index.indexed_node_count(), 0);
        assert!(
            index
                .affected_pages(&["page:app/actions_test.spg".to_string()])
                .is_empty()
        );
    }
}
