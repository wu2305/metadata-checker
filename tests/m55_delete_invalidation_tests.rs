#![cfg(feature = "cli-local")]
//! M55：删除节点参与失效（P0 回归）
//!
//! 钉死：纯删除 ChangeSet 时 deleted_node_ids 也必须进入
//! prepare_replacement 的失效计算——被删页 cache 摘除、
//! 共享 Model 的表文件删除导致多页失效、改名后旧路径页失效。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    DiffRefreshCheckpoint, DiffRefreshOrchestrator, FixtureMetaFilesChangeSource, SourceCursor,
};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{IndexCommit, IndexStateStore};
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::runtime::{
    GraphRuntime, RuntimeMode, RuntimeQueryCommand, RuntimeQueryRequest,
};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::session::manifest::SessionManifest;
use metadata_checker::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::sync::{
    SessionSyncItem, SessionSyncMode, project_mirror_root, sync_remote_files_to_session,
};
use metadata_checker::session::{RemoteSessionProvider, SessionManager};

const PAGE_A: &str = "page:app/page_a.spg";
const PAGE_B: &str = "page:app/page_b.spg";
const BUDGET: &str = "normal";

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m55-delete-inv-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// 引用共享 tbl 的最小页面（本地模型 id 按页区分，避免节点合并）。
fn page_spg(model_id: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "sources": [{"id": model_id, "modelType": "dwtable", "path": "data/table1.tbl"}],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "input1", "type": "input", "submitField": format!("{model_id}.name")},
            {"id": "text1", "type": "text", "value": format!("${{{model_id}.name}}")}
        ]}
    })
    .to_string()
}

const TABLE_V1: &str = r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#;

/// fetch stub provider：按 source_path 提供内容。
struct StubProvider {
    contents: HashMap<String, RemoteFileContent>,
}

impl RemoteSessionProvider for StubProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        Err(anyhow!("stub"))
    }

    fn list_metafiles(&self, _p: &str) -> Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow!("stub"))
    }

    fn fetch_metafile_info(&self, _f: &RemoteFileRef) -> Result<RemoteFileInfo> {
        Err(anyhow!("stub"))
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        self.contents
            .get(&file_ref.source_path)
            .cloned()
            .ok_or_else(|| anyhow!("no content for {}", file_ref.source_path))
    }

    fn fetch_changed_since(&self, _p: &str, _r: &str) -> Result<RemoteChangeSet> {
        Err(anyhow!("stub"))
    }
}

fn seed_file(
    session_dir: &Path,
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
    .expect("seed mirror file");
}

/// 完整初始状态：page_a/page_b（共享 table1.tbl）+ tbl + graph + checkpoint。
fn setup_session(name: &str) -> (SessionManager, PathBuf, SessionManifest, PathBuf) {
    let root = test_root(name);
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
        &page_spg("model_a"),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_b.spg",
        "file-b",
        "1",
        &page_spg("model_b"),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "data/table1.tbl",
        "file-t",
        "1",
        TABLE_V1,
    );
    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    // 预置 checkpoint 走 poll 路径
    let mut graph = GraphDB::open(&db_path).expect("open graph");
    let commit = IndexCommit {
        file_states: graph.load_file_states().expect("load states"),
        dirty_nodes: Vec::new(),
        deleted_nodes: Vec::new(),
        checkpoint: Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        }),
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    IndexStateStore::persist_index(&mut graph, commit).expect("seed checkpoint");
    (manager, session_dir, manifest, db_path)
}

fn fixture_source(events: serde_json::Value) -> FixtureMetaFilesChangeSource {
    let json = serde_json::json!({
        "schema_version": 1,
        "events": events,
    });
    FixtureMetaFilesChangeSource::from_json_str(&json.to_string()).expect("fixture source")
}

fn delete_event(event_id: &str, file_id: &str, source_path: &str, ms: u64) -> serde_json::Value {
    serde_json::json!({
        "event_id": event_id,
        "file_id": file_id,
        "source_path": source_path,
        "previous_source_path": null,
        "content_type": if source_path.ends_with(".tbl") { "table" } else { "super_page" },
        "updated_at_ms": ms,
        "deleted": true,
    })
}

fn page_query_request(page_id: &str) -> RuntimeQueryRequest {
    RuntimeQueryRequest {
        command: RuntimeQueryCommand::QueryPageLogic,
        target: page_id.to_string(),
        budget: BUDGET.to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    }
}

fn build_orchestrator(
    manager: SessionManager,
    session_dir: PathBuf,
    manifest: SessionManifest,
    db_path: &Path,
    source: FixtureMetaFilesChangeSource,
    provider: StubProvider,
) -> DiffRefreshOrchestrator {
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    runtime
        .warm_page_logic_availability(PAGE_A, BUDGET)
        .expect("warm page_a");
    runtime
        .warm_page_logic_availability(PAGE_B, BUDGET)
        .expect("warm page_b");
    DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    )
}

/// P0-a：删除 page_a 的 .spg → page_a warm cache 摘除、cold 查询走既定错误语义、
/// page_b cache 保持。
#[test]
fn m55_delete_invalidation_deleted_page_cache_evicted() {
    let (manager, session_dir, manifest, db_path) = setup_session("page-delete");
    let source = fixture_source(serde_json::json!([delete_event(
        "deleted:uuid-a-1",
        "file-a",
        "app/page_a.spg",
        1000
    )]));
    let provider = StubProvider {
        contents: HashMap::new(),
    };
    let mut orchestrator = build_orchestrator(
        manager,
        session_dir.clone(),
        manifest,
        &db_path,
        source,
        provider,
    );

    let report = orchestrator.refresh_once().expect("refresh once");

    assert!(
        report.invalidated_pages.iter().any(|page| page == PAGE_A),
        "deleted page must be invalidated: {:?}",
        report.invalidated_pages
    );
    let read_model = orchestrator
        .runtime()
        .read_model
        .as_ref()
        .expect("read model");
    assert!(
        !read_model.has_page_logic_availability(PAGE_A, BUDGET),
        "deleted page warm cache must be evicted"
    );
    assert!(
        read_model.has_page_logic_availability(PAGE_B, BUDGET),
        "unrelated page cache must be preserved"
    );

    // cold 查询被删页：既定语义为 target-not-found 结构化输出（而非陈旧缓存结果）
    let cold = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("cold query should return structured not-found output");
    let summary = cold.result["summary"].as_object().expect("summary object");
    assert_eq!(summary["resolved_count"].as_u64(), Some(0));
    assert!(
        summary["what_is_it"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"),
        "deleted page query must report target not found: {}",
        cold.result
    );
    // page_b cold/warm 查询仍正确
    orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_B))
        .expect("page_b query should still succeed");

    let _ = std::fs::remove_dir_all(session_dir.parent().expect("session root"));
}

/// P0-b：共享 Model 的表文件删除 → page_a/page_b 两页 warm cache 都失效。
#[test]
fn m55_delete_invalidation_shared_model_table_delete_invalidates_all_pages() {
    let (manager, session_dir, manifest, db_path) = setup_session("table-delete");
    let source = fixture_source(serde_json::json!([delete_event(
        "deleted:uuid-t-1",
        "file-t",
        "data/table1.tbl",
        1000
    )]));
    let provider = StubProvider {
        contents: HashMap::new(),
    };
    let mut orchestrator = build_orchestrator(
        manager,
        session_dir.clone(),
        manifest,
        &db_path,
        source,
        provider,
    );

    let report = orchestrator.refresh_once().expect("refresh once");

    // P0 核心钉死：shared tbl 删除前 invalidated_pages 为空（bug），修复后两页都失效
    for page in [PAGE_A, PAGE_B] {
        assert!(
            report.invalidated_pages.iter().any(|p| p == page),
            "shared model table delete must invalidate {page}: {:?}",
            report.invalidated_pages
        );
    }
    // 页面仍存在 → best-effort re-warm 重建 cache（失效语义由 invalidated_pages 钉死）
    assert_eq!(report.warm_failures, Vec::<String>::new());
    let read_model = orchestrator
        .runtime()
        .read_model
        .as_ref()
        .expect("read model");
    assert!(
        read_model.has_page_logic_availability(PAGE_A, BUDGET),
        "page_a should be re-warmed against the new graph"
    );
    assert!(
        read_model.has_page_logic_availability(PAGE_B, BUDGET),
        "page_b should be re-warmed against the new graph"
    );

    let _ = std::fs::remove_dir_all(session_dir.parent().expect("session root"));
}

/// P0-c：改名 .spg → 旧路径页 cache 摘除、新路径页 cold 查询正常。
#[test]
fn m55_delete_invalidation_rename_evicts_old_path_page() {
    const PAGE_A2: &str = "page:app/page_a2.spg";
    let (manager, session_dir, manifest, db_path) = setup_session("rename");
    let source = fixture_source(serde_json::json!([{
        "event_id": "active:file-a:2",
        "file_id": "file-a",
        "source_path": "app/page_a2.spg",
        "previous_source_path": "app/page_a.spg",
        "content_type": "super_page",
        "updated_at_ms": 1000,
        "deleted": false
    }]));
    let mut contents = HashMap::new();
    contents.insert(
        "app/page_a2.spg".to_string(),
        RemoteFileContent {
            source_path: "app/page_a2.spg".to_string(),
            file_id: Some("file-a".to_string()),
            revision: Some("2".to_string()),
            content_type: MetadataContentType::SuperPage,
            raw_text: page_spg("model_a"),
        },
    );
    let provider = StubProvider { contents };
    let mut orchestrator = build_orchestrator(
        manager,
        session_dir.clone(),
        manifest,
        &db_path,
        source,
        provider,
    );

    let report = orchestrator.refresh_once().expect("refresh once");

    assert!(
        report.invalidated_pages.iter().any(|page| page == PAGE_A),
        "old path page must be invalidated after rename"
    );
    let read_model = orchestrator
        .runtime()
        .read_model
        .as_ref()
        .expect("read model");
    assert!(
        !read_model.has_page_logic_availability(PAGE_A, BUDGET),
        "old path warm cache must be evicted"
    );
    // 新路径页 cold 查询正常
    orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A2))
        .expect("cold query on renamed page should succeed");

    let _ = std::fs::remove_dir_all(session_dir.parent().expect("session root"));
}
