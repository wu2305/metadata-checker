//! M41.10-A: 远程 provider -> session 同步
//!
//! 使用 RemoteSessionProvider 拉取内容，调用 sync_remote_files_to_session 写入 mirror。
//! 覆盖 partial / full 两种同步模式。


use anyhow::{Context, Result};

use crate::session::{
    RemoteSessionProvider, SessionManager,
    manifest::RemoteSessionFile,
    sync::{SessionSyncItem, SessionSyncMode, SessionSyncReport, sync_remote_files_to_session},
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

    let mut items = Vec::with_capacity(remote_files.len());
    for entry in &remote_files {
        if entry.deleted {
            continue;
        }
        let file_ref = crate::remote_metadata::RemoteFileRef::try_new(
            &entry.project_ref,
            &entry.source_path,
            entry.file_id.clone(),
        ).map_err(|e| anyhow::anyhow!("invalid file ref: {}", e))?;

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

    let report = sync_remote_files_to_session(
        &session_dir,
        &mut manifest,
        &items,
        mode,
    )
    .with_context(|| format!("failed to sync files to session {}", session_id))?;

    manifest.files = remote_files
        .into_iter()
        .map(|entry| RemoteSessionFile {
            source_path: entry.source_path,
            file_id: entry.file_id,
            revision: entry.revision,
            etag: entry.etag,
            mtime: entry.mtime,
            size: entry.size,
            hash: None,
            deleted: entry.deleted,
        })
        .collect();

    manager.write_manifest(&manifest)?;

    Ok(report)
}

/// 对 session project mirror 调用现有 scanner 建图。
///
/// 返回 IndexReport，调用方可选择是否打印日志。
#[cfg(feature = "cli-local")]
pub fn build_session_graph(
    session_dir: &std::path::Path,
    graph_db_path: &std::path::Path,
) -> Result<crate::graph_store::IndexReport> {
    let mirror = crate::session::sync::project_mirror_root(session_dir);
    crate::scanner::indexer::ProjectIndexer::scan(&mirror,
        graph_db_path,
    )
    .with_context(|| format!("failed to build graph for session {}", session_dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_metadata::RemoteFileContent;
    use crate::session::{
        remote_provider::{InMemoryRemoteSessionProvider, RemoteProjectInfo, RemoteMetafileEntry},
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

        let report = sync_project_from_remote(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();

        assert_eq!(report.written, 1);

        let manifest = manager.read_manifest("s1").unwrap();
        assert_eq!(manifest.files.len(), 1);
        assert_eq!(manifest.files[0].source_path, "app/Page.spg");

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

        let report = sync_project_from_remote(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Partial,
        )
        .unwrap();

        assert_eq!(report.written, 1);
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
                    raw_text: r#"{"pageName":"Page","components":[{"name":"btn1","type":"Button"}]}"#.to_string(),
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

        let report = sync_project_from_remote(
            &provider,
            &manager,
            "s1",
            "proj",
            SessionSyncMode::Full,
        )
        .unwrap();
        assert_eq!(report.written, 2);

        let graph_db_path = root.join("s1").join("graph.redb");
        let index_report = build_session_graph(
            &manager.session_dir("s1"),
            &graph_db_path,
        )
        .unwrap();
        assert!(index_report.indexed >= 1);
        assert!(graph_db_path.exists());

        let _ = fs::remove_dir_all(root);
    }
}
