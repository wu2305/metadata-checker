#![cfg(feature = "cli-local")]

//! M58.3 PR2 回归测试：diff-refresh 链路的 scanner 诊断（SCANNER_*）持久化
//! 与 live runtime 诊断缓存刷新。
//!
//! 覆盖：
//! - 同步（one-shot）模式：文件由坏修好 → 持久化 entry 消失；文件新变坏 → entry 出现；
//! - deferred 模式：未达阈值时 durable 不变但 live status/load_diagnostics 立即反映，
//!   轮次回退触发 persist_pending 后 durable 反映；
//! - deferred 跨轮合并：同一文件后者覆盖前者、删除文件移除 entry。

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
use metadata_checker::graph_store::{IndexCommit, IndexStateStore};
use metadata_checker::output::Diagnostic;
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

const CODE_UNRECOGNIZED: &str = "SCANNER_UNRECOGNIZED_CONTAINER_KEY";
const CODE_DUPLICATE: &str = "SCANNER_DUPLICATE_COMPONENT_ID";

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr2-test-{pid}-{seq}-{name}"
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

/// 含未识别容器键的页面：计 1 次 SCANNER_UNRECOGNIZED_CONTAINER_KEY
fn spg_with_unrecognized_key() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "panel_a", "type": "panel", "myContainer": [{"id": "child1", "type": "button"}, {"label": "no-id"}]}
            ]
        }
    })
}

/// 含重复组件 id 的页面：计 1 次 SCANNER_DUPLICATE_COMPONENT_ID
fn spg_with_duplicate_id() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "dup1", "type": "button"},
                {"id": "dup1", "type": "input"}
            ]
        }
    })
}

/// 合法页面（marker 区分内容版本，避免与初始版本同 hash）
fn spg_fixed(marker: &str) -> serde_json::Value {
    serde_json::json!({
        "x-marker": marker,
        "canvas": {
            "components": [
                {"id": "panel_a", "type": "panel", "children": [{"id": "child1", "type": "button"}]}
            ]
        }
    })
}

/// 按 code 过滤诊断
fn code_hits<'a>(diags: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.code == code).collect()
}

/// 内容按 source_path 排队吐出的 provider：同一文件多轮变更时按 fetch 顺序
/// 依次返回不同 revision 的内容（m54 的 HashMap stub 无法区分同路径多版本）。
struct QueuedProvider {
    contents: RefCell<HashMap<String, VecDeque<RemoteFileContent>>>,
}

impl QueuedProvider {
    fn new() -> Self {
        Self {
            contents: RefCell::new(HashMap::new()),
        }
    }

    /// 登记某个 path 下一次 fetch 应返回的内容（多次登记按序消费）
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

/// 按轮次吐出预制 ChangeSet 的变更源：fixture 源一次 poll 会吃光所有
/// 晚于 cursor 的事件，无法驱动「多轮不同变更」的 deferred 跨轮场景。
struct QueuedChangeSource {
    rounds: RefCell<VecDeque<(Vec<ChangedRemoteFile>, MetaFilesWatermark)>>,
}

impl QueuedChangeSource {
    fn new(rounds: Vec<(Vec<ChangedRemoteFile>, MetaFilesWatermark)>) -> Self {
        Self {
            rounds: RefCell::new(rounds.into_iter().collect()),
        }
    }
}

impl MetaFilesChangeSource for QueuedChangeSource {
    fn poll(&self, since: &MetaFilesWatermark) -> Result<ChangeSet> {
        let mut rounds = self.rounds.borrow_mut();
        if let Some((changed, next_watermark)) = rounds.pop_front() {
            Ok(ChangeSet::new(changed, next_watermark))
        } else {
            // 队列耗尽后按契约返回空集合且 cursor 原样保留
            Ok(ChangeSet::new(Vec::new(), since.clone()))
        }
    }

    fn bootstrap(&self, _manifest: &SessionManifest) -> Result<ChangeSet> {
        Err(anyhow!("queued source does not support bootstrap"))
    }
}

/// 构造活跃变更事件
fn active_event(file_id: &str, source_path: &str, revision: &str, ms: u64) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: format!("active:{file_id}:{revision}"),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: None,
        content_type: MetadataContentType::SuperPage,
        updated_at_ms: ms,
        deleted: false,
    }
}

/// 构造删除事件
fn deleted_event(file_id: &str, source_path: &str, uuid: &str, ms: u64) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: format!("deleted:{uuid}"),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: None,
        content_type: MetadataContentType::SuperPage,
        updated_at_ms: ms,
        deleted: true,
    }
}

/// 构造双 cursor 水位
fn watermark(active_ms: u64, active_ids: Vec<String>, deleted_ms: u64) -> MetaFilesWatermark {
    MetaFilesWatermark {
        active: SourceCursor::new(active_ms, active_ids),
        deleted: SourceCursor::new(deleted_ms, Vec::new()),
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

/// 完整 session 初始状态：mirror 指定页面 + graph + manifest 落盘 + 预置 checkpoint。
///
/// files: (source_path, file_id, revision, spg JSON)。
fn setup_session(
    name: &str,
    files: &[(&str, &str, &str, serde_json::Value)],
) -> (SessionManager, PathBuf, SessionManifest, PathBuf) {
    let root = test_root(name);
    let manager = SessionManager::new(&root);
    manager
        .create_session("s1", "https://bi.test", "proj", "proj", "remote")
        .expect("create session");
    let session_dir = manager.session_dir("s1");
    let mut manifest = manager.read_manifest("s1").expect("read manifest");
    for (path, file_id, revision, value) in files {
        seed_file(
            &session_dir,
            &mut manifest,
            path,
            file_id,
            revision,
            &serde_json::to_string_pretty(value).expect("serialize spg"),
        );
    }
    let db_path = session_dir.join("graph.redb");
    ProjectIndexer::scan(&project_mirror_root(&session_dir), &db_path).expect("initial scan");
    manifest.graph_db_path = db_path.to_string_lossy().to_string();
    manager.write_manifest(&manifest).expect("write manifest");

    // 预置 checkpoint，使 refresh_once 走 poll 路径
    let mut graph = GraphDB::open(&db_path).expect("open graph for checkpoint seed");
    let commit = IndexCommit {
        file_states: graph.load_file_states().expect("load states"),
        dirty_nodes: Vec::new(),
        deleted_nodes: Vec::new(),
        checkpoint: Some(DiffRefreshCheckpoint {
            active: SourceCursor::new(500, Vec::new()),
            deleted: SourceCursor::new(0, Vec::new()),
        }),
        delta: None,
    };
    IndexStateStore::persist_index(&mut graph, commit).expect("seed checkpoint");

    (manager, session_dir, manifest, db_path)
}

fn build_orchestrator(
    manager: SessionManager,
    session_dir: &Path,
    manifest: SessionManifest,
    source: QueuedChangeSource,
    provider: QueuedProvider,
) -> DiffRefreshOrchestrator {
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &manifest.graph_db_path,
        Some(project_mirror_root(session_dir)),
        RuntimeMode::LongLived,
    )
    .expect("load long-lived runtime");
    DiffRefreshOrchestrator::new(
        manager,
        session_dir.to_path_buf(),
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    )
}

/// 重新打开 durable graphdb 并合并全库 scanner 诊断（与 scan 报告口径一致）
fn durable_scanner_diagnostics(db_path: &Path) -> Result<Vec<Diagnostic>> {
    let graph = GraphDB::open(db_path).expect("open durable graph");
    ProjectIndexer::merge_scanner_diagnostic_entries(
        &graph
            .load_scanner_diagnostic_entries()
            .expect("load durable scanner entries"),
    )
}

fn cleanup(session_dir: &Path) {
    let _ = fs::remove_dir_all(
        session_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default(),
    );
}

/// 同步模式：一轮内 a.spg 由坏修好（unrec 消失）、b.spg 新变坏（dup 出现），
/// durable 与 live runtime 诊断都必须立即反映。
#[test]
fn sync_mode_scanner_diagnostics_follow_fix_and_break() -> Result<()> {
    let (manager, session_dir, manifest, db_path) = setup_session(
        "sync",
        &[
            ("app/page_a.spg", "file-a", "1", spg_with_unrecognized_key()),
            ("app/page_b.spg", "file-b", "1", spg_fixed("b-v1")),
        ],
    );
    // 初始构建后 durable 已有 page_a 的 unrec 计数
    assert_eq!(
        code_hits(&durable_scanner_diagnostics(&db_path)?, CODE_UNRECOGNIZED).len(),
        1
    );

    let provider = QueuedProvider::new();
    provider.push(
        "app/page_a.spg",
        "file-a",
        "2",
        &serde_json::to_string_pretty(&spg_fixed("a-v2"))?,
    );
    provider.push(
        "app/page_b.spg",
        "file-b",
        "2",
        &serde_json::to_string_pretty(&spg_with_duplicate_id())?,
    );
    let source = QueuedChangeSource::new(vec![(
        vec![
            active_event("file-a", "app/page_a.spg", "2", 1000),
            active_event("file-b", "app/page_b.spg", "2", 2000),
        ],
        watermark(
            2000,
            vec!["active:file-a:2".into(), "active:file-b:2".into()],
            0,
        ),
    )]);

    let mut orchestrator = build_orchestrator(manager, &session_dir, manifest, source, provider);
    orchestrator.set_one_shot_mode(true);

    let report = orchestrator.refresh_once().expect("sync refresh");
    assert_eq!(report.persisted, true, "{report:?}");

    // durable：unrec 消失（修复覆盖），dup 出现（新变坏）
    let durable = durable_scanner_diagnostics(&db_path)?;
    assert_eq!(
        code_hits(&durable, CODE_UNRECOGNIZED).len(),
        0,
        "同步持久化后修复文件的 unrec 必须消失: {durable:?}"
    );
    let durable_dup = code_hits(&durable, CODE_DUPLICATE);
    assert_eq!(
        durable_dup.len(),
        1,
        "新变坏文件的 dup 必须出现: {durable:?}"
    );
    assert_eq!(durable_dup[0].count, Some(1), "{durable:?}");

    // live runtime：load_diagnostics 与 status 口径一致
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(
        code_hits(live, CODE_UNRECOGNIZED).len(),
        0,
        "install 后 live unrec 必须消失: {live:?}"
    );
    assert_eq!(
        code_hits(live, CODE_DUPLICATE).len(),
        1,
        "install 后 live dup 必须出现: {live:?}"
    );
    let status = orchestrator.runtime().status();
    assert_eq!(
        code_hits(&status.load_diagnostics, CODE_DUPLICATE).len(),
        1,
        "status 诊断必须反映新值: {:?}",
        status.load_diagnostics
    );

    cleanup(&session_dir);
    Ok(())
}

/// deferred 模式：未达阈值时 durable 保持旧诊断、live 立即反映修复；
/// 轮次回退触发 persist_pending 后 durable 才更新。
#[test]
fn deferred_mode_live_updates_before_persist_and_durable_after() -> Result<()> {
    let (manager, session_dir, manifest, db_path) = setup_session(
        "deferred",
        &[("app/page_a.spg", "file-a", "1", spg_with_unrecognized_key())],
    );

    let provider = QueuedProvider::new();
    provider.push(
        "app/page_a.spg",
        "file-a",
        "2",
        &serde_json::to_string_pretty(&spg_fixed("a-v2"))?,
    );
    let source = QueuedChangeSource::new(vec![(
        vec![active_event("file-a", "app/page_a.spg", "2", 1000)],
        watermark(1000, vec!["active:file-a:2".into()], 0),
    )]);

    let mut orchestrator = build_orchestrator(manager, &session_dir, manifest, source, provider);
    // 阈值不触发；第二轮（空 poll）达到轮次上限时持久化
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: usize::MAX,
        max_pending_rounds: 2,
    });

    let first = orchestrator.refresh_once().expect("first deferred refresh");
    assert_eq!(first.persisted, false, "{first:?}");

    // durable 未写：旧 unrec 仍在
    let durable_before = durable_scanner_diagnostics(&db_path)?;
    assert_eq!(
        code_hits(&durable_before, CODE_UNRECOGNIZED).len(),
        1,
        "deferred 未持久化时 durable 必须保留旧诊断: {durable_before:?}"
    );
    // live 已反映修复（durable entries + pending overlay 口径）
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(
        code_hits(live, CODE_UNRECOGNIZED).len(),
        0,
        "install 后 live 必须立即反映修复: {live:?}"
    );

    let second = orchestrator
        .refresh_once()
        .expect("round fallback persist refresh");
    assert_eq!(second.change_count, 0, "{second:?}");
    assert_eq!(second.persisted, true, "{second:?}");

    // persist_pending 后 durable 反映修复
    let durable_after = durable_scanner_diagnostics(&db_path)?;
    assert_eq!(
        code_hits(&durable_after, CODE_UNRECOGNIZED).len(),
        0,
        "persist 后 durable unrec 必须消失: {durable_after:?}"
    );
    assert_eq!(
        code_hits(&orchestrator.runtime().load_diagnostics, CODE_UNRECOGNIZED).len(),
        0
    );

    cleanup(&session_dir);
    Ok(())
}

/// deferred 跨轮合并：第 1 轮 a 变坏、第 2 轮 b 变坏、第 3 轮 a 修好
/// （同文件后者覆盖前者）、第 4 轮 b 被删除（entry 移除）；第 5 轮空 poll
/// 触发轮次回退持久化。每轮 live 口径与最终 durable 口径都必须正确。
#[test]
fn deferred_mode_cross_round_merge_override_and_delete() -> Result<()> {
    let (manager, session_dir, manifest, db_path) = setup_session(
        "cross-round",
        &[
            ("app/page_a.spg", "file-a", "1", spg_fixed("a-v1")),
            ("app/page_b.spg", "file-b", "1", spg_fixed("b-v1")),
        ],
    );

    let provider = QueuedProvider::new();
    provider.push(
        "app/page_a.spg",
        "file-a",
        "2",
        &serde_json::to_string_pretty(&spg_with_unrecognized_key())?,
    );
    provider.push(
        "app/page_a.spg",
        "file-a",
        "3",
        &serde_json::to_string_pretty(&spg_fixed("a-v3"))?,
    );
    provider.push(
        "app/page_b.spg",
        "file-b",
        "2",
        &serde_json::to_string_pretty(&spg_with_duplicate_id())?,
    );
    let source = QueuedChangeSource::new(vec![
        (
            vec![active_event("file-a", "app/page_a.spg", "2", 1000)],
            watermark(1000, vec!["active:file-a:2".into()], 0),
        ),
        (
            vec![active_event("file-b", "app/page_b.spg", "2", 2000)],
            watermark(2000, vec!["active:file-b:2".into()], 0),
        ),
        (
            vec![active_event("file-a", "app/page_a.spg", "3", 3000)],
            watermark(3000, vec!["active:file-a:3".into()], 0),
        ),
        (
            vec![deleted_event("file-b", "app/page_b.spg", "uuid-b-1", 4000)],
            MetaFilesWatermark {
                active: SourceCursor::new(3000, vec!["active:file-a:3".into()]),
                deleted: SourceCursor::new(4000, vec!["deleted:uuid-b-1".into()]),
            },
        ),
    ]);

    let mut orchestrator = build_orchestrator(manager, &session_dir, manifest, source, provider);
    orchestrator.set_persist_policy(LongLivedPersistPolicy {
        dirty_node_threshold: usize::MAX,
        max_pending_rounds: 5,
    });

    // 第 1 轮：a 变坏 → live 只有 unrec
    let first = orchestrator.refresh_once().expect("round 1");
    assert_eq!(first.persisted, false, "{first:?}");
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(code_hits(live, CODE_UNRECOGNIZED).len(), 1, "{live:?}");
    assert_eq!(code_hits(live, CODE_DUPLICATE).len(), 0, "{live:?}");

    // 第 2 轮：b 变坏 → live 同时有 unrec 与 dup
    let second = orchestrator.refresh_once().expect("round 2");
    assert_eq!(second.persisted, false, "{second:?}");
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(code_hits(live, CODE_UNRECOGNIZED).len(), 1, "{live:?}");
    assert_eq!(code_hits(live, CODE_DUPLICATE).len(), 1, "{live:?}");

    // 第 3 轮：a 修好（后者覆盖前者）→ live 只剩 dup
    let third = orchestrator.refresh_once().expect("round 3");
    assert_eq!(third.persisted, false, "{third:?}");
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(
        code_hits(live, CODE_UNRECOGNIZED).len(),
        0,
        "a 修好后 live unrec 必须消失: {live:?}"
    );
    assert_eq!(code_hits(live, CODE_DUPLICATE).len(), 1, "{live:?}");

    // 第 4 轮：b 被删除 → live 清空
    let fourth = orchestrator.refresh_once().expect("round 4");
    assert_eq!(fourth.persisted, false, "{fourth:?}");
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(code_hits(live, CODE_UNRECOGNIZED).len(), 0, "{live:?}");
    assert_eq!(
        code_hits(live, CODE_DUPLICATE).len(),
        0,
        "b 删除后 live dup 必须消失: {live:?}"
    );

    // 整个 deferred 阶段 durable 不得提前反映任何一轮
    let durable_pending = durable_scanner_diagnostics(&db_path)?;
    assert_eq!(
        code_hits(&durable_pending, CODE_UNRECOGNIZED).len(),
        0,
        "初始文件均为合法，durable 不应有 unrec: {durable_pending:?}"
    );
    assert_eq!(
        code_hits(&durable_pending, CODE_DUPLICATE).len(),
        0,
        "deferred 未持久化时 durable 不应出现 dup: {durable_pending:?}"
    );

    // 第 5 轮：空 poll 达到轮次上限 → persist_pending
    let fifth = orchestrator.refresh_once().expect("round 5 persist");
    assert_eq!(fifth.change_count, 0, "{fifth:?}");
    assert_eq!(fifth.persisted, true, "{fifth:?}");

    // durable 最终口径：a 的 entry 被零计数覆盖、b 的 entry 被删除——无任何 SCANNER_*
    let durable_final = durable_scanner_diagnostics(&db_path)?;
    assert_eq!(
        code_hits(&durable_final, CODE_UNRECOGNIZED).len(),
        0,
        "{durable_final:?}"
    );
    assert_eq!(
        code_hits(&durable_final, CODE_DUPLICATE).len(),
        0,
        "persist 后 durable dup 必须消失: {durable_final:?}"
    );
    let live = &orchestrator.runtime().load_diagnostics;
    assert_eq!(code_hits(live, CODE_UNRECOGNIZED).len(), 0, "{live:?}");
    assert_eq!(code_hits(live, CODE_DUPLICATE).len(), 0, "{live:?}");

    cleanup(&session_dir);
    Ok(())
}
