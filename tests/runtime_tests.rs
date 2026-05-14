use metadata_checker::runtime::{GraphRuntime, RuntimeQueryCommand, RuntimeQueryRequest};
use metadata_checker::scanner;

/// 构建 fixture graphdb 到临时目录
fn build_fixture_graphdb() -> (std::path::PathBuf, std::path::PathBuf) {
    let unique = format!("metadata-checker-runtime-test-{:?}-{}", std::thread::current().id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let temp_dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let src = std::path::Path::new("tests/fixtures/test_project");
    copy_dir_all(src, &temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");
    scanner::scan_project(&temp_dir, &db_path,
    ).expect("scan_project must succeed");
    (temp_dir, db_path)
}

fn copy_dir_all(src: impl AsRef<std::path::Path>, dst: impl AsRef<std::path::Path>) {
    std::fs::create_dir_all(&dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name())).unwrap();
        }
    }
}

/// M23 核心验收：同一个 GraphRuntime 连续执行两次 ExplainCondition，
/// 第二次不再重复加载 graphdb
#[test]
fn test_graph_runtime_reuses_loaded_graph_for_explain_condition() {
    let (_temp_dir, db_path) = build_fixture_graphdb();

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
    };

    // 第一次查询
    let resp1 = runtime.query(request.clone()).expect("第一次 query 必须成功");
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
        result1.get("summary").unwrap().get("blocking_conditions_count"),
        result2.get("summary").unwrap().get("blocking_conditions_count"),
        "两次查询的 blocking_conditions_count 必须一致"
    );

    // 第二次 graph_load_ms 必须为 0（热查询复用）
    assert_eq!(
        resp2.timing.graph_load_ms, 0,
        "第二次查询 graph_load_ms 应为 0，因为 graph 已在内存中"
    );

    // load_count 仍为 1
    assert_eq!(
        runtime.load_count, 1,
        "连续两次查询后 load_count 仍为 1"
    );
}

/// M23 验收：真实项目 input3  explain-condition
/// 通过 runtime 复用 graphdb，结果仍包含 model22.phoneNumber / fact_qwSidebar.phoneNumber
#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_runtime_input3_explain_condition_reuses_graph() {
    let project_dir = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
    let db_path = std::path::PathBuf::from("/tmp/m23_runtime_test.graphdb");
    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(std::path::Path::new(project_dir), &db_path).expect("scan real project must succeed");

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
    };

    // 第一次查询
    let resp1 = runtime.query(request.clone()).expect("第一次 query 必须成功");
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

    // primary_path 非空
    let primary_path = result1
        .get("details")
        .and_then(|d| d.get("primary_path"))
        .and_then(|v| v.as_array())
        .expect("primary_path must be array");
    assert!(!primary_path.is_empty(), "primary_path must not be empty");

    // 必须包含 model22.phoneNumber
    let has_model22 = primary_path.iter().any(|p| {
        p.get("path_id")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("model22.phoneNumber"))
            .unwrap_or(false)
    });
    assert!(has_model22, "primary_path must contain model22.phoneNumber");

    // 必须包含 fact_qwSidebar.phoneNumber
    let has_fact = primary_path.iter().any(|p| {
        p.get("path_id")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("fact_qwSidebar.phoneNumber"))
            .unwrap_or(false)
    });
    assert!(
        has_fact,
        "primary_path must contain fact_qwSidebar.phoneNumber"
    );

    // 必须包含 action1 或 action4
    let has_writer = primary_path.iter().any(|p| {
        p.get("path_id")
            .and_then(|v| v.as_str())
            .map(|s| s.contains("action1") || s.contains("action4"))
            .unwrap_or(false)
    });
    assert!(
        has_writer,
        "primary_path must contain action1 or action4"
    );

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
    let (_temp_dir, db_path) = build_fixture_graphdb();
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
    let (_temp_dir, db_path) = build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");

    let result = runtime.reload_if_changed().expect("reload_if_changed must succeed");
    assert_eq!(result, metadata_checker::runtime::ReloadResult::Unchanged, "未变更时应返回 Unchanged");
    assert_eq!(runtime.reload_count, 0, "未变更时 reload_count 仍为 0");
}

/// M25 验收：graphdb 替换后 reload_if_changed 返回 true
#[test]
fn test_runtime_reload_if_changed_after_replace() {
    let (temp_dir, db_path) = build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");
    let old_node_count = runtime.status().node_count;

    // 重建 graphdb（模拟外部更新）
    let _ = std::fs::remove_file(&db_path);
    scanner::scan_project(&temp_dir, &db_path,
    ).expect("re-scan must succeed");

    let result = runtime.reload_if_changed().expect("reload_if_changed must succeed");
    assert_eq!(result, metadata_checker::runtime::ReloadResult::Reloaded, "文件变更后应返回 Reloaded");
    assert_eq!(runtime.reload_count, 1, "reload_count 应增加到 1");
    assert_eq!(runtime.status().node_count, old_node_count, "reload 后 node_count 应一致");
    assert!(runtime.last_reload_error.is_none());
}

/// M25 验收：reload 失败时保留旧 graph，仍可查询
#[test]
fn test_runtime_reload_failure_preserves_old_graph() {
    let (_temp_dir, db_path) = build_fixture_graphdb();
    let mut runtime = GraphRuntime::load(&db_path).expect("load must succeed");
    let old_node_count = runtime.status().node_count;

    // 破坏 graphdb 文件
    std::fs::write(&db_path, b"not a valid graphdb").unwrap();

    let result = runtime.reload();
    assert!(result.is_err(), "损坏的 graphdb reload 必须失败");
    assert!(runtime.last_reload_error.is_some(), "失败时应记录 last_reload_error");
    assert_eq!(runtime.status().node_count, old_node_count, "失败时应保留旧 graph");
    assert_eq!(runtime.reload_count, 0, "失败时 reload_count 不应增加");

    // 旧 graph 仍可查询
    let request = RuntimeQueryRequest {
        command: RuntimeQueryCommand::ExplainCondition,
        target: "comp:app/actions_test.spg|input1".to_string(),
        budget: "compact".to_string(),
        human: false,
    };
    let resp = runtime.query(request).expect("旧 graph 仍可查询");
    assert_eq!(resp.result.get("kind").and_then(|v| v.as_str()), Some("Explain"));
}
