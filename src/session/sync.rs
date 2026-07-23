//! 远程元数据同步到本地 session 镜像
//!
//! 本模块只负责把已经获取到的远程元数据内容安全写入 session 目录，
//! 不处理真实 HTTP、登录或图数据库扫描。

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::remote_metadata::{RemoteFileContent, is_logical_source_path};

use super::manifest::{RemoteSessionFile, SessionManifest};

/// 单次同步模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSyncMode {
    /// 只同步调用方传入的文件集合。
    Partial,
    /// 调用方传入完整远端文件集合，可用于标记删除。
    Full,
}

/// 单次同步结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionSyncReport {
    pub written: usize,
    pub skipped: usize,
    pub deleted: usize,
}

/// 远端文件同步输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSyncItem {
    pub content: RemoteFileContent,
    pub etag: Option<String>,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
}

impl SessionSyncItem {
    /// 使用远程内容创建同步输入。
    pub fn new(content: RemoteFileContent) -> Self {
        Self {
            content,
            etag: None,
            mtime: None,
            size: None,
        }
    }
}

/// 将一批远程文件写入 session 镜像并更新 manifest。
pub fn sync_remote_files_to_session(
    session_dir: &Path,
    manifest: &mut SessionManifest,
    items: &[SessionSyncItem],
    mode: SessionSyncMode,
) -> Result<SessionSyncReport> {
    let project_root = project_mirror_root(session_dir);
    fs::create_dir_all(&project_root).with_context(|| {
        format!(
            "failed to create session project mirror {}",
            project_root.display()
        )
    })?;

    let mut report = SessionSyncReport::default();
    let mut seen_paths = Vec::new();
    for item in items {
        sync_one_file(&project_root, manifest, item, &mut report)?;
        seen_paths.push(item.content.source_path.clone());
    }

    if mode == SessionSyncMode::Full {
        mark_missing_files_deleted(&project_root, manifest, &seen_paths, &mut report)?;
    }

    Ok(report)
}

/// session 中可被现有 scanner 直接扫描的项目镜像根目录。
pub fn project_mirror_root(session_dir: &Path) -> PathBuf {
    session_dir.join("project")
}

fn sync_one_file(
    project_root: &Path,
    manifest: &mut SessionManifest,
    item: &SessionSyncItem,
    report: &mut SessionSyncReport,
) -> Result<()> {
    let source_path = &item.content.source_path;
    let relative_path = checked_relative_path(source_path)?;
    let hash = hash_text(&item.content.raw_text);
    let size = item.size.or(Some(item.content.raw_text.len() as u64));

    let existing_index = manifest
        .files
        .iter()
        .position(|file| file.source_path == *source_path);

    if let Some(index) = existing_index {
        let existing = &manifest.files[index];
        if !existing.deleted
            && existing.revision == item.content.revision
            && existing.etag == item.etag
            && existing.hash.as_deref() == Some(hash.as_str())
        {
            report.skipped += 1;
            return Ok(());
        }
    }

    let target_path = project_root.join(relative_path);
    atomic_write_text(&target_path, &item.content.raw_text)?;

    let file_record = RemoteSessionFile {
        source_path: source_path.clone(),
        file_id: item.content.file_id.clone(),
        revision: item.content.revision.clone(),
        etag: item.etag.clone(),
        mtime: item.mtime,
        size,
        hash: Some(hash),
        deleted: false,
    };

    if let Some(index) = existing_index {
        manifest.files[index] = file_record;
    } else {
        manifest.files.push(file_record);
        manifest
            .files
            .sort_by(|left, right| left.source_path.cmp(&right.source_path));
    }
    report.written += 1;
    Ok(())
}

fn mark_missing_files_deleted(
    project_root: &Path,
    manifest: &mut SessionManifest,
    seen_paths: &[String],
    report: &mut SessionSyncReport,
) -> Result<()> {
    let mut deleted_paths = Vec::new();
    for file in &mut manifest.files {
        if file.deleted || seen_paths.iter().any(|path| path == &file.source_path) {
            continue;
        }
        file.deleted = true;
        deleted_paths.push(file.source_path.clone());
    }

    for source_path in deleted_paths {
        let target_path = project_root.join(checked_relative_path(&source_path)?);
        if target_path.exists() {
            fs::remove_file(&target_path).with_context(|| {
                format!(
                    "failed to remove deleted session file {}",
                    target_path.display()
                )
            })?;
        }
        report.deleted += 1;
    }
    Ok(())
}

pub(crate) fn checked_relative_path(source_path: &str) -> Result<PathBuf> {
    if !is_safe_logical_path(source_path) {
        bail!("invalid session source_path: {}", source_path);
    }
    let path = PathBuf::from(source_path);
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => bail!("invalid session source_path: {}", source_path),
        }
    }
    Ok(path)
}

fn is_safe_logical_path(source_path: &str) -> bool {
    if !is_logical_source_path(source_path) {
        return false;
    }
    if source_path.len() >= 2 && source_path.as_bytes()[1] == b':' {
        return false;
    }
    true
}

pub(crate) fn atomic_write_text(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create session file dir {}", parent.display()))?;
    }
    let tmp_path = path.with_extension(format!(
        "{}tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| format!("{ext}."))
            .unwrap_or_default()
    ));
    fs::write(&tmp_path, text).with_context(|| {
        format!(
            "failed to write temporary session file {}",
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, path)
        .with_context(|| format!("failed to replace session file {}", path.display()))?;
    Ok(())
}

pub(crate) fn hash_text(text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_metadata::MetadataContentType;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    fn test_root(name: &str) -> PathBuf {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!(
            "metadata-checker-session-sync-test-{pid}-{seq}-{}",
            hash_text(name)
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn manifest() -> SessionManifest {
        SessionManifest::new(
            "s1",
            "https://host",
            "analyzer",
            "analyzer",
            "remote",
            "graph.redb",
            1,
        )
    }

    fn content(path: &str, revision: &str, raw_text: &str) -> RemoteFileContent {
        RemoteFileContent {
            source_path: path.to_string(),
            file_id: Some(format!("id-{}", path.replace('/', "-"))),
            revision: Some(revision.to_string()),
            content_type: MetadataContentType::from_extension(
                path.rsplit('.').next().unwrap_or(""),
            ),
            raw_text: raw_text.to_string(),
        }
    }

    #[test]
    fn sync_remote_files_writes_project_mirror_and_manifest() {
        let root = test_root("write");
        let mut manifest = manifest();
        let report = sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content(
                "app/Page.spg",
                "1",
                "{\"a\":1}",
            ))],
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.written, 1);
        assert_eq!(manifest.files.len(), 1);
        assert!(project_mirror_root(&root).join("app/Page.spg").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_remote_files_rejects_path_escape() {
        let root = test_root("escape");
        let mut manifest = manifest();
        let err = sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content("../Page.spg", "1", "{}"))],
            SessionSyncMode::Partial,
        )
        .unwrap_err();

        assert!(err.to_string().contains("invalid session source_path"));
    }

    #[test]
    fn sync_remote_files_skips_unchanged_file() {
        let root = test_root("skip");
        let mut manifest = manifest();
        let item = SessionSyncItem {
            etag: Some("e1".to_string()),
            ..SessionSyncItem::new(content("app/Page.spg", "1", "{}"))
        };
        sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[item.clone()],
            SessionSyncMode::Partial,
        )
        .unwrap();
        let report =
            sync_remote_files_to_session(&root, &mut manifest, &[item], SessionSyncMode::Partial)
                .unwrap();

        assert_eq!(report.skipped, 1);
        assert_eq!(report.written, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_remote_files_full_mode_marks_missing_deleted() {
        let root = test_root("delete");
        let mut manifest = manifest();
        sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[
                SessionSyncItem::new(content("app/A.spg", "1", "A")),
                SessionSyncItem::new(content("app/B.spg", "1", "B")),
            ],
            SessionSyncMode::Partial,
        )
        .unwrap();

        let report = sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content("app/A.spg", "2", "A2"))],
            SessionSyncMode::Full,
        )
        .unwrap();

        assert_eq!(report.deleted, 1);
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/B.spg" && file.deleted)
        );
        assert!(!project_mirror_root(&root).join("app/B.spg").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_remote_files_failed_write_keeps_existing_file() {
        let root = test_root("keep-old");
        let mut manifest = manifest();
        sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content("app/Page.spg", "1", "old"))],
            SessionSyncMode::Partial,
        )
        .unwrap();

        let blocked_file = project_mirror_root(&root).join("app/Blocked.spg");
        fs::write(&blocked_file, "not a directory").unwrap();
        let err = sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content(
                "app/Blocked.spg/Child.spg",
                "1",
                "new",
            ))],
            SessionSyncMode::Partial,
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("failed to create session file dir")
        );
        let old_text = fs::read_to_string(project_mirror_root(&root).join("app/Page.spg")).unwrap();
        assert_eq!(old_text, "old");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_remote_files_manifest_does_not_store_secret_words() {
        let root = test_root("secret");
        let mut manifest = manifest();
        sync_remote_files_to_session(
            &root,
            &mut manifest,
            &[SessionSyncItem::new(content("app/Page.spg", "1", "{}"))],
            SessionSyncMode::Partial,
        )
        .unwrap();
        let json = serde_json::to_string(&manifest).unwrap();

        assert!(!json.contains("token"));
        assert!(!json.contains("cookie"));
        assert!(!json.contains("password"));
        let _ = fs::remove_dir_all(root);
    }
}
