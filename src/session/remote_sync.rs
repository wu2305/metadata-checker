//! M41.10-A: 远程 provider -> session 同步
//!
//! 使用 RemoteSessionProvider 拉取内容，调用 sync_remote_files_to_session 写入 mirror。
//! 覆盖 partial / full 两种同步模式。

use std::fs;

use anyhow::{Context, Result, anyhow};

use crate::remote_metadata::RemoteFileRef;

use crate::graph_store::IndexReport;
use crate::session::{
    RemoteSessionProvider, SessionManager,
    manifest::RemoteSessionFile,
    sync::{
        SessionSyncItem, SessionSyncMode, SessionSyncReport, project_mirror_root,
        sync_remote_files_to_session,
    },
};

/// 从远程 provider 同步项目元数据到本地 session。
pub fn sync_project_from_remote(
    provider: &dyn RemoteSessionProvider,
    manager: &SessionManager,
    session_id: &str,
    project_ref: &str,
    mode: SessionSyncMode,
) -> Result<SessionSyncReport> {
    let session_dir = manager.session_dir(session_id);
    let mut manifest = manager.read_manifest(session_id)?;

    let remote_files = provider
        .list_metafiles(project_ref)
        .with_context(|| format!("failed to list metafiles for project {}", project_ref))?;

    for entry in &remote_files {
        validate_remote_entry(entry)?;
    }

    let mut items = Vec::with_capacity(remote_files.len());
    for entry in &remote_files {
        if entry.deleted {
            continue;
        }
        let file_ref = file_ref_from_entry(entry)?;

        let content = provider
            .fetch_metafile_content(&file_ref)
            .with_context(|| {
                format!(
                    "failed to fetch content for {} in project {}",
                    entry.source_path, project_ref
                )
            })?;

        items.push(SessionSyncItem {
            content,
            etag: entry.etag.clone(),
            mtime: entry.mtime,
            size: entry.size,
        });
    }

    let mut report = sync_remote_files_to_session(&session_dir, &mut manifest, &items, mode)
        .with_context(|| format!("failed to sync files to session {}", session_id))?;

    for entry in &remote_files {
        if entry.deleted {
            if mark_entry_deleted(&session_dir, &mut manifest, entry)? {
                report.deleted += 1;
            }
        } else {
            merge_remote_metadata(&mut manifest, entry);
        }
    }

    manager.write_manifest(&manifest)?;

    Ok(report)
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
    let graph_db_path = session_dir.join("graph.redb");

    // 同步远程文件
    let sync_report =
        sync_project_from_remote(provider, manager, &session_id, &project_ref, sync_mode)
            .with_context(|| {
                format!(
                    "failed to sync project {} to session {}",
                    project_ref, session_id
                )
            })?;

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

    fn test_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "metadata-checker-remote-sync-test-{}-{}",
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
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
        let before = manager.read_manifest("s1").unwrap();

        let failing_provider = FailingContentProvider;
        let err = sync_project_from_remote(
            &failing_provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap_err();

        assert!(err.to_string().contains("failed to fetch content"));
        let after = manager.read_manifest("s1").unwrap();
        assert_eq!(before, after);
        let mirror_text =
            fs::read_to_string(root.join("s1").join("project").join("app").join("Page.spg"))
                .unwrap();
        assert_eq!(mirror_text, "old");
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
}
