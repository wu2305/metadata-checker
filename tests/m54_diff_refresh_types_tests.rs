#![cfg(feature = "cli-local")]
//! M54 Task 2：typed ChangeSet 契约测试
//!
//! 固定 SourceCursor / MetaFilesWatermark / ChangedRemoteFile / ChangeSet
//! 与 cursor_accepts 的行为契约，供后续 fixture/BI change source 消费。

use metadata_checker::diff_refresh::{
    ChangeSet, ChangedRemoteFile, MetaFilesWatermark, SourceCursor, cursor_accepts,
};
use metadata_checker::remote_metadata::MetadataContentType;

/// 构造包含一条活跃更新和一条删除事件的 fixture ChangeSet。
fn fixture_change_set() -> ChangeSet {
    let changed = vec![
        ChangedRemoteFile {
            event_id: "active:file-a:2".into(),
            file_id: "file-a".into(),
            source_path: "app/page_a.spg".into(),
            previous_source_path: None,
            content_type: MetadataContentType::SuperPage,
            updated_at_ms: 20,
            deleted: false,
        },
        ChangedRemoteFile {
            event_id: "deleted:uuid-b".into(),
            file_id: "file-b".into(),
            source_path: "app/page_b.spg".into(),
            previous_source_path: Some("app/page_b.spg".into()),
            content_type: MetadataContentType::SuperPage,
            updated_at_ms: 21,
            deleted: true,
        },
    ];
    let next_watermark = MetaFilesWatermark {
        active: SourceCursor::new(20, vec!["active:file-a:2".into()]),
        deleted: SourceCursor::new(21, vec!["deleted:uuid-b".into()]),
    };
    ChangeSet::new(changed, next_watermark)
}

/// 同毫秒事件：未出现在 boundary_event_ids 中的接受，已出现的拒绝。
#[test]
fn m54_cursor_accepts_unseen_same_millisecond_event() {
    let cursor = SourceCursor {
        updated_at_ms: 10,
        boundary_event_ids: vec!["active:a:1".into()],
    };
    assert_eq!(cursor_accepts(&cursor, 10, "active:b:1"), true);
    assert_eq!(cursor_accepts(&cursor, 10, "active:a:1"), false);
}

/// 同毫秒已见事件被拒绝（含多条 boundary ids 的命中场景）。
#[test]
fn m54_cursor_rejects_seen_same_millisecond_event() {
    let cursor = SourceCursor::new(10, vec!["active:a:1".into(), "deleted:uuid-c".into()]);
    assert_eq!(cursor_accepts(&cursor, 10, "deleted:uuid-c"), false);
    assert_eq!(cursor_accepts(&cursor, 9, "active:new:1"), false);
    assert_eq!(cursor_accepts(&cursor, 11, "active:a:1"), true);
}

/// ChangeSet 不变式：change_count 必须等于 changed 长度。
#[test]
fn m54_change_count_matches_changed_length() {
    let changes = fixture_change_set();
    assert_eq!(changes.change_count, changes.changed.len());
}

/// boundary_event_ids 在构造时排序去重。
#[test]
fn m54_boundary_event_ids_sorted_and_deduped_on_construction() {
    let cursor = SourceCursor::new(
        10,
        vec![
            "active:b:1".into(),
            "active:a:1".into(),
            "active:b:1".into(),
        ],
    );
    assert_eq!(
        cursor.boundary_event_ids,
        vec!["active:a:1".to_string(), "active:b:1".to_string()]
    );
}

/// 序列化与反序列化后 boundary_event_ids 仍保持排序去重。
#[test]
fn m54_boundary_event_ids_sorted_and_deduped_after_serde_round_trip() {
    // 通过公开字段绕过构造函数制造未排序的 cursor，
    // 序列化路径必须兜底排序去重。
    let cursor = SourceCursor {
        updated_at_ms: 10,
        boundary_event_ids: vec![
            "deleted:uuid-b".into(),
            "active:a:1".into(),
            "deleted:uuid-b".into(),
        ],
    };
    let json = serde_json::to_string(&cursor).expect("serialize SourceCursor");
    let restored: SourceCursor = serde_json::from_str(&json).expect("deserialize SourceCursor");
    assert_eq!(
        restored.boundary_event_ids,
        vec!["active:a:1".to_string(), "deleted:uuid-b".to_string()]
    );
    assert_eq!(restored.updated_at_ms, 10);
}
