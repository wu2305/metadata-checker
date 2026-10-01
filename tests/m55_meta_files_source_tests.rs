#![cfg(feature = "cli-local")]
//! M55 Task 9：BI META_FILES 活跃/删除事件变更源测试
//!
//! 覆盖：双 cursor 独立过滤推进、active/deleted 同毫秒、回收站 file_id 重复、
//! 空数组、非法时间、缺失 revision/uuid、孤立单变更、退避计算器、
//! 404/403 错误码（本机回环 HTTP，不打外网）、交错竞态与真实 bootstrap。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Mutex;

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    BackoffSchedule, BiActiveFileInfo, BiDeletedMetaFileInfo, BiMetaFilesChangeSource,
    BiMetaFilesTransport, MetaFilesChangeSource, MetaFilesWatermark, SourceCursor,
};
use metadata_checker::session::manifest::{RemoteSessionFile, SessionManifest};
use metadata_checker::session::reqwest_provider::ReqwestRemoteSessionProvider;

const FIXTURE_JSON: &str = include_str!("fixtures/diff_refresh/bi_active_and_deleted.json");

/// 脚本化响应的 test double transport：每次调用弹出一条脚本响应。
#[derive(Default)]
struct StubTransport {
    active_responses: Mutex<VecDeque<Result<Vec<BiActiveFileInfo>>>>,
    deleted_responses: Mutex<VecDeque<Result<Vec<BiDeletedMetaFileInfo>>>>,
}

impl StubTransport {
    fn push_active(&self, entries: Vec<BiActiveFileInfo>) {
        self.active_responses
            .lock()
            .expect("lock active")
            .push_back(Ok(entries));
    }

    fn push_deleted(&self, entries: Vec<BiDeletedMetaFileInfo>) {
        self.deleted_responses
            .lock()
            .expect("lock deleted")
            .push_back(Ok(entries));
    }

    fn push_active_json(&self, json: &str) {
        let result = serde_json::from_str::<Vec<BiActiveFileInfo>>(json)
            .map_err(|error| anyhow!("invalid active json: {error}"));
        self.active_responses
            .lock()
            .expect("lock active")
            .push_back(result);
    }

    fn push_deleted_json(&self, json: &str) {
        let result = serde_json::from_str::<Vec<BiDeletedMetaFileInfo>>(json)
            .map_err(|error| anyhow!("invalid deleted json: {error}"));
        self.deleted_responses
            .lock()
            .expect("lock deleted")
            .push_back(result);
    }
}

impl BiMetaFilesTransport for StubTransport {
    fn list_active_files(&self, _project_ref: &str) -> Result<Vec<BiActiveFileInfo>> {
        self.active_responses
            .lock()
            .expect("lock active")
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn list_deleted_files(&self, _project_ref: &str) -> Result<Vec<BiDeletedMetaFileInfo>> {
        self.deleted_responses
            .lock()
            .expect("lock deleted")
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

fn zero_watermark() -> MetaFilesWatermark {
    MetaFilesWatermark {
        active: SourceCursor::new(0, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    }
}

fn fixture_entries() -> (Vec<BiActiveFileInfo>, Vec<BiDeletedMetaFileInfo>) {
    let value: serde_json::Value = serde_json::from_str(FIXTURE_JSON).expect("parse fixture");
    let active = serde_json::from_value(value["active"].clone()).expect("parse active");
    let deleted = serde_json::from_value(value["deleted"].clone()).expect("parse deleted");
    (active, deleted)
}

fn error_chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

/// poll：active/deleted 同毫秒、parentDir+name 兜底、回收站 file_id 重复不丢事件，
/// 合并后严格升序，双 cursor 各自推进。
#[test]
fn m55_meta_files_poll_filters_sorts_and_keeps_duplicate_recyclebin_file_ids() {
    let (active, deleted) = fixture_entries();
    let transport = StubTransport::default();
    transport.push_active(active);
    transport.push_deleted(deleted);
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let changes = source.poll(&zero_watermark()).expect("poll");

    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    // 同毫秒 active 两个 + deleted 一个（1000），回收站同 file_id 第二条（1001）不丢
    assert_eq!(
        event_ids,
        vec![
            "active:file-a:2",
            "active:file-b:3",
            "deleted:uuid-c-1",
            "deleted:uuid-c-2",
        ]
    );
    assert_eq!(changes.change_count, changes.changed.len());
    // parentDir+name 兜底还原路径
    assert_eq!(changes.changed[1].source_path, "app/page_b.spg");
    // 双 cursor 各自推进
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(
            1000,
            vec!["active:file-a:2".into(), "active:file-b:3".into()]
        )
    );
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(1001, vec!["deleted:uuid-c-2".into()])
    );
}

/// 空数组：空 ChangeSet，双 cursor 原样保留。
#[test]
fn m55_meta_files_poll_empty_arrays_preserve_cursors() {
    let transport = StubTransport::default();
    transport.push_active(Vec::new());
    transport.push_deleted(Vec::new());
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let since = MetaFilesWatermark {
        active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
        deleted: SourceCursor::new(999, vec!["deleted:uuid-x".into()]),
    };

    let changes = source.poll(&since).expect("poll empty");

    assert_eq!(changes.change_count, 0);
    assert_eq!(changes.next_watermark, since);
}

/// 非法时间与缺失 revision → INVALID_ACTIVE_CHANGE_EVENT，整次 poll 失败。
#[test]
fn m55_meta_files_poll_invalid_time_or_revision_fails_whole_poll() {
    // 非法 modifyTime（字符串非数字）
    let transport = StubTransport::default();
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": "soon", "revision": "2"}]"#,
    );
    transport.push_deleted(Vec::new());
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let error = source
        .poll(&zero_watermark())
        .expect_err("invalid time must fail");
    assert!(
        error_chain(&error).contains("INVALID_ACTIVE_CHANGE_EVENT"),
        "unexpected error: {error:#}"
    );

    // 缺失 revision
    let transport = StubTransport::default();
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1000}]"#,
    );
    transport.push_deleted(Vec::new());
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let error = source
        .poll(&zero_watermark())
        .expect_err("missing revision must fail");
    assert!(
        error_chain(&error).contains("INVALID_ACTIVE_CHANGE_EVENT"),
        "unexpected error: {error:#}"
    );
}

/// 删除事件缺失 uuid → INVALID_DELETE_EVENT_ID，整次 poll 失败。
#[test]
fn m55_meta_files_poll_missing_uuid_fails_whole_poll() {
    let transport = StubTransport::default();
    transport.push_active(Vec::new());
    transport.push_deleted_json(
        r#"[{"FILE_ID": "file-c", "PARENT_DIR": "proj/app", "NAME": "page_c.spg", "deleteTime": 1000}]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let error = source
        .poll(&zero_watermark())
        .expect_err("missing uuid must fail");
    assert!(
        error_chain(&error).contains("INVALID_DELETE_EVENT_ID"),
        "unexpected error: {error:#}"
    );
}

/// 孤立变更：只有一个非空事件时 poll 直接返回含该事件的 ChangeSet，不等待。
#[test]
fn m55_meta_files_poll_single_orphan_change_returned_immediately() {
    let transport = StubTransport::default();
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1000, "revision": "2"}]"#,
    );
    transport.push_deleted(Vec::new());
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let changes = source.poll(&zero_watermark()).expect("poll orphan");

    assert_eq!(changes.change_count, 1);
    assert_eq!(changes.changed[0].event_id, "active:file-a:2");
    // deleted 路无新事件，cursor 原样保留
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(0, Vec::new())
    );
}

/// 退避计算器：1/2/4/8/16/32/60 封顶，成功清零。
#[test]
fn m55_meta_files_backoff_schedule_caps_and_resets() {
    let mut backoff = BackoffSchedule::new();
    let delays: Vec<u64> = (0..8).map(|_| backoff.record_failure()).collect();
    assert_eq!(delays, vec![1, 2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(backoff.consecutive_failures(), 8);

    backoff.record_success();
    assert_eq!(backoff.consecutive_failures(), 0);
    assert_eq!(
        backoff.record_failure(),
        1,
        "成功后失败计数清零，退避从 1 秒重启"
    );
}

/// 交错竞态：active 第一次快照返回后注入新 active 事件，deleted 快照返回更晚
/// 事件；提交后第二次 poll 必须仍返回该 active 事件（deleted cursor 不推进 active）。
#[test]
fn m55_meta_files_deleted_cursor_does_not_advance_active_cursor() {
    let transport = StubTransport::default();
    // 第一次 poll：active 只有 file-a@1000，deleted 有更晚的 file-c@1002
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1000, "revision": "2"}]"#,
    );
    transport.push_deleted_json(
        r#"[{"FILE_ID": "file-c", "PARENT_DIR": "proj/app", "NAME": "page_c.spg", "deleteTime": 1002, "uuid": "uuid-c-1"}]"#,
    );
    // 第二次 poll：active 快照返回后注入 file-b@1001；deleted 无新事件
    transport.push_active_json(
        r#"[
        {"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1000, "revision": "2"},
        {"id": "file-b", "path": "proj/app/page_b.spg", "modifyTime": 1001, "revision": "1"}
    ]"#,
    );
    transport.push_deleted_json(
        r#"[{"FILE_ID": "file-c", "PARENT_DIR": "proj/app", "NAME": "page_c.spg", "deleteTime": 1002, "uuid": "uuid-c-1"}]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let first = source.poll(&zero_watermark()).expect("first poll");
    assert_eq!(
        first.next_watermark.deleted,
        SourceCursor::new(1002, vec!["deleted:uuid-c-1".into()])
    );

    // 提交后第二次 poll：deleted cursor 在 1002，但 active cursor 独立，
    // 注入的 file-b@1001 必须仍被返回
    let second = source.poll(&first.next_watermark).expect("second poll");
    let event_ids: Vec<&str> = second
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(event_ids, vec!["active:file-b:1"]);
    assert_eq!(
        second.next_watermark.active,
        SourceCursor::new(1001, vec!["active:file-b:1".into()])
    );
}

/// 真实 bootstrap：manifest 未变/revision 变化/远端缺失三态；
/// 历史 recycle-bin 只初始化 deleted cursor，不逐条重放。
#[test]
fn m55_meta_files_bootstrap_compares_manifest_and_initializes_cursors() {
    let transport = StubTransport::default();
    // 固定顺序：先 deleted 快照（含一个与 manifest 无关的历史墓碑，不重放）
    transport.push_deleted_json(r#"[
        {"FILE_ID": "file-old", "PARENT_DIR": "proj/app", "NAME": "old.spg", "deleteTime": 900, "uuid": "uuid-old-1"},
        {"FILE_ID": "file-w", "PARENT_DIR": "proj/app", "NAME": "page_w.spg", "deleteTime": 1999, "uuid": "uuid-w-1"}
    ]"#);
    // 再 active 快照：file-u 未变、file-v revision 变化；file-w 缺失
    transport.push_active_json(
        r#"[
        {"id": "file-u", "path": "proj/app/page_u.spg", "modifyTime": 2000, "revision": "1"},
        {"id": "file-v", "path": "proj/app/page_v.spg", "modifyTime": 2001, "revision": "2"}
    ]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let mut manifest = SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        "graph.redb",
        100,
    );
    let manifest_file = |file_id: &str, path: &str, revision: &str| RemoteSessionFile {
        source_path: path.to_string(),
        file_id: Some(file_id.to_string()),
        revision: Some(revision.to_string()),
        etag: None,
        mtime: None,
        size: None,
        hash: None,
        indexed_hash: None,
        deleted: false,
    };
    manifest
        .files
        .push(manifest_file("file-u", "app/page_u.spg", "1"));
    manifest
        .files
        .push(manifest_file("file-v", "app/page_v.spg", "1"));
    manifest
        .files
        .push(manifest_file("file-w", "app/page_w.spg", "1"));

    let changes = source.bootstrap(&manifest).expect("bootstrap");

    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    // 只含 revision 变化与远端缺失；历史墓碑 uuid-old-1 不重放
    assert_eq!(event_ids, vec!["deleted:uuid-w-1", "active:file-v:2"]);
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(2001, vec!["active:file-v:2".into()])
    );
    // deleted cursor 初始化到回收站快照边界（含历史墓碑时间最大值）
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(1999, vec!["deleted:uuid-w-1".into()])
    );
}

/// bootstrap 交错：deleted 快照返回后 manifest 中一个文件在 active 快照缺失，
/// bootstrap 必须产出删除。
#[test]
fn m55_meta_files_bootstrap_emits_delete_for_file_missing_in_active_snapshot() {
    let transport = StubTransport::default();
    transport.push_deleted_json(
        r#"[{"FILE_ID": "file-b", "PARENT_DIR": "proj/app", "NAME": "page_b.spg", "deleteTime": 1500, "uuid": "uuid-b-9"}]"#,
    );
    // active 快照不含 file-b（deleted 快照返回后发生的删除）
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1600, "revision": "1"}]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let mut manifest = SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        "graph.redb",
        100,
    );
    let manifest_file = |file_id: &str, path: &str| RemoteSessionFile {
        source_path: path.to_string(),
        file_id: Some(file_id.to_string()),
        revision: Some("1".to_string()),
        etag: None,
        mtime: None,
        size: None,
        hash: None,
        indexed_hash: None,
        deleted: false,
    };
    manifest
        .files
        .push(manifest_file("file-a", "app/page_a.spg"));
    manifest
        .files
        .push(manifest_file("file-b", "app/page_b.spg"));

    let changes = source.bootstrap(&manifest).expect("bootstrap");
    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(event_ids, vec!["deleted:uuid-b-9"]);
    assert_eq!(changes.changed[0].deleted, true);
}

/// bootstrap 字段错误 → DIFF_REFRESH_BOOTSTRAP_FAILED。
#[test]
fn m55_meta_files_bootstrap_invalid_snapshot_fails_with_bootstrap_code() {
    let transport = StubTransport::default();
    transport.push_deleted(Vec::new());
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": "bad", "revision": "1"}]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let manifest = SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        "graph.redb",
        100,
    );

    let error = source
        .bootstrap(&manifest)
        .expect_err("invalid snapshot must fail");
    assert!(
        error_chain(&error).contains("DIFF_REFRESH_BOOTSTRAP_FAILED"),
        "unexpected error: {error:#}"
    );
}

/// active 快照返回后发生删除：本轮 bootstrap 可不含，下一次 deleted poll 必须返回。
#[test]
fn m55_meta_files_delete_after_active_snapshot_returned_by_next_poll() {
    let transport = StubTransport::default();
    // bootstrap：deleted 快照为空，active 含 file-a
    transport.push_deleted(Vec::new());
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1600, "revision": "1"}]"#,
    );
    // 下一次 poll：file-a 进入回收站
    transport.push_active_json(
        r#"[{"id": "file-a", "path": "proj/app/page_a.spg", "modifyTime": 1600, "revision": "1"}]"#,
    );
    transport.push_deleted_json(
        r#"[{"FILE_ID": "file-a", "PARENT_DIR": "proj/app", "NAME": "page_a.spg", "deleteTime": 1700, "uuid": "uuid-a-1"}]"#,
    );
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let mut manifest = SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        "graph.redb",
        100,
    );
    manifest.files.push(RemoteSessionFile {
        source_path: "app/page_a.spg".to_string(),
        file_id: Some("file-a".to_string()),
        revision: Some("1".to_string()),
        etag: None,
        mtime: None,
        size: None,
        hash: None,
        indexed_hash: None,
        deleted: false,
    });

    let bootstrapped = source.bootstrap(&manifest).expect("bootstrap");
    assert_eq!(
        bootstrapped.change_count, 0,
        "manifest 与快照一致，本轮不含事件"
    );

    let changes = source
        .poll(&bootstrapped.next_watermark)
        .expect("poll after delete");
    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(event_ids, vec!["deleted:uuid-a-1"]);
}

/// 本机回环一次性 HTTP 服务器：捕获请求体，返回固定状态与响应体（不打外网）。
fn serve_once(
    status: &'static str,
    body: &'static str,
) -> (String, std::sync::mpsc::Receiver<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("set read timeout");
        // 读完 headers 后按 Content-Length 读 body
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let read = stream.read(&mut buf).unwrap_or(0);
            if read == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..read]);
            if let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&raw[..header_end]).to_string();
                let content_length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse().ok())
                    })
                    .unwrap_or(0);
                if raw.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&raw).to_string());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    (format!("http://{addr}"), rx)
}

/// 404 → DELETE_CHANGE_SOURCE_UNAVAILABLE；403 → DELETE_CHANGE_SOURCE_FORBIDDEN；
/// 请求体固定为 {"projectName": project_ref, "recur": true}。
#[test]
fn m55_meta_files_recyclebin_http_error_codes_and_fixed_body() {
    for (status, expected_code) in [
        ("404 Not Found", "DELETE_CHANGE_SOURCE_UNAVAILABLE"),
        ("403 Forbidden", "DELETE_CHANGE_SOURCE_FORBIDDEN"),
    ] {
        let (url, rx) = serve_once(status, "{}");
        let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");
        let error = provider
            .list_recyclebin_files("proj")
            .expect_err("error status must fail");
        assert!(
            error_chain(&error).contains(expected_code),
            "status {status} should map to {expected_code}, got: {error:#}"
        );
        let request = rx.recv().expect("captured request");
        assert!(
            request.contains(r#"{"projectName":"proj","recur":true}"#),
            "request body mismatch: {request}"
        );
    }
}

/// 200：响应数组解析为删除文件条目（FILE_ID 大写键）。
#[test]
fn m55_meta_files_recyclebin_parses_deleted_entries() {
    let (url, _rx) = serve_once(
        "200 OK",
        r#"[{"FILE_ID": "file-c", "PARENT_DIR": "proj/app", "NAME": "page_c.spg", "TYPE": "spg", "deleteTime": 1000, "uuid": "uuid-c-1"}]"#,
    );
    let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");
    let entries = provider
        .list_recyclebin_files("proj")
        .expect("list recyclebin");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_id.as_deref(), Some("file-c"));
    assert_eq!(entries[0].uuid.as_deref(), Some("uuid-c-1"));
    assert_eq!(entries[0].delete_time, Some(1000));
}

/// 401 → 稳定 Unauthorized；其它非成功状态 → `HTTP {code}`。
#[test]
fn m55_meta_files_recyclebin_unauthorized_and_http_status() {
    let (url, _rx) = serve_once("401 Unauthorized", r#"{"ok":false}"#);
    let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");
    let error = provider
        .list_recyclebin_files("proj")
        .expect_err("401 must fail");
    assert!(
        error_chain(&error).contains("401 Unauthorized"),
        "unexpected error: {error:#}"
    );

    let (url, _rx) = serve_once("500 Internal Server Error", r#"{"ok":false}"#);
    let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");
    let error = provider
        .list_recyclebin_files("proj")
        .expect_err("500 must fail");
    assert!(
        error_chain(&error).contains("HTTP 500"),
        "unexpected error: {error:#}"
    );
}

/// 非可分析条目（空 name / 文件夹 / 无路径）在 poll 前被过滤，不产生事件、不拖垮 poll。
#[test]
fn m55_meta_files_poll_skips_non_analyzable_active_and_deleted() {
    let transport = StubTransport::default();
    transport.push_active(vec![
        BiActiveFileInfo {
            file_id: Some("folder-1".into()),
            path: Some("proj/app".into()),
            parent_dir: None,
            name: None,
            file_type: Some("folder".into()),
            is_folder: true,
            revision: Some("1".into()),
            modify_time: Some(100),
        },
        BiActiveFileInfo {
            file_id: Some("empty-name".into()),
            path: None,
            parent_dir: Some("proj/app".into()),
            name: Some("".into()),
            file_type: Some("spg".into()),
            is_folder: false,
            revision: Some("1".into()),
            modify_time: Some(101),
        },
        BiActiveFileInfo {
            file_id: Some("docx-1".into()),
            path: Some("proj/app/readme.docx".into()),
            parent_dir: None,
            name: None,
            file_type: Some("docx".into()),
            is_folder: false,
            revision: Some("1".into()),
            modify_time: Some(102),
        },
    ]);
    transport.push_deleted(vec![
        BiDeletedMetaFileInfo {
            file_id: Some("del-folder".into()),
            parent_dir: Some("proj/app".into()),
            name: Some("gone".into()),
            file_type: Some("folder".into()),
            delete_time: Some(200),
            uuid: Some("uuid-folder".into()),
            is_folder: true,
        },
        BiDeletedMetaFileInfo {
            file_id: Some("del-empty".into()),
            parent_dir: Some("proj/app".into()),
            name: Some("".into()),
            file_type: Some("spg".into()),
            delete_time: Some(201),
            uuid: Some("uuid-empty".into()),
            is_folder: false,
        },
    ]);

    let source = BiMetaFilesChangeSource::new(transport, "proj");
    let changes = source.poll(&zero_watermark()).expect("poll must succeed");
    assert_eq!(
        changes.change_count, 0,
        "non-analyzable entries must not produce events"
    );
}
