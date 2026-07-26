#![cfg(feature = "cli-local")]
//! M57-修复回归：首次 bootstrap 空变更集仅 checkpoint-only，不计入 durable graph 提交。

use std::fs;

use anyhow::anyhow;
use metadata_checker::diff_refresh::{
    DiffRefreshCheckpoint, DiffRefreshOrchestrator, FixtureMetaFilesChangeSource, SourceCursor,
};
use metadata_checker::graph::GraphDB;
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::runtime::{GraphRuntime, RuntimeMode};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::session::manifest::SessionManifest;
use metadata_checker::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::sync::{
    SessionSyncItem, SessionSyncMode, project_mirror_root, sync_remote_files_to_session,
};
use metadata_checker::session::{RemoteSessionProvider, SessionManager};

const PAGE_A_V1: &str = r#"{
  "version": "4.19.7",
  "theme": "default",
  "sources": [
    {"id": "model1", "modelType": "dwtable", "path": "data/table1.tbl"}
  ],
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {"id": "text1", "type": "text", "value": "hello"}
    ]
  }
}"#;

#[derive(Default)]
struct StubProvider;

impl RemoteSessionProvider for StubProvider {
    fn list_projects(&self) -> anyhow::Result<Vec<RemoteProjectInfo>> {
        Err(anyhow!("stub does not support list_projects"))
    }

    fn list_metafiles(&self, _project_ref: &str) -> anyhow::Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow!("stub does not support list_metafiles"))
    }

    fn fetch_metafile_info(&self, _file_ref: &RemoteFileRef) -> anyhow::Result<RemoteFileInfo> {
        Err(anyhow!("stub does not support fetch_metafile_info"))
    }

    fn fetch_metafile_content(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> anyhow::Result<RemoteFileContent> {
        Err(anyhow!("stub does not support fetch_metafile_content"))
    }

    fn fetch_changed_since(
        &self,
        _project_ref: &str,
        _watermark: &str,
    ) -> anyhow::Result<RemoteChangeSet> {
        Err(anyhow!("stub does not support fetch_changed_since"))
    }
}

fn test_root() -> std::path::PathBuf {
    let mut root = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    root.push(format!("metadata-checker-fix-bootstrap-{nanos}"));
    let _ = fs::remove_dir_all(&root);
    root
}

fn seed_file(
    session_dir: &std::path::Path,
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
        session_dir,
        manifest,
        &[SessionSyncItem::new(content)],
        SessionSyncMode::Partial,
    )
    .expect("seed manifest file");
}

#[test]
fn m57_bootstrap_empty_changeset_still_reports_not_durable_persist() {
    let root = test_root();
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");

    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_a.spg",
        "file-a",
        "1",
        PAGE_A_V1,
    );

    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager
        .write_manifest(&manifest)
        .expect("write manifest for bootstrap fixture");

    let source = FixtureMetaFilesChangeSource::from_json_str(
        r#"{
  "schema_version": 1,
  "snapshot": {
    "active": [
      {
        "file_id": "file-a",
        "source_path": "app/page_a.spg",
        "revision": "1",
        "content_type": "super_page",
        "updated_at_ms": 1000
      }
    ]
  }
}"#,
    )
    .expect("empty bootstrap fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(StubProvider),
        runtime,
    );

    let report = orchestrator
        .refresh_once()
        .expect("bootstrap empty refresh");

    assert_eq!(report.change_count, 0);
    assert_eq!(report.persisted, false);
    assert!(report.persist_report.is_some());
    assert_eq!(
        report.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:1".to_string()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open graph db")
            .load_diff_refresh_checkpoint()
            .expect("read checkpoint after bootstrap"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:1".to_string()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    let _ = fs::remove_dir_all(root);
}
