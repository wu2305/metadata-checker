#![cfg(feature = "cli-local")]

use metadata_checker::tool_contract::ToolRegistry;
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

/// 读取 stdio 错误码
fn stdio_error_code(resp: &serde_json::Value) -> Option<&str> {
    resp["error"]["code"].as_str()
}

/// 读取 stdio 错误消息
fn stdio_error_message(resp: &serde_json::Value) -> &str {
    resp["error"]["message"].as_str().unwrap_or("")
}

/// 断言 stdio 固定响应 envelope
fn assert_stdio_envelope(resp: &serde_json::Value, ok: bool) {
    let obj = resp
        .as_object()
        .expect("stdio response must be JSON object");
    assert!(
        obj.contains_key("request_id"),
        "stdio response must contain request_id"
    );
    assert!(obj.contains_key("ok"), "stdio response must contain ok");
    assert!(
        obj.contains_key("diagnostics"),
        "stdio response must contain diagnostics even when empty"
    );
    assert!(
        obj.contains_key("timing"),
        "stdio response must contain timing"
    );
    assert_eq!(resp["ok"].as_bool(), Some(ok), "stdio ok flag must match");
    assert!(
        resp["diagnostics"].is_array(),
        "stdio diagnostics must be an array"
    );
    assert!(resp["timing"].is_object(), "stdio timing must be an object");
    assert!(
        resp["timing"]["output_size_bytes"].as_u64().unwrap_or(0) > 0,
        "stdio timing.output_size_bytes must be recorded"
    );
    if ok {
        assert!(
            obj.contains_key("result"),
            "success response must contain result"
        );
    } else {
        assert!(
            obj.contains_key("error"),
            "error response must contain error"
        );
        assert!(
            resp["error"]["code"].is_string(),
            "error response must contain error.code"
        );
        assert!(
            resp["error"]["message"].is_string(),
            "error response must contain error.message"
        );
    }
}

/// 断言 timing.output_size_bytes 与实际 stdout 行长度一致
fn assert_output_size_matches_line(resp: &serde_json::Value, line: &str) {
    assert_eq!(
        resp["timing"]["output_size_bytes"].as_u64(),
        Some(line.trim_end_matches(&['\r', '\n'][..]).len() as u64),
        "timing.output_size_bytes must match serialized stdout response size"
    );
}

/// 判断 query_page_logic 是否读取到了原始 .spg 文件中的索引式 action json_path
fn has_raw_index_action_json_path(result: &serde_json::Value) -> bool {
    let Some(action_flows) = result["details"]["action_flows"]["items"].as_array() else {
        return false;
    };
    action_flows.iter().any(|flow| {
        let json_path = flow["json_path"].as_str().unwrap_or("");
        json_path.starts_with("canvas.components[")
            && json_path.contains("].actions[")
            && !json_path.contains("[id='")
    })
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
    assert_eq!(
        resp1["request_id"].as_str(),
        Some("req-1"),
        "resp1 request_id must match"
    );
    assert!(resp1["result"].is_object(), "resp1 result must be object");
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
    assert_eq!(
        resp2["request_id"].as_str(),
        Some("req-2"),
        "resp2 request_id must match"
    );

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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "invalid JSON must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("INVALID_JSON"));
    assert!(
        stdio_error_message(&resp).contains("JSON parse error"),
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "unknown command must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("UNKNOWN_COMMAND"));
    assert!(
        stdio_error_message(&resp).contains("Unknown command"),
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "missing target must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("MISSING_TARGET"));
    assert!(
        stdio_error_message(&resp).contains("Missing target"),
        "error must mention Missing target: got {}",
        stdio_error_message(&resp)
    );

    let _ = child.wait();
}

/// M28 负例：非法 budget 返回结构化 INVALID_BUDGET
#[test]
fn test_stdio_server_invalid_budget() {
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
        "request_id": "req-invalid-budget",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "maximum",
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "invalid budget must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("INVALID_BUDGET"));
    assert!(stdio_error_message(&resp).contains("Invalid budget"));

    let _ = child.wait();
}

/// M33：stdio explain_condition 支持 intent，并返回 answer_facts
#[test]
fn test_stdio_server_explain_condition_intent_answer_facts() {
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
        "request_id": "req-m33-intent",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|text_bare_field_child",
        "budget": "compact",
        "intent": "value-source",
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(resp["ok"].as_bool(), Some(true));
    assert_eq!(
        resp["result"]["summary"]["intent"].as_str(),
        Some("value-source")
    );
    assert_eq!(
        resp["result"]["details"]["answer_facts"]["value_source_facts"]["result"].as_str(),
        Some("data/table1.tbl")
    );

    let _ = child.wait();
}

/// M35：advise_query 在 page_scope 下允许裸组件 ID，并生成页面作用域推荐目标
#[test]
fn test_stdio_server_advise_query_accepts_bare_target_with_page_scope() {
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
        "request_id": "req-advise-bare-target",
        "command": "advise_query",
        "target": "text_bare_field_child",
        "page_scope": "app/actions_test.spg",
        "intent": "value-source",
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(resp["ok"].as_bool(), Some(true));
    assert_eq!(
        resp["result"]["primary_command"].as_str(),
        Some("--explain-condition")
    );
    assert_eq!(
        resp["result"]["primary_target"].as_str(),
        Some("comp:app/actions_test.spg|text_bare_field_child")
    );
    assert_eq!(
        resp["result"]["primary_fact_path"].as_str(),
        Some("value_source_facts")
    );

    let _ = child.wait();
}

/// M33：非法 intent 返回结构化 INVALID_INTENT
#[test]
fn test_stdio_server_invalid_intent() {
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
        "request_id": "req-invalid-intent",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "intent": "everything",
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(stdio_error_code(&resp), Some("INVALID_INTENT"));
    assert!(stdio_error_message(&resp).contains("Invalid intent"));

    let _ = child.wait();
}

/// M28 负例：命令不接受的 target 形态返回结构化 INVALID_TARGET
#[test]
fn test_stdio_server_invalid_target() {
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
        "request_id": "req-invalid-target",
        "command": "query_page_logic",
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "invalid target must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("INVALID_TARGET"));
    assert!(stdio_error_message(&resp).contains("Invalid target"));

    let _ = child.wait();
}

/// M28 负例：非法 context depth 返回结构化 INVALID_DEPTH
#[test]
fn test_stdio_server_invalid_depth() {
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
        "request_id": "req-invalid-depth",
        "command": "context",
        "target": "comp:app/actions_test.spg|input1",
        "depth": "deep",
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "invalid depth must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("INVALID_DEPTH"));
    assert!(stdio_error_message(&resp).contains("Invalid depth"));

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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "status must return ok=true"
    );
    assert_eq!(resp["request_id"].as_str(), Some("req-status"));
    let result = resp["result"]
        .as_object()
        .expect("status result must be object");
    assert!(
        result
            .get("node_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
            > 0,
        "status node_count must be > 0"
    );
    assert_eq!(
        result["reload_count"].as_u64(),
        Some(0),
        "status reload_count must be 0"
    );
    assert_eq!(
        result["load_count"].as_u64(),
        Some(1),
        "status load_count must be 1"
    );

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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "reload must return ok=true"
    );
    let result = resp["result"]
        .as_object()
        .expect("reload result must be object");
    assert_eq!(
        result["reload_count"].as_u64(),
        Some(1),
        "reload 后 reload_count 必须为 1"
    );
    assert!(
        resp["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("GRAPH_RELOADED")),
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
    assert_stdio_envelope(&resp, false);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "损坏 graphdb reload 必须返回 ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("GRAPH_RELOAD_FAILED"));
    assert!(
        resp["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("GRAPH_RELOAD_FAILED")),
        "diagnostics 必须包含 GRAPH_RELOAD_FAILED"
    );
    assert!(
        stdio_error_message(&resp).contains("Reload failed"),
        "error 必须包含 Reload failed"
    );

    let _ = child.wait();
}

/// M28 验收：显式 reload 失败后旧 graph 仍可继续服务查询
#[test]
fn test_stdio_server_reload_failure_keeps_old_graph_usable() {
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

    std::thread::sleep(std::time::Duration::from_millis(100));
    std::fs::write(&db_path, b"not a valid graphdb").unwrap();

    let reload_req = serde_json::json!({
        "request_id": "req-reload-fail-old-graph",
        "command": "reload"
    });
    let query_req = serde_json::json!({
        "request_id": "req-after-reload-fail",
        "command": "explain_condition",
        "target": "comp:app/actions_test.spg|input1",
        "budget": "compact",
        "human": false
    });

    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", reload_req).unwrap();
        writeln!(stdin_lock, "{}", query_req).unwrap();
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    let mut reload_line = String::new();
    stdout_reader.read_line(&mut reload_line).unwrap();
    let reload_resp: serde_json::Value =
        serde_json::from_str(&reload_line).expect("reload resp must be valid JSON");
    assert_eq!(reload_resp["ok"].as_bool(), Some(false));
    assert_eq!(stdio_error_code(&reload_resp), Some("GRAPH_RELOAD_FAILED"));

    let mut query_line = String::new();
    stdout_reader.read_line(&mut query_line).unwrap();
    let query_resp: serde_json::Value =
        serde_json::from_str(&query_line).expect("query resp must be valid JSON");
    assert_eq!(
        query_resp["ok"].as_bool(),
        Some(true),
        "reload 失败后旧 graph 必须仍可查询"
    );
    assert_eq!(
        query_resp["request_id"].as_str(),
        Some("req-after-reload-fail")
    );
    assert_eq!(query_resp["result"]["kind"].as_str(), Some("Explain"));

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

    let status_req = serde_json::json!({
        "request_id": "req-check-reload-ready",
        "command": "status"
    });
    {
        let mut stdin_lock = stdin;
        writeln!(stdin_lock, "{}", status_req).unwrap();
        stdin_lock.flush().unwrap();

        let mut ready_line = String::new();
        stdout_reader.read_line(&mut ready_line).unwrap();
        let ready_resp: serde_json::Value =
            serde_json::from_str(&ready_line).expect("ready resp must be valid JSON");
        assert_eq!(ready_resp["ok"].as_bool(), Some(true));

        // server 确认加载完成后再破坏 graphdb，避免 baseline fingerprint 记录到坏文件。
        std::fs::write(&db_path, b"not a valid graphdb").unwrap();

        let req = serde_json::json!({
            "request_id": "req-check-reload-fail",
            "command": "explain_condition",
            "target": "comp:app/actions_test.spg|input1",
            "budget": "compact",
            "human": false,
            "check_reload": true
        });

        writeln!(stdin_lock, "{}", req).unwrap();
        stdin_lock.flush().unwrap();
    }

    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: serde_json::Value = serde_json::from_str(&line).expect("resp must be valid JSON");

    // 查询本身仍应成功（旧 graph 还在）
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "check_reload 失败不应导致查询失败"
    );
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("Explain"),
        "result.kind 必须是 Explain"
    );

    // 但 diagnostics 必须包含 GRAPH_RELOAD_FAILED
    assert!(
        resp["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("GRAPH_RELOAD_FAILED")),
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "query_model must return ok=true"
    );
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "context must return ok=true"
    );
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "explain must return ok=true"
    );
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "query_page_logic must return ok=true"
    );
    assert_eq!(resp["request_id"].as_str(), Some("req-page-logic"));
    assert!(resp["result"].is_object(), "result must be object");
    assert_eq!(
        resp["result"]["kind"].as_str(),
        Some("PageLogic"),
        "result.kind must be PageLogic"
    );

    // 验证 project_dir 已正确传递：details 中应包含 entrypoints/data_sources/action_flows
    let details = resp["result"]["details"]
        .as_object()
        .expect("details must be object");
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
    assert_eq!(
        resp["ok"].as_bool(),
        Some(false),
        "missing target must return ok=false"
    );
    assert_eq!(stdio_error_code(&resp), Some("MISSING_TARGET"));
    assert!(
        stdio_error_message(&resp).contains("Missing target for query_model"),
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
    assert_eq!(stdio_error_code(&resp), Some("MISSING_TARGET"));
    assert!(stdio_error_message(&resp).contains("Missing target for context"));

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
    assert_eq!(stdio_error_code(&resp), Some("MISSING_TARGET"));
    assert!(stdio_error_message(&resp).contains("Missing target for query_page_logic"));

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
    assert_eq!(stdio_error_code(&resp), Some("MISSING_TARGET"));
    assert!(stdio_error_message(&resp).contains("Missing target for explain"));

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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "human=true 不应导致查询失败"
    );
    assert!(
        resp["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d
                .as_str()
                .unwrap_or("")
                .contains("HUMAN_MODE_NOT_SUPPORTED")),
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
    assert_stdio_envelope(&resp, true);
    assert_output_size_matches_line(&resp, &line);
    assert_eq!(
        resp["ok"].as_bool(),
        Some(true),
        "external graphdb + project_dir must work"
    );
    assert_eq!(resp["request_id"].as_str(), Some("req-ext"));

    let details = resp["result"]["details"]
        .as_object()
        .expect("details must be object");
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
        has_raw_index_action_json_path(&resp["result"]),
        "external graphdb + explicit project_dir must read raw .spg json_path, not graph-derived id fallback"
    );

    // 清理外部 graphdb
    let _ = std::fs::remove_file(&external_db);
    let _ = child.wait();
}

/// M27 回归：外部 graphdb reload 后仍保留显式 project_dir
#[test]
fn test_stdio_server_reload_preserves_project_dir_for_external_graphdb() {
    let (project_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let external_db = std::env::temp_dir().join(format!(
        "m27-reload-external-{}.graphdb",
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

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let before = serde_json::json!({
        "request_id": "before",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false
    });
    let reload = serde_json::json!({
        "request_id": "reload",
        "command": "reload"
    });
    let after = serde_json::json!({
        "request_id": "after",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false
    });

    writeln!(stdin, "{}", before).unwrap();
    writeln!(stdin, "{}", reload).unwrap();
    writeln!(stdin, "{}", after).unwrap();
    stdin.flush().unwrap();

    let mut line1 = String::new();
    stdout_reader.read_line(&mut line1).unwrap();
    let resp1: serde_json::Value = serde_json::from_str(&line1).expect("resp1 must be valid JSON");
    assert_eq!(resp1["ok"].as_bool(), Some(true));
    assert!(
        has_raw_index_action_json_path(&resp1["result"]),
        "before reload must read raw .spg json_path"
    );

    let mut line2 = String::new();
    stdout_reader.read_line(&mut line2).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(&line2).expect("resp2 must be valid JSON");
    assert_eq!(resp2["ok"].as_bool(), Some(true));
    assert!(
        resp2["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("GRAPH_RELOADED")),
        "reload diagnostics must contain GRAPH_RELOADED"
    );

    let mut line3 = String::new();
    stdout_reader.read_line(&mut line3).unwrap();
    let resp3: serde_json::Value = serde_json::from_str(&line3).expect("resp3 must be valid JSON");
    assert_eq!(resp3["ok"].as_bool(), Some(true));
    assert!(
        has_raw_index_action_json_path(&resp3["result"]),
        "after reload must still read raw .spg json_path; project_dir must be preserved"
    );

    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_file(&external_db);
}

/// M27 回归：check_reload 成功 reload 后仍保留显式 project_dir
#[test]
fn test_stdio_server_check_reload_preserves_project_dir_for_external_graphdb() {
    let (project_dir, db_path) = common::build_fixture_graphdb();
    let bin = env!("CARGO_BIN_EXE_metadata-checker");

    let external_db = std::env::temp_dir().join(format!(
        "m27-check-reload-external-{}.graphdb",
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

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut stdout_reader = std::io::BufReader::new(stdout);

    let before = serde_json::json!({
        "request_id": "before",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false
    });
    writeln!(stdin, "{}", before).unwrap();
    stdin.flush().unwrap();

    let mut line1 = String::new();
    stdout_reader.read_line(&mut line1).unwrap();
    let resp1: serde_json::Value = serde_json::from_str(&line1).expect("resp1 must be valid JSON");
    assert_eq!(resp1["ok"].as_bool(), Some(true));
    assert!(
        has_raw_index_action_json_path(&resp1["result"]),
        "before check_reload must read raw .spg json_path"
    );

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::copy(&db_path, &external_db).expect("touch external graphdb by copying valid db");

    let after = serde_json::json!({
        "request_id": "after",
        "command": "query_page_logic",
        "target": "page:app/actions_test.spg",
        "budget": "compact",
        "human": false,
        "check_reload": true
    });
    writeln!(stdin, "{}", after).unwrap();
    stdin.flush().unwrap();

    let mut line2 = String::new();
    stdout_reader.read_line(&mut line2).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(&line2).expect("resp2 must be valid JSON");
    assert_eq!(resp2["ok"].as_bool(), Some(true));
    assert!(
        resp2["diagnostics"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .any(|d| d.as_str().unwrap_or("").contains("GRAPH_RELOADED")),
        "check_reload diagnostics must contain GRAPH_RELOADED"
    );
    assert!(
        has_raw_index_action_json_path(&resp2["result"]),
        "after check_reload reload must still read raw .spg json_path; project_dir must be preserved"
    );

    drop(stdin);
    let _ = child.wait();
    let _ = std::fs::remove_file(&external_db);
}

/// M27 真实项目回归：stdio query_model fact_qwSidebar 必须包含跨页 writer
#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_stdio_query_model_fact_qw_sidebar() {
    let project_dir = std::path::Path::new(
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
    );
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
    assert!(
        has_writer,
        "fact_qwSidebar writers must include 潜客信息跟进 action1/action4"
    );

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

#[test]
fn test_stdio_server_all_registry_commands_accepted() {
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

    fn request_extra(command: &str) -> serde_json::Value {
        match command {
            "explain_condition" | "explain-condition" => {
                serde_json::json!({"target": "comp:app/actions_test.spg|button1"})
            }
            "explain" => serde_json::json!({"target": "page:app/actions_test.spg"}),
            "query_model" | "query-model" => serde_json::json!({"target": "model:model1"}),
            "query_page" | "query-page" => {
                serde_json::json!({"target": "page:app/actions_test.spg"})
            }
            "query_cross" | "query-cross" => serde_json::json!({
                "target": "page:app/actions_test.spg,page:app/page_relations.spg"
            }),
            "query_dataflow" | "query-dataflow" => {
                serde_json::json!({"target": "model:dataflow_a"})
            }
            "query_page_logic" | "query-page-logic" => {
                serde_json::json!({"target": "page:app/actions_test.spg"})
            }
            "find_page" | "find-page" => serde_json::json!({"target": "actions"}),
            "find_model" | "find-model" => serde_json::json!({"target": "model1"}),
            "find_component" | "find-component" => serde_json::json!({"target": "button"}),
            "advise_query" | "advise-query" => serde_json::json!({"target": "model:model1"}),
            "context" => serde_json::json!({"target": "page:app/actions_test.spg", "depth": 1}),
            "status" | "reload_graph" | "reload" | "check_reload" | "check-reload" => {
                serde_json::json!({})
            }
            other => panic!(
                "missing request fixture for registry command alias: {}",
                other
            ),
        }
    }

    let mut commands: Vec<String> = Vec::new();
    for spec in ToolRegistry::all_specs() {
        commands.push(spec.name.to_string());
        commands.extend(spec.aliases.iter().map(|alias| alias.to_string()));
    }

    {
        let mut stdin_lock = stdin;
        for (i, cmd) in commands.iter().enumerate() {
            let extra = request_extra(cmd);
            let mut req = serde_json::json!({
                "request_id": format!("req-all-{}", i),
                "command": cmd,
                "budget": "compact",
                "human": false,
            });
            if let Some(obj) = req.as_object_mut() {
                if let serde_json::Value::Object(extra_obj) = extra {
                    for (k, v) in extra_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            writeln!(stdin_lock, "{}", req).unwrap();
        }
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    for i in 0..commands.len() {
        let mut line = String::new();
        stdout_reader.read_line(&mut line).expect("read line");
        let resp: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_stdio_envelope(&resp, true);
        let code = stdio_error_code(&resp);
        assert_ne!(
            code,
            Some("UNKNOWN_COMMAND"),
            "command '{}' should not be UNKNOWN_COMMAND (got error: {:?})",
            commands[i],
            resp["error"]
        );
    }

    let _ = child.kill();
}

#[test]
fn test_stdio_server_query_cross_invalid_target_returns_invalid_argument() {
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
        "request_id": "req-query-cross-invalid",
        "command": "query_cross",
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
    stdout_reader.read_line(&mut line).expect("read line");
    let resp: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    assert_stdio_envelope(&resp, false);
    assert_eq!(stdio_error_code(&resp), Some("INVALID_ARGUMENT"));

    let _ = child.kill();
}

#[test]
fn test_stdio_server_initializer_commands_are_unknown() {
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
    let commands = ["build_graph", "refresh_index", "rebuild_graph"];

    {
        let mut stdin_lock = stdin;
        for command in commands {
            let req = serde_json::json!({
                "request_id": format!("req-{}", command),
                "command": command,
                "budget": "compact",
                "human": false
            });
            writeln!(stdin_lock, "{}", req).unwrap();
        }
        stdin_lock.flush().unwrap();
        drop(stdin_lock);
    }

    for command in commands {
        let mut line = String::new();
        stdout_reader.read_line(&mut line).expect("read line");
        let resp: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
        assert_stdio_envelope(&resp, false);
        assert_eq!(
            stdio_error_code(&resp),
            Some("UNKNOWN_COMMAND"),
            "initializer command '{}' must not be exposed as runtime tool",
            command
        );
    }

    let _ = child.kill();
}

#[test]
fn test_stdio_server_check_reload_command_returns_unchanged_or_reloaded() {
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
        "request_id": "req-check-reload",
        "command": "check_reload",
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
    stdout_reader.read_line(&mut line).expect("read line");
    let resp: serde_json::Value = serde_json::from_str(&line).expect("valid JSON");
    assert_stdio_envelope(&resp, true);
    let diagnostics: Vec<String> = resp["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    assert!(
        diagnostics.contains(&"GRAPH_UNCHANGED".to_string())
            || diagnostics.contains(&"GRAPH_RELOADED".to_string()),
        "check_reload should produce GRAPH_UNCHANGED or GRAPH_RELOADED diagnostics, got: {:?}",
        diagnostics
    );

    let _ = child.kill();
}
