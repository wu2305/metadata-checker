use std::io::{BufRead, Write};
use std::process::{Command, Stdio};

/// 构建 fixture graphdb 到临时目录
fn build_fixture_graphdb() -> (std::path::PathBuf, std::path::PathBuf) {
    let unique = format!("metadata-checker-stdio-test-{:#?}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());
    let temp_dir = std::env::temp_dir().join(unique);
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let src = std::path::Path::new("tests/fixtures/test_project");
    copy_dir_all(src, &temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");
    metadata_checker::scanner::scan_project(&temp_dir,
        &db_path,
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

/// M24 核心验收：启动 stdio server，连续两次 explain_condition，
/// 第二次 graph_load_ms == 0
#[test]
fn test_stdio_server_reuses_graph() {
    let (_temp_dir, db_path) = build_fixture_graphdb();
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
    let (_temp_dir, db_path) = build_fixture_graphdb();
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
    let (_temp_dir, db_path) = build_fixture_graphdb();
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
