#![cfg(feature = "cli-local")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use lz_str::{compress_to_base64, compress_to_encoded_uri_component};
use metadata_checker::remote_metadata::RemoteFileRef;
use metadata_checker::session::RemoteSessionProvider;
use metadata_checker::session::reqwest_provider::ReqwestRemoteSessionProvider;

fn serve_once(status: u16, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("read test server addr");
    let body = body.to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        let mut request = [0_u8; 2048];
        let _ = stream.read(&mut request);
        let status_text = match status {
            200 => "OK",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Error",
        };
        let response = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
            body.len(),
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    format!("http://{addr}")
}

fn serve_sequence(responses: Vec<(&'static str, u16, &'static str)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("read test server addr");
    thread::spawn(move || {
        for (expected_path, status, body) in responses {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 4096];
            let n = stream.read(&mut request).expect("read request");
            let req_str = String::from_utf8_lossy(&request[..n]);
            assert!(
                req_str.contains(expected_path),
                "expected request path {expected_path}, got {req_str}"
            );
            let status_text = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                404 => "Not Found",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {status_text}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        }
    });
    format!("http://{addr}")
}

fn file_ref(project: &str, source_path: &str, file_id: &str) -> RemoteFileRef {
    RemoteFileRef::try_new(project, source_path, Some(file_id.to_string())).expect("valid file ref")
}

#[test]
fn list_projects_200_json() {
    let body = r#"{"metaProjects":[{"projectName":"xiaoshouyi","desc":"销售项目","path":"/xiaoshouyi","id":"p1","name":"xiaoshouyi","modifyTime":1711111111111,"isFolder":false}]}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, body)).unwrap();
    let projects = provider.list_projects().unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_ref, "xiaoshouyi");
    assert_eq!(projects[0].project_name, "销售项目");
    assert_eq!(projects[0].source_origin, "/xiaoshouyi");
}

#[test]
fn list_projects_200_compressed_success() {
    let raw = r#"{"metaProjects":[{"projectName":"alpha","desc":"alpha project","path":"/alpha","revision":"3"}]}"#;
    let compressed = compress_to_base64(raw);
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, &compressed)).unwrap();
    let projects = provider.list_projects().unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_ref, "alpha");
    assert_eq!(projects[0].project_name, "alpha project");
}

#[test]
fn list_projects_200_quoted_compressed_success() {
    let raw = r#"{"metaProjects":[{"projectName":"quoted","desc":"quoted project"}]}"#;
    let compressed_json_string = serde_json::to_string(&compress_to_base64(raw)).unwrap();
    let provider =
        ReqwestRemoteSessionProvider::new(serve_once(200, &compressed_json_string)).unwrap();
    let projects = provider.list_projects().unwrap();

    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_ref, "quoted");
    assert_eq!(projects[0].project_name, "quoted project");
}

#[test]
fn list_projects_compressed_but_invalid_should_error() {
    let provider =
        ReqwestRemoteSessionProvider::new(serve_once(200, "not-a-valid-lz-string")).unwrap();
    let err = provider.list_projects().unwrap_err();

    assert!(err.to_string().contains("LZString decompression failed"));
}

#[test]
fn list_metafiles_200_filter_folders() {
    let body = r#"[
        {"id":"file-a","path":"/xiaoshouyi/app/page.spg","name":"page.spg","revision":"1","modifyTime":111,"isFolder":false},
        {"id":"folder-1","path":"/xiaoshouyi/app/folder","name":"folder","isFolder":true},
        {"id":"file-b","path":"/xiaoshouyi/data/tables/tb.tbl","name":"tb.tbl","revision":"2","modifyTime":222}
    ]"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, body)).unwrap();

    let files = provider.list_metafiles("xiaoshouyi").unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].source_path, "app/page.spg");
    assert_eq!(files[0].file_id, Some("file-a".to_string()));
    assert_eq!(files[1].source_path, "data/tables/tb.tbl");
    assert_eq!(files[1].file_id, Some("file-b".to_string()));
}

#[test]
fn list_metafiles_200_compressed_success() {
    let raw = r#"{"children":[
        {"id":"file-a","path":"/xiaoshouyi/app/page.spg","name":"page.spg","revision":"1","modifyTime":111,"isFolder":false},
        {"id":"folder-1","path":"/xiaoshouyi/app/folder","name":"folder","isFolder":true}
    ]}"#;
    let compressed = compress_to_base64(raw);
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, &compressed)).unwrap();

    let files = provider.list_metafiles("xiaoshouyi").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].source_path, "app/page.spg");
    assert_eq!(files[0].file_id, Some("file-a".to_string()));
}

#[test]
fn list_metafiles_200_uri_component_compressed_success() {
    let raw = r#"{"children":[
        {"id":"file-a","path":"/xiaoshouyi/app/page.spg","name":"page.spg","revision":"1","modifyTime":111,"isFolder":false}
    ]}"#;
    let compressed = compress_to_encoded_uri_component(raw);
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, &compressed)).unwrap();

    let files = provider.list_metafiles("xiaoshouyi").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].source_path, "app/page.spg");
    assert_eq!(files[0].file_id, Some("file-a".to_string()));
}

#[test]
fn list_metafiles_falls_back_to_project_children_and_module_descendants() {
    let children_body = r#"{"file":{"name":"proj","type":"project","projectName":"proj","isFolder":true},"children":[
        {"id":"app-folder","name":"app","projectName":"proj","parentDir":"/proj","isFolder":true},
        {"id":"root-page","name":"Root.spg","projectName":"proj","parentDir":"/proj","revision":"1","isFolder":false}
    ]}"#;
    let descendant_body = r#"{"project":{"projectName":"proj"},"module":{"name":"app"},"files":[
        {"id":"app-folder-file","name":"App.app","projectName":"proj","parentDir":"/proj/app","isFolder":true},
        {"id":"page-file","name":"Page.spg","projectName":"proj","parentDir":"/proj/app/App.app","revision":"2","isFolder":false}
    ]}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_sequence(vec![
        ("/api/meta/services/getFileDescendant/proj", 400, "{}"),
        (
            "/api/meta/services/getFileChildren/proj",
            200,
            children_body,
        ),
        (
            "/api/meta/services/getFileDescendant/proj/app",
            200,
            descendant_body,
        ),
    ]))
    .unwrap();

    let files = provider.list_metafiles("proj").unwrap();

    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| {
        file.source_path == "Root.spg" && file.file_id == Some("root-page".to_string())
    }));
    assert!(files.iter().any(|file| {
        file.source_path == "app/App.app/Page.spg" && file.file_id == Some("page-file".to_string())
    }));
}

#[test]
fn fetch_metafile_info_200_wrapper() {
    let body = r#"{"data":{"file":{"id":"file-a","path":"/xiaoshouyi/app/page.spg","name":"page.spg","revision":"3","modifyTime":333}}}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, body)).unwrap();
    let info = provider
        .fetch_metafile_info(&file_ref("xiaoshouyi", "app/page.spg", "file-a"))
        .unwrap();

    assert_eq!(info.source_path, "app/page.spg");
    assert_eq!(info.file_id, Some("file-a".to_string()));
    assert_eq!(info.revision, Some("3".to_string()));
}

#[test]
fn fetch_metafile_content_200_text() {
    let body = r#"{"canvas":{"components":[]}}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, body)).unwrap();
    let content = provider
        .fetch_metafile_content(&file_ref("xiaoshouyi", "app/page.spg", "file-a"))
        .unwrap();

    assert_eq!(content.source_path, "app/page.spg");
    assert_eq!(content.file_id, Some("file-a".to_string()));
    assert_eq!(content.raw_text, r#"{"canvas":{"components":[]}}"#);
}

#[test]
fn fetch_metafile_content_200_compressed_text() {
    let body = r#"{"canvas":{"components":[]}}"#;
    let compressed = compress_to_base64(body);
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, &compressed)).unwrap();
    let content = provider
        .fetch_metafile_content(&file_ref("xiaoshouyi", "app/page.spg", "file-a"))
        .unwrap();

    assert_eq!(content.raw_text, body);
}

#[test]
fn fetch_metafile_content_200_uri_component_compressed_text() {
    let body = r#"{"canvas":{"components":[]}}"#;
    let compressed = compress_to_encoded_uri_component(body);
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, &compressed)).unwrap();
    let content = provider
        .fetch_metafile_content(&file_ref("xiaoshouyi", "app/page.spg", "file-a"))
        .unwrap();

    assert_eq!(content.raw_text, body);
}

#[test]
fn request_status_errors_and_empty_body() {
    for (status, expect) in [
        (401, "401 Unauthorized"),
        (403, "403 Forbidden"),
        (404, "404 Not Found"),
    ] {
        let provider = ReqwestRemoteSessionProvider::new(serve_once(status, "{}")).unwrap();
        let err = provider.list_projects().unwrap_err();
        assert!(
            err.to_string().contains(expect),
            "unexpected error for status {status}: {err}"
        );
    }

    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, "")).unwrap();
    let err = provider
        .fetch_metafile_content(&file_ref("xiaoshouyi", "app/page.spg", "file-a"))
        .unwrap_err();
    assert!(err.to_string().contains("empty"));
}

#[test]
fn login_success_and_reuse_cookie() {
    let login_body = r#"{"ok":true}"#;
    let projects_body = r#"{"metaProjects":[{"projectName":"test","desc":"test"}]}"#;
    let (request_sender, request_receiver) = std::sync::mpsc::channel::<(String, String)>();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("read test server addr");

    thread::spawn(move || {
        for (path, status, body, set_cookie) in [
            (
                "/api/auth/signin",
                200,
                login_body,
                Some("JSESSIONID=abc; Path=/"),
            ),
            ("/api/me/getPermissionInfo", 200, projects_body, None),
        ] {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut request = [0_u8; 2048];
            let n = stream.read(&mut request).expect("read request");
            let req_str = String::from_utf8_lossy(&request[..n]);
            assert!(req_str.contains(path), "expected request to {}", path);
            request_sender
                .send((path.to_string(), req_str.to_string()))
                .expect("send captured request");

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

    let url = format!("http://{}", addr);
    let provider = ReqwestRemoteSessionProvider::new(&url).unwrap();

    provider
        .login("user", "pass", "sys")
        .expect("login should succeed");

    let projects = provider
        .list_projects()
        .expect("list_projects should succeed");
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].project_ref, "test");

    let (_, login_request) = request_receiver.recv().expect("capture login request");
    let (_, permission_request) = request_receiver
        .recv()
        .expect("capture permission info request");

    assert!(login_request.contains("POST /api/auth/signin"));
    assert!(permission_request.contains("GET /api/me/getPermissionInfo"));
    assert!(
        permission_request
            .to_ascii_lowercase()
            .contains("cookie: jsessionid=abc")
    );
}

#[test]
fn login_401_returns_stable_error() {
    let body = r#"{"ok":false,"message":"invalid credentials"}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(401, body)).unwrap();
    let err = provider.login("user", "pass", "sys").unwrap_err();
    assert!(err.to_string().contains("401"));
    assert!(
        !err.to_string().contains("pass"),
        "error must not leak password"
    );
}

#[test]
fn login_403_returns_stable_error() {
    let provider = ReqwestRemoteSessionProvider::new(serve_once(403, "")).unwrap();
    let err = provider.login("user", "pass", "sys").unwrap_err();
    assert!(err.to_string().contains("403"));
}

#[test]
fn login_invalid_json_returns_stable_error() {
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, "not-json")).unwrap();
    let err = provider.login("user", "pass", "sys").unwrap_err();
    assert!(err.to_string().contains("JSON"));
    assert!(
        !err.to_string().contains("pass"),
        "error must not leak password"
    );
}

#[test]
fn login_failed_message_is_sanitized() {
    let body = r#"{"ok":false,"message":"password=secret token=abc cookie=JSESSIONID cipherPassport=xxx set-cookie=JSESSIONID=abc"}"#;
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, body)).unwrap();
    let err = provider.login("user", "pass", "sys").unwrap_err();

    assert!(!err.to_string().contains("password=secret"));
    assert!(!err.to_string().contains("token=abc"));
    assert!(!err.to_string().contains("cookie=JSESSIONID"));
    assert!(!err.to_string().contains("cipherPassport=xxx"));
    assert!(!err.to_string().contains("set-cookie="));
    assert!(err.to_string().contains("login failed"));
}

#[test]
fn login_empty_body_returns_stable_error() {
    let provider = ReqwestRemoteSessionProvider::new(serve_once(200, "")).unwrap();
    let err = provider.login("user", "pass", "sys").unwrap_err();
    assert!(err.to_string().contains("empty"));
    assert!(
        !err.to_string().contains("pass"),
        "error must not leak password"
    );
}
