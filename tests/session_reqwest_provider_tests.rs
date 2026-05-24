#![cfg(feature = "cli-local")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use lz_str::compress_to_base64;
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
fn list_projects_compressed_but_invalid_should_error() {
    let provider =
        ReqwestRemoteSessionProvider::new(serve_once(200, "not-a-valid-lz-string")).unwrap();
    let err = provider.list_projects().unwrap_err();

    assert!(
        err.to_string()
            .contains("LZString+Base64 decompression failed")
    );
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
