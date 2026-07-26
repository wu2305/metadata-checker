#![cfg(feature = "cli-local")]
//! Phase 1：同一 session 的重新绑定路径与凭证边界。
//!
//! rebind 通过重新创建进程内 `DiffRefreshRuntimeContext` 完成；凭据只用于
//! 本次登录，不进入 session manifest，也不由 context 持有。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};

use metadata_checker::session::{DiffRefreshRuntimeContext, SessionManager};

fn serve_two_successful_logins() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind rebind test server");
    let address = listener
        .local_addr()
        .expect("read rebind test server address");
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("accept rebind login");
            let mut request = [0_u8; 4096];
            let bytes = stream.read(&mut request).expect("read rebind login");
            let request_text = String::from_utf8_lossy(&request[..bytes]);
            assert_eq!(request_text.contains("POST /api/auth/signin"), true);
            let body = r#"{"ok":true}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write login response");
        }
    });
    (format!("http://{address}"), handle)
}

/// 同一 session 可用新凭据重新绑定，且 manifest 不保存任一轮密码或会话秘密。
#[test]
fn m57_rebind_same_session_does_not_persist_credentials() {
    let root = std::env::temp_dir().join(format!(
        "metadata-checker-m57-rebind-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let manager = SessionManager::new(&root);
    let (server, server_thread) = serve_two_successful_logins();
    manager
        .create_session("s1", &server, "project", "project", "remote")
        .expect("create rebind session");

    let first =
        DiffRefreshRuntimeContext::bind_bi_session(manager.clone(), "s1", "user", "first-password")
            .expect("first session binding");
    drop(first);
    let second = DiffRefreshRuntimeContext::bind_bi_session(
        manager.clone(),
        "s1",
        "user",
        "second-password",
    )
    .expect("rebind session with new credentials");
    drop(second);

    let manifest =
        std::fs::read_to_string(manager.manifest_path("s1")).expect("read rebind session manifest");
    assert_eq!(manifest.contains("first-password"), false);
    assert_eq!(manifest.contains("second-password"), false);
    assert_eq!(manifest.contains("password"), false);
    assert_eq!(manifest.contains("token"), false);
    assert_eq!(manifest.contains("cookie"), false);

    server_thread.join().expect("rebind server thread");
    let _ = std::fs::remove_dir_all(root);
}
