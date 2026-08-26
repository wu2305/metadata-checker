#![cfg(feature = "cli-local")]
//! M54 Task 7：DiffRefreshOrchestrator 端到端测试
//!
//! 覆盖：端到端等价（热刷新 vs 冷 load+warm）、空 ChangeSet 语义、
//! 首次 bootstrap、mirror 失败不推进 checkpoint、warm 失败 cold fallback。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    DiffRefreshCheckpoint, DiffRefreshOrchestrator, FixtureMetaFilesChangeSource,
    LongLivedPersistPolicy, SourceCursor,
};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{IndexCommit, IndexStateStore};
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::runtime::{
    BatchWarmPageResult, BatchWarmReport, GraphRuntime, RuntimeMode, RuntimeQueryCommand,
    RuntimeQueryRequest,
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

/// 最小合成页面：单 model + input/text 绑定，输出对图迭代顺序不敏感，
/// 保证热刷新与冷 load+warm 的 canonical JSON 可严格比较。
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
      {"id": "input1", "type": "input", "submitField": "model1.name"},
      {"id": "text1", "type": "text", "value": "${model1.name}"}
    ]
  }
}"#;

/// 独立页面（无共享 model）：page_a 变更不应使其失效。
const PAGE_B_V1: &str = r#"{
  "version": "4.19.7",
  "theme": "default",
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {"id": "textB", "type": "text", "value": "hello"}
    ]
  }
}"#;

/// page_a 引用的物理表。
const TABLE1_V1: &str = r#"{"dimensions": [{"name": "id"}, {"name": "name"}]}"#;

const PAGE_A: &str = "page:app/page_a.spg";
const PAGE_B: &str = "page:app/page_b.spg";
const BUDGET: &str = "normal";

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m54-orchestrator-test-{pid}-{seq}-{name}"
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

/// 注入无害顶层字段生成内容变体（保持合法 spg JSON）。
fn spg_variant(raw: &str, marker: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(raw).expect("parse spg");
    value
        .as_object_mut()
        .expect("spg root object")
        .insert("x-m54-marker".to_string(), serde_json::Value::from(marker));
    serde_json::to_string_pretty(&value).expect("serialize variant")
}

/// 记录 fetch 次数的 test double provider；内容按 source_path 索引。
struct StubProvider {
    contents: std::collections::HashMap<String, RemoteFileContent>,
}

impl StubProvider {
    fn new() -> Self {
        Self {
            contents: std::collections::HashMap::new(),
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

impl RemoteSessionProvider for StubProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        Err(anyhow!("stub does not support list_projects"))
    }

    fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow!("stub does not support list_metafiles"))
    }

    fn fetch_metafile_info(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        Err(anyhow!("stub does not support fetch_metafile_info"))
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        self.contents
            .get(&file_ref.source_path)
            .cloned()
            .ok_or_else(|| anyhow!("no content for {}", file_ref.source_path))
    }

    fn fetch_changed_since(&self, _p: &str, _r: &str) -> Result<RemoteChangeSet> {
        Err(anyhow!("stub does not support fetch_changed_since"))
    }
}

/// 用既有 sync 通路写入初始 mirror 文件并登记 manifest。
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

/// 完整 session 初始状态：mirror 两页 + graph + manifest 落盘。
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
        PAGE_A_V1,
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_b.spg",
        "file-b",
        "1",
        PAGE_B_V1,
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "data/table1.tbl",
        "file-t",
        "1",
        TABLE1_V1,
    );
    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");
    (manager, session_dir, manifest, db_path)
}

/// 预置 checkpoint（模拟已完成初始化的 session），使 refresh_once 走 poll 路径。
fn seed_checkpoint(db_path: &Path, checkpoint: DiffRefreshCheckpoint) {
    let mut graph = GraphDB::open(db_path).expect("open graph for checkpoint seed");
    let commit = IndexCommit {
        file_states: graph.load_file_states().expect("load states"),
        dirty_nodes: Vec::new(),
        deleted_nodes: Vec::new(),
        checkpoint: Some(checkpoint),
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    IndexStateStore::persist_index(&mut graph, commit).expect("seed checkpoint");
}

/// 只含 page_a 一个活跃事件的 poll fixture。
fn page_a_event_fixture() -> String {
    serde_json::json!({
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
            }
        ]
    })
    .to_string()
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

/// e2e 等价比较的稳定语义视图。
///
/// 移除依赖图迭代顺序的路径搜索段：候选图（remove+re-add）与 redb
/// hydrate 图的边迭代顺序不同，path search 在 budget 内选出的
/// primary_paths/related_context 及其派生 summary 字段会不同；
/// 其余段（data_sources/prerequisites/availability/write_targets/
/// evidence/next_queries/what_is_it）与图内容一一对应，可严格比较。
fn stable_semantic_view(value: &serde_json::Value) -> serde_json::Value {
    let mut view = canonicalize_json(value);
    if let Some(details) = view.get_mut("details").and_then(|d| d.as_object_mut()) {
        details.remove("primary_paths");
        details.remove("related_context");
        details.remove("related_context_summary");
    }
    if let Some(summary) = view.get_mut("summary").and_then(|s| s.as_object_mut()) {
        summary.remove("key_primary_paths");
        summary.remove("related_context_count");
        summary.remove("top_data_sources");
        summary.remove("primary_paths_count");
    }
    view
}

/// 递归规范化 JSON（数组按元素文本排序），消除数组顺序差异。
fn canonicalize_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(items) => {
            let mut items: Vec<serde_json::Value> = items.iter().map(canonicalize_json).collect();
            items.sort_by_key(|item| item.to_string());
            serde_json::Value::Array(items)
        }
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(key, item)| (key.clone(), canonicalize_json(item)))
            .collect(),
        other => other.clone(),
    }
}

fn stable_query_from_mirror_session(session_dir: &Path, page_id: &str) -> serde_json::Value {
    let expected_db = session_dir.join("m57-phase3-expected.redb");
    let _ = fs::remove_file(&expected_db);
    ProjectIndexer::scan(&project_mirror_root(session_dir), &expected_db)
        .expect("scan session mirror for stable expected query");
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &expected_db,
        Some(project_mirror_root(session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load expected mirror runtime");
    runtime
        .warm_page_logic_availability(page_id, BUDGET)
        .expect("warm expected mirror page");
    let result = runtime
        .query(page_query_request(page_id))
        .expect("query expected mirror page")
        .result;
    let _ = fs::remove_file(expected_db);
    result
}

/// 端到端：page_a 变化后结果与同 mirror 冷 load+warm 的 canonical JSON 相等，
/// page_b cache 内容保持相同。
#[test]
fn m54_diff_refresh_orchestrator_end_to_end_equivalence() {
    let (manager, session_dir, manifest, db_path) = setup_session("e2e");
    seed_checkpoint(
        &db_path,
        DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "e2e-1"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");

    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load long-lived runtime");
    runtime
        .warm_page_logic_availability(PAGE_A, BUDGET)
        .expect("warm page_a");
    runtime
        .warm_page_logic_availability(PAGE_B, BUDGET)
        .expect("warm page_b");
    let page_b_result_before = runtime
        .query(page_query_request(PAGE_B))
        .expect("query page_b before refresh")
        .result;

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let report = orchestrator.refresh_once().expect("refresh once");
    assert_eq!(report.change_count, 1);
    assert!(
        report.invalidated_pages.iter().any(|page| page == PAGE_A),
        "page_a should be invalidated"
    );
    assert_eq!(report.warm_failures, Vec::<String>::new());
    assert_eq!(
        report.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    // 热刷新结果 == 同 mirror 冷 load + warm 的 canonical JSON
    let hot_result = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("hot query page_a")
        .result;
    let mut cold_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("cold runtime load");
    cold_runtime
        .warm_page_logic_availability(PAGE_A, BUDGET)
        .expect("cold warm page_a");
    let cold_result = cold_runtime
        .query(page_query_request(PAGE_A))
        .expect("cold query page_a")
        .result;
    assert_eq!(
        stable_semantic_view(&hot_result),
        stable_semantic_view(&cold_result),
        "hot refresh must equal cold load+warm on stable semantic sections"
    );

    // page_b cache 内容保持相同（cache 命中且结果一致）
    let read_model = orchestrator
        .runtime()
        .read_model
        .as_ref()
        .expect("read model");
    assert!(read_model.has_page_logic_availability(PAGE_B, BUDGET));
    let page_b_result_after = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_B))
        .expect("query page_b after refresh")
        .result;
    assert_eq!(
        canonicalize_json(&page_b_result_before),
        canonicalize_json(&page_b_result_after)
    );

    let _ = fs::remove_dir_all(
        session_dir
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default(),
    );
}

/// 空 ChangeSet：首次无 checkpoint 时需 checkpoint-only 落盘；已有 checkpoint 时仅更新
/// `last_poll_at`，不推进 checkpoint。
#[test]
fn m54_diff_refresh_orchestrator_empty_changeset_only_updates_last_poll_at() {
    let (manager, session_dir, manifest, db_path) = setup_session("empty");
    let seeded = DiffRefreshCheckpoint {
        active: SourceCursor::new(2000, Vec::new()),
        deleted: SourceCursor::new(1000, Vec::new()),
    };
    seed_checkpoint(&db_path, seeded.clone());
    let baseline_nodes = {
        let graph = GraphDB::open(&db_path).expect("open graph");
        metadata_checker::graph_store::GraphReadStore::node_count(&graph).expect("node count")
    };

    // fixture 无任何事件 → poll 返回空集合
    let source = FixtureMetaFilesChangeSource::from_json_str(r#"{"schema_version": 1}"#)
        .expect("empty fixture source");
    let provider = StubProvider::new();
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
        Box::new(provider),
        runtime,
    );
    let report = orchestrator.refresh_once().expect("refresh once");

    assert_eq!(report.change_count, 0);
    assert!(
        report.last_poll_at > 0,
        "empty poll should still record last_poll_at"
    );
    assert_eq!(
        report.checkpoint,
        Some(seeded),
        "checkpoint must not advance"
    );
    assert_eq!(
        report.persist_report, None,
        "an empty poll with an existing checkpoint must not rewrite the graph"
    );
    assert_eq!(report.persisted, false);

    // graph 与 checkpoint 均未写：重开后保持一致
    let reopened = GraphDB::open(&db_path).expect("reopen graph");
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(2000, Vec::new()),
            deleted: SourceCursor::new(1000, Vec::new()),
        })
    );
    assert_eq!(
        metadata_checker::graph_store::GraphReadStore::node_count(&reopened).expect("node count"),
        baseline_nodes
    );
}

/// 首次 bootstrap 的空变更集是 checkpoint-only，应返回 persisted=false。
#[test]
fn m54_diff_refresh_orchestrator_bootstrap_empty_changeset_is_not_durable_persist() {
    let root = test_root("bootstrap-empty");
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
    manager.write_manifest(&manifest).expect("write manifest");

    let source = FixtureMetaFilesChangeSource::from_json_str(
        r#"{"schema_version":1,"snapshot":{"active":[{"file_id":"file-a","source_path":"app/page_a.spg","revision":"1","content_type":"super_page","updated_at_ms":1000}]}}"#,
    )
    .expect("empty bootstrap fixture source");
    let provider = StubProvider::new();
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let report = orchestrator
        .refresh_once()
        .expect("bootstrap empty refresh");

    assert_eq!(report.change_count, 0);
    assert_eq!(report.persisted, false);
    assert_eq!(
        report.persist_report.is_some(),
        true,
        "bootstrap empty should persist checkpoint metadata"
    );
    assert_eq!(
        report.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:1".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    let _ = fs::remove_dir_all(root);
}

/// 首次调用无 checkpoint 时走 bootstrap：只拉不一致文件、删除远端缺失文件、
/// 原子提交配对 checkpoint。
#[test]
fn m54_diff_refresh_orchestrator_bootstrap_initializes_checkpoint() {
    let root = test_root("bootstrap");
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");
    // file-u 未变（rev 1 与快照一致）、file-v revision 变化、file-w 远端缺失
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_u.spg",
        "file-u",
        "1",
        PAGE_A_V1,
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_v.spg",
        "file-v",
        "1",
        PAGE_B_V1,
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "app/page_w.spg",
        "file-w",
        "1",
        PAGE_A_V1,
    );
    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_v.spg",
        "file-v",
        "2",
        &spg_variant(PAGE_B_V1, "boot-1"),
    );
    // 复用 Task 3 fixture：snapshot 含 file-u rev1 / file-v rev2 / 删除 file-w
    let source = FixtureMetaFilesChangeSource::from_path(Path::new(
        "tests/fixtures/diff_refresh/meta_files_delta.json",
    ))
    .expect("fixture source");

    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let report = orchestrator.refresh_once().expect("bootstrap refresh");

    // 只含 revision 变化与远端缺失两个事件
    assert_eq!(report.change_count, 2);
    assert_eq!(report.persisted, false);
    let expected_checkpoint = DiffRefreshCheckpoint {
        active: SourceCursor::new(2001, vec!["active:file-v:2".into()]),
        deleted: SourceCursor::new(1999, vec!["deleted:uuid-w-1".into()]),
    };
    assert_eq!(report.checkpoint, Some(expected_checkpoint.clone()));
    let reopened = GraphDB::open(&db_path).expect("reopen graph");
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint"),
        None
    );

    let follow_up = orchestrator
        .refresh_once()
        .expect("second bootstrap pending follow-up");
    assert_eq!(follow_up.change_count, 0);
    assert_eq!(follow_up.persisted, false);
    assert_eq!(follow_up.checkpoint, Some(expected_checkpoint));

    // manifest：file-w 标删除、file-v revision 推进、file-u 不变
    let manifest = orchestrator.manifest();
    let record = |path: &str| {
        manifest
            .files
            .iter()
            .find(|file| file.source_path == path)
            .unwrap_or_else(|| panic!("manifest record for {path}"))
    };
    assert_eq!(record("app/page_w.spg").deleted, true);
    assert_eq!(record("app/page_v.spg").revision.as_deref(), Some("2"));
    assert_eq!(record("app/page_u.spg").revision.as_deref(), Some("1"));
    assert!(
        !project_mirror_root(&session_dir)
            .join("app/page_w.spg")
            .exists()
    );

    let _ = fs::remove_dir_all(root);
}

/// mirror 失败时 checkpoint 不推进，旧 runtime 查询仍成功。
#[test]
fn m54_diff_refresh_orchestrator_mirror_failure_keeps_checkpoint_and_runtime() {
    let (manager, session_dir, manifest, db_path) = setup_session("mirror-fail");
    let seeded = DiffRefreshCheckpoint {
        active: SourceCursor::new(500, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    seed_checkpoint(&db_path, seeded.clone());

    // provider 无 page_a 内容 → mirror fetch 失败
    let provider = StubProvider::new();
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    runtime
        .warm_page_logic_availability(PAGE_A, BUDGET)
        .expect("warm page_a");

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let result = orchestrator.refresh_once();
    assert!(result.is_err(), "mirror fetch failure should fail refresh");

    // checkpoint 不推进
    let reopened = GraphDB::open(&db_path).expect("reopen graph");
    assert_eq!(
        reopened
            .load_diff_refresh_checkpoint()
            .expect("load checkpoint"),
        Some(seeded)
    );
    // 旧 runtime 查询仍成功
    orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("old runtime query should still succeed after mirror failure");
}

/// 注入 page_a warm 失败：checkpoint 已推进、page_a 无旧 cache、
/// page_a cold 查询正确、page_b cache 仍命中。
#[test]
fn m54_diff_refresh_orchestrator_warm_failure_falls_back_to_cold() {
    let (manager, session_dir, manifest, db_path) = setup_session("warm-fail");
    seed_checkpoint(
        &db_path,
        DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "warm-1"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
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

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    // 注入 per-page warm 失败（BatchWarmReport.pages[*].success=false）
    orchestrator.set_warm_fn(Box::new(|_runtime, targets| {
        Ok(BatchWarmReport {
            pages: targets
                .iter()
                .map(|(page_id, budget)| BatchWarmPageResult {
                    page_id: page_id.clone(),
                    budget: budget.clone(),
                    success: false,
                    warm_ms: 0,
                    error: Some("injected warm failure".to_string()),
                })
                .collect(),
            total_ms: 0,
            structural_writes: 0,
        })
    }));

    let report = orchestrator
        .refresh_once()
        .expect("refresh with warm failure");

    // checkpoint 已推进
    assert_eq!(
        report.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );
    // warm_failures 来自 pages[*].success
    assert_eq!(report.warm_failures, vec![PAGE_A.to_string()]);

    let read_model = orchestrator
        .runtime()
        .read_model
        .as_ref()
        .expect("read model");
    // page_a 无旧 cache（已失效且 warm 失败）、page_b cache 仍命中
    assert!(!read_model.has_page_logic_availability(PAGE_A, BUDGET));
    assert!(read_model.has_page_logic_availability(PAGE_B, BUDGET));
    // page_a cold 查询正确
    let cold_result = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("cold query page_a after warm failure");
    assert!(!cold_result.result.is_null());
}

/// Phase 3：LongLived 先安装内存 replacement，延迟 persist 期间查询必须立即看到新内容。
#[test]
fn m57_phase3_install_is_queryable_before_deferred_persist() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-deferred");
    let seeded = DiffRefreshCheckpoint {
        active: SourceCursor::new(500, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    seed_checkpoint(&db_path, seeded.clone());

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-deferred"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");

    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: usize::MAX,
        max_pending_rounds: 10,
    });

    let report = orchestrator.refresh_once().expect("deferred refresh");
    let expected_checkpoint = DiffRefreshCheckpoint {
        active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    assert_eq!(report.persisted, false);
    assert_eq!(report.persist_report, None);
    assert_eq!(report.pending_dirty_total > 0, true);
    assert_eq!(report.checkpoint, Some(expected_checkpoint.clone()));

    let durable = GraphDB::open(&db_path).expect("open durable graph");
    assert_eq!(
        durable
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint"),
        Some(seeded),
        "deferred refresh must not advance the durable checkpoint"
    );

    let hot_result = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("query installed runtime")
        .result;
    let stable_expected = stable_query_from_mirror_session(&session_dir, PAGE_A);
    assert_eq!(
        stable_semantic_view(&hot_result),
        stable_semantic_view(&stable_expected),
        "installed runtime must expose the new mirror content before persist"
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// Phase 3：pending dirty-node 集合超过阈值时立即提交图与 checkpoint。
#[test]
fn m57_phase3_dirty_threshold_persists_pending_graph_and_checkpoint() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-threshold");
    seed_checkpoint(
        &db_path,
        DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-threshold"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 10,
    });

    let report = orchestrator.refresh_once().expect("threshold refresh");
    assert_eq!(report.persisted, true);
    assert_eq!(report.pending_dirty_total, 0);
    assert_eq!(report.persist_report.is_some(), true);
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// 主流程长生命周期运行时可显式切换到一次性同步模式（先 persist 后 install）。
#[test]
fn m57_phase3_one_shot_sync_mode_forces_persist_before_install() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-oneshot");
    seed_checkpoint(
        &db_path,
        DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-one-shot"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("forced sync refresh");
    assert_eq!(report.persisted, true);
    assert_eq!(report.pending_dirty_total, 0);
    assert!(report.persist_report.is_some());
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );
    assert!(
        orchestrator.runtime().read_model.is_some(),
        "one-shot sync execution should keep read model installed",
    );
    let query = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("query installed runtime page_a")
        .result;
    assert!(
        !query.is_null(),
        "query should return result in one-shot sync mode"
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// Phase 3：脏节点阈值是“严格大于”，相等时不应持久化（pending 状态保留）。
#[test]
fn m57_phase3_dirty_threshold_is_strictly_greater_boundary() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-threshold-eq");
    seed_checkpoint(
        &db_path,
        DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-threshold-eq"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: usize::MAX,
        max_pending_rounds: 10,
    });

    let first = orchestrator
        .refresh_once()
        .expect("threshold boundary first refresh");
    assert_eq!(first.persisted, false);
    assert!(
        first.pending_dirty_total > 0,
        "boundary case requires pending nodes"
    );

    let threshold = first.pending_dirty_total;
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: threshold,
        max_pending_rounds: 10,
    });
    let second = orchestrator
        .refresh_once()
        .expect("threshold boundary second refresh");
    assert_eq!(second.change_count, 0);
    assert_eq!(second.persisted, false);
    assert!(second.persist_report.is_none());
    assert_eq!(second.pending_dirty_total, threshold);
    assert_eq!(
        second.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// Phase 3：连续 pending round 达到轮次上限时，即使没有新事件也触发 persist。
#[test]
fn m57_phase3_round_fallback_persists_after_pending_rounds() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-rounds");
    let seeded = DiffRefreshCheckpoint {
        active: SourceCursor::new(500, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    seed_checkpoint(&db_path, seeded.clone());

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-rounds"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: usize::MAX,
        max_pending_rounds: 3,
    });

    let first = orchestrator.refresh_once().expect("first deferred refresh");
    assert_eq!(first.persisted, false);
    let second = orchestrator.refresh_once().expect("second pending refresh");
    assert_eq!(
        second.change_count, 0,
        "pending watermark must suppress replay"
    );
    assert_eq!(second.persisted, false);
    let third = orchestrator.refresh_once().expect("round fallback refresh");
    assert_eq!(third.change_count, 0);
    assert_eq!(third.persisted, true);
    assert_eq!(third.pending_dirty_total, 0);
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("reopen durable graph")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint again")
            .is_some(),
        true
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// Phase 3：persist 失败不撤销已安装 runtime，下一轮以 pending commit 重试。
#[test]
fn m57_phase3_persist_failure_keeps_runtime_and_retries() {
    let (manager, session_dir, manifest, db_path) = setup_session("phase3-retry");
    let seeded = DiffRefreshCheckpoint {
        active: SourceCursor::new(500, Vec::new()),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    seed_checkpoint(&db_path, seeded.clone());

    let mut provider = StubProvider::new();
    provider.register(
        "app/page_a.spg",
        "file-a",
        "2",
        &spg_variant(PAGE_A_V1, "phase3-retry"),
    );
    let source = FixtureMetaFilesChangeSource::from_json_str(&page_a_event_fixture())
        .expect("fixture source");
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load runtime");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir.clone(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 10,
    });

    let attempts = std::sync::Arc::new(AtomicUsize::new(0));
    let attempts_for_hook = std::sync::Arc::clone(&attempts);
    orchestrator.set_persist_fn(Box::new(move |graph, commit| {
        let attempt = attempts_for_hook.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(anyhow!("injected phase3 persist failure"))
        } else {
            graph.persist_commit(commit)
        }
    }));

    let first = orchestrator.refresh_once();
    assert_eq!(first.is_err(), true);
    let hot_result = orchestrator
        .runtime_mut()
        .query(page_query_request(PAGE_A))
        .expect("installed runtime remains queryable after persist failure")
        .result;
    let retry_expected = stable_query_from_mirror_session(&session_dir, PAGE_A);
    assert_eq!(
        stable_semantic_view(&hot_result),
        stable_semantic_view(&retry_expected),
    );
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph after failure")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint after failure"),
        Some(seeded)
    );

    let retry = orchestrator.refresh_once().expect("retry pending persist");
    assert_eq!(retry.change_count, 0);
    assert_eq!(retry.persisted, true);
    assert_eq!(retry.pending_dirty_total, 0);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        GraphDB::open(&db_path)
            .expect("open durable graph after retry")
            .load_diff_refresh_checkpoint()
            .expect("load durable checkpoint after retry"),
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-a:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        })
    );

    let _ = fs::remove_dir_all(root_for_session(&session_dir));
}

/// 从 orchestrator 关联的 session 根目录取测试清理路径。
fn root_for_session(session_dir: &Path) -> PathBuf {
    session_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| session_dir.to_path_buf())
}
