#![cfg(feature = "cli-local")]
//! M54 Task 4：FileMirrorSync 差量写、删除与改名测试
//!
//! 验证 apply_changeset_to_mirror：仅变更文件被重写、fetch 次数等于
//! 非删除事件数、删除移除镜像并标记 manifest、改名后新路径存在且
//! manifest 只保留新路径、内容一致时跳过写入。

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    ChangeSet, ChangedRemoteFile, MetaFilesWatermark, SourceCursor, apply_changeset_to_mirror,
};
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::session::RemoteSessionProvider;
use metadata_checker::session::manifest::SessionManifest;
use metadata_checker::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::sync::{
    SessionSyncItem, SessionSyncMode, project_mirror_root, sync_remote_files_to_session,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// 每次测试使用独立的临时 session 目录。
fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m54-mirror-test-{pid}-{seq}-{name}"
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

/// 记录 fetch 次数的 test double provider；内容按 source_path 索引。
struct CountingProvider {
    contents: HashMap<String, RemoteFileContent>,
    fetch_count: Cell<usize>,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            contents: HashMap::new(),
            fetch_count: Cell::new(0),
        }
    }

    fn register(&mut self, path: &str, file_id: &str, revision: &str, raw_text: &str) {
        self.contents.insert(
            path.to_string(),
            RemoteFileContent {
                source_path: path.to_string(),
                file_id: Some(file_id.to_string()),
                revision: Some(revision.to_string()),
                content_type: MetadataContentType::from_extension(
                    path.rsplit('.').next().unwrap_or(""),
                ),
                raw_text: raw_text.to_string(),
            },
        );
    }
}

impl RemoteSessionProvider for CountingProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        Err(anyhow!("test double does not support list_projects"))
    }

    fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow!("test double does not support list_metafiles"))
    }

    fn fetch_metafile_info(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        Err(anyhow!("test double does not support fetch_metafile_info"))
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        self.fetch_count.set(self.fetch_count.get() + 1);
        self.contents
            .get(&file_ref.source_path)
            .cloned()
            .ok_or_else(|| anyhow!("no content for {}", file_ref.source_path))
    }

    fn fetch_changed_since(
        &self,
        _project_ref: &str,
        _since_revision: &str,
    ) -> Result<RemoteChangeSet> {
        Err(anyhow!("test double does not support fetch_changed_since"))
    }
}

fn make_manifest() -> SessionManifest {
    SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        "graph.redb",
        1,
    )
}

/// 用既有 sync 通路构造初始 mirror + manifest 状态。
fn seed_file(
    root: &std::path::Path,
    manifest: &mut SessionManifest,
    path: &str,
    file_id: &str,
    revision: &str,
    raw_text: &str,
) {
    let content = RemoteFileContent {
        source_path: path.to_string(),
        file_id: Some(file_id.to_string()),
        revision: Some(revision.to_string()),
        content_type: MetadataContentType::from_extension(path.rsplit('.').next().unwrap_or("")),
        raw_text: raw_text.to_string(),
    };
    sync_remote_files_to_session(
        root,
        manifest,
        &[SessionSyncItem::new(content)],
        SessionSyncMode::Partial,
    )
    .expect("seed mirror file");
}

fn active_event(
    event_id: &str,
    file_id: &str,
    source_path: &str,
    previous_source_path: Option<&str>,
    updated_at_ms: u64,
) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: event_id.to_string(),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: previous_source_path.map(str::to_string),
        content_type: MetadataContentType::SuperPage,
        updated_at_ms,
        deleted: false,
    }
}

fn delete_event(
    event_id: &str,
    file_id: &str,
    source_path: &str,
    updated_at_ms: u64,
) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: event_id.to_string(),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: None,
        content_type: MetadataContentType::SuperPage,
        updated_at_ms,
        deleted: true,
    }
}

/// manifest 记录断言辅助：按路径取记录。
fn manifest_record<'a>(
    manifest: &'a SessionManifest,
    source_path: &str,
) -> Option<&'a metadata_checker::session::manifest::RemoteSessionFile> {
    manifest
        .files
        .iter()
        .find(|file| file.source_path == source_path)
}

fn mirror_read(root: &std::path::Path, source_path: &str) -> String {
    fs::read_to_string(project_mirror_root(root).join(source_path)).expect("read mirror file")
}

/// 仅 page_a 更新时 page_b 字节不变，fetch 次数等于非删除事件数。
#[test]
fn m54_diff_refresh_mirror_updates_only_changed_file_and_fetches_active_events() {
    let root = test_root("update-only");
    let mut manifest = make_manifest();
    seed_file(&root, &mut manifest, "app/page_a.spg", "file-a", "1", "A1");
    seed_file(&root, &mut manifest, "app/page_b.spg", "file-b", "1", "B1");

    let mut provider = CountingProvider::new();
    provider.register("app/page_a.spg", "file-a", "2", "A2");

    let changeset = ChangeSet::new(
        vec![active_event(
            "active:file-a:2",
            "file-a",
            "app/page_a.spg",
            None,
            1000,
        )],
        MetaFilesWatermark {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );
    let report = apply_changeset_to_mirror(&root, &mut manifest, &changeset, &provider)
        .expect("apply changeset");

    assert_eq!(report.written, 1);
    assert_eq!(report.deleted, 0);
    assert_eq!(report.renamed, 0);
    assert_eq!(report.unchanged, 0);
    // fetch 次数等于非删除事件数
    assert_eq!(provider.fetch_count.get(), 1);
    // page_a 更新、page_b 字节不变
    assert_eq!(mirror_read(&root, "app/page_a.spg"), "A2");
    assert_eq!(mirror_read(&root, "app/page_b.spg"), "B1");
    // manifest：page_a revision 推进，page_b 记录不受影响
    assert_eq!(
        manifest_record(&manifest, "app/page_a.spg")
            .expect("page_a record")
            .revision
            .as_deref(),
        Some("2")
    );
    assert_eq!(
        manifest_record(&manifest, "app/page_b.spg")
            .expect("page_b record")
            .revision
            .as_deref(),
        Some("1")
    );
    let _ = fs::remove_dir_all(root);
}

/// 删除事件移除 mirror 文件并把 manifest 记录标为 deleted，且不触发 fetch。
#[test]
fn m54_diff_refresh_mirror_delete_removes_file_and_marks_manifest_deleted() {
    let root = test_root("delete");
    let mut manifest = make_manifest();
    seed_file(&root, &mut manifest, "app/page_c.spg", "file-c", "1", "C1");

    let provider = CountingProvider::new();
    let changeset = ChangeSet::new(
        vec![delete_event(
            "deleted:uuid-c-1",
            "file-c",
            "app/page_c.spg",
            1001,
        )],
        MetaFilesWatermark {
            active: SourceCursor::new(0, Vec::new()),
            deleted: SourceCursor::new(1001, vec!["deleted:uuid-c-1".into()]),
        },
    );
    let report = apply_changeset_to_mirror(&root, &mut manifest, &changeset, &provider)
        .expect("apply changeset");

    assert_eq!(report.deleted, 1);
    assert_eq!(report.written, 0);
    assert_eq!(provider.fetch_count.get(), 0);
    assert!(!project_mirror_root(&root).join("app/page_c.spg").exists());
    let record = manifest_record(&manifest, "app/page_c.spg").expect("page_c record");
    assert_eq!(record.deleted, true);
    let _ = fs::remove_dir_all(root);
}

/// 同 file_id 改名：新路径内容存在、旧路径不存在、manifest 只保留新路径。
#[test]
fn m54_diff_refresh_mirror_rename_moves_path_and_keeps_single_manifest_record() {
    let root = test_root("rename");
    let mut manifest = make_manifest();
    seed_file(
        &root,
        &mut manifest,
        "app/old_name.spg",
        "file-d",
        "1",
        "D1",
    );

    let mut provider = CountingProvider::new();
    provider.register("app/new_name.spg", "file-d", "2", "D2");

    let changeset = ChangeSet::new(
        vec![active_event(
            "active:file-d:2",
            "file-d",
            "app/new_name.spg",
            Some("app/old_name.spg"),
            1002,
        )],
        MetaFilesWatermark {
            active: SourceCursor::new(1002, vec!["active:file-d:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );
    let report = apply_changeset_to_mirror(&root, &mut manifest, &changeset, &provider)
        .expect("apply changeset");

    assert_eq!(report.renamed, 1);
    assert_eq!(report.written, 0);
    assert_eq!(provider.fetch_count.get(), 1);
    // 新路径内容存在、旧路径不存在
    assert_eq!(mirror_read(&root, "app/new_name.spg"), "D2");
    assert!(!project_mirror_root(&root).join("app/old_name.spg").exists());
    // manifest 只保留新路径记录
    assert!(manifest_record(&manifest, "app/old_name.spg").is_none());
    let records: Vec<_> = manifest
        .files
        .iter()
        .filter(|file| file.file_id.as_deref() == Some("file-d"))
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source_path, "app/new_name.spg");
    assert_eq!(records[0].revision.as_deref(), Some("2"));
    assert_eq!(records[0].deleted, false);
    let _ = fs::remove_dir_all(root);
}

/// 拉取内容与本地 hash 一致时跳过写入，计入 unchanged（fetch 仍发生）。
#[test]
fn m54_diff_refresh_mirror_unchanged_content_skips_write() {
    let root = test_root("unchanged");
    let mut manifest = make_manifest();
    seed_file(&root, &mut manifest, "app/page_a.spg", "file-a", "1", "A1");

    let mut provider = CountingProvider::new();
    provider.register("app/page_a.spg", "file-a", "1", "A1");

    let changeset = ChangeSet::new(
        vec![active_event(
            "active:file-a:2",
            "file-a",
            "app/page_a.spg",
            None,
            1000,
        )],
        MetaFilesWatermark {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );
    let report = apply_changeset_to_mirror(&root, &mut manifest, &changeset, &provider)
        .expect("apply changeset");

    assert_eq!(report.unchanged, 1);
    assert_eq!(report.written, 0);
    assert_eq!(provider.fetch_count.get(), 1);
    assert_eq!(mirror_read(&root, "app/page_a.spg"), "A1");
    let _ = fs::remove_dir_all(root);
}
