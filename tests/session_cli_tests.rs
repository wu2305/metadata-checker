#![cfg(feature = "cli-local")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

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
