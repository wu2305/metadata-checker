#![cfg(feature = "cli-local")]
//! M54 Task 3：fixture META_FILES 变更源测试
//!
//! 覆盖 poll 的双 cursor 过滤、稳定排序、event_id 去重、
//! change_count 不变式，以及 bootstrap 对照 manifest 产出差异事件。

use metadata_checker::diff_refresh::{
    FixtureMetaFilesChangeSource, MetaFilesChangeSource, MetaFilesWatermark, SourceCursor,
};
use metadata_checker::session::{RemoteSessionFile, SessionManifest};

const FIXTURE_JSON: &str = include_str!("fixtures/diff_refresh/meta_files_delta.json");

/// 零水位：两路 cursor 均从 Unix 毫秒 0、空 boundary 开始。
fn zero_watermark() -> MetaFilesWatermark {
    MetaFilesWatermark {
        active: SourceCursor::new(0, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    }
}

fn fixture_source() -> FixtureMetaFilesChangeSource {
    FixtureMetaFilesChangeSource::from_json_str(FIXTURE_JSON).expect("parse fixture")
}

/// poll 从零水位返回全部 4 个事件（同毫秒两个更新、一个删除、一个改名），
/// 按 `(updated_at_ms, event_id)` 严格升序且无重复，双 cursor 各自推进。
#[test]
fn m54_diff_refresh_fixture_poll_filters_sorts_and_advances_watermark() {
    let source = fixture_source();
    let changes = source.poll(&zero_watermark()).expect("poll");

    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert_eq!(
        event_ids,
        vec![
            "active:file-a:2",
            "active:file-b:3",
            "deleted:uuid-c-1",
            "active:file-d:2",
        ]
    );
    assert_eq!(changes.change_count, changes.changed.len());
    assert_eq!(changes.change_count, 4);

    // 严格升序：时间非递减，同毫秒 event_id 递增
    for window in changes.changed.windows(2) {
        let (prev, next) = (&window[0], &window[1]);
        assert!(
            (prev.updated_at_ms, &prev.event_id) < (next.updated_at_ms, &next.event_id),
            "events must be strictly ascending"
        );
    }

    // 改名事件保留旧路径，删除事件标记 deleted
    let renamed = &changes.changed[3];
    assert_eq!(
        renamed.previous_source_path.as_deref(),
        Some("app/old_name.spg")
    );
    assert_eq!(changes.changed[2].deleted, true);

    // 双 cursor 各自推进：active 取最大活跃事件时间，deleted 取删除事件时间
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(1002, vec!["active:file-d:2".into()])
    );
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(1001, vec!["deleted:uuid-c-1".into()])
    );
}

/// 同毫秒已见事件被拒绝、未见事件仍接收；用推进后的水位再 poll 返回空集合并保留 cursor。
#[test]
fn m54_diff_refresh_fixture_poll_respects_same_millisecond_boundary() {
    let source = fixture_source();
    let since = MetaFilesWatermark {
        active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    let changes = source.poll(&since).expect("poll");

    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    // active:file-a:2 已在 boundary 中被拒绝；同毫秒未见的 file-b 仍接收
    assert_eq!(
        event_ids,
        vec!["active:file-b:3", "deleted:uuid-c-1", "active:file-d:2"]
    );
    assert_eq!(changes.change_count, changes.changed.len());

    // active cursor 推进到新的最大时间（大于旧时间，不合并旧 boundary）
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(1002, vec!["active:file-d:2".into()])
    );

    // 用推进后的水位再次 poll：无新事件，返回空集合且 cursor 原样保留
    let second = source.poll(&changes.next_watermark).expect("poll again");
    assert_eq!(second.change_count, 0);
    assert_eq!(second.next_watermark, changes.next_watermark);
}

/// 去重键使用唯一 event_id：fixture 中重复 event_id 只保留首个，结果仍严格升序。
#[test]
fn m54_diff_refresh_fixture_poll_dedups_by_event_id() {
    let json = r#"{
        "schema_version": 1,
        "events": [
            {
                "event_id": "active:file-a:2",
                "file_id": "file-a",
                "source_path": "app/page_a.spg",
                "previous_source_path": null,
                "content_type": "super_page",
                "updated_at_ms": 1000,
                "deleted": false
            },
            {
                "event_id": "active:file-a:2",
                "file_id": "file-a",
                "source_path": "app/page_a.spg",
                "previous_source_path": null,
                "content_type": "super_page",
                "updated_at_ms": 1000,
                "deleted": false
            }
        ]
    }"#;
    let source = FixtureMetaFilesChangeSource::from_json_str(json).expect("parse fixture");
    let changes = source.poll(&zero_watermark()).expect("poll");
    assert_eq!(changes.changed.len(), 1);
    assert_eq!(changes.change_count, changes.changed.len());
}

/// bootstrap：manifest 中未变文件不产出事件，revision 变化与远端缺失文件分别
/// 产出活跃/删除事件，双 cursor 初始化到各自快照边界。
#[test]
fn m54_diff_refresh_fixture_bootstrap_emits_only_changed_and_missing() {
    let mut manifest = SessionManifest::new(
        "s1",
        "https://bi.example.test",
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
        deleted: false,
    };
    // 未变文件：revision/path 与活跃快照一致
    manifest
        .files
        .push(manifest_file("file-u", "app/page_u.spg", "1"));
    // revision 变化文件：快照为 rev 2
    manifest
        .files
        .push(manifest_file("file-v", "app/page_v.spg", "1"));
    // 远端已缺失文件：不在活跃快照，出现在删除快照
    manifest
        .files
        .push(manifest_file("file-w", "app/page_w.spg", "1"));

    let source = fixture_source();
    let changes = source.bootstrap(&manifest).expect("bootstrap");

    let event_ids: Vec<&str> = changes
        .changed
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    // 只含 revision 变化与远端缺失两个事件，按时间升序（删除 1999 在前）
    assert_eq!(event_ids, vec!["deleted:uuid-w-1", "active:file-v:2"]);
    assert_eq!(changes.change_count, changes.changed.len());
    assert_eq!(changes.changed[0].deleted, true);
    assert_eq!(changes.changed[1].deleted, false);

    // 双 cursor 分别初始化到各自快照边界
    assert_eq!(
        changes.next_watermark.active,
        SourceCursor::new(2001, vec!["active:file-v:2".into()])
    );
    assert_eq!(
        changes.next_watermark.deleted,
        SourceCursor::new(1999, vec!["deleted:uuid-w-1".into()])
    );
}
