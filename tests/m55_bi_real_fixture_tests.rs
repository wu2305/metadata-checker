#![cfg(feature = "cli-local")]
//! M55：真实 BI 录制 fixture 验证（autocrm-test 匿名化采样）
//!
//! 覆盖：getFileDescendant 真实信封解析、getRecyclebinFiles 真实裸数组解析、
//! 真实数据上的 poll 语义、source 侧 Unknown 类型过滤、
//! bootstrap 对照 manifest 时 docx 不被当成新增文件、
//! 重复 file_id 墓碑按 uuid 区分。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Mutex;

use anyhow::Result;

use metadata_checker::diff_refresh::{
    BiActiveFileInfo, BiDeletedMetaFileInfo, BiMetaFilesChangeSource, BiMetaFilesTransport,
    MetaFilesChangeSource, MetaFilesWatermark, SourceCursor,
};
use metadata_checker::remote_metadata::MetadataContentType;
use metadata_checker::session::RemoteSessionProvider;
use metadata_checker::session::manifest::{RemoteSessionFile, SessionManifest};
use metadata_checker::session::reqwest_provider::ReqwestRemoteSessionProvider;

const ACTIVE_JSON: &str = include_str!("fixtures/diff_refresh/bi_real_active_app.json");
const RECYCLEBIN_JSON: &str = include_str!("fixtures/diff_refresh/bi_real_recyclebin.json");

fn active_entries() -> Vec<BiActiveFileInfo> {
    let value: serde_json::Value = serde_json::from_str(ACTIVE_JSON).expect("parse active fixture");
    serde_json::from_value(value["files"].clone()).expect("parse active files")
}

fn deleted_entries() -> Vec<BiDeletedMetaFileInfo> {
    serde_json::from_str(RECYCLEBIN_JSON).expect("parse recyclebin fixture")
}

/// 脚本化 test double transport（与 m55_meta_files_source_tests 同款最小实现）。
#[derive(Default)]
struct StubTransport {
    active_responses: Mutex<VecDeque<Result<Vec<BiActiveFileInfo>>>>,
    deleted_responses: Mutex<VecDeque<Result<Vec<BiDeletedMetaFileInfo>>>>,
}

impl StubTransport {
    fn push_active(&self, entries: Vec<BiActiveFileInfo>) {
        self.active_responses
            .lock()
            .expect("lock")
            .push_back(Ok(entries));
    }

    fn push_deleted(&self, entries: Vec<BiDeletedMetaFileInfo>) {
        self.deleted_responses
            .lock()
            .expect("lock")
            .push_back(Ok(entries));
    }
}

impl BiMetaFilesTransport for StubTransport {
    fn list_active_files(&self, _project_ref: &str) -> Result<Vec<BiActiveFileInfo>> {
        self.active_responses
            .lock()
            .expect("lock")
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
    }

    fn list_deleted_files(&self, _project_ref: &str) -> Result<Vec<BiDeletedMetaFileInfo>> {
        self.deleted_responses
            .lock()
            .expect("lock")
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

/// 本机回环一次性 HTTP 服务器（不打外网）。
fn serve_once(status: &'static str, body: &'static str) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 8192];
        let _ = stream.read(&mut buf);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("write response");
    });
    format!("http://{addr}")
}

/// 真实信封经 list_metafiles 解析路径（getFileDescendant → files 键解包）
/// 能解析出全部非文件夹条目，字段映射正确。
#[test]
fn m55_bi_real_active_envelope_parses_via_descendant_path() {
    let url = serve_once("200 OK", ACTIVE_JSON);
    let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");

    let entries = provider.list_metafiles("proj").expect("list metafiles");

    // 15 条中 4 个文件夹（fold/app 且 isFolder=true）被跳过
    assert_eq!(entries.len(), 11, "non-folder entries only");
    for entry in &entries {
        assert!(entry.file_id.is_some(), "entry must have file_id");
        assert!(entry.revision.is_some(), "entry must have revision");
        assert!(entry.mtime.is_some(), "entry must have modifyTime");
    }
    // 抽查 AID0001：parentDir+name 还原路径、数字 revision 归一化为字符串
    let first = entries
        .iter()
        .find(|entry| entry.file_id.as_deref() == Some("AID0001"))
        .expect("AID0001 entry");
    assert_eq!(first.source_path, "seg1/dir2.app/file0.spg");
    assert_eq!(first.revision.as_deref(), Some("8"));
    assert_eq!(first.mtime, Some(1764811495925));
}

/// 回收站真实裸数组：46 条全部解析成功（小写 camelCase），uuid/deleteTime/id 无缺失。
#[test]
fn m55_bi_real_recyclebin_parses_all_tombstones() {
    let url = serve_once("200 OK", RECYCLEBIN_JSON);
    let provider = ReqwestRemoteSessionProvider::new(&url).expect("provider");

    let entries = provider
        .list_recyclebin_files("proj")
        .expect("list recyclebin");

    assert_eq!(entries.len(), 46);
    for entry in &entries {
        assert!(entry.uuid.is_some(), "tombstone must have uuid");
        assert!(
            entry.delete_time.is_some(),
            "tombstone must have deleteTime"
        );
        assert!(entry.file_id.is_some(), "tombstone must have file id");
    }
    // 重复 file_id 组 DID0009×4 全部保留（uuid 互不相同）
    let did9: Vec<_> = entries
        .iter()
        .filter(|entry| entry.file_id.as_deref() == Some("DID0009"))
        .collect();
    assert_eq!(did9.len(), 4);
    let uuids: std::collections::HashSet<_> =
        did9.iter().map(|entry| entry.uuid.as_deref()).collect();
    assert_eq!(uuids.len(), 4, "duplicate file_id must keep distinct uuids");
}

/// 真实数据上的 poll：只保留 spg/tbl 事件（Unknown 类型与文件夹被过滤），
/// change_count 不变式、严格升序、双 cursor 正确推进。
#[test]
fn m55_bi_real_poll_filters_unknown_types_and_keeps_cursor_semantics() {
    let transport = StubTransport::default();
    transport.push_active(active_entries());
    transport.push_deleted(deleted_entries());
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let changes = source.poll(&zero_watermark()).expect("poll");

    // 活跃 spg/tbl 5 条 + 回收站 spg/tbl 4 条 = 9 个事件
    assert_eq!(changes.change_count, 9);
    assert_eq!(changes.change_count, changes.changed.len());
    for event in &changes.changed {
        assert_ne!(
            event.content_type,
            MetadataContentType::Unknown,
            "unknown content type must be filtered: {}",
            event.event_id
        );
    }
    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(
        event_ids,
        vec![
            "deleted:00000000-0000-4000-8000-000000000006",
            "deleted:00000000-0000-4000-8000-000000000001",
            "deleted:00000000-0000-4000-8000-000000000002",
            "active:AID0005:0",
            "active:AID0001:8",
            "deleted:00000000-0000-4000-8000-000000000005",
            "active:AID0003:1524",
            "active:AID0004:7",
            "active:AID0002:263",
        ],
        "events must be strictly ascending by (updated_at_ms, event_id)"
    );
    for window in changes.changed.windows(2) {
        let (prev, next) = (&window[0], &window[1]);
        assert!(
            (prev.updated_at_ms, &prev.event_id) < (next.updated_at_ms, &next.event_id),
            "strictly ascending"
        );
    }

    // active cursor：spg/tbl 最大 modifyTime（AID0002），boundary 含该毫秒已见 id
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(1782193683488, vec!["active:AID0002:263".into()])
    );
    // deleted cursor：spg/tbl 最大 deleteTime（DID0005）
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(
            1768183888976,
            vec!["deleted:00000000-0000-4000-8000-000000000005".into()]
        )
    );
}

/// bootstrap：docx/ts/json 不被当成「新增文件」，文件夹墓碑不产出事件；
/// cursor 覆盖全部快照条目（含非 spg/tbl），避免后续 poll 重放。
#[test]
fn m55_bi_real_bootstrap_docx_not_treated_as_new_file() {
    let transport = StubTransport::default();
    // 固定顺序：deleted 快照 → active 快照
    transport.push_deleted(deleted_entries());
    transport.push_active(active_entries());
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
    // AID0001 未变（revision/path 与快照一致）；file-ghost 远端已缺失
    manifest
        .files
        .push(manifest_file("AID0001", "seg1/dir2.app/file0.spg", "8"));
    manifest
        .files
        .push(manifest_file("file-ghost", "app/ghost.spg", "1"));

    let changes = source.bootstrap(&manifest).expect("bootstrap");

    // docx/ts/json/fold/app 不产出事件；AID0001 未变不产出；
    // 其余 4 个 spg/tbl 为新文件 + ghost 删除（复用 DID0001 墓碑? ghost 无墓碑 → deleted:file-ghost）
    assert_eq!(changes.change_count, 5);
    for event in &changes.changed {
        assert_ne!(event.content_type, MetadataContentType::Unknown);
        assert!(
            !event.source_path.ends_with(".docx")
                && !event.source_path.ends_with(".ts")
                && !event.source_path.ends_with(".json"),
            "non-spg/tbl file must not produce event: {}",
            event.source_path
        );
    }
    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert!(event_ids.contains(&"deleted:file-ghost"));
    assert!(event_ids.contains(&"active:AID0002:263"));
    assert!(event_ids.contains(&"active:AID0003:1524"));
    assert!(event_ids.contains(&"active:AID0004:7"));
    assert!(event_ids.contains(&"active:AID0005:0"));

    // active cursor 覆盖全部非文件夹快照条目（含 ts/docx/json 等 Unknown 类型；
    // 全局最大 modifyTime 是 app 文件夹，文件夹不是文件事件、正确地不覆盖）
    assert_eq!(
        changes.next_watermark.active.updated_at_ms, 1782193683488,
        "active cursor must cover all non-folder snapshot entries"
    );
    // deleted cursor 覆盖全部墓碑（含文件夹与非 spg/tbl 的全局最大 deleteTime）
    assert_eq!(
        changes.next_watermark.deleted.updated_at_ms, 1770085302078,
        "deleted cursor must cover all tombstones to avoid replay"
    );
    // boundary 只保存最大毫秒内的已见 ID（DID0021，tpg 墓碑也计入 cursor），
    // 重复 file_id 组 DID0009×4 的 deleteTime 均早于 cursor 时间，
    // 已被 cursor 覆盖、不会在后续 poll 重放（uuid 唯一性见解析测试）
    assert_eq!(
        changes.next_watermark.deleted.boundary_event_ids,
        vec!["deleted:00000000-0000-4000-8000-000000000024".to_string()]
    );
    for delete_time in [
        1764811485061_u64,
        1763517888620,
        1763517536259,
        1764209889053,
    ] {
        assert!(
            delete_time < changes.next_watermark.deleted.updated_at_ms,
            "duplicate file_id tombstones must be covered by deleted cursor"
        );
    }
}

/// 链路行为钉死：manifest 只登记 spg/tbl，非 spg/tbl 事件不会进入 mirror。
///（source 侧过滤是兜底；此处验证过滤后事件集与 manifest 类型域一致。）
#[test]
fn m55_bi_real_filtered_events_stay_in_manifest_type_domain() {
    let transport = StubTransport::default();
    transport.push_active(active_entries());
    transport.push_deleted(deleted_entries());
    let source = BiMetaFilesChangeSource::new(transport, "proj");

    let changes = source.poll(&zero_watermark()).expect("poll");
    for event in &changes.changed {
        assert!(
            matches!(
                event.content_type,
                MetadataContentType::SuperPage | MetadataContentType::Table
            ),
            "every event must be spg/tbl so mirror only fetches analyzable files: {}",
            event.event_id
        );
    }
}
