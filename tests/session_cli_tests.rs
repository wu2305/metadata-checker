#![cfg(feature = "cli-local")]

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
