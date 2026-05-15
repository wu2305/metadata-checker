use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

mod common;

/// 递归检查 JSON 中任意字符串字段是否包含指定片段
fn json_contains_str(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(s) => s.contains(needle),
        serde_json::Value::Array(items) => items.iter().any(|v| json_contains_str(v, needle)),
        serde_json::Value::Object(map) => map.values().any(|v| json_contains_str(v, needle)),
        _ => false,
    }
}

/// M24 核心验收：启动 stdio server，连续两次 explain_condition，
/// 第二次 graph_load_ms == 0
#[test]
fn test_stdio_server_reuses_graph() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req1 = serde_json::json!({
        "request_id": "req-1",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "human": false
    });
    let req2 = serde_json::json!({
        "request_id": "req-2",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req1).unwrap();
        writeln!(stdin_lock, "{}", req2).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    // 读取第一行响应
    let mut line1 = String::new();
    stdout_reader.read_line(&mut line1).unwrap();
    let resp1: serde_json::Value = serde_json::from_str(&line1).expect("resp1 must be valid JSON");
    assert_eq!(resp1["ok"].as_bool(), Some(true), "resp1 ok must be true");
    assert_eq!(resp1["request_id"].as_str(), Some("req-1"), "resp1 request_id must match");
    assert!(
        resp1["result"].is_object(),
        "resp1 result must be object"
    );
    assert_eq!(
        resp1["result"]["kind"].as_str(),
        Some("Explain"),
        "resp1 result.kind must be Explain"
    );

    // 读取第二行响应
    let mut line2 = String::new();
    stdout_reader.read_line(&mut line2).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(&line2).expect("resp2 must be valid JSON");
    assert_eq!(resp2["ok"].as_bool(), Some(true), "resp2 ok must be true");
    assert_eq!(resp2["request_id"].as_str(), Some("req-2"), "resp2 request_id must match");

    // 第二次 graph_load_ms == 0（热查询复用）
    assert_eq!(
        resp2["timing"]["graph_load_ms"].as_u64(),
        Some(0),
        "第二次查询 graph_load_ms 必须为 0"
    );

    // stdout 行必须是纯 JSON
    assert!(line1.trim().starts_with('{'), "resp1 must be JSON");
    assert!(line2.trim().starts_with('{'), "resp2 must be JSON");

    let _ = child.wait();
}

/// 负例：非法 JSON 请求
#[test]
fn test_stdio_server_invalid_json() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "not-json-at-all").unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false), "invalid JSON must return ok=false");
    assert!(
        resp["error"].as_str().unwrap_or("").contains("JSON parse error"),
        "error must mention JSON parse"
    );

    let _ = child.wait();
}

/// 负例：未知 command
#[test]
fn test_stdio_server_unknown_command() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-unknown",
        "command": "magic_spell",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false), "unknown command must return ok=false");
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Unknown command"),
        "error must mention unknown command"
    );

    let _ = child.wait();
}

/// 负例：缺少 target 字段
#[test]
fn test_stdio_server_missing_target() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-missing-target",
        "command": "explain_condition",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false), "missing target must return ok=false");
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Missing target"),
        "error must mention Missing target: got {}",
        resp["error"].as_str().unwrap_or("(none)")
    );

    let _ = child.wait();
}

/// M25 验收：stdio status 命令返回 runtime 状态
#[test]
fn test_stdio_server_status() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-status",
        "command": "status"
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "status must return ok=true");
    assert_eq!(resp["request_id"].as_str(), Some("req-status"));
    let result = resp["result"].as_object().expect("status result must be object");
    assert!(
        result.get("node_count").and_then(|v| v.as_u64()).unwrap_or(0) > 0,
        "status node_count must be > 0"
    );
    assert_eq!(result["reload_count"].as_u64(), Some(0), "status reload_count must be 0");
    assert_eq!(result["load_count"].as_u64(), Some(1), "status load_count must be 1");

    let _ = child.wait();
}

/// M25 验收：stdio reload 成功后 reload_count 增加
#[test]
fn test_stdio_server_reload_success() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-reload",
        "command": "reload"
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "reload must return ok=true");
    let result = resp["result"].as_object().expect("reload result must be object");
    assert_eq!(
        result["reload_count"].as_u64(),
        Some(1),
        "reload 后 reload_count 必须为 1"
    );
    assert!(
        resp["diagnostics"].as_array().unwrap_or(&vec![]).iter().any(|d|
            d.as_str().unwrap_or("").contains("GRAPH_RELOADED")
        ),
        "diagnostics 必须包含 GRAPH_RELOADED"
    );

    let _ = child.wait();
}

/// M25 验收：stdio reload 失败时返回 GRAPH_RELOAD_FAILED
#[test]
fn test_stdio_server_reload_failure() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    // 等待 server 启动完成
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 在 server 启动后破坏 graphdb
    std::fs::write(&db_path, b"not a valid graphdb").unwrap();

    let req = serde_json::json!({
        "request_id": "req-reload-fail",
        "command": "reload"
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false), "损坏 graphdb reload 必须返回 ok=false");
    assert!(
        resp["diagnostics"].as_array().unwrap_or(&vec![]).iter().any(|d|
            d.as_str().unwrap_or("").contains("GRAPH_RELOAD_FAILED")
        ),
        "diagnostics 必须包含 GRAPH_RELOAD_FAILED"
    );
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Reload failed"),
        "error 必须包含 Reload failed"
    );

    let _ = child.wait();
}

/// M25 验收：check_reload=true + graphdb 损坏时，查询仍成功但返回 GRAPH_RELOAD_FAILED diagnostic
#[test]
fn test_stdio_server_check_reload_failure() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    // 等待 server 启动完成
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 破坏 graphdb 文件
    std::fs::write(&db_path, b"not a valid graphdb").unwrap();

    let req = serde_json::json!({
        "request_id": "req-check-reload-fail",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "human": false,
        "check_reload": true
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");

    // 查询本身仍应成功（旧 graph 还在）
    assert_eq!(resp["ok"].as_bool(), Some(true), "check_reload 失败不应导致查询失败");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("Explain"),
        "result.kind 必须是 Explain"
    );

    // 但 diagnostics 必须包含 GRAPH_RELOAD_FAILED
    assert!(
        resp["diagnostics"].as_array().unwrap_or(&vec![]).iter().any(|d|
            d.as_str().unwrap_or("").contains("GRAPH_RELOAD_FAILED")
        ),
        "diagnostics 必须包含 GRAPH_RELOAD_FAILED"
    );

    let _ = child.wait();
}

/// M27 验收：stdio query_model 命令
#[test]
fn test_stdio_server_query_model() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-query-model",
        "command": "query_model",
        "target": "model:model1",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "query_model must return ok=true");
    assert_eq!(resp["request_id"].as_str(), Some("req-query-model"));
    assert!(resp["result"].is_object(), "result must be object");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("ModelQuery"),
        "result.kind must be ModelQuery"
    );
    assert!(
        resp["result"]["summary"]["model_id"].is_string(),
        "summary.model_id must exist"
    );

    let _ = child.wait();
}

/// M27 验收：stdio context 命令
#[test]
fn test_stdio_server_context() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-context",
        "command": "context",
        "target": "comp:app/actions_test.spg|input1",
        "depth": 1,
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "context must return ok=true");
    assert_eq!(resp["request_id"].as_str(), Some("req-context"));
    assert!(resp["result"].is_object(), "result must be object");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("Context"),
        "result.kind must be Context"
    );
    assert!(
        resp["result"]["summary"]["center_node"].is_string(),
        "summary.center_node must exist"
    );

    let _ = child.wait();
}

/// M27 验收：stdio explain 命令
#[test]
fn test_stdio_server_explain() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-explain",
        "command": "explain",
        "target": "model:model1",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "explain must return ok=true");
    assert_eq!(resp["request_id"].as_str(), Some("req-explain"));
    assert!(resp["result"].is_object(), "result must be object");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("Explain"),
        "result.kind must be Explain"
    );
    assert!(
        resp["result"]["summary"]["what_is_it"].is_string(),
        "summary.what_is_it must exist"
    );

    let _ = child.wait();
}

/// M27 验收：stdio query_page_logic 命令，且与 CLI JSON 输出一致
#[test]
fn test_stdio_server_query_page_logic() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-page-logic",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "query_page_logic must return ok=true");
    assert_eq!(resp["request_id"].as_str(), Some("req-page-logic"));
    assert!(resp["result"].is_object(), "result must be object");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("PageLogic"),
        "result.kind must be PageLogic"
    );

    // 验证 project_dir 已正确传递：details 中应包含 entrypoints/data_sources/action_flows
    let details = resp["result"]["details"].as_object().expect("details must be object");
    assert!(
        details.contains_key("entrypoints"),
        "details must have entrypoints"
    );
    assert!(
        details.contains_key("data_sources"),
        "details must have data_sources"
    );
    assert!(
        details.contains_key("action_flows"),
        "details must have action_flows"
    );
    assert!(
        details.contains_key("display_prerequisites"),
        "details must have display_prerequisites"
    );
    assert!(
        details.contains_key("data_prerequisites"),
        "details must have data_prerequisites"
    );

    let _ = child.wait();
}

/// M27 验收：第二次 stdio 查询 graph_load_ms 仍为 0
#[test]
fn test_stdio_server_second_query_reuses_graph() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req1 = serde_json::json!({
        "request_id": "req-1",
        "command": "query_model",
        "target": "model:model1",
        "budget": "compact",
        "human": false
    });
    let req2 = serde_json::json!({
        "request_id": "req-2",
        "command": "context",
        "target": "comp:app/actions_test.spg|input1",
        "depth": 1,
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req1).unwrap();
        writeln!(stdin_lock, "{}", req2).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    // 读取第一行
    let mut line1 = String::new();
    stdout_reader.read_line(&mut line1).unwrap();
    let resp1: serde_json::Value = serde_json::from_str(&line1).expect("resp1 must be valid JSON");
    assert_eq!(resp1["ok"].as_bool(), Some(true));

    // 读取第二行
    let mut line2 = String::new();
    stdout_reader.read_line(&mut line2).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(&line2).expect("resp2 must be valid JSON");
    assert_eq!(resp2["ok"].as_bool(), Some(true));

    // 第二次查询 graph_load_ms == 0（热查询复用）
    assert_eq!(
        resp2["timing"]["graph_load_ms"].as_u64(),
        Some(0),
        "第二次查询 graph_load_ms 必须为 0"
    );

    let _ = child.wait();
}

/// M27 负例：query_model missing target
#[test]
fn test_stdio_server_query_model_missing_target() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-qm-missing",
        "command": "query_model",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false), "missing target must return ok=false");
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Missing target for query_model"),
        "error must mention missing target"
    );

    let _ = child.wait();
}

/// M27 负例：context missing target
#[test]
fn test_stdio_server_context_missing_target() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-ctx-missing",
        "command": "context",
        "depth": 1,
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false));
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Missing target for context")
    );

    let _ = child.wait();
}

/// M27 负例：query_page_logic missing target
#[test]
fn test_stdio_server_query_page_logic_missing_target() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-qpl-missing",
        "command": "query_page_logic",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false));
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Missing target for query_page_logic")
    );

    let _ = child.wait();
}

/// M27 负例：explain missing target
#[test]
fn test_stdio_server_explain_missing_target() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-exp-missing",
        "command": "explain",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(false));
    assert!(
        resp["error"].as_str().unwrap_or("").contains("Missing target for explain")
    );

    let _ = child.wait();
}

/// M27 验收：human=true 对非 explain_condition 命令返回 HUMAN_MODE_NOT_SUPPORTED diagnostic
#[test]
fn test_stdio_server_human_not_supported_diagnostic() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-human",
        "command": "query_model",
        "target": "model:model1",
        "budget": "compact",
        "human": true
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "human=true 不应导致查询失败");
    assert!(
        resp["diagnostics"].as_array().unwrap_or(&vec![]).iter().any(|d|
            d.as_str().unwrap_or("").contains("HUMAN_MODE_NOT_SUPPORTED")
        ),
        "diagnostics 必须包含 HUMAN_MODE_NOT_SUPPORTED"
    );

    let _ = child.wait();
}

/// M27 验收：外部 graphdb + 显式 project_dir 场景
///
/// 把 graphdb 复制到外部目录，启动 stdio server 时指定 --project-dir，
/// 验证 query_page_logic 仍能读取原始 .spg 文件。
#[test]
fn test_stdio_server_external_graphdb_with_project_dir() {
    let (project_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    // 把 graphdb 移动到外部目录
    let external_db = std::env::temp_dir().join(format!(
        "m27-external-{}.graphdb",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(&db_path, &external_db).expect("copy graphdb must succeed");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            external_db.to_str().unwrap(),
            "--project-dir",
            project_dir.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req = serde_json::json!({
        "request_id": "req-ext",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");
    assert_eq!(resp["ok"].as_bool(), Some(true), "external graphdb + project_dir must work");
    assert_eq!(resp["request_id"].as_str(), Some("req-ext"));

    let details = resp["result"]["details"].as_object().expect("details must be object");
    assert!(
        details.contains_key("entrypoints"),
        "details must have entrypoints"
    );
    assert!(
        details.contains_key("data_sources"),
        "details must have data_sources"
    );
    assert!(
        details.contains_key("action_flows"),
        "details must have action_flows"
    );

    let action_flows = details["action_flows"]["items"]
        .as_array()
        .expect("action_flows.items must be array");
    let has_raw_index_json_path = action_flows.iter().any(|flow| {
        let json_path = flow["json_path"].as_str().unwrap_or("");
        json_path.starts_with("canvas.components[")
            && json_path.contains("].actions[")
            && !json_path.contains("[id='")
    });
    assert!(
        has_raw_index_json_path,
        "external graphdb + explicit project_dir must read raw .spg json_path, not graph-derived id fallback"
    );

    // 清理外部 graphdb
    let _ = std::fs::remove_file(&external_db);
    let _ = child.wait();
}

/// M27 真实项目回归：stdio query_model fact_qwSidebar 必须包含跨页 writer
#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_stdio_query_model_fact_qw_sidebar() {
    let project_dir = std::path::Path::new("/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi");
    if !project_dir.exists() {
        return;
    }

    let project_db_path = project_dir.join(".metadata-checker.graphdb");
    let tmp_db_path = std::path::PathBuf::from("/tmp/m23_runtime_test.graphdb");
    let (db_path, cleanup_db) = if project_db_path.exists() {
        (project_db_path, None)
    } else if tmp_db_path.exists() {
        (tmp_db_path, None)
    } else {
        let generated = std::env::temp_dir().join(format!(
            "m27-real-{}.graphdb",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        metadata_checker::scanner::scan_project(project_dir, &generated)
            .expect("real project graphdb build must succeed");
        (generated.clone(), Some(generated))
    };
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let mut child = Command::new(bin)
        .args([
            "--serve-stdio",
            "--graph-db-path",
            db_path.to_str().unwrap(),
            "--project-dir",
            project_dir.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn stdio server");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let req1 = serde_json::json!({
        "request_id": "r1",
        "command": "query_model",
        "target": "model:fact_qwSidebar",
        "budget": "compact",
        "human": false
    });
    let req2 = serde_json::json!({
        "request_id": "r2",
        "command": "query_page_logic",
        "target": "page:app/销售.app/销售/合同协议.spg",
        "budget": "compact",
        "human": false
    });

    {
        let stdin_lock = &mut stdin;
        writeln!(stdin_lock, "{}", req1).unwrap();
        writeln!(stdin_lock, "{}", req2).unwrap();
        stdin_lock.flush().unwrap();
    }

    let mut line1 = String::new();
    stdout_reader.read_line(&mut line1).unwrap();
    let resp1: serde_json::Value = serde_json::from_str(&line1).expect("resp1 must be valid JSON");
    assert_eq!(resp1["ok"].as_bool(), Some(true));

    let writers = resp1["result"]["details"]["writers"]["items"]
        .as_array()
        .expect("details.writers.items must be array");
    let has_writer = writers.iter().any(|w| {
        let id = w["node_id"].as_str().unwrap_or("");
        id.contains("潜客信息跟进") && (id.contains("action1") || id.contains("action4"))
    });
    assert!(has_writer, "fact_qwSidebar writers must include 潜客信息跟进 action1/action4");

    let mut line2 = String::new();
    stdout_reader.read_line(&mut line2).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(&line2).expect("resp2 must be valid JSON");
    assert_eq!(resp2["ok"].as_bool(), Some(true));
    for needle in [
        "input3",
        "model22.phoneNumber",
        "fact_qwSidebar.phoneNumber",
        "action1",
        "action4",
    ] {
        assert!(
            json_contains_str(&resp2["result"], needle),
            "query_page_logic stdio result must contain primary-chain token: {}",
            needle
        );
    }
    assert_eq!(
        resp2["timing"]["graph_load_ms"].as_u64(),
        Some(0),
        "第二次查询 graph_load_ms 必须为 0"
    );

    drop(stdin);
    let _ = child.wait();
    if let Some(path) = cleanup_db {
        let _ = std::fs::remove_file(path);
    }
}
