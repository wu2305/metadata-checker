#![cfg(feature = "cli-local")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

use metadata_checker::remote_metadata::{
    AsyncRemoteMetadataProvider, MetadataContentType, RemoteFileRef, RemoteMetadataErrorCode,
    diff_visible_manifest,
};
use metadata_checker::remote_metadata_provider::ReqwestRemoteMetadataProvider;
use serde_json::json;

fn serve_once(status: u16, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
    let addr = listener.local_addr().expect("read test server addr");
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

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime")
}

fn file_ref() -> RemoteFileRef {
    RemoteFileRef::try_new(
        "analyzer",
        "app/M40HookSmoke.app/M40HookDesign.spg",
        Some("fallback-id".to_string()),
    )
    .expect("valid file ref")
}

#[test]
fn reqwest_provider_reads_wrapped_content_response() {
    let body = r#"{"data":{"file":{"fileId":"remote-id","revision":"11","content":"{\"canvas\":{\"components\":[]}}"}}}"#;
    let provider = ReqwestRemoteMetadataProvider::new(serve_once(200, body)).unwrap();
    let result = runtime()
        .block_on(AsyncRemoteMetadataProvider::get_file_content(
            &provider,
            &file_ref(),
        ))
        .unwrap();

    assert_eq!(result.source_path, "app/M40HookSmoke.app/M40HookDesign.spg");
    assert_eq!(result.file_id, Some("remote-id".to_string()));
    assert_eq!(result.revision, Some("11".to_string()));
    assert_eq!(result.content_type, MetadataContentType::SuperPage);
    assert_eq!(result.raw_text, r#"{"canvas":{"components":[]}}"#);
}

#[test]
fn reqwest_provider_preserves_plain_raw_text_response() {
    let body = r#"{"canvas":{"components":[]}}"#;
    let provider = ReqwestRemoteMetadataProvider::new(serve_once(200, body)).unwrap();
    let result = runtime()
        .block_on(AsyncRemoteMetadataProvider::get_file_content(
            &provider,
            &file_ref(),
        ))
        .unwrap();

    assert_eq!(result.file_id, Some("fallback-id".to_string()));
    assert_eq!(result.revision, None);
    assert_eq!(result.raw_text, body);
}

#[test]
fn reqwest_provider_maps_http_status_codes() {
    for (status, expected) in [
        (401, RemoteMetadataErrorCode::RemoteFetchUnauthorized),
        (403, RemoteMetadataErrorCode::RemoteFetchForbidden),
        (404, RemoteMetadataErrorCode::RemoteFetchNotFound),
    ] {
        let provider = ReqwestRemoteMetadataProvider::new(serve_once(status, "no")).unwrap();
        let err = runtime()
            .block_on(AsyncRemoteMetadataProvider::get_file_content(
                &provider,
                &file_ref(),
            ))
            .unwrap_err();

        assert_eq!(err.code, expected);
    }
}

#[test]
fn reqwest_provider_maps_empty_body_to_invalid_response() {
    let provider = ReqwestRemoteMetadataProvider::new(serve_once(200, "")).unwrap();
    let err = runtime()
        .block_on(AsyncRemoteMetadataProvider::get_file_content(
            &provider,
            &file_ref(),
        ))
        .unwrap_err();

    assert_eq!(err.code, RemoteMetadataErrorCode::RemoteResponseInvalid);
    assert!(err.message.contains("empty"));
}

#[test]
fn reqwest_provider_parses_visible_manifest_wrapped_by_result_and_data() {
    let previous = json!({
        "result": [
            {
                "path": "app/page-old.spg",
                "revision": "1",
                "modifyTime": 1000
            }
        ]
    })
    .to_string();

    let visible = json!({
        "data": {
            "result": {
                "children": [
                    {
                        "resourcePath": "app/page-new.spg",
                        "revision": "1",
                        "modifyTime": 1000
                    },
                    {
                        "resourcePath": "app/page-old.spg",
                        "revision": "2",
                        "modifyTime": 1000
                    }
                ]
            }
        }
    })
    .to_string();

    let diff = diff_visible_manifest("analyzer", &previous, &visible).unwrap();

    assert_eq!(diff.added[0].source_path, "app/page-new.spg");
    assert_eq!(diff.modified[0].source_path, "app/page-old.spg");
    assert!(diff.deleted.is_empty());
    assert!(diff.unchanged.is_empty());
    assert_eq!(
        diff.content_queue.len(),
        2,
        "added+modified 且可分析的条目都应进入 content_queue"
    );
}

#[test]
fn diff_visible_manifest_marks_modified_when_revision_missing_but_modify_time_changes() {
    let previous = json!([
        {
            "path": "app/revision-missing.spg",
            "modifyTime": 1000
        }
    ])
    .to_string();

    let visible = json!([
        {
            "path": "app/revision-missing.spg",
            "modifyTime": 2000
        }
    ])
    .to_string();

    let diff = diff_visible_manifest("analyzer", &previous, &visible).unwrap();

    assert_eq!(diff.modified[0].source_path, "app/revision-missing.spg");
    assert!(!diff.changed_files.is_empty());
    assert_eq!(
        diff.content_queue[0].source_path,
        "app/revision-missing.spg"
    );
}
