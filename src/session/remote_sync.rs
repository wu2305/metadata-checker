//! M41.10-A: 远程 provider -> session 同步
//!
//! 使用 RemoteSessionProvider 拉取内容，调用 sync_remote_files_to_session 写入 mirror。
//! 覆盖 partial / full 两种同步模式。

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

use crate::remote_metadata::{MetadataContentType, RemoteFileRef};

use crate::graph_store::IndexReport;
use crate::session::{
    RemoteSessionProvider, SessionManager,
    manifest::RemoteSessionFile,
    sync::{
        SessionSyncItem, SessionSyncMode, SessionSyncReport, project_mirror_root,
        sync_remote_files_to_session,
    },
};

/// 远程发现与筛选参数。
#[derive(Debug, Clone)]
pub struct SessionRefreshFilter {
    /// 目录/模块筛选条件。
    pub module: Option<String>,
    /// 精确 source_path 筛选条件。
    pub source_path: Option<String>,
    /// 精确 file_id 筛选条件。
    pub file_id: Option<String>,
    /// 当前页面优先顺序标记。
    pub current_source_path: Option<String>,
}

/// 远程刷新文件处理统计。
#[derive(Debug, Clone, Default)]
pub struct SessionRefreshFileReport {
    /// 发现到的远程文件总量（仅按筛选器后的集合统计）。
    pub discovered: usize,
    /// 成功进入可分析队列的文件数（仅 .spg/.tbl）。
    pub analyzable: usize,
    /// 成功写入 session 的文件数。
    pub synced: usize,
    /// 跳过文件数（不可分析/非目标/已跳过）。
    pub skipped: usize,
    /// 获取失败文件数。
    pub failed: usize,
}

/// 远程刷新诊断。
#[derive(Debug, Clone)]
pub struct SessionRefreshDiagnostic {
    /// 诊断码。
    pub code: String,
    /// 诊断消息。
    pub message: String,
}

/// 从远程 provider 同步项目元数据到本地 session。
pub fn sync_project_from_remote(
    provider: &dyn RemoteSessionProvider,
    manager: &SessionManager,
    session_id: &str,
    project_ref: &str,
    mode: SessionSyncMode,
) -> Result<SessionSyncReport> {
    let (report, _, _) = sync_project_from_remote_with_filter(
        provider,
        manager,
        session_id,
        project_ref,
        mode,
        None,
    )?;
    Ok(report)
}

/// 带筛选能力的远程同步核心逻辑，返回 sync 报告、文件报告与诊断。
pub fn sync_project_from_remote_with_filter(
    provider: &dyn RemoteSessionProvider,
    manager: &SessionManager,
    session_id: &str,
    project_ref: &str,
    mode: SessionSyncMode,
    filter: Option<&SessionRefreshFilter>,
) -> Result<(
    SessionSyncReport,
    SessionRefreshFileReport,
    Vec<SessionRefreshDiagnostic>,
)> {
    let session_dir = manager.session_dir(session_id);
    let mut manifest = manager.read_manifest(session_id)?;

    let remote_files = provider
        .list_metafiles(project_ref)
        .with_context(|| format!("failed to list metafiles for project {}", project_ref))?;

    for entry in &remote_files {
        validate_remote_entry(entry)?;
    }

    let mut diagnostics = Vec::new();
    let filtered = filter_remote_entries(&remote_files, filter);
    let is_filter_miss = filter.is_some_and(has_entry_filter) && filtered.is_empty();
    if is_filter_miss {
        diagnostics.push(SessionRefreshDiagnostic {
            code: "REMOTE_FILTER_MISS".to_string(),
            message: format!("remote filter matched no files for project {}", project_ref),
        });
    }
    let ordered = order_remote_entries(filtered, filter);

    let mut file_report = SessionRefreshFileReport::default();
    file_report.discovered = ordered.len();
    let analyzable_paths: Vec<String> = ordered
        .iter()
        .filter(|entry| {
            if entry.deleted {
                return true;
            }
            is_analyzable_entry(entry)
        })
        .map(|entry| entry.source_path.clone())
        .collect();

    let mut sync_items = Vec::new();
    let mut synced_paths = Vec::new();
    let mut failed_paths = Vec::new();
    for entry in &ordered {
        if !entry.deleted && is_analyzable_entry(entry) {
            file_report.analyzable += 1;
            let file_ref = file_ref_from_entry(entry)?;
            match provider.fetch_metafile_content(&file_ref).with_context(|| {
                format!(
                    "failed to fetch content for {} in project {}",
                    entry.source_path, project_ref
                )
            }) {
                Ok(content) => {
                    sync_items.push(SessionSyncItem {
                        content: content.clone(),
                        etag: entry.etag.clone(),
                        mtime: entry.mtime,
                        size: entry.size,
                    });
                    synced_paths.push(entry.source_path.clone());
                    file_report.synced += 1;
                }
                Err(error) => {
                    file_report.failed += 1;
                    failed_paths.push(entry.source_path.clone());
                    diagnostics.push(SessionRefreshDiagnostic {
                        code: "REMOTE_FETCH_FAILED".to_string(),
                        message: error.to_string(),
                    });
                }
            }
        } else if !entry.deleted {
            file_report.skipped += 1;
            diagnostics.push(SessionRefreshDiagnostic {
                code: "UNSUPPORTED_FILE_TYPE".to_string(),
                message: format!("skip unsupported entry {}", entry.source_path),
            });
        }
    }

    let mut report = sync_remote_files_to_session(
        &session_dir,
        &mut manifest,
        &sync_items,
        SessionSyncMode::Partial,
    )
    .with_context(|| format!("failed to sync files to session {}", session_id))?;

    let scope = discover_scope_paths(&ordered, filter);
    for entry in &ordered {
        if entry.deleted {
            if should_apply_scope(entry, &scope) && mode == SessionSyncMode::Full {
                if let Some(record) = manifest
                    .files
                    .iter_mut()
                    .find(|file| file.source_path == entry.source_path)
                {
                    if !record.deleted {
                        if mark_entry_deleted(&session_dir, &mut manifest, entry)? {
                            report.deleted += 1;
                        }
                    }
                } else if should_apply_scope(entry, &scope) {
                    if mark_entry_deleted(&session_dir, &mut manifest, entry)? {
                        report.deleted += 1;
                    }
                }
            } else if mode != SessionSyncMode::Full {
                if mark_entry_deleted(&session_dir, &mut manifest, entry)? {
                    report.deleted += 1;
                }
            }
        } else {
            merge_remote_metadata(&mut manifest, entry);
        }
    }

    if mode == SessionSyncMode::Full && !is_filter_miss {
        file_report.skipped += mark_missing_files_deleted(
            &session_dir,
            &mut manifest,
            &synced_paths,
            &failed_paths,
            &scope,
            &analyzable_paths,
            &mut report,
        )?;
    } else if mode == SessionSyncMode::Full && is_filter_miss {
        diagnostics.push(SessionRefreshDiagnostic {
            code: "REMOTE_FILTER_MISS_NO_DELETE".to_string(),
            message: "full sync skipped missing-scope file deletion because filter had no matches"
                .to_string(),
        });
    }

    manager.write_manifest(&manifest)?;

    Ok((report, file_report, diagnostics))
}

fn filter_remote_entries(
    entries: &[crate::session::remote_provider::RemoteMetafileEntry],
    filter: Option<&SessionRefreshFilter>,
) -> Vec<crate::session::remote_provider::RemoteMetafileEntry> {
    let Some(filter) = filter else {
        return entries.to_vec();
    };
    entries
        .iter()
        .filter(|entry| match_entry_filter(entry, filter))
        .cloned()
        .collect()
}

fn match_entry_filter(
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
    filter: &SessionRefreshFilter,
) -> bool {
    if let Some(module) = &filter.module {
        let prefix = format!("{module}/");
        if entry.source_path != *module && !entry.source_path.starts_with(&prefix) {
            return false;
        }
    }
    if let Some(source_path) = &filter.source_path {
        if entry.source_path != *source_path {
            return false;
        }
    }
    if let Some(file_id) = &filter.file_id {
        if entry.file_id.as_deref() != Some(file_id.as_str()) {
            return false;
        }
    }
    true
}

fn has_entry_filter(filter: &SessionRefreshFilter) -> bool {
    filter.module.is_some() || filter.source_path.is_some() || filter.file_id.is_some()
}

fn discover_scope_paths(
    entries: &[crate::session::remote_provider::RemoteMetafileEntry],
    filter: Option<&SessionRefreshFilter>,
) -> Vec<String> {
    if let Some(filter) = filter {
        if let Some(module) = &filter.module {
            return vec![format!("{module}/")];
        }
        if let Some(source_path) = &filter.source_path {
            return vec![source_path.clone()];
        }
        let file_id_path = filter
            .file_id
            .as_ref()
            .and_then(|file_id| {
                entries
                    .iter()
                    .find(|entry| entry.file_id.as_ref() == Some(file_id))
                    .map(|entry| entry.source_path.clone())
            })
            .into_iter()
            .collect::<Vec<_>>();
        if !file_id_path.is_empty() {
            return file_id_path;
        }
    }

    Vec::new()
}

fn should_apply_scope(
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
    scope_paths: &[String],
) -> bool {
    if scope_paths.is_empty() {
        return true;
    }
    scope_paths
        .iter()
        .any(|scope| should_match_scope(&entry.source_path, scope))
}

fn should_match_scope_paths(source_path: &str, scope_paths: &[String]) -> bool {
    scope_paths.is_empty()
        || scope_paths
            .iter()
            .any(|scope| should_match_scope(source_path, scope))
}

fn should_match_scope(source_path: &str, scope: &str) -> bool {
    if source_path == scope {
        return true;
    }
    if !scope.ends_with('/') {
        return source_path.starts_with(&format!("{scope}/"));
    }
    source_path.starts_with(scope)
}

fn order_remote_entries(
    mut entries: Vec<crate::session::remote_provider::RemoteMetafileEntry>,
    filter: Option<&SessionRefreshFilter>,
) -> Vec<crate::session::remote_provider::RemoteMetafileEntry> {
    let current = filter.and_then(|f| f.current_source_path.as_deref());
    entries.sort_by(|left, right| {
        let left_score = if current == Some(left.source_path.as_str()) {
            0
        } else {
            1
        };
        let right_score = if current == Some(right.source_path.as_str()) {
            0
        } else {
            1
        };
        left_score
            .cmp(&right_score)
            .then(left.source_path.cmp(&right.source_path))
    });
    entries
}

fn is_analyzable_entry(entry: &crate::session::remote_provider::RemoteMetafileEntry) -> bool {
    matches!(
        entry.content_type(),
        MetadataContentType::SuperPage | MetadataContentType::Table
    )
}

fn validate_remote_entry(
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
) -> Result<()> {
    if entry.source_path.contains('\\')
        || (entry.source_path.len() >= 2 && entry.source_path.as_bytes()[1] == b':')
    {
        return Err(anyhow!(
            "invalid file ref: source_path '{}' is not a project-internal logical path",
            entry.source_path
        ));
    }
    file_ref_from_entry(entry)?;
    Ok(())
}

fn file_ref_from_entry(
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
) -> Result<RemoteFileRef> {
    RemoteFileRef::try_new(
        &entry.project_ref,
        &entry.source_path,
        entry.file_id.clone(),
    )
    .map_err(|e| anyhow!("invalid file ref: {}", e))
}

fn merge_remote_metadata(
    manifest: &mut crate::session::manifest::SessionManifest,
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
) {
    if let Some(record) = manifest
        .files
        .iter_mut()
        .find(|file| file.source_path == entry.source_path)
    {
        record.file_id = entry.file_id.clone();
        record.revision = entry.revision.clone();
        record.etag = entry.etag.clone();
        record.mtime = entry.mtime;
        if entry.size.is_some() {
            record.size = entry.size;
        }
        record.deleted = false;
    }
}

fn mark_entry_deleted(
    session_dir: &std::path::Path,
    manifest: &mut crate::session::manifest::SessionManifest,
    entry: &crate::session::remote_provider::RemoteMetafileEntry,
) -> Result<bool> {
    let mut changed = if let Some(record) = manifest
        .files
        .iter_mut()
        .find(|file| file.source_path == entry.source_path)
    {
        let was_active = !record.deleted;
        record.file_id = entry.file_id.clone();
        record.revision = entry.revision.clone();
        record.etag = entry.etag.clone();
        record.mtime = entry.mtime;
        record.size = entry.size;
        record.deleted = true;
        was_active
    } else {
        manifest.files.push(RemoteSessionFile {
            source_path: entry.source_path.clone(),
            file_id: entry.file_id.clone(),
            revision: entry.revision.clone(),
            etag: entry.etag.clone(),
            mtime: entry.mtime,
            size: entry.size,
            hash: None,
            deleted: true,
        });
        manifest
            .files
            .sort_by(|left, right| left.source_path.cmp(&right.source_path));
        true
    };

    let mirror_path = project_mirror_root(session_dir).join(&entry.source_path);
    if mirror_path.exists() {
        fs::remove_file(&mirror_path).with_context(|| {
            format!(
                "failed to remove deleted remote file {}",
                mirror_path.display()
            )
        })?;
        changed = true;
    }
    Ok(changed)
}

fn mark_missing_files_deleted(
    session_dir: &std::path::Path,
    manifest: &mut crate::session::manifest::SessionManifest,
    synced_paths: &[String],
    failed_paths: &[String],
    scope_paths: &[String],
    analyzable_paths: &[String],
    report: &mut SessionSyncReport,
) -> Result<usize> {
    let mut skipped = 0usize;
    let target_paths: Vec<String> = manifest
        .files
        .iter()
        .map(|file| file.source_path.clone())
        .collect();
    for path in target_paths {
        if failed_paths.iter().any(|candidate| candidate == &path) {
            continue;
        }
        if should_match_scope_paths(&path, scope_paths)
            && !analyzable_paths.iter().any(|candidate| candidate == &path)
            && !synced_paths.iter().any(|item| item == &path)
        {
            if let Some(record) = manifest
                .files
                .iter_mut()
                .find(|file| file.source_path == path)
            {
                if record.deleted {
                    continue;
                }
            }
            if mark_missing_path_deleted(session_dir, manifest, &path)? {
                report.deleted += 1;
            }
            skipped += 1;
        }
    }
    Ok(skipped)
}

fn mark_missing_path_deleted(
    session_dir: &std::path::Path,
    manifest: &mut crate::session::manifest::SessionManifest,
    source_path: &str,
) -> Result<bool> {
    let changed = if let Some(record) = manifest
        .files
        .iter_mut()
        .find(|file| file.source_path == source_path)
    {
        let was_active = !record.deleted;
        record.deleted = true;
        was_active
    } else {
        manifest.files.push(RemoteSessionFile {
            source_path: source_path.to_string(),
            file_id: None,
            revision: None,
            etag: None,
            mtime: None,
            size: None,
            hash: None,
            deleted: true,
        });
        manifest
            .files
            .sort_by(|left, right| left.source_path.cmp(&right.source_path));
        true
    };

    let mirror_path = project_mirror_root(session_dir).join(source_path);
    if mirror_path.exists() {
        fs::remove_file(&mirror_path).with_context(|| {
            format!(
                "failed to remove deleted remote file {}",
                mirror_path.display()
            )
        })?;
    }
    Ok(changed)
}

#[cfg(feature = "cli-local")]
pub fn build_session_graph(
    session_dir: &std::path::Path,
    graph_db_path: &std::path::Path,
) -> Result<crate::graph_store::IndexReport> {
    let mirror = crate::session::sync::project_mirror_root(session_dir);
    crate::scanner::indexer::ProjectIndexer::scan(&mirror, graph_db_path).with_context(|| {
        format!(
            "failed to build graph for session {}",
            session_dir.display()
        )
    })
}

/// Session 刷新选项。
#[derive(Debug, Clone)]
pub struct SessionRefreshOptions {
    pub session_id: String,
    pub remote_server: String,
    pub project_ref: String,
    pub project_name: String,
    pub sync_mode: SessionSyncMode,
    pub create_if_missing: bool,
    /// 可选的同步筛选条件。
    pub filter: Option<SessionRefreshFilter>,
    /// 可选 graph 数据库路径覆盖。
    pub graph_db_path: Option<PathBuf>,
}

/// Session 刷新报告。
#[derive(Debug, Clone)]
pub struct SessionRefreshReport {
    pub ok: bool,
    pub session_id: String,
    pub project_ref: String,
    pub session_dir: String,
    pub graph_db_path: String,
    pub sync: SessionSyncReport,
    pub files: SessionRefreshFileReport,
    pub diagnostics: Vec<SessionRefreshDiagnostic>,
    pub index: IndexReport,
    pub error: Option<SessionRefreshError>,
}

#[derive(Debug, Clone)]
pub struct SessionRefreshError {
    pub code: String,
    pub message: String,
}

/// 统一的 session refresh runner。
///
/// 覆盖：创建/读取 session -> sync -> build graph -> 统一 report。
#[cfg(feature = "cli-local")]
pub fn refresh_session_from_remote(
    provider: &dyn RemoteSessionProvider,
    manager: &SessionManager,
    options: SessionRefreshOptions,
) -> Result<SessionRefreshReport> {
    let SessionRefreshOptions {
        session_id,
        remote_server,
        project_ref,
        project_name,
        sync_mode,
        create_if_missing,
        filter,
        graph_db_path,
    } = options;

    // 读取或创建 session
    let _manifest = match manager.read_manifest(&session_id) {
        Ok(m) => m,
        Err(_) if create_if_missing => manager.create_session(
            &session_id,
            &remote_server,
            &project_ref,
            &project_name,
            "remote",
        )?,
        Err(e) => return Err(e),
    };

    let session_dir = manager.session_dir(&session_id);
    let graph_db_path = graph_db_path.unwrap_or_else(|| session_dir.join("graph.redb"));
    if let Some(parent) = graph_db_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to prepare graph db parent {}", parent.display()))?;
    }

    // 同步远程文件
    let (sync_report, file_report, diagnostics) = sync_project_from_remote_with_filter(
        provider,
        manager,
        &session_id,
        &project_ref,
        sync_mode,
        filter.as_ref(),
    )
    .with_context(|| {
        format!(
            "failed to sync project {} to session {}",
            project_ref, session_id
        )
    })?;

    let mut manifest = manager.read_manifest(&session_id)?;
    let graph_db_path_text = graph_db_path.to_string_lossy().to_string();
    if manifest.graph_db_path != graph_db_path_text {
        manifest.graph_db_path = graph_db_path_text;
        manager.write_manifest(&manifest)?;
    }

    // 构建 graph
    let index_report = build_session_graph(&session_dir, &graph_db_path)
        .with_context(|| format!("failed to build graph for session {}", session_id))?;

    Ok(SessionRefreshReport {
        ok: true,
        session_id: session_id.clone(),
        project_ref: project_ref.clone(),
        session_dir: session_dir.to_string_lossy().to_string(),
        graph_db_path: graph_db_path.to_string_lossy().to_string(),
        sync: sync_report,
        files: file_report,
        diagnostics,
        index: index_report,
        error: None,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_metadata::{
        MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
    };
    use crate::session::remote_provider::{
        InMemoryRemoteSessionProvider, RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
    };
    use std::fs;
    use std::sync::{Arc, Mutex};

    fn test_root(name: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "metadata-checker-remote-sync-test-{}-{}-{}",
            name,
            std::process::id(),
            id
        ))
    }

    #[test]
    fn sync_project_from_remote_full_mode() {
        let root = test_root("full");
        let manager = SessionManager::new(&root);
        let _session = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(100),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: crate::remote_metadata::MetadataContentType::SuperPage,
                    raw_text: r#"{"components": []}"#.to_string(),
                },
            )
            .unwrap();

        let report =
            sync_project_from_remote(&provider, &manager, "s1", "proj", SessionSyncMode::Full)
                .unwrap();

        assert_eq!(report.written, 1);

        let manifest = manager.read_manifest("s1").unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].source_path, "app/Page.spg");
        assert!(manifest.files[0].hash.is_some());

        let mirror_file = root.join("s1").join("project").join("app").join("Page.spg");
        assert!(mirror_file.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_partial_mode() {
        let root = test_root("partial");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(100),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: crate::remote_metadata::MetadataContentType::SuperPage,
                    raw_text: r#"{"components": []}"#.to_string(),
                },
            )
            .unwrap();

        let report =
            sync_project_from_remote(&provider, &manager, "s1", "proj", SessionSyncMode::Partial)
                .unwrap();

        assert_eq!(report.written, 1);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_partial_keeps_unmentioned_files() {
        let root = test_root("partial-keeps");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/PageA.spg",
            "id-a",
            "1",
            false,
            "old-a",
        );
        add_page(
            &mut initial_provider,
            "app/PageB.spg",
            "id-b",
            "1",
            false,
            "old-b",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let mut partial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut partial_provider);
        add_page(
            &mut partial_provider,
            "app/PageA.spg",
            "id-a",
            "2",
            false,
            "new-a",
        );
        let report = sync_project_from_remote(
            &partial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.written, 1);
        assert_eq!(report.deleted, 0);
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("PageA.spg")
            )
            .unwrap(),
            "new-a"
        );
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("PageB.spg")
            )
            .unwrap(),
            "old-b"
        );

        let manifest = manager.read_manifest("s1").unwrap();
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/PageB.spg" && !file.deleted)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_partial_deleted_entry_removes_existing_file() {
        let root = test_root("deleted");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut first_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut first_provider);
        add_page(
            &mut first_provider,
            "app/Page.spg",
            "id1",
            "1",
            false,
            "old",
        );
        sync_project_from_remote(
            &first_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        let mirror_file = root.join("s1").join("project").join("app").join("Page.spg");
        assert!(mirror_file.exists());

        let mut deleted_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut deleted_provider);
        add_page(
            &mut deleted_provider,
            "app/Page.spg",
            "id1",
            "2",
            true,
            "ignored",
        );
        let report = sync_project_from_remote(
            &deleted_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.deleted, 1);
        let manifest = manager.read_manifest("s1").unwrap();
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/Page.spg" && file.deleted)
        );
        assert!(!mirror_file.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_deleted_entry_does_not_fetch_content() {
        let root = test_root("deleted-no-fetch");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/Page.spg",
            "id1",
            "1",
            false,
            "old",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        let report = sync_project_from_remote(
            &DeletedEntryProvider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.deleted, 1);
        assert!(
            manager
                .read_manifest("s1")
                .unwrap()
                .files
                .iter()
                .any(|file| file.source_path == "app/Page.spg" && file.deleted)
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_fetch_failure_keeps_previous_session() {
        let root = test_root("failure");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/Page.spg",
            "id1",
            "1",
            false,
            "old",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();
        let failing_provider = FailingContentProvider;
        let report = sync_project_from_remote(
            &failing_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.written, 1);
        let after = manager.read_manifest("s1").unwrap();
        assert!(after.files.len() > 0);
        let mirror_text =
            fs::read_to_string(root.join("s1").join("project").join("app").join("Page.spg"))
                .unwrap();
        assert_eq!(mirror_text, "new");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_full_deletes_remote_missing_files() {
        let root = test_root("full-delete-missing");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/Keep.spg",
            "keep",
            "1",
            false,
            "keep-old",
        );
        add_page(
            &mut initial_provider,
            "app/Missing.spg",
            "missing",
            "1",
            false,
            "missing-old",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let mut next_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut next_provider);
        add_page(
            &mut next_provider,
            "app/Keep.spg",
            "keep",
            "2",
            false,
            "keep-new",
        );
        let report = sync_project_from_remote(
            &next_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        assert_eq!(report.deleted, 1);
        let manifest = manager.read_manifest("s1").unwrap();
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/Missing.spg" && file.deleted)
        );
        assert!(
            !root
                .join("s1")
                .join("project")
                .join("app")
                .join("Missing.spg")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(root.join("s1").join("project").join("app").join("Keep.spg"))
                .unwrap(),
            "keep-new"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_full_keeps_failed_fetch_file() {
        let root = test_root("full-fetch-failure-keeps");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/Broken.spg",
            "broken",
            "1",
            false,
            "old-broken",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let (report, file_report, diagnostics) = sync_project_from_remote_with_filter(
            &FailingContentProvider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
            None,
        )
        .unwrap();

        assert_eq!(file_report.failed, 1);
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "REMOTE_FETCH_FAILED")
        );
        assert_eq!(report.deleted, 0);
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("Broken.spg")
            )
            .unwrap(),
            "old-broken"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_only_spg_tbl_are_analyzable() {
        let root = test_root("analyzable-filter");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(10),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: MetadataContentType::SuperPage,
                    raw_text: "{}".to_string(),
                },
            )
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "data/tables/test.tbl".to_string(),
                    file_id: Some("id2".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(10),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "data/tables/test.tbl".to_string(),
                    file_id: Some("id2".to_string()),
                    revision: Some("1".to_string()),
                    content_type: MetadataContentType::Table,
                    raw_text: "{}".to_string(),
                },
            )
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "readme.md".to_string(),
                    file_id: Some("id3".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(10),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "readme.md".to_string(),
                    file_id: Some("id3".to_string()),
                    revision: Some("1".to_string()),
                    content_type: MetadataContentType::Unknown,
                    raw_text: "ignore me".to_string(),
                },
            )
            .unwrap();

        let (_sync_report, file_report, diagnostics) = sync_project_from_remote_with_filter(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
            None,
        )
        .unwrap();

        assert_eq!(file_report.discovered, 3);
        assert_eq!(file_report.analyzable, 2);
        assert_eq!(file_report.synced, 2);
        assert_eq!(file_report.skipped, 1);
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "UNSUPPORTED_FILE_TYPE")
        );

        let manifest = manager.read_manifest("s1").unwrap();
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/Page.spg")
        );
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "data/tables/test.tbl")
        );
        assert!(
            !manifest
                .files
                .iter()
                .any(|file| file.source_path == "readme.md")
        );

        assert!(!root.join("s1").join("project").join("readme.md").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_current_source_path_does_not_limit_full_scope() {
        let root = test_root("current-not-scope");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/current.spg",
            "current",
            "1",
            false,
            "current-old",
        );
        add_page(
            &mut initial_provider,
            "app/removed.spg",
            "removed",
            "1",
            false,
            "removed-old",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let provider = OrderedFetchProvider::new(vec![(
            "app/current.spg".to_string(),
            "current".to_string(),
            "current-new".to_string(),
        )]);
        let filter = SessionRefreshFilter {
            module: None,
            source_path: None,
            file_id: None,
            current_source_path: Some("app/current.spg".to_string()),
        };
        let (report, _, _) = sync_project_from_remote_with_filter(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
            Some(&filter),
        )
        .unwrap();

        assert_eq!(report.deleted, 1);
        assert!(
            manager
                .read_manifest("s1")
                .unwrap()
                .files
                .iter()
                .any(|file| file.source_path == "app/removed.spg" && file.deleted)
        );
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("current.spg")
            )
            .unwrap(),
            "current-new"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_rejects_unsafe_paths_before_fetch() {
        let root = test_root("unsafe");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let provider = UnsafePathProvider {
            source_path: "C:\\escape.spg".to_string(),
        };
        let err =
            sync_project_from_remote(&provider, &manager, "s1", "proj", SessionSyncMode::Partial)
                .unwrap_err();

        assert!(err.to_string().contains("invalid file ref"));
        assert!(
            !root
                .join("s1")
                .join("project")
                .join("C:\\escape.spg")
                .exists()
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_current_source_path_first() {
        let root = test_root("current-order");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let provider = OrderedFetchProvider::new(vec![
            (
                "app/zzz.spg".to_string(),
                "id-zzz".to_string(),
                "{}".to_string(),
            ),
            (
                "app/current.spg".to_string(),
                "id-current".to_string(),
                "{}".to_string(),
            ),
            (
                "app/aaa.spg".to_string(),
                "id-aaa".to_string(),
                "{}".to_string(),
            ),
        ]);

        let filter = SessionRefreshFilter {
            module: None,
            source_path: None,
            file_id: None,
            current_source_path: Some("app/current.spg".to_string()),
        };
        let (_sync_report, file_report, _) = sync_project_from_remote_with_filter(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
            Some(&filter),
        )
        .unwrap();

        assert_eq!(file_report.synced, 3);
        assert_eq!(
            provider.fetch_order().unwrap()[0],
            "app/current.spg".to_string()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(feature = "cli-local")]
    fn sync_and_build_graph_end_to_end() {
        let root = test_root("e2e");
        let manager = SessionManager::new(&root);
        let _session = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();

        // 同步一个最小 .spg
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("spg1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(200),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("spg1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: crate::remote_metadata::MetadataContentType::SuperPage,
                    raw_text:
                        r#"{"pageName":"Page","components":[{"name":"btn1","type":"Button"}]}"#
                            .to_string(),
                },
            )
            .unwrap();

        // 同步 .tbl
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "data/tables/test.tbl".to_string(),
                    file_id: Some("tbl1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(100),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "data/tables/test.tbl".to_string(),
                    file_id: Some("tbl1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: crate::remote_metadata::MetadataContentType::Table,
                    raw_text: r#"{"tableName": "test"}"#.to_string(),
                },
            )
            .unwrap();

        let report =
            sync_project_from_remote(&provider, &manager, "s1", "proj", SessionSyncMode::Full)
                .unwrap();
        assert_eq!(report.written, 2);

        let graph_db_path = root.join("s1").join("graph.redb");
        let index_report = build_session_graph(&manager.session_dir("s1"), &graph_db_path).unwrap();
        assert!(index_report.indexed >= 1);
        assert!(graph_db_path.exists());

        let _ = fs::remove_dir_all(root);
    }

    fn register_project(provider: &mut InMemoryRemoteSessionProvider) {
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
    }

    fn add_page(
        provider: &mut InMemoryRemoteSessionProvider,
        source_path: &str,
        file_id: &str,
        revision: &str,
        deleted: bool,
        raw_text: &str,
    ) {
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: source_path.to_string(),
                    file_id: Some(file_id.to_string()),
                    revision: Some(revision.to_string()),
                    etag: Some(format!("etag-{revision}")),
                    mtime: Some(1715000000000),
                    size: Some(raw_text.len() as u64),
                    deleted,
                },
                RemoteFileContent {
                    source_path: source_path.to_string(),
                    file_id: Some(file_id.to_string()),
                    revision: Some(revision.to_string()),
                    content_type: MetadataContentType::SuperPage,
                    raw_text: raw_text.to_string(),
                },
            )
            .unwrap();
    }

    struct FailingContentProvider;

    impl RemoteSessionProvider for FailingContentProvider {
        fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
            Ok(Vec::new())
        }

        fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
            Ok(vec![
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("2".to_string()),
                    etag: Some("etag-2".to_string()),
                    mtime: Some(1715000000001),
                    size: Some(3),
                    deleted: false,
                },
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Broken.spg".to_string(),
                    file_id: Some("broken".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: None,
                    size: None,
                    deleted: false,
                },
            ])
        }

        fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
            Ok(RemoteFileInfo {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: file_ref.revision.clone(),
                content_type: MetadataContentType::SuperPage,
                updated_at: None,
            })
        }

        fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
            if file_ref.source_path == "app/Broken.spg" {
                return Err(anyhow!("mock remote failure"));
            }
            Ok(RemoteFileContent {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: Some("2".to_string()),
                content_type: MetadataContentType::SuperPage,
                raw_text: "new".to_string(),
            })
        }

        fn fetch_changed_since(
            &self,
            project_ref: &str,
            since_revision: &str,
        ) -> Result<RemoteChangeSet> {
            Ok(RemoteChangeSet {
                project_ref: project_ref.to_string(),
                project_name: project_ref.to_string(),
                since_revision: since_revision.to_string(),
                to_revision: None,
                changed_files: self.list_metafiles(project_ref)?,
            })
        }
    }

    struct DeletedEntryProvider;

    impl RemoteSessionProvider for DeletedEntryProvider {
        fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
            Ok(Vec::new())
        }

        fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
            Ok(vec![RemoteMetafileEntry {
                project_ref: "proj".to_string(),
                source_path: "app/Page.spg".to_string(),
                file_id: Some("id1".to_string()),
                revision: Some("2".to_string()),
                etag: Some("etag-2".to_string()),
                mtime: Some(1715000000001),
                size: Some(0),
                deleted: true,
            }])
        }

        fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
            Ok(RemoteFileInfo {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: file_ref.revision.clone(),
                content_type: MetadataContentType::SuperPage,
                updated_at: None,
            })
        }

        fn fetch_metafile_content(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
            Err(anyhow!("deleted entries must not fetch content"))
        }

        fn fetch_changed_since(
            &self,
            project_ref: &str,
            since_revision: &str,
        ) -> Result<RemoteChangeSet> {
            Ok(RemoteChangeSet {
                project_ref: project_ref.to_string(),
                project_name: project_ref.to_string(),
                since_revision: since_revision.to_string(),
                to_revision: None,
                changed_files: self.list_metafiles(project_ref)?,
            })
        }
    }

    struct UnsafePathProvider {
        source_path: String,
    }

    impl RemoteSessionProvider for UnsafePathProvider {
        fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
            Ok(Vec::new())
        }

        fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
            Ok(vec![RemoteMetafileEntry {
                project_ref: "proj".to_string(),
                source_path: self.source_path.clone(),
                file_id: Some("bad".to_string()),
                revision: Some("1".to_string()),
                etag: None,
                mtime: None,
                size: None,
                deleted: false,
            }])
        }

        fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
            Ok(RemoteFileInfo {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: file_ref.revision.clone(),
                content_type: MetadataContentType::SuperPage,
                updated_at: None,
            })
        }

        fn fetch_metafile_content(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
            Err(anyhow!("fetch should not be called for unsafe path"))
        }

        fn fetch_changed_since(
            &self,
            project_ref: &str,
            since_revision: &str,
        ) -> Result<RemoteChangeSet> {
            Ok(RemoteChangeSet {
                project_ref: project_ref.to_string(),
                project_name: project_ref.to_string(),
                since_revision: since_revision.to_string(),
                to_revision: None,
                changed_files: self.list_metafiles(project_ref)?,
            })
        }
    }

    #[derive(Clone)]
    struct OrderedFetchProvider {
        calls: Arc<Mutex<Vec<String>>>,
        files: Vec<(String, String, String)>,
    }

    impl OrderedFetchProvider {
        fn new(raw_files: Vec<(String, String, String)>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                files: raw_files,
            }
        }

        fn fetch_order(&self) -> Option<Vec<String>> {
            self.calls.lock().ok().map(|calls| calls.clone())
        }
    }

    impl RemoteSessionProvider for OrderedFetchProvider {
        fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
            Ok(vec![RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "ordered".to_string(),
                source_origin: "remote".to_string(),
            }])
        }

        fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
            if project_ref != "proj" {
                return Err(anyhow!("project not found"));
            }
            let items = self
                .files
                .iter()
                .map(|(source_path, file_id, _)| RemoteMetafileEntry {
                    project_ref: project_ref.to_string(),
                    source_path: source_path.clone(),
                    file_id: Some(file_id.clone()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(10),
                    deleted: false,
                })
                .collect::<Vec<_>>();
            Ok(items)
        }

        fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
            Ok(RemoteFileInfo {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: Some("1".to_string()),
                content_type: MetadataContentType::SuperPage,
                updated_at: None,
            })
        }

        fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
            let mut calls = self
                .calls
                .lock()
                .map_err(|_| anyhow!("failed to record fetch"))?;
            calls.push(file_ref.source_path.clone());

            let raw_text = self
                .files
                .iter()
                .find(|(path, _, _)| path == &file_ref.source_path)
                .ok_or_else(|| anyhow!("file not found"))?
                .2
                .clone();

            Ok(RemoteFileContent {
                source_path: file_ref.source_path.clone(),
                file_id: file_ref.file_id.clone(),
                revision: Some("1".to_string()),
                content_type: MetadataContentType::SuperPage,
                raw_text,
            })
        }

        fn fetch_changed_since(
            &self,
            project_ref: &str,
            since_revision: &str,
        ) -> Result<RemoteChangeSet> {
            Ok(RemoteChangeSet {
                project_ref: project_ref.to_string(),
                project_name: "ordered".to_string(),
                since_revision: since_revision.to_string(),
                to_revision: Some("1".to_string()),
                changed_files: self.list_metafiles(project_ref)?,
            })
        }
    }

    #[test]
    fn sync_project_from_remote_filter_miss_returns_diagnostic() {
        let root = test_root("filter-miss");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(10),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: MetadataContentType::SuperPage,
                    raw_text: "{}".to_string(),
                },
            )
            .unwrap();

        let filter = SessionRefreshFilter {
            module: None,
            source_path: Some("app/missing.spg".to_string()),
            file_id: None,
            current_source_path: None,
        };
        let (sync_report, file_report, diagnostics) = sync_project_from_remote_with_filter(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
            Some(&filter),
        )
        .unwrap();

        assert_eq!(sync_report.written, 0);
        assert_eq!(file_report.discovered, 0);
        assert_eq!(file_report.analyzable, 0);
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "REMOTE_FILTER_MISS")
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_project_from_remote_full_filter_file_id_miss_keeps_existing_files() {
        let root = test_root("full-filter-miss");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut initial_provider);
        add_page(
            &mut initial_provider,
            "app/Keep.spg",
            "keep",
            "1",
            false,
            "keep-old",
        );
        add_page(
            &mut initial_provider,
            "app/KeepOther.spg",
            "other",
            "1",
            false,
            "other-old",
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let mut filtered_provider = InMemoryRemoteSessionProvider::new();
        register_project(&mut filtered_provider);
        add_page(
            &mut filtered_provider,
            "app/Unknown.spg",
            "unknown",
            "2",
            false,
            "unknown-new",
        );
        let filter = SessionRefreshFilter {
            module: None,
            source_path: None,
            file_id: Some("file-id-not-exist".to_string()),
            current_source_path: None,
        };
        let (sync_report, file_report, diagnostics) = sync_project_from_remote_with_filter(
            &filtered_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
            Some(&filter),
        )
        .unwrap();

        assert_eq!(sync_report.deleted, 0);
        assert_eq!(file_report.discovered, 0);
        assert_eq!(file_report.analyzable, 0);
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "REMOTE_FILTER_MISS")
        );
        assert!(
            diagnostics
                .iter()
                .any(|item| item.code == "REMOTE_FILTER_MISS_NO_DELETE")
        );

        let manifest = manager.read_manifest("s1").unwrap();
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/Keep.spg" && !file.deleted)
        );
        assert!(
            manifest
                .files
                .iter()
                .any(|file| file.source_path == "app/KeepOther.spg" && !file.deleted)
        );
        assert!(
            root.join("s1")
                .join("project")
                .join("app")
                .join("Keep.spg")
                .exists()
        );
        assert!(
            root.join("s1")
                .join("project")
                .join("app")
                .join("KeepOther.spg")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(root.join("s1").join("project").join("app").join("Keep.spg"))
                .unwrap(),
            "keep-old"
        );
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("KeepOther.spg")
            )
            .unwrap(),
            "other-old"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sync_debug_does_not_leak_token_cookie_password() {
        let root = test_root("debug-secrets");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id-secret".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(40),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("id-secret".to_string()),
                    revision: Some("1".to_string()),
                    content_type: MetadataContentType::SuperPage,
                    raw_text: r#"{"token":"secret-token","cookie":"session-cookie","password":"secret-passwd"}"#
                        .to_string(),
                },
            )
            .unwrap();

        let options = SessionRefreshOptions {
            session_id: "s1".to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: "proj".to_string(),
            project_name: "proj".to_string(),
            sync_mode: SessionSyncMode::Partial,
            create_if_missing: false,
            filter: None,
            graph_db_path: None,
        };
        let report = refresh_session_from_remote(&provider, &manager, options).unwrap();
        let debug_text = format!("{:?}", report);
        assert!(!debug_text.contains("secret-token"));
        assert!(!debug_text.contains("session-cookie"));
        assert!(!debug_text.contains("secret-passwd"));
        assert!(!debug_text.contains("token=secret-token"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(feature = "cli-local")]
    fn refresh_session_from_remote_creates_session_when_missing() {
        let root = test_root("refresh-create");
        let manager = SessionManager::new(&root);

        let mut provider = InMemoryRemoteSessionProvider::new();
        provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        provider
            .add_metafile(
                RemoteMetafileEntry {
                    project_ref: "proj".to_string(),
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("spg1".to_string()),
                    revision: Some("1".to_string()),
                    etag: None,
                    mtime: Some(1715000000000),
                    size: Some(200),
                    deleted: false,
                },
                RemoteFileContent {
                    source_path: "app/Page.spg".to_string(),
                    file_id: Some("spg1".to_string()),
                    revision: Some("1".to_string()),
                    content_type: crate::remote_metadata::MetadataContentType::SuperPage,
                    raw_text: r#"{"pageName":"Page","components":[]}"#.to_string(),
                },
            )
            .unwrap();

        let options = SessionRefreshOptions {
            session_id: "s1".to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: "proj".to_string(),
            project_name: "proj".to_string(),
            sync_mode: SessionSyncMode::Full,
            create_if_missing: true,
            filter: None,
            graph_db_path: None,
        };

        let report = refresh_session_from_remote(&provider, &manager, options).unwrap();

        assert!(report.ok);
        assert_eq!(report.session_id, "s1");
        assert_eq!(report.project_ref, "proj");
        assert_eq!(report.sync.written, 1);
        assert!(report.index.indexed >= 1);
        assert!(std::path::Path::new(&report.graph_db_path).exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(feature = "cli-local")]
    fn refresh_session_from_remote_uses_filter_and_graph_db_path_override() {
        let root = test_root("refresh-filter-graph-override");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        let mut initial_provider = InMemoryRemoteSessionProvider::new();
        initial_provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        add_page(
            &mut initial_provider,
            "app/Target.spg",
            "target",
            "1",
            false,
            r#"{"components":[]}"#,
        );
        add_page(
            &mut initial_provider,
            "app/Keep.spg",
            "keep",
            "1",
            false,
            r#"{"components":[]}"#,
        );
        sync_project_from_remote(
            &initial_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        let mut refreshed_provider = InMemoryRemoteSessionProvider::new();
        refreshed_provider
            .register_project(RemoteProjectInfo {
                project_ref: "proj".to_string(),
                project_name: "proj".to_string(),
                source_origin: "remote".to_string(),
            })
            .unwrap();
        add_page(
            &mut refreshed_provider,
            "app/Target.spg",
            "target",
            "2",
            false,
            r#"{"components":[{"name":"c"}]}"#,
        );
        let override_graph_db = root.join("override-session.graphdb");
        let filter = SessionRefreshFilter {
            module: None,
            source_path: Some("app/Target.spg".to_string()),
            file_id: None,
            current_source_path: None,
        };
        let options = SessionRefreshOptions {
            session_id: "s1".to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: "proj".to_string(),
            project_name: "proj".to_string(),
            sync_mode: SessionSyncMode::Full,
            create_if_missing: false,
            filter: Some(filter),
            graph_db_path: Some(override_graph_db.clone()),
        };

        let report = refresh_session_from_remote(&refreshed_provider, &manager, options).unwrap();

        assert_eq!(
            report.graph_db_path,
            override_graph_db.to_string_lossy().to_string()
        );
        assert!(std::path::Path::new(&report.graph_db_path).exists());
        assert_eq!(report.files.discovered, 1);
        assert_eq!(report.sync.deleted, 0);
        assert_eq!(
            fs::read_to_string(
                root.join("s1")
                    .join("project")
                    .join("app")
                    .join("Target.spg")
            )
            .unwrap(),
            r#"{"components":[{"name":"c"}]}"#
        );
        assert_eq!(
            fs::read_to_string(root.join("s1").join("project").join("app").join("Keep.spg"))
                .unwrap(),
            r#"{"components":[]}"#
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    #[cfg(feature = "cli-local")]
    fn refresh_session_from_remote_failure_keeps_existing_session() {
        let root = test_root("refresh-failure");
        let manager = SessionManager::new(&root);
        let _ = manager
            .create_session("s1", "https://bi.test", "proj", "proj", "remote")
            .unwrap();

        // Write an existing mirror file
        let mirror = crate::session::sync::project_mirror_root(&manager.session_dir("s1"))
            .join("app/Page.spg");
        std::fs::create_dir_all(mirror.parent().unwrap()).unwrap();
        std::fs::write(&mirror, "old content").unwrap();

        // Provider that always fails list_metafiles
        let provider = FailingRemoteSessionProvider;

        let options = SessionRefreshOptions {
            session_id: "s1".to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: "proj".to_string(),
            project_name: "proj".to_string(),
            sync_mode: SessionSyncMode::Full,
            create_if_missing: false,
            filter: None,
            graph_db_path: None,
        };

        let _err = refresh_session_from_remote(&provider, &manager, options).unwrap_err();

        // Existing file should still exist
        assert!(mirror.exists());
        let content = std::fs::read_to_string(&mirror).unwrap();
        assert_eq!(content, "old content");

        let _ = fs::remove_dir_all(root);
    }

    struct FailingRemoteSessionProvider;

    impl RemoteSessionProvider for FailingRemoteSessionProvider {
        fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
            Err(anyhow!("always fails"))
        }

        fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
            Err(anyhow!("always fails"))
        }

        fn fetch_metafile_info(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
            Err(anyhow!("always fails"))
        }

        fn fetch_metafile_content(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
            Err(anyhow!("always fails"))
        }

        fn fetch_changed_since(
            &self,
            _project_ref: &str,
            _since_revision: &str,
        ) -> Result<RemoteChangeSet> {
            Err(anyhow!("always fails"))
        }
    }
}
