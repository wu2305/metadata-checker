#![cfg(feature = "cli-local")]

//! M59-2 B：refresh 的成功 / 部分失败 / 提交边界。
//!
//! approved spec（M59 语义完整性修复「M59-2 必须消费的归属契约」）：
//! > 候选贡献、派生图、文件 hash、diagnostic 和 refresh checkpoint 一起提交。
//! > 解析失败保留旧贡献并保持脏；**失败不得推进 checkpoint**。
//!
//! 已发现的偏离（`codex/m59-2-ownership@2a65d7b`）：prepare 把坏 TBL 转成诊断后
//! 仍返回成功，diff-refresh 随后提交 `next_checkpoint`——事件被消费但内容从未
//! 入图，且文件 hash 不推进的话重试只能等未来新事件偶然触发。
//!
//! 本文件做**动态复现**（真实 DiffRefreshOrchestrator + 真源 fixture），覆盖
//! 同步/延迟持久化、checkpoint、空 poll、重试、恢复与重启。

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    ChangeSet, ChangedRemoteFile, DiffRefreshCheckpoint, DiffRefreshOrchestrator,
    LongLivedPersistPolicy, MetaFilesChangeSource, MetaFilesWatermark, SourceCursor,
};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, IndexCommit, IndexStateStore};
use metadata_checker::ownership::ProjectBinding;
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

const CODE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";
const CODE_CONFLICT: &str = "GRAPH_OWNERSHIP_CONFLICT";

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path =
        std::env::temp_dir().join(format!("metadata-checker-m59-2-refresh-{pid}-{seq}-{name}"));
    let _ = fs::remove_dir_all(&path);
    path
}

fn page_with_dwtable(title: &str, source_id: &str, tbl_path: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [{"id": source_id, "modelType": "dwtable", "path": tbl_path}],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [{"id": "label", "type": "text", "value": title}]
        }
    })
    .to_string()
}

fn good_table(fields: &[&str]) -> String {
    let dims: Vec<_> = fields
        .iter()
        .map(|f| serde_json::json!({"name": f, "dataType": "C"}))
        .collect();
    serde_json::json!({"version": "1.0", "dimensions": dims}).to_string()
}

const BAD_TABLE: &str = "{ invalid json syntax -- not closed";

/// 内容按 source_path 排队吐出的 provider（同一文件多轮变更按 fetch 顺序消费）。
struct QueuedProvider {
    contents: RefCell<HashMap<String, VecDeque<RemoteFileContent>>>,
}

impl QueuedProvider {
    fn new() -> Self {
        Self {
            contents: RefCell::new(HashMap::new()),
        }
    }

    fn push(&self, path: &str, file_id: &str, revision: &str, raw_text: &str) {
        self.contents
            .borrow_mut()
            .entry(path.to_string())
            .or_default()
            .push_back(RemoteFileContent {
                source_path: path.to_string(),
                file_id: Some(file_id.to_string()),
                revision: Some(revision.to_string()),
                content_type: MetadataContentType::from_extension(
                    path.rsplit('.').next().unwrap_or(""),
                ),
                raw_text: raw_text.to_string(),
            });
    }
}

impl RemoteSessionProvider for QueuedProvider {
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
            .borrow_mut()
            .get_mut(&file_ref.source_path)
            .and_then(|queue| queue.pop_front())
            .ok_or_else(|| anyhow!("no queued content for {}", file_ref.source_path))
    }
    fn fetch_changed_since(&self, _p: &str, _r: &str) -> Result<RemoteChangeSet> {
        Err(anyhow!("stub does not support fetch_changed_since"))
    }
}

/// 重放式变更源：一批事件只要**尚未被 cursor 消费**就每次 poll 都重新投递。
///
/// 这模拟真实远端语义（按水位拉取，而不是按「已推过一次」记账）。checkpoint
/// 未推进时同一批事件会重复出现，正是「失败保持可重试」的前提；旧 fixture 源
/// 一次吃光事件，无法复现重试。
struct ReplayChangeSource {
    rounds: RefCell<Vec<(Vec<ChangedRemoteFile>, MetaFilesWatermark)>>,
}

impl ReplayChangeSource {
    fn new(rounds: Vec<(Vec<ChangedRemoteFile>, MetaFilesWatermark)>) -> Self {
        Self {
            rounds: RefCell::new(rounds),
        }
    }

    /// 该批事件是否已被 `since` 消费（水位已推进且边界事件都在 cursor 里）。
    fn consumed(watermark: &MetaFilesWatermark, since: &MetaFilesWatermark) -> bool {
        watermark.active.updated_at_ms <= since.active.updated_at_ms
            && !watermark.active.boundary_event_ids.is_empty()
            && watermark
                .active
                .boundary_event_ids
                .iter()
                .all(|id| since.active.boundary_event_ids.contains(id))
    }
}

impl MetaFilesChangeSource for ReplayChangeSource {
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet> {
        let rounds = self.rounds.borrow();
        for (changed, watermark) in rounds.iter() {
            if !Self::consumed(watermark, since) {
                return Ok(ChangeSet::new(changed.clone(), watermark.clone()));
            }
        }
        Ok(ChangeSet::new(Vec::new(), since.clone()))
    }

    fn bootstrap(&self, _manifest: &SessionManifest) -> Result<ChangeSet> {
        Err(anyhow!("replay source does not support bootstrap"))
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

/// 建立 bound session：mirror（page + table）+ ownership graph + 预置 checkpoint。
fn setup_bound_session(
    name: &str,
    table_text: &str,
) -> (
    SessionManager,
    PathBuf,
    SessionManifest,
    PathBuf,
    ProjectBinding,
) {
    setup_bound_session_opts(name, table_text, true)
}

/// 同上，但 `seed_checkpoint=false` 时不预置 checkpoint——用于复现 bootstrap 轮。
fn setup_bound_session_opts(
    name: &str,
    table_text: &str,
    seed_checkpoint: bool,
) -> (
    SessionManager,
    PathBuf,
    SessionManifest,
    PathBuf,
    ProjectBinding,
) {
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
        &page_with_dwtable("Page A", "src_a", "$DATA:/tables/orders.tbl"),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "tables/orders.tbl",
        "file-t",
        "1",
        table_text,
    );
    let db_path = session_dir.join("graph.redb");
    let binding = ProjectBinding::new("proj").expect("valid binding");
    ProjectIndexer::scan_for_project(&project_mirror_root(&session_dir), &db_path, &binding)
        .expect("initial ownership scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    if seed_checkpoint {
        let mut graph =
            GraphDB::open_with_ownership(&db_path, &binding).expect("open for checkpoint");
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
    }
    (manager, session_dir, manifest, db_path, binding)
}

fn build_orchestrator(
    manager: SessionManager,
    session_dir: &Path,
    manifest: SessionManifest,
    source: impl MetaFilesChangeSource + 'static,
    provider: QueuedProvider,
    binding: &ProjectBinding,
) -> DiffRefreshOrchestrator {
    let runtime = GraphRuntime::load_with_project_dir_and_mode_for_project(
        &manifest.graph_db_path,
        Some(project_mirror_root(session_dir)),
        RuntimeMode::LongLived,
        binding,
    )
    .expect("load bound long-lived runtime");
    DiffRefreshOrchestrator::new(
        manager,
        session_dir.to_path_buf(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    )
}

fn active_event(file_id: &str, source_path: &str, revision: &str, ms: u64) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: format!("active:{file_id}:{revision}"),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: None,
        content_type: MetadataContentType::from_extension(
            source_path.rsplit('.').next().unwrap_or(""),
        ),
        updated_at_ms: ms,
        deleted: false,
    }
}

fn watermark(active_ms: u64, active_ids: Vec<String>) -> MetaFilesWatermark {
    MetaFilesWatermark {
        active: SourceCursor::new(active_ms, active_ids),
        deleted: SourceCursor::new(0, Vec::new()),
    }
}

fn durable_checkpoint(db_path: &Path, binding: &ProjectBinding) -> Option<DiffRefreshCheckpoint> {
    let graph = GraphDB::open_readonly_with_ownership(db_path, binding).expect("reopen");
    graph
        .load_diff_refresh_checkpoint()
        .expect("load checkpoint")
}

fn field_present(db_path: &Path, binding: &ProjectBinding, field_id: &str) -> bool {
    let graph = GraphDB::open_readonly_with_ownership(db_path, binding).expect("reopen");
    GraphReadStore::get_node(&graph, field_id)
        .expect("get_node")
        .is_some()
}

/// 同步持久化模式：坏 TBL 事件被消费，但 checkpoint 不得推进，且旧图保留。
#[test]
fn sync_mode_bad_tbl_does_not_advance_checkpoint() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bound_session("sync-bad", &good_table(&["order_id", "amount"]));
    let initial_checkpoint = durable_checkpoint(&db_path, &binding).expect("seeded checkpoint");
    assert!(
        field_present(&db_path, &binding, "field:orders.amount"),
        "初始图必须含 orders.amount"
    );

    let provider = QueuedProvider::new();
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    let source = ReplayChangeSource::new(vec![(
        vec![active_event("file-t", "tables/orders.tbl", "2", 1000)],
        watermark(1000, vec!["active:file-t:2".into()]),
    )]);
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("refresh with bad tbl");
    assert_eq!(report.change_count, 1, "本轮确实消费了一个变更事件");
    assert!(
        report
            .parse_failures
            .iter()
            .any(|path| path == "tables/orders.tbl"),
        "报告必须声明解析失败的文件：{:?}",
        report.parse_failures
    );
    assert_eq!(
        report.checkpoint,
        Some(initial_checkpoint.clone()),
        "解析失败不得推进 checkpoint（spec 原子性契约）：报告水位 {:?}",
        report.checkpoint
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        Some(initial_checkpoint.clone()),
        "durable checkpoint 也必须未推进"
    );
    // 旧贡献保留：图里仍是上一次成功解析的事实
    assert!(
        field_present(&db_path, &binding, "field:orders.amount"),
        "坏 TBL 不得删除旧图内容"
    );
    // 诊断可见
    let status = orchestrator.runtime().status();
    assert!(
        status
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_PARSE_FAILED),
        "解析失败必须在 status 诊断中可见：{:?}",
        status.load_diagnostics
    );
}

/// 同一批事件在 checkpoint 未推进时必须被重新投递（重试），修复后成功入图并推进。
#[test]
fn failed_round_is_retried_without_new_events_and_recovers() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bound_session("retry-bad", &good_table(&["order_id", "amount"]));
    let initial_checkpoint = durable_checkpoint(&db_path, &binding).expect("seeded checkpoint");

    let provider = QueuedProvider::new();
    // 第 1 轮投坏内容；第 2 轮（同一批事件重投）投修好的内容
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let source = ReplayChangeSource::new(vec![(
        vec![active_event("file-t", "tables/orders.tbl", "2", 1000)],
        watermark(1000, vec!["active:file-t:2".into()]),
    )]);
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    orchestrator.set_one_shot_mode(true);

    // 轮 1：失败，checkpoint 不动
    let first = orchestrator.refresh_once().expect("round 1");
    assert_eq!(first.change_count, 1);
    assert!(!first.parse_failures.is_empty());
    assert_eq!(first.checkpoint, Some(initial_checkpoint.clone()));
    assert!(
        !field_present(&db_path, &binding, "field:orders.customer"),
        "失败轮不得写入新字段"
    );

    // 轮 2：同一批事件重新投递（没有新事件），修好后成功
    let second = orchestrator.refresh_once().expect("round 2");
    assert_eq!(
        second.change_count, 1,
        "checkpoint 未推进 ⇒ 同一批事件必须被重新投递，而不是静默丢失"
    );
    assert!(
        second.parse_failures.is_empty(),
        "修复后不得再有解析失败：{:?}",
        second.parse_failures
    );
    assert_eq!(
        second.checkpoint,
        Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(1000, vec!["active:file-t:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        }),
        "成功轮必须推进 checkpoint"
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        second.checkpoint,
        "durable 必须反映推进后的水位"
    );
    assert!(
        field_present(&db_path, &binding, "field:orders.customer"),
        "修复后新字段必须入图"
    );
    // 解析失败诊断随之消失
    let status = orchestrator.runtime().status();
    assert!(
        !status
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_PARSE_FAILED),
        "恢复后 SCANNER_FILE_PARSE_FAILED 必须消失：{:?}",
        status.load_diagnostics
    );
}

/// 延迟持久化模式：坏 TBL 轮不推进 checkpoint；pending 提交后水位仍是旧值。
#[test]
fn deferred_mode_bad_tbl_keeps_old_watermark_after_pending_persist() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bound_session("deferred-bad", &good_table(&["order_id"]));
    let initial_checkpoint = durable_checkpoint(&db_path, &binding).expect("seeded checkpoint");

    let provider = QueuedProvider::new();
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    let source = ReplayChangeSource::new(vec![(
        vec![active_event("file-t", "tables/orders.tbl", "2", 1000)],
        watermark(1000, vec!["active:file-t:2".into()]),
    )]);
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    // 强制本轮就持久化 pending，验证「提交边界」而不是「没落盘所以没推进」
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 1,
    });

    let report = orchestrator.refresh_once().expect("deferred refresh");
    assert_eq!(report.change_count, 1);
    assert!(!report.parse_failures.is_empty());
    assert_eq!(report.checkpoint, Some(initial_checkpoint.clone()));
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        Some(initial_checkpoint),
        "deferred 落盘后 durable 水位仍必须是旧值"
    );
}

/// 空 poll：无变更时不得推进水位，也不得产生解析失败。
#[test]
fn empty_poll_keeps_watermark_and_reports_no_failure() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bound_session("empty-poll", &good_table(&["order_id"]));
    let initial_checkpoint = durable_checkpoint(&db_path, &binding).expect("seeded checkpoint");

    let provider = QueuedProvider::new();
    let source = ReplayChangeSource::new(Vec::new());
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("empty poll");
    assert_eq!(report.change_count, 0);
    assert!(report.parse_failures.is_empty());
    assert_eq!(report.checkpoint, Some(initial_checkpoint));
}

/// 重启：失败轮之后重启 runtime，durable 仍是旧水位与旧图，可继续重试。
#[test]
fn restart_after_failure_keeps_old_graph_and_watermark() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bound_session("restart-bad", &good_table(&["order_id", "amount"]));
    let initial_checkpoint = durable_checkpoint(&db_path, &binding).expect("seeded checkpoint");

    let provider = QueuedProvider::new();
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    let source = ReplayChangeSource::new(vec![(
        vec![active_event("file-t", "tables/orders.tbl", "2", 1000)],
        watermark(1000, vec!["active:file-t:2".into()]),
    )]);
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    orchestrator.set_one_shot_mode(true);
    let report = orchestrator.refresh_once().expect("fail round");
    assert!(!report.parse_failures.is_empty());
    drop(orchestrator);

    // 重启：重新以 binding 打开，图与水位都保持旧值
    let restarted = GraphRuntime::load_with_project_dir_and_mode_for_project(
        &db_path,
        Some(project_mirror_root(&session_dir)),
        RuntimeMode::LongLived,
        &binding,
    )
    .expect("restart runtime");
    assert!(
        restarted
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_PARSE_FAILED),
        "重启后解析失败诊断必须仍可见（陈旧而非缺失）：{:?}",
        restarted.status().load_diagnostics
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        Some(initial_checkpoint),
        "重启后 durable 水位必须是旧值"
    );
    assert!(
        field_present(&db_path, &binding, "field:orders.amount"),
        "重启后旧图必须保留"
    );
}

/// bootstrap 每次重投同一批事件：真实远端 bootstrap = 全量列表语义。checkpoint
/// 未推进 ⇒ 下一轮 orchestrator 必须重新 bootstrap 拿到同一批事件（失败重试的
/// 前提）；已推进 ⇒ poll 空转。
struct BootstrapReplaySource {
    changed: Vec<ChangedRemoteFile>,
    watermark: MetaFilesWatermark,
}

impl MetaFilesChangeSource for BootstrapReplaySource {
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet> {
        Ok(ChangeSet::new(Vec::new(), since.clone()))
    }

    fn bootstrap(&self, _manifest: &SessionManifest) -> Result<ChangeSet> {
        Ok(ChangeSet::new(self.changed.clone(), self.watermark.clone()))
    }
}

/// deferred（LongLived）模式下 **bootstrap 轮**解析失败：checkpoint 必须保持
/// 「无」，下一轮重新 bootstrap 重投同一批事件；修好后才入图并推进水位。
///
/// 独立预期（先于实现细节，逐段断言）：
/// - 失败轮报告的水位是 `None`，durable 也不落任何水位（spec：失败不得推进
///   checkpoint——对 bootstrap 轮即「不得从无到有」）；
/// - 旧图保留，解析失败诊断在 status 可见；
/// - 下一轮**不依赖任何新事件**：同一批事件经重新 bootstrap 重投（这正是
///   「不得消费失败事件后靠未来事件偶然触发重试」的反面）；
/// - 成功轮才推进并落库水位，新字段入图。
#[test]
fn deferred_bootstrap_bad_tbl_rebootstraps_until_fixed() {
    let (manager, session_dir, manifest, db_path, binding) = setup_bound_session_opts(
        "deferred-bootstrap-bad",
        &good_table(&["order_id", "amount"]),
        false,
    );
    assert!(
        durable_checkpoint(&db_path, &binding).is_none(),
        "本测试必须从无 checkpoint 状态开始（bootstrap 轮）"
    );

    let provider = QueuedProvider::new();
    // 第 1 轮 bootstrap 投坏内容；第 2 轮（重新 bootstrap）投修好的内容
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let source = BootstrapReplaySource {
        changed: vec![active_event("file-t", "tables/orders.tbl", "2", 1000)],
        watermark: watermark(1000, vec!["active:file-t:2".into()]),
    };
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);

    // 轮 1：默认 deferred 策略（pending 不落盘）——失败轮不得产生任何水位
    let first = orchestrator
        .refresh_once()
        .expect("bootstrap round with bad tbl");
    assert_eq!(first.change_count, 1, "本轮确实消费了一个变更事件");
    assert_eq!(
        first.parse_failures,
        vec!["tables/orders.tbl".to_string()],
        "报告必须声明解析失败的文件：{:?}",
        first.parse_failures
    );
    assert_eq!(
        first.checkpoint, None,
        "bootstrap 失败轮不得报告任何水位（spec 原子性契约）：{:?}",
        first.checkpoint
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        None,
        "durable 不得落任何水位"
    );
    assert!(
        field_present(&db_path, &binding, "field:orders.amount"),
        "失败轮不得破坏旧图"
    );
    assert!(
        orchestrator
            .runtime()
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_PARSE_FAILED),
        "解析失败必须在 status 诊断中可见"
    );

    // 轮 2：强制本轮持久化（验证提交边界）。checkpoint 未推进 ⇒ 必须重新
    // bootstrap 重投同一批事件，修好后成功入图并推进。
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 1,
    });
    let second = orchestrator.refresh_once().expect("round 2 after fix");
    assert_eq!(
        second.change_count, 1,
        "checkpoint 未推进 ⇒ 下一轮必须重新 bootstrap 重投同一批事件，而不是静默丢失"
    );
    assert!(
        second.parse_failures.is_empty(),
        "修复后不得再有解析失败：{:?}",
        second.parse_failures
    );
    let expected = DiffRefreshCheckpoint {
        active: SourceCursor::new(1000, vec!["active:file-t:2".into()]),
        deleted: SourceCursor::new(0, Vec::new()),
    };
    assert_eq!(
        second.checkpoint,
        Some(expected.clone()),
        "成功轮必须推进 checkpoint"
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        Some(expected),
        "durable 必须反映推进后的水位"
    );
    assert!(
        field_present(&db_path, &binding, "field:orders.customer"),
        "修复后新字段必须入图"
    );
}

/// prepare（diff-refresh）入口的冲突生命周期：产生 → 查询侧可见 → 重启仍可见
/// → 修复后清除。独立预期逐段断言，不与其他路径比较。
#[test]
fn prepare_entry_conflict_lifecycle_via_refresh() {
    let root = test_root("prepare-conflict-lifecycle");
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");
    seed_file(
        &session_dir,
        &mut manifest,
        "tables/a.tbl",
        "file-a",
        "1",
        &good_table(&["f_a"]),
    );
    let db_path = session_dir.join("graph.redb");
    let binding = ProjectBinding::new("proj").expect("valid binding");
    ProjectIndexer::scan_for_project(&project_mirror_root(&session_dir), &db_path, &binding)
        .expect("seed scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    // 预置 checkpoint → 后续轮走 poll 而非 bootstrap
    {
        let mut graph =
            GraphDB::open_with_ownership(&db_path, &binding).expect("open for checkpoint");
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
    }
    let restarted_status = |db: &Path, b: &ProjectBinding| {
        let runtime = GraphRuntime::load_with_project_dir_and_mode_for_project(
            db,
            None::<&std::path::Path>,
            RuntimeMode::OneShot,
            b,
        )
        .expect("restart runtime");
        runtime.status()
    };

    // 轮 1：新增 tables/other/a.tbl（同 stem a）→ prepare 产生 model:a 冲突
    let provider = QueuedProvider::new();
    provider.push("tables/other/a.tbl", "file-o", "1", &good_table(&["f_b"]));
    let source = ReplayChangeSource::new(vec![
        (
            vec![active_event("file-o", "tables/other/a.tbl", "1", 1000)],
            watermark(1000, vec!["active:file-o:1".into()]),
        ),
        // 轮 2：删除冲突文件 → 冲突必须消失
        (
            vec![ChangedRemoteFile {
                event_id: "active:file-o:1-del".to_string(),
                file_id: "file-o".to_string(),
                source_path: "tables/other/a.tbl".to_string(),
                previous_source_path: None,
                content_type: MetadataContentType::from_extension("tbl"),
                updated_at_ms: 2000,
                deleted: true,
            }],
            watermark(2000, vec!["active:file-o:1-del".into()]),
        ),
    ]);
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    // 每轮强制持久化：重启可见性必须建立在 durable 状态上，而不是内存 pending
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 1,
    });

    // 1. 冲突产生：report 与 runtime status 都必须可见
    let report = orchestrator
        .refresh_once()
        .expect("refresh adding conflict file");
    assert!(
        report.ownership_conflicts.iter().any(|id| id == "model:a"),
        "report 必须登记冲突节点：{:?}",
        report.ownership_conflicts
    );
    assert!(
        orchestrator
            .runtime()
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "status.load_diagnostics 必须可见冲突：{:?}",
        orchestrator.runtime().status().load_diagnostics
    );
    drop(orchestrator);

    // 2. 重启后仍可见
    let after_add = restarted_status(&db_path, &binding);
    assert!(
        after_add
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "重启后冲突必须仍可见：{:?}",
        after_add.load_diagnostics
    );

    // 3. 修复（删除冲突文件）后清除，重启后仍清除
    let (manager2, session_dir2, manifest2) = {
        let manager = SessionManager::new(&root);
        let manifest = manager.read_manifest("s1").expect("read manifest 2");
        (manager, session_dir.clone(), manifest)
    };
    let provider2 = QueuedProvider::new();
    let source2 = ReplayChangeSource::new(vec![(
        vec![ChangedRemoteFile {
            event_id: "active:file-o:1-del".to_string(),
            file_id: "file-o".to_string(),
            source_path: "tables/other/a.tbl".to_string(),
            previous_source_path: None,
            content_type: MetadataContentType::from_extension("tbl"),
            updated_at_ms: 2000,
            deleted: true,
        }],
        watermark(2000, vec!["active:file-o:1-del".into()]),
    )]);
    let mut orchestrator2 = build_orchestrator(
        manager2,
        &session_dir2,
        manifest2,
        source2,
        provider2,
        &binding,
    );
    orchestrator2.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 1,
    });
    let fixed = orchestrator2
        .refresh_once()
        .expect("refresh deleting conflict");
    assert!(
        !fixed.ownership_conflicts.iter().any(|id| id == "model:a"),
        "修复后 report 不得再登记冲突：{:?}",
        fixed.ownership_conflicts
    );
    assert!(
        !orchestrator2
            .runtime()
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "修复后 status 不得再有冲突：{:?}",
        orchestrator2.runtime().status().load_diagnostics
    );
    drop(orchestrator2);
    let after_fix = restarted_status(&db_path, &binding);
    assert!(
        !after_fix
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "修复后重启也不得再有冲突：{:?}",
        after_fix.load_diagnostics
    );
}

/// 空 poll 轮也必须如实报告账本中仍然存在的来源冲突。
///
/// 报告字段契约是「本轮账本中仍然存在的来源冲突」；空轮不得把它硬编码成空——
/// 否则持久冲突在无事件轮次会从报告里「闪灭」，消费方会误判冲突已修复。
/// status/query/重启三面读 runtime 状态不受影响，这里钉的是报告口径。
#[test]
fn empty_poll_reports_persisting_ownership_conflicts() {
    let root = test_root("empty-poll-conflict");
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");
    seed_file(
        &session_dir,
        &mut manifest,
        "tables/a.tbl",
        "file-a",
        "1",
        &good_table(&["f_a"]),
    );
    seed_file(
        &session_dir,
        &mut manifest,
        "tables/other/a.tbl",
        "file-o",
        "1",
        &good_table(&["f_b"]),
    );
    let db_path = session_dir.join("graph.redb");
    let binding = ProjectBinding::new("proj").expect("valid binding");
    ProjectIndexer::scan_for_project(&project_mirror_root(&session_dir), &db_path, &binding)
        .expect("seed scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    // 预置 checkpoint → 本轮走 poll 且无任何事件
    {
        let mut graph =
            GraphDB::open_with_ownership(&db_path, &binding).expect("open for checkpoint");
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
    }

    let provider = QueuedProvider::new();
    let source = ReplayChangeSource::new(Vec::new());
    let mut orchestrator =
        build_orchestrator(manager, &session_dir, manifest, source, provider, &binding);
    let report = orchestrator.refresh_once().expect("empty poll");
    assert_eq!(report.change_count, 0);
    assert!(
        report.ownership_conflicts.iter().any(|id| id == "model:a"),
        "空 poll 轮必须如实报告账本中仍然存在的冲突：{:?}",
        report.ownership_conflicts
    );
}
