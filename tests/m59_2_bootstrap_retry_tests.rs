#![cfg(feature = "cli-local")]

//! M59-2 B：真实 bootstrap 源下的失败重试（P1-1 修复前反例）。
//!
//! 源码链路结论（只读核对）：
//! - `orchestrator.rs` 在 prepare **之前** `apply_changeset_to_mirror` +
//!   `write_manifest`；`mirror.rs` 把本轮拉取到的内容 revision/hash 写进
//!   manifest。
//! - TBL 解析失败时 `commit.checkpoint` 保留 `None`（bootstrap 轮），但 manifest
//!   里该文件的 `revision`/`source_path` 已经是最新值。
//! - 下一轮真实 `BiMetaFilesChangeSource::bootstrap` 按 `file_id` 对照 manifest
//!   的 `revision + source_path` 判 `unchanged` ⇒ 同一批事件**不再投递** ⇒
//!   `change_count == 0` ⇒ 走 orchestrator 空 ChangeSet 分支直接
//!   `persist_with_checkpoint`，**没有 prepare**。失败文件从未成功入图，水位
//!   却推进了。
//!
//! 触发条件：远端快照保持不变即可（不需要远端清空文件）。
//!
//! 现有 `BootstrapReplaySource`（m59_2_refresh_checkpoint_tests.rs）忽略
//! manifest 强制重投，因此它的绿色不能证明生产重试正确。本文件用**真实**
//! `BiMetaFilesChangeSource` + 可控 transport/provider 建立反例。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    BiActiveFileInfo, BiDeletedMetaFileInfo, BiMetaFilesChangeSource, BiMetaFilesTransport,
    DiffRefreshCheckpoint, DiffRefreshOrchestrator, LongLivedPersistPolicy,
};
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
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
use metadata_checker::session::sync::{SessionSyncItem, SessionSyncMode, project_mirror_root};
use metadata_checker::session::{RemoteSessionProvider, SessionManager};

const CODE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";
const BAD_TABLE: &str = "{ invalid json syntax -- not closed";

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m59-2-bootretry-{pid}-{seq}-{name}"
    ));
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

/// 可控远端快照：active/deleted 列表可逐轮改写，模拟「远端快照不变」的场景。
struct SnapshotTransport {
    active: RefCell<Vec<BiActiveFileInfo>>,
    deleted: RefCell<Vec<BiDeletedMetaFileInfo>>,
}

impl SnapshotTransport {
    fn new(active: Vec<BiActiveFileInfo>) -> Self {
        Self {
            active: RefCell::new(active),
            deleted: RefCell::new(Vec::new()),
        }
    }
}

impl BiMetaFilesTransport for SnapshotTransport {
    fn list_active_files(&self, _project_ref: &str) -> Result<Vec<BiActiveFileInfo>> {
        Ok(self.active.borrow().clone())
    }
    fn list_deleted_files(&self, _project_ref: &str) -> Result<Vec<BiDeletedMetaFileInfo>> {
        Ok(self.deleted.borrow().clone())
    }
}

fn active_info(file_id: &str, path: &str, revision: &str, ms: u64) -> BiActiveFileInfo {
    BiActiveFileInfo {
        file_id: Some(file_id.to_string()),
        path: Some(path.to_string()),
        parent_dir: None,
        name: None,
        file_type: Some("tbl".to_string()),
        is_folder: false,
        revision: Some(revision.to_string()),
        modify_time: Some(ms),
    }
}

/// 内容按 (path) 排队吐出的 provider：同一文件多轮变更按 fetch 顺序消费。
struct QueuedProvider {
    contents: RefCell<VecDeque<RemoteFileContent>>,
}

impl QueuedProvider {
    fn new() -> Self {
        Self {
            contents: RefCell::new(VecDeque::new()),
        }
    }

    fn push(&self, path: &str, file_id: &str, revision: &str, raw_text: &str) {
        self.contents.borrow_mut().push_back(RemoteFileContent {
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
            .pop_front()
            .ok_or_else(|| anyhow!("no queued content for {}", file_ref.source_path))
    }
    fn fetch_changed_since(&self, _p: &str, _r: &str) -> Result<RemoteChangeSet> {
        Err(anyhow!("stub does not support fetch_changed_since"))
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
    metadata_checker::session::sync::sync_remote_files_to_session(
        session_dir,
        manifest,
        &[SessionSyncItem::new(content)],
        SessionSyncMode::Partial,
    )
    .expect("seed mirror file");
}

/// 建立无 checkpoint（bootstrap 轮）的 bound session。
#[allow(clippy::type_complexity)]
fn setup_bootstrap_session(
    name: &str,
    table_text: &str,
) -> (
    SessionManager,
    PathBuf,
    SessionManifest,
    PathBuf,
    ProjectBinding,
) {
    setup_bootstrap_session_in(&test_root(name), table_text)
}

/// 在指定 session 根目录下建立无 checkpoint 的 bound session。
///
/// 与 [`setup_bootstrap_session`] 的差别只在根目录可控：模拟「进程重启」时
/// 需要用同一个根重新构造 `SessionManager`，才能读到同一份 manifest。
#[allow(clippy::type_complexity)]
fn setup_bootstrap_session_in(
    root: &Path,
    table_text: &str,
) -> (
    SessionManager,
    PathBuf,
    SessionManifest,
    PathBuf,
    ProjectBinding,
) {
    let manager = SessionManager::new(root);
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
    (manager, session_dir, manifest, db_path, binding)
}

fn build_orchestrator(
    manager: SessionManager,
    session_dir: &Path,
    manifest: SessionManifest,
    transport: SnapshotTransport,
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
    let source = BiMetaFilesChangeSource::new(transport, "proj");
    DiffRefreshOrchestrator::new(
        manager,
        session_dir.to_path_buf(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    )
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

fn parse_failed_visible(runtime: &GraphRuntime) -> bool {
    runtime
        .status()
        .load_diagnostics
        .iter()
        .any(|d| d.code == CODE_PARSE_FAILED)
}

/// 真实 bootstrap 源：失败轮之后**远端快照不变**，同一批事件必须重投重试。
///
/// 独立预期（先于实现，逐段断言）：
/// 1. 轮 1（bootstrap，坏 TBL）：消费 1 个事件、报解析失败、**不产生任何水位**；
///    旧图保留（`field:orders.amount` 仍在）、诊断可见。
/// 2. 轮 2（无新事件，远端快照与轮 1 完全相同）：必须重新 bootstrap 并**再次
///    投递同一批事件**（change_count == 1），而不是被 manifest 判 unchanged 跳过。
/// 3. 轮 3（远端内容修好）：成功入图并推进水位，诊断消失。
#[test]
fn real_bootstrap_source_retries_failed_file_without_new_events() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bootstrap_session("real-bootstrap-retry", &good_table(&["order_id", "amount"]));
    assert!(
        durable_checkpoint(&db_path, &binding).is_none(),
        "本测试必须从无 checkpoint 状态开始（bootstrap 轮）"
    );
    assert!(field_present(&db_path, &binding, "field:orders.amount"));

    // 远端快照固定不变：file-t revision=2 —— 正是「远端快照不变」这一触发条件。
    let transport = SnapshotTransport::new(vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "2", 1000),
    ]);
    let provider = QueuedProvider::new();
    // 轮 1：坏内容；轮 2（重投）：坏内容；轮 3：修好的内容
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let mut orchestrator = build_orchestrator(
        manager,
        &session_dir,
        manifest,
        transport,
        provider,
        &binding,
    );
    orchestrator.set_one_shot_mode(true);

    // 轮 1：解析失败，不推进水位
    let first = orchestrator.refresh_once().expect("round 1");
    assert_eq!(first.change_count, 1, "轮 1 确实消费了一个变更事件");
    assert_eq!(
        first.parse_failures,
        vec!["tables/orders.tbl".to_string()],
        "轮 1 必须报告解析失败文件：{:?}",
        first.parse_failures
    );
    assert_eq!(
        first.checkpoint, None,
        "bootstrap 失败轮不得产生任何水位：{:?}",
        first.checkpoint
    );
    assert_eq!(durable_checkpoint(&db_path, &binding), None);
    assert!(
        field_present(&db_path, &binding, "field:orders.amount"),
        "失败轮不得破坏旧图"
    );
    assert!(parse_failed_visible(orchestrator.runtime()));

    // 轮 2：远端快照完全不变 ⇒ 真实 bootstrap 必须重投同一批事件。
    // 这里断言的是「镜像已获取」与「图已成功索引」的边界：manifest 已记录
    // revision=2，但该文件从未成功入图，因此不得判 unchanged。
    let second = orchestrator.refresh_once().expect("round 2");
    assert_eq!(
        second.change_count, 1,
        "远端快照不变时，未成功入图的文件必须被重新投递重试（change_count 应为 1，实际 {}）",
        second.change_count
    );
    assert_eq!(
        second.checkpoint, None,
        "再次失败仍不得推进水位：{:?}",
        second.checkpoint
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        None,
        "重复失败轮的 durable 仍必须是「无水位」"
    );
    assert!(
        !field_present(&db_path, &binding, "field:orders.customer"),
        "失败轮不得写入新字段"
    );

    // 轮 3：远端内容修好 ⇒ 成功入图并推进水位
    let third = orchestrator.refresh_once().expect("round 3");
    assert!(
        third.parse_failures.is_empty(),
        "修复后不得再有解析失败：{:?}",
        third.parse_failures
    );
    let advanced = third.checkpoint.clone().expect("成功轮必须推进水位");
    assert_eq!(durable_checkpoint(&db_path, &binding), Some(advanced));
    assert!(
        field_present(&db_path, &binding, "field:orders.customer"),
        "修复后新字段必须入图"
    );
    assert!(
        !parse_failed_visible(orchestrator.runtime()),
        "恢复后解析失败诊断必须消失：{:?}",
        orchestrator.runtime().status().load_diagnostics
    );
}

/// 延迟（deferred）持久化下的同一边界：多轮 tick 后失败文件仍保持脏、可重试，
/// 水位不推进；修好后清诊断并正确推进。
#[test]
fn deferred_mode_real_bootstrap_keeps_retryable_until_fixed() {
    let (manager, session_dir, manifest, db_path, binding) = setup_bootstrap_session(
        "deferred-bootstrap-retry",
        &good_table(&["order_id", "amount"]),
    );

    let transport = SnapshotTransport::new(vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "2", 1000),
    ]);
    let provider = QueuedProvider::new();
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let mut orchestrator = build_orchestrator(
        manager,
        &session_dir,
        manifest,
        transport,
        provider,
        &binding,
    );
    // 强制每轮都落盘，验证「提交边界」而不是「没落盘所以没推进」
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: 0,
        max_pending_rounds: 1,
    });

    for round in 1..=2 {
        let report = orchestrator.refresh_once().expect("fail round");
        assert_eq!(
            report.change_count, 1,
            "第 {round} 轮必须重投同一批事件（实际 {}）",
            report.change_count
        );
        assert!(!report.parse_failures.is_empty());
        assert_eq!(report.checkpoint, None, "第 {round} 轮不得推进水位");
        assert_eq!(
            durable_checkpoint(&db_path, &binding),
            None,
            "第 {round} 轮 deferred 落盘后 durable 仍必须是「无水位」"
        );
    }

    let fixed = orchestrator.refresh_once().expect("fixed round");
    assert!(fixed.parse_failures.is_empty());
    let advanced = fixed.checkpoint.clone().expect("修复轮必须推进水位");
    assert_eq!(durable_checkpoint(&db_path, &binding), Some(advanced));
    assert!(field_present(&db_path, &binding, "field:orders.customer"));
    assert!(!parse_failed_visible(orchestrator.runtime()));
}

/// 重启后重试：失败轮之后重建 orchestrator（重现 manifest 从磁盘读取），
/// 失败文件仍必须重投，不得因为 manifest 已落 revision 而永久跳过。
#[test]
fn restart_after_failed_bootstrap_still_redelivers() {
    let root = test_root("restart-bootstrap-retry");
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
        &good_table(&["order_id", "amount"]),
    );
    let db_path = session_dir.join("graph.redb");
    let binding = ProjectBinding::new("proj").expect("valid binding");
    ProjectIndexer::scan_for_project(&project_mirror_root(&session_dir), &db_path, &binding)
        .expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    let active_snapshot = vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "2", 1000),
    ];

    // 第一次运行：坏内容，失败后退出（模拟进程重启）
    {
        let provider = QueuedProvider::new();
        provider.push("tables/orders.tbl", "file-t", "2", BAD_TABLE);
        let mut orchestrator = build_orchestrator(
            SessionManager::new(&root),
            &session_dir,
            manager.read_manifest("s1").expect("read manifest"),
            SnapshotTransport::new(active_snapshot.clone()),
            provider,
            &binding,
        );
        orchestrator.set_one_shot_mode(true);
        let report = orchestrator.refresh_once().expect("fail round");
        assert_eq!(report.change_count, 1);
        assert!(!report.parse_failures.is_empty());
        assert_eq!(report.checkpoint, None);
    }

    // 重启：重新读 manifest（含已落盘的 revision=2）与 durable（无水位）
    let restarted_manifest = manager
        .read_manifest("s1")
        .expect("read manifest after restart");
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        None,
        "重启后 durable 仍必须是无水位"
    );
    let provider = QueuedProvider::new();
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let mut orchestrator = build_orchestrator(
        SessionManager::new(&root),
        &session_dir,
        restarted_manifest,
        SnapshotTransport::new(active_snapshot),
        provider,
        &binding,
    );
    orchestrator.set_one_shot_mode(true);
    let report = orchestrator.refresh_once().expect("restart round");
    assert_eq!(
        report.change_count, 1,
        "重启后失败文件必须重投（实际 {}）：manifest 已记录 revision 不等于已成功入图",
        report.change_count
    );
    assert!(report.parse_failures.is_empty());
    assert!(report.checkpoint.is_some(), "修复轮必须推进水位");
    assert!(field_present(&db_path, &binding, "field:orders.customer"));
    assert!(!parse_failed_visible(orchestrator.runtime()));
}

/// 持久化失败（Synchronous/one-shot）：**镜像已获取**不等于**图已成功索引**，
/// 更不等于**已 durable**。
///
/// 独立预期（先于实现）：
/// 1. 轮 1：bootstrap 成功 prepare，但 durable persist 失败 ⇒ `refresh_once` 返回
///    Err，且 manifest 不得把该文件记为「已索引」——它的内容只在内存候选图里，
///    durable 图里没有，进程重启后就没有了。
/// 2. 轮 2（持久化恢复，远端快照仍不变）：必须重新投递该文件并真正落库。
///    若轮 1 已经把 manifest 标成已索引，轮 2 会判 unchanged ⇒ `change_count == 0`
///    ⇒ 走空 ChangeSet 分支写下 bootstrap 水位 ⇒ 失败文件**永久**不再重试。
#[test]
fn sync_persist_failure_keeps_file_retryable_until_durable() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bootstrap_session("persist-fail-retry", &good_table(&["order_id", "amount"]));
    assert!(durable_checkpoint(&db_path, &binding).is_none());

    let transport = SnapshotTransport::new(vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "2", 1000),
    ]);
    let provider = QueuedProvider::new();
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    provider.push(
        "tables/orders.tbl",
        "file-t",
        "2",
        &good_table(&["order_id", "amount", "customer"]),
    );
    let mut orchestrator = build_orchestrator(
        manager.clone(),
        &session_dir,
        manifest,
        transport,
        provider,
        &binding,
    );
    orchestrator.set_one_shot_mode(true);

    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_hook = Arc::clone(&attempts);
    orchestrator.set_persist_fn(Box::new(move |graph, commit| {
        let attempt = attempts_for_hook.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(anyhow!("injected bootstrap persist failure"))
        } else {
            graph.persist_commit(commit)
        }
    }));

    // 轮 1：prepare 成功、durable 写入失败
    let first = orchestrator.refresh_once();
    assert!(
        first.is_err(),
        "注入的持久化失败必须让本轮以 Err 结束，而不是静默算成功"
    );
    assert!(
        !field_present(&db_path, &binding, "field:orders.customer"),
        "持久化失败后 durable 图里不得出现新字段"
    );
    let manifest_after_failure = manager.read_manifest("s1").expect("read manifest");
    let record = manifest_after_failure
        .files
        .iter()
        .find(|file| file.source_path == "tables/orders.tbl")
        .expect("manifest 必须仍有 orders.tbl 记录");
    assert!(
        record.needs_index_retry(),
        "持久化失败的文件不得被记为已索引（manifest 只能证明镜像已获取）：hash={:?} indexed_hash={:?}",
        record.hash,
        record.indexed_hash
    );

    // 轮 2：持久化恢复，远端快照与轮 1 完全相同 ⇒ 必须重投并真正落库
    let second = orchestrator.refresh_once().expect("round 2");
    assert_eq!(
        second.change_count, 1,
        "未 durable 的文件必须重投（change_count 应为 1，实际 {}）：否则空 ChangeSet 分支会推水位并永久丢失该文件",
        second.change_count
    );
    assert!(second.checkpoint.is_some(), "本轮已真正落库，必须推进水位");
    assert!(
        field_present(&db_path, &binding, "field:orders.customer"),
        "重投后新字段必须进入 durable 图"
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        second.checkpoint,
        "durable 水位必须与本轮报告一致"
    );
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        2,
        "持久化钩子应被调用两次（失败一次、成功一次）"
    );
}

/// 重启前未落盘（Deferred）：内存候选图已安装，但 durable 里没有。
///
/// 进程重启会丢掉 pending 状态；若 manifest 此时已把文件记为「已索引」，
/// 下一轮真实 bootstrap 判 unchanged ⇒ 空 ChangeSet ⇒ 推进水位 ⇒ 永久丢失。
/// 独立预期：重启后同一批事件必须重投。
#[test]
fn deferred_restart_before_persist_keeps_file_retryable() {
    let root = test_root("deferred-restart-retry");
    let (manager, session_dir, _manifest, db_path, binding) =
        setup_bootstrap_session_in(&root, &good_table(&["order_id", "amount"]));

    let active_snapshot = vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "2", 1000),
    ];
    let fixed_table = good_table(&["order_id", "amount", "customer"]);

    // 第一次运行：Deferred（默认策略：阈值未到，本轮不落盘），随后「进程重启」
    {
        let provider = QueuedProvider::new();
        provider.push("tables/orders.tbl", "file-t", "2", &fixed_table);
        let mut orchestrator = build_orchestrator(
            SessionManager::new(&root),
            &session_dir,
            manager.read_manifest("s1").expect("read manifest"),
            SnapshotTransport::new(active_snapshot.clone()),
            provider,
            &binding,
        );
        let report = orchestrator.refresh_once().expect("deferred round 1");
        assert_eq!(report.change_count, 1, "轮 1 应消费该事件");
        assert!(
            !report.persisted,
            "默认策略下轮 1 不应落盘（本测试要覆盖「未落盘即重启」）"
        );
    }

    assert!(
        !field_present(&db_path, &binding, "field:orders.customer"),
        "未落盘 ⇒ durable 图里不得有新字段"
    );
    assert_eq!(
        durable_checkpoint(&db_path, &binding),
        None,
        "未落盘 ⇒ durable 不得有水位"
    );

    // 重启：pending 状态随进程消失，只剩磁盘上的 manifest 与 durable 图
    let restarted_manifest = manager
        .read_manifest("s1")
        .expect("read manifest after restart");
    let provider = QueuedProvider::new();
    provider.push("tables/orders.tbl", "file-t", "2", &fixed_table);
    let mut orchestrator = build_orchestrator(
        SessionManager::new(&root),
        &session_dir,
        restarted_manifest,
        SnapshotTransport::new(active_snapshot),
        provider,
        &binding,
    );
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("restart round");
    assert_eq!(
        report.change_count, 1,
        "重启后未 durable 的文件必须重投（实际 {}）：manifest 记录不等于已落库",
        report.change_count
    );
    assert!(
        field_present(&db_path, &binding, "field:orders.customer"),
        "重投后新字段必须进入 durable 图"
    );
    assert!(report.checkpoint.is_some(), "落库成功必须推进水位");
}

/// 空 bootstrap（远端快照与 manifest 完全一致且文件已成功入图）：
/// 必须落 checkpoint-only 提交，使下一轮走 poll 而非重复 bootstrap。
#[test]
fn unchanged_snapshot_after_success_is_not_redelivered() {
    let (manager, session_dir, manifest, db_path, binding) =
        setup_bootstrap_session("bootstrap-stable", &good_table(&["order_id", "amount"]));

    // 远端快照与初始 manifest 完全一致（revision=1），且文件已成功入图
    let transport = SnapshotTransport::new(vec![
        active_info("file-a", "proj/app/page_a.spg", "1", 900),
        active_info("file-t", "proj/tables/orders.tbl", "1", 950),
    ]);
    let provider = QueuedProvider::new();
    let mut orchestrator = build_orchestrator(
        manager,
        &session_dir,
        manifest,
        transport,
        provider,
        &binding,
    );
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("stable bootstrap");
    assert_eq!(
        report.change_count, 0,
        "已入图且快照一致的文件不得重复投递：{:?}",
        report
    );
    assert!(report.parse_failures.is_empty());
    assert!(
        report.checkpoint.is_some(),
        "稳定 bootstrap 必须落 checkpoint-only 提交，使下一轮走 poll"
    );
    assert_eq!(durable_checkpoint(&db_path, &binding), report.checkpoint);

    // 下一轮：有 checkpoint ⇒ 走 poll，快照仍不变 ⇒ 空转且不推进
    let second = orchestrator.refresh_once().expect("poll round");
    assert_eq!(second.change_count, 0);
    assert_eq!(second.checkpoint, report.checkpoint);
}
