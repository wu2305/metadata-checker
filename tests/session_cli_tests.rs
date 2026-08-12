#![cfg(feature = "cli-local")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use metadata_checker::session::SessionManager;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_metadata-checker")
}

fn test_root(name: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir().join(format!(
        "metadata-checker-session-cli-{name}-{}-{id}",
        std::process::id()
    ))
}

fn run_session_command(root: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let output = run_session_command_raw(root, args);
    assert!(
        output.status.success(),
        "command failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

fn run_session_command_raw(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    let output = Command::new(bin_path())
        .arg("--session-dir")
        .arg(root)
        .args(args)
        .output()
        .expect("session command must run");
    output
}

fn as_json_from_stdout(output: &std::process::Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("stdout must be JSON")
}

#[test]
fn session_list_empty_directory_returns_empty_array() {
    let root = test_root("list-empty");
    let json = run_session_command(&root, &["--session-list"]);

    assert_eq!(json["ok"], true);
    assert_eq!(json["sessions"].as_array().unwrap().len(), 0);
    assert!(
        json["session_dir"]
            .as_str()
            .unwrap()
            .contains("session-cli")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_show_status_and_delete_existing_manifest() {
    let root = test_root("show-status-delete");
    let manager = SessionManager::new(&root);
    manager
        .create_session(
            "s1",
            "https://autocrm-test.xiaoshouyi.com",
            "analyzer",
            "analyzer",
            "remote",
        )
        .expect("create session fixture");
    let project_dir = root.join("s1").join("project").join("app");
    std::fs::create_dir_all(&project_dir).expect("create mirror dir");
    std::fs::write(project_dir.join("Page.spg"), "{}").expect("write mirror file");

    let listed = run_session_command(&root, &["--session-list"]);
    assert_eq!(listed["sessions"], serde_json::json!(["s1"]));

    let shown = run_session_command(&root, &["--session-show", "s1"]);
    assert_eq!(shown["ok"], true);
    assert_eq!(shown["manifest"]["session_id"], "s1");
    assert_eq!(shown["manifest"]["project_ref"], "analyzer");

    let status = run_session_command(&root, &["--session-status", "s1"]);
    assert_eq!(status["ok"], true);
    assert_eq!(status["session_id"], "s1");
    assert_eq!(status["project_ref"], "analyzer");

    let deleted = run_session_command(&root, &["--session-delete", "s1"]);
    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["deleted"], "s1");
    assert!(!root.join("s1").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_show_missing_session_returns_stable_error_envelope() {
    let root = test_root("show-missing");
    let output = run_session_command_raw(&root, &["--session-show", "ghost"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json = as_json_from_stdout(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_NOT_FOUND");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("failed to read session manifest")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_status_missing_session_returns_stable_error_envelope() {
    let root = test_root("status-missing");
    let output = run_session_command_raw(&root, &["--session-status", "ghost"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json = as_json_from_stdout(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_NOT_FOUND");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("failed to read session manifest")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_show_invalid_session_id_returns_stable_error_code() {
    let root = test_root("show-invalid");
    let output = run_session_command_raw(&root, &["--session-show", "../bad"]);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json = as_json_from_stdout(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_INVALID_ID");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("invalid session_id")
    );
    assert!(!root.join("bad").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_delete_missing_session_is_idempotent_success() {
    let root = test_root("delete-missing");
    let deleted = run_session_command(&root, &["--session-delete", "ghost"]);

    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["deleted"], "ghost");
    assert!(!root.join("ghost").exists());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_missing_server_returns_stable_error() {
    let root = test_root("refresh-missing-server");
    let output = run_session_command_raw(
        &root,
        &["--session-refresh", "s1", "--remote-project", "analyzer"],
    );
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_MISSING_REMOTE_SERVER");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("remote-server")
    );
    let text = serde_json::to_string(&json).unwrap();
    assert!(!text.contains("password"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_missing_project_returns_stable_error() {
    let root = test_root("refresh-missing-project");
    let output = run_session_command_raw(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--remote-server",
            "https://example.com",
        ],
    );
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_MISSING_REMOTE_PROJECT");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("remote-project")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_alias_remote_index_missing_server_returns_stable_error() {
    let root = test_root("refresh-missing-server-alias");
    let output = run_session_command_raw(&root, &["--remote-index", "s1", "--project", "analyzer"]);
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_MISSING_REMOTE_SERVER");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("base-url")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_missing_project_with_base_url_alias_returns_stable_error() {
    let root = test_root("refresh-missing-project-alias");
    let output = run_session_command_raw(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--base-url",
            "https://example.com",
        ],
    );
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_MISSING_REMOTE_PROJECT");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("project")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_invalid_sync_mode_returns_stable_error() {
    let root = test_root("refresh-invalid-mode");
    let output = run_session_command_raw(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--remote-server",
            "https://example.com",
            "--remote-project",
            "analyzer",
            "--session-sync-mode",
            "bad-mode",
        ],
    );
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);

    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_INVALID_SYNC_MODE");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_login_error_message_is_sanitized() {
    let root = test_root("refresh-login-error");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");

    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut request = [0_u8; 2048];
        let n = stream.read(&mut request).expect("read request");
        let request = String::from_utf8_lossy(&request[..n]).to_string();
        assert!(
            request.contains("/api/auth/signin"),
            "expected request to signin, got: {request}"
        );

        let body = r#"{"ok":false,"message":"password=secret token=abc cookie=JSESSIONID cipherPassport=xxx set-cookie=JSESSIONID=abc"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });

    let output = run_session_command_raw(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--remote-server",
            &format!("http://{addr}"),
            "--remote-project",
            "analyzer",
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
        ],
    );
    assert!(output.status.success());
    let json = as_json_from_stdout(&output);
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "SESSION_AUTH_REQUIRED");

    let json_text = serde_json::to_string(&json).expect("serialize output");
    assert!(!json_text.contains("password=secret"));
    assert!(!json_text.contains("token=abc"));
    assert!(!json_text.contains("JSESSIONID"));
    assert!(!json_text.contains("cipherPassport=xxx"));
    assert!(!json_text.contains("set-cookie="));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_full_mode_success_with_mock_remote_server() {
    let root = test_root("refresh-success");
    let minimal_superpage = r#"{"pageName":"Page","components":[]}"#;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let (request_sender, request_receiver) = mpsc::channel::<(String, String)>();

    thread::spawn(move || {
        for (path, status, body, set_cookie) in [
            (
                "/api/auth/signin",
                200,
                r#"{"ok":true}"#,
                Some("JSESSIONID=abc; Path=/"),
            ),
            (
                "/api/meta/services/getFileDescendant/proj",
                200,
                r#"{"children":[{"path":"/proj/app/Page.spg","name":"Page.spg","id":"spg1","revision":"1","isFolder":false}]}"#,
                None,
            ),
            (
                "/api/meta/services/getFileContent/spg1",
                200,
                minimal_superpage,
                None,
            ),
        ] {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();

            let request_line = request.lines().next().unwrap_or_default();
            let method = if path == "/api/auth/signin" {
                "POST"
            } else {
                "GET"
            };
            assert!(
                request_line == format!("{method} {path} HTTP/1.1"),
                "expected exact request line for {path}, got: {request_line}"
            );

            if path != "/api/auth/signin" {
                let request_lower = request.to_ascii_lowercase();
                assert!(
                    request_lower.contains("cookie: jsessionid=abc"),
                    "expected cookie for {path}, got: {request}"
                );
            }

            request_sender
                .send((path.to_string(), request))
                .expect("send request snapshot");

            let set_cookie_header = set_cookie
                .map(|value| format!("Set-Cookie: {value}\r\n"))
                .unwrap_or_default();

            let response = format!(
                "HTTP/1.1 {status} OK\r\n{set_cookie_header}Content-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let mock_url = format!("http://{addr}");
    let json = run_session_command(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--remote-server",
            &mock_url,
            "--remote-project",
            "proj",
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
            "--session-sync-mode",
            "full",
        ],
    );

    let (_, signin_request) = request_receiver.recv().expect("capture signin request");
    let (_, descendants_request) = request_receiver
        .recv()
        .expect("capture descendants request");
    let (_, content_request) = request_receiver.recv().expect("capture content request");

    assert!(signin_request.contains("POST /api/auth/signin"));
    assert!(
        descendants_request
            .to_ascii_lowercase()
            .contains("cookie: jsessionid=abc"),
        "expected signin cookie on descendants"
    );
    assert!(
        content_request
            .to_ascii_lowercase()
            .contains("cookie: jsessionid=abc"),
        "expected signin cookie on content"
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["session_id"], "s1");
    assert_eq!(json["project_ref"], "proj");
    assert!(
        json["sync"]["written"]
            .as_u64()
            .expect("written must be integer")
            >= 1
    );
    let manifest_path = root.join("s1").join("session.json");
    assert!(manifest_path.exists());
    let manifest_text = std::fs::read_to_string(&manifest_path).expect("read session manifest");
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("manifest should be valid JSON");
    assert_eq!(manifest_json["session_id"].as_str(), Some("s1"));
    assert_eq!(manifest_json["project_ref"].as_str(), Some("proj"));

    let manifest_files = manifest_json["files"]
        .as_array()
        .expect("manifest files should be array");
    let page_entry = manifest_files
        .iter()
        .find(|file| file["source_path"] == "app/Page.spg")
        .expect("manifest files should contain app/Page.spg");
    assert_eq!(page_entry["revision"].as_str(), Some("1"));
    assert_eq!(page_entry["deleted"].as_bool(), Some(false));

    let mirror_path = root.join("s1").join("project").join("app").join("Page.spg");
    assert!(mirror_path.exists());
    let mirror_text = std::fs::read_to_string(&mirror_path).expect("read mirrored Page.spg");
    assert_eq!(mirror_text, minimal_superpage);

    let graph_db_path = std::path::Path::new(
        json["graph_db_path"]
            .as_str()
            .expect("graph_db_path must be string"),
    );
    assert!(graph_db_path.exists());

    let json_text = serde_json::to_string(&json).expect("serialize output");
    assert!(!json_text.contains("pass"));
    assert!(!json_text.contains("JSESSIONID"));
    assert!(!json_text.contains("cookie"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_remote_index_alias_success_reuses_mock_server() {
    let root = test_root("refresh-success-alias");
    let minimal_superpage = r#"{"pageName":"Page","components":[]}"#;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let (request_sender, request_receiver) = mpsc::channel::<(String, String)>();

    thread::spawn(move || {
        for (path, status, body, set_cookie) in [
            (
                "/api/auth/signin",
                200,
                r#"{"ok":true}"#,
                Some("JSESSIONID=abc; Path=/"),
            ),
            (
                "/api/meta/services/getFileDescendant/proj",
                200,
                r#"{"children":[{"path":"/proj/app/Page.spg","name":"Page.spg","id":"spg1","revision":"1","isFolder":false}]}"#,
                None,
            ),
            (
                "/api/meta/services/getFileContent/spg1",
                200,
                minimal_superpage,
                None,
            ),
        ] {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();

            let request_line = request.lines().next().unwrap_or_default();
            let method = if path == "/api/auth/signin" {
                "POST"
            } else {
                "GET"
            };
            assert!(
                request_line == format!("{method} {path} HTTP/1.1"),
                "expected exact request line for {path}, got: {request_line}"
            );

            if path != "/api/auth/signin" {
                let request_lower = request.to_ascii_lowercase();
                assert!(
                    request_lower.contains("cookie: jsessionid=abc"),
                    "expected cookie for {path}, got: {request}"
                );
            }

            request_sender
                .send((path.to_string(), request))
                .expect("send request snapshot");

            let set_cookie_header = set_cookie
                .map(|value| format!("Set-Cookie: {value}\r\n"))
                .unwrap_or_default();

            let response = format!(
                "HTTP/1.1 {status} OK\r\n{set_cookie_header}Content-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let mock_url = format!("http://{addr}");
    let json = run_session_command(
        &root,
        &[
            "--remote-index",
            "s2",
            "--base-url",
            &mock_url,
            "--project",
            "proj",
            "--remote-source",
            "app/Page.spg",
            "--remote-module",
            "app",
            "--remote-file",
            "spg1",
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
        ],
    );

    let (_, signin_request) = request_receiver.recv().expect("capture signin request");
    let (_, descendants_request) = request_receiver
        .recv()
        .expect("capture descendants request");
    let (_, content_request) = request_receiver.recv().expect("capture content request");

    assert!(signin_request.contains("POST /api/auth/signin"));
    assert!(
        descendants_request
            .to_ascii_lowercase()
            .contains("cookie: jsessionid=abc"),
        "expected signin cookie on descendants"
    );
    assert!(
        content_request
            .to_ascii_lowercase()
            .contains("cookie: jsessionid=abc"),
        "expected signin cookie on content"
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["session_id"], "s2");
    assert_eq!(json["project_ref"], "proj");
    assert_eq!(json["files"]["discovered"], 1);
    assert_eq!(json["files"]["analyzable"], 1);
    assert_eq!(json["files"]["synced"], 1);
    assert_eq!(json["files"]["failed"], 0);
    assert_eq!(json["diagnostics"].as_array().unwrap().len(), 0);

    let manifest_text = std::fs::read_to_string(root.join("s2").join("session.json"))
        .expect("read session manifest");
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("manifest should be valid JSON");
    assert_eq!(manifest_json["session_id"].as_str(), Some("s2"));
    assert_eq!(manifest_json["project_ref"].as_str(), Some("proj"));

    let page_entry = manifest_json["files"]
        .as_array()
        .expect("manifest files should be array")
        .iter()
        .find(|file| file["source_path"] == "app/Page.spg")
        .expect("manifest files should contain app/Page.spg");
    assert_eq!(page_entry["revision"].as_str(), Some("1"));

    let json_text = serde_json::to_string(&json).expect("serialize output");
    assert!(!json_text.contains("pass"));
    assert!(!json_text.contains("JSESSIONID"));
    assert!(!json_text.contains("cookie"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_remote_filter_reduces_fetch_and_write() {
    let root = test_root("refresh-filter");
    let page_a = r#"{"pageName":"PageA","components":[]}"#;
    let page_b = r#"{"pageName":"PageB","components":[]}"#;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let (request_sender, request_receiver) = mpsc::channel::<String>();

    thread::spawn(move || {
        for _ in 0..8 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let request_line = request.lines().next().unwrap_or_default();
            let request_path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            request_sender
                .send(request_line.to_string())
                .expect("send request snapshot");

            if request_path == "/api/auth/signin" {
                let body = r#"{"ok":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: JSESSIONID=abc; Path=/\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }

            if request_path == "/api/meta/services/getFileDescendant/proj" {
                let body = r#"{"children":[{"path":"/proj/app/PageA.spg","name":"PageA.spg","id":"pageA","revision":"1","isFolder":false},{"path":"/proj/app/PageB.spg","name":"PageB.spg","id":"pageB","revision":"1","isFolder":false},{"path":"/proj/data/tables/test.tbl","name":"test.tbl","id":"tbl1","revision":"1","isFolder":false}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }

            if let Some(file_id) = request_path.strip_prefix("/api/meta/services/getFileContent/") {
                let body = match file_id {
                    "pageA" => page_a,
                    "pageB" => page_b,
                    "tbl1" => "a,b,c\n1,2,3\n",
                    _ => "{\"ok\":false}",
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }

            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let mock_url = format!("http://{addr}");
    let json = run_session_command(
        &root,
        &[
            "--session-refresh",
            "s1",
            "--remote-server",
            &mock_url,
            "--project",
            "proj",
            "--remote-module",
            "app",
            "--remote-file",
            "pageA",
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
        ],
    );

    let mut requests = Vec::new();
    while let Ok(req) = request_receiver.recv_timeout(Duration::from_millis(10)) {
        requests.push(req);
    }
    assert_eq!(
        requests
            .iter()
            .filter(|line| line.starts_with("GET /api/meta/services/getFileContent"))
            .count(),
        1
    );
    assert!(
        requests
            .iter()
            .any(|line| line.contains("getFileContent/pageA"))
    );
    assert!(
        !requests
            .iter()
            .any(|line| line.contains("getFileContent/pageB"))
    );

    assert_eq!(json["ok"], true);
    assert_eq!(json["files"]["synced"].as_u64().expect("synced"), 1);

    let mirror_root = root.join("s1").join("project");
    assert!(mirror_root.join("app").join("PageA.spg").exists());
    assert!(!mirror_root.join("app").join("PageB.spg").exists());
    assert!(
        !mirror_root
            .join("data")
            .join("tables")
            .join("test.tbl")
            .exists()
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_refresh_remote_index_query_page_outputs_query_result() {
    let root = test_root("refresh-query");
    let page = r#"{"pageName":"Foo","components":[]}"#;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let (request_sender, request_receiver) = mpsc::channel::<String>();
    let mock_url = format!("http://{addr}");

    thread::spawn(move || {
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let request_line = request.lines().next().unwrap_or_default();
            request_sender
                .send(request_line.to_string())
                .expect("send request snapshot");
            let request_path = request_line.split_whitespace().nth(1).unwrap_or("");
            if request_path == "/api/auth/signin" {
                let body = r#"{"ok":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: JSESSIONID=abc; Path=/\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileDescendant/proj" {
                let body = r#"{"children":[{"path":"/proj/app/Foo.spg","name":"Foo.spg","id":"foo-id","revision":"1","isFolder":false}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileContent/foo-id" {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let output = run_session_command_raw(
        &root,
        &[
            "--remote-index",
            "s1",
            "--base-url",
            &mock_url,
            "--project",
            "proj",
            "--query-page",
            "page:app/Foo.spg",
            "--remote-file",
            "foo-id",
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
        ],
    );

    assert!(output.status.success());
    let json = as_json_from_stdout(&output);
    assert!(!json.get("error").is_some());
    assert_eq!(json["query_target"], "page:app/Foo.spg");
    assert_eq!(json["summary"]["page_id"], "page:app/Foo.spg");
    assert!(!json.as_object().unwrap().contains_key("session_id"));

    let manifest_path = root.join("s1").join("session.json");
    assert!(manifest_path.exists());
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(manifest_path).expect("read session manifest"),
    )
    .expect("manifest should be valid JSON");
    assert_eq!(manifest["session_id"], "s1");

    let mut requests = Vec::new();
    while let Ok(req) = request_receiver.recv_timeout(Duration::from_millis(10)) {
        requests.push(req);
    }
    assert!(
        requests
            .iter()
            .any(|line| line.contains("getFileContent/foo-id"))
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn session_remote_index_respects_graph_db_path_override() {
    let root = test_root("refresh-graph-db");
    let page = r#"{"pageName":"Foo","components":[]}"#;
    let graph_db_path = root.join("custom-session.graphdb");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let (request_sender, request_receiver) = mpsc::channel::<String>();

    thread::spawn(move || {
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let request_line = request.lines().next().unwrap_or_default();
            request_sender
                .send(request_line.to_string())
                .expect("send request snapshot");
            let request_path = request_line.split_whitespace().nth(1).unwrap_or("");
            if request_path == "/api/auth/signin" {
                let body = r#"{"ok":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: JSESSIONID=abc; Path=/\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileDescendant/proj" {
                let body = r#"{"children":[{"path":"/proj/app/Foo.spg","name":"Foo.spg","id":"foo-id","revision":"1","isFolder":false}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileContent/foo-id" {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let mock_url = format!("http://{addr}");
    let json = run_session_command(
        &root,
        &[
            "--remote-index",
            "s1",
            "--base-url",
            &mock_url,
            "--project",
            "proj",
            "--graph-db-path",
            &graph_db_path.to_string_lossy(),
            "--remote-username",
            "user",
            "--remote-password",
            "pass",
        ],
    );

    let _ = request_receiver
        .recv_timeout(Duration::from_millis(10))
        .expect("auth request");
    assert_eq!(json["ok"], true);
    assert_eq!(
        json["graph_db_path"].as_str().unwrap(),
        graph_db_path.to_string_lossy().as_ref(),
    );
    assert!(graph_db_path.exists());

    let query_output = run_session_command_raw(
        &root,
        &[
            "--graph-db-path",
            &graph_db_path.to_string_lossy(),
            "--query-page",
            "page:app/Foo.spg",
        ],
    );
    assert!(query_output.status.success());
    let query_json = as_json_from_stdout(&query_output);
    assert_eq!(query_json["query_target"], "page:app/Foo.spg");

    let manifest_text = std::fs::read_to_string(root.join("s1").join("session.json"))
        .expect("read session manifest");
    let manifest_json: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("manifest should be valid JSON");
    assert_eq!(
        manifest_json["graph_db_path"].as_str().unwrap(),
        graph_db_path.to_string_lossy().as_ref(),
    );

    let _ = std::fs::remove_dir_all(root);
}

/// 带 mock 远端跑一遍 refresh-and-query，返回查询输出的 JSON。
///
/// `--find`/`--relations` 此前不在 `has_session_query_request` 名单里：refresh 照常
/// 完成，但请求的查询被静默跳过，stdout 是 refresh 报告而不是查询结果。
fn run_refresh_query_with_mock_page(tag: &str, query_args: &[&str]) -> serde_json::Value {
    let root = test_root(tag);
    let page = r#"{"pageName":"Foo","components":[]}"#;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
    let addr = listener.local_addr().expect("read mock server addr");
    let mock_url = format!("http://{addr}");

    thread::spawn(move || {
        for _ in 0..6 {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let request = String::from_utf8_lossy(&request[..n]).to_string();
            let request_line = request.lines().next().unwrap_or_default();
            let request_path = request_line.split_whitespace().nth(1).unwrap_or("");
            if request_path == "/api/auth/signin" {
                let body = r#"{"ok":true}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: JSESSIONID=abc; Path=/\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileDescendant/proj" {
                let body = r#"{"children":[{"path":"/proj/app/Foo.spg","name":"Foo.spg","id":"foo-id","revision":"1","isFolder":false}]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            if request_path == "/api/meta/services/getFileContent/foo-id" {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{page}",
                    page.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                continue;
            }
            let response =
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });

    let mut args = vec![
        "--remote-index",
        "s1",
        "--base-url",
        &mock_url,
        "--project",
        "proj",
        "--remote-file",
        "foo-id",
        "--remote-username",
        "user",
        "--remote-password",
        "pass",
    ];
    args.extend_from_slice(query_args);
    let output = run_session_command_raw(&root, &args);
    assert!(
        output.status.success(),
        "command failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = as_json_from_stdout(&output);
    let _ = std::fs::remove_dir_all(root);
    json
}

#[test]
fn session_refresh_remote_index_find_outputs_query_result() {
    let json = run_refresh_query_with_mock_page("refresh-query-find", &["--find", "Foo"]);
    assert!(
        !json.as_object().unwrap().contains_key("session_id"),
        "应输出查询结果而不是 refresh 报告: {json}"
    );
    let matches = json["details"]["matches"]
        .as_array()
        .expect("find 应返回 matches");
    assert!(
        matches
            .iter()
            .any(|entry| entry["id"].as_str() == Some("page:app/Foo.spg")),
        "{matches:?}"
    );
}

#[test]
fn session_refresh_remote_index_relations_outputs_query_result() {
    let json = run_refresh_query_with_mock_page(
        "refresh-query-relations",
        &["--relations", "page:app/Foo.spg"],
    );
    assert!(
        !json.as_object().unwrap().contains_key("session_id"),
        "应输出查询结果而不是 refresh 报告: {json}"
    );
    assert_eq!(json["query_target"], "page:app/Foo.spg");
}
