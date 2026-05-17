use metadata_checker::runtime::{GraphRuntime, RuntimeQueryCommand, RuntimeQueryRequest};
use metadata_checker::scanner;

mod common;

/// M23 核心验收：同一个 GraphRuntime 连续执行两次 ExplainCondition，
/// 第二次不再重复加载 graphdb
#[test]
fn test_graph_runtime_reuses_loaded_graph_for_explain_condition() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();

    let runtime = GraphRuntime::load(&db_path).expect("GraphRuntime::load must succeed");
    assert_eq!(runtime.load_count, 1, "首次加载后 load_count 应为 1");
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

    let runtime = GraphRuntime::load(&db_path).expect("load real project graph must succeed");
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
    assert!(runtime.last_reload_error.is_none());
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
    };
    let resp = runtime.query(request).expect("旧 graph 仍可查询");
    assert_eq!(
        resp.result.get("kind").and_then(|v| v.as_str()),
        Some("Explain")
    );
}
