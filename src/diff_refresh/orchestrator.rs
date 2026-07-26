//! M54 差量刷新编排器
//!
//! `DiffRefreshOrchestrator` 把 Tasks 2-6 的组件串成一轮原子刷新：
//! load checkpoint →（缺失则 `bootstrap`，否则 `poll`）→ mirror →
//! prepare candidate graph → prepare replacement read model →
//! persist（按运行时策略）+ install replacement → best-effort batch warm。
//!
//! 生命周期假设：
//! - orchestrator 独占持有 `GraphRuntime` 与内存 `SessionManifest`；
//!   manifest 变更后通过 `SessionManager::write_manifest` 落盘。
//! - `source` / `provider` 以 trait object 注入；fixture 与 BI 真源共用同一编排。
//! - 单轮语义为「一次 tick」：退避/定时循环由上层（M55）负责。
//! - 失败语义：mirror/prepare/commit 任一失败直接返回 Err，checkpoint 不推进、
//!   旧 runtime 保持可用；re-warm 是 best-effort，失败只记入 `warm_failures`。

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use super::mirror::apply_changeset_to_mirror;
use super::source::MetaFilesChangeSource;
use super::tick::run_tick_loop_with_hooks;
use super::types::{DiffRefreshCheckpoint, MetaFilesWatermark, SourceCursor};
use crate::diff_refresh::DiffRefreshTickReport;
use crate::graph::GraphDB;
use crate::graph_store::{IndexCommit, PersistReport};
use crate::runtime::{BatchWarmReport, GraphRuntime, RuntimeMode};
use crate::scanner::indexer::ProjectIndexer;
use crate::session::RefreshScope;
use crate::session::SessionManager;
use crate::session::manifest::SessionManifest;
use crate::session::remote_provider::RemoteSessionProvider;
use crate::session::sync::project_mirror_root;

/// 一轮 diff refresh 的分阶段耗时（毫秒）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiffRefreshTiming {
    pub poll_or_bootstrap_ms: u128,
    pub mirror_ms: u128,
    pub prepare_ms: u128,
    pub read_model_ms: u128,
    pub commit_ms: u128,
    pub swap_ms: u128,
    pub rewarm_ms: u128,
}

/// 一轮 diff refresh 的结果报告。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiffRefreshReport {
    /// 稳定机器契约版本号。
    pub schema_version: String,
    /// 稳定机器契约类型名。
    pub kind: String,
    /// 本轮刷新执行范围声明；默认回退为 `project/fallback`。
    pub scope: RefreshScope,
    /// 本轮消费的变更事件数（等于 ChangeSet.change_count）。
    pub change_count: usize,
    /// 本轮失效的页面（旧、新 PageDependencyIndex 并集，已排序）。
    pub invalidated_pages: Vec<String>,
    /// 持久化统计，空提交/仅写 checkpoint 时也应返回真实值。
    pub persist_report: Option<PersistReport>,
    /// 本轮是否完成了 durable graph 的持久化提交。
    pub persisted: bool,
    /// 本轮仍待持久化提交的脏节点总数（用于 pending 追踪）。
    pub pending_dirty_total: usize,
    /// best-effort re-warm 失败的页面（来自 BatchWarmReport.pages[*].success，
    /// 不得仅凭外层 Result::Ok 判定全部成功）。
    pub warm_failures: Vec<String>,
    /// 本轮提交后的 checkpoint；空 ChangeSet 时为当前（未推进的）checkpoint。
    pub checkpoint: Option<DiffRefreshCheckpoint>,
    /// 本轮 poll/bootstrap 完成时间（Unix 毫秒）；空 ChangeSet 时只更新该字段。
    pub last_poll_at: u64,
    /// 页面依赖索引覆盖度；`partial` 表示覆盖无法证明、
    /// invalidated_pages 已保守扩大到全部 warm 页面。
    pub page_dep_index_coverage: crate::query::PageDependencyIndexCoverage,
    pub timing: DiffRefreshTiming,
}

/// 长生命周期持久化策略控制配置。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LongLivedPersistPolicy {
    /// 允许累积的脏节点上限，超过后触发持久化或转入待决流程。
    pub dirty_node_threshold: usize,
    /// 最大 pending 轮次，达到上限应触发持久化回退。
    pub max_pending_rounds: usize,
}

/// 差量刷新持久化执行策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffRefreshPersistMode {
    /// 延迟持久化：先 install replacement，按 threshold/round 聚合提交。
    Deferred,
    /// 同步持久化：先 persist checkpoint，成功后 install replacement。
    Synchronous,
}

impl Default for LongLivedPersistPolicy {
    fn default() -> Self {
        Self {
            dirty_node_threshold: read_env_usize("METADATA_CHECKER_PERSIST_DIRTY_THRESHOLD", 100),
            max_pending_rounds: read_env_usize("METADATA_CHECKER_PERSIST_MAX_ROUNDS", 10),
        }
    }
}

/// re-warm 执行器：默认委托 `GraphRuntime::warm_page_logic_batch`，
/// 测试可注入失败以覆盖 warm-failure cold fallback 语义。
type WarmFn = Box<dyn FnMut(&mut GraphRuntime, &[(String, String)]) -> Result<BatchWarmReport>>;
type PersistFn = Box<dyn FnMut(&mut GraphDB, &IndexCommit) -> Result<PersistReport>>;

/// 差量刷新编排器（持有方见模块注释的生命周期假设）。
pub struct DiffRefreshOrchestrator {
    session_manager: SessionManager,
    session_dir: PathBuf,
    manifest: SessionManifest,
    graph_db_path: PathBuf,
    source: Box<dyn MetaFilesChangeSource>,
    provider: Box<dyn RemoteSessionProvider>,
    runtime: GraphRuntime,
    persist_policy: LongLivedPersistPolicy,
    persist_mode: DiffRefreshPersistMode,
    pending_checkpoint: Option<DiffRefreshCheckpoint>,
    pending_dirty_node_ids: Vec<String>,
    pending_commit: Option<IndexCommit>,
    pending_rounds: usize,
    warm_fn: WarmFn,
    persist_fn: PersistFn,
}

impl DiffRefreshOrchestrator {
    fn new_machine_report(
        change_count: usize,
        invalidated_pages: Vec<String>,
        warm_failures: Vec<String>,
        checkpoint: Option<DiffRefreshCheckpoint>,
        last_poll_at: u64,
        page_dep_index_coverage: crate::query::PageDependencyIndexCoverage,
        timing: DiffRefreshTiming,
        persist_report: Option<PersistReport>,
        persisted: bool,
        pending_dirty_total: usize,
    ) -> DiffRefreshReport {
        DiffRefreshReport {
            schema_version: "1.0".to_string(),
            kind: "DiffRefresh".to_string(),
            scope: RefreshScope::default(),
            change_count,
            invalidated_pages,
            persist_report,
            persisted,
            pending_dirty_total,
            warm_failures,
            checkpoint,
            last_poll_at,
            page_dep_index_coverage,
            timing,
        }
    }

    /// 组装编排器；graphdb 路径取自 manifest，调用方需保证
    /// `runtime` 以 LongLived 模式加载同一 graphdb。
    pub fn new(
        session_manager: SessionManager,
        session_dir: PathBuf,
        manifest: SessionManifest,
        source: Box<dyn MetaFilesChangeSource>,
        provider: Box<dyn RemoteSessionProvider>,
        runtime: GraphRuntime,
    ) -> Self {
        let graph_db_path = PathBuf::from(&manifest.graph_db_path);
        Self {
            session_manager,
            session_dir,
            manifest,
            graph_db_path,
            source,
            provider,
            runtime,
            persist_policy: LongLivedPersistPolicy::default(),
            persist_mode: if runtime.runtime_mode == RuntimeMode::OneShot {
                DiffRefreshPersistMode::Synchronous
            } else {
                DiffRefreshPersistMode::Deferred
            },
            pending_checkpoint: None,
            pending_dirty_node_ids: Vec::new(),
            pending_commit: None,
            pending_rounds: 0,
            warm_fn: Box::new(|runtime, targets| runtime.warm_page_logic_batch(targets)),
            persist_fn: Box::new(|graph, commit| graph.persist_commit(commit)),
        }
    }

    /// 注入自定义 re-warm 执行器（测试用）。
    pub fn set_warm_fn(&mut self, warm_fn: WarmFn) {
        self.warm_fn = warm_fn;
    }

    /// 注入自定义持久化执行器（测试可覆盖）。
    pub fn set_persist_fn(&mut self, persist_fn: PersistFn) {
        self.persist_fn = persist_fn;
    }

    /// 显式设置单次命令同一次持久化：true 时先 persist 后 install。
    pub fn set_one_shot_mode(&mut self, one_shot: bool) {
        self.persist_mode = if one_shot {
            DiffRefreshPersistMode::Synchronous
        } else {
            DiffRefreshPersistMode::Deferred
        };
    }

    /// 配置长生命周期持久化策略，便于测试与调用方注入固定参数。
    pub fn set_persist_policy(&mut self, policy: LongLivedPersistPolicy) {
        self.persist_policy = policy;
    }

    /// 读取当前内存 manifest（mirror 后的最新状态）。
    pub fn manifest(&self) -> &SessionManifest {
        &self.manifest
    }

    /// 读取持有的 runtime（测试与上层查询用）。
    pub fn runtime(&self) -> &GraphRuntime {
        &self.runtime
    }

    /// 可变读取持有的 runtime（测试注入 warm 状态用）。
    pub fn runtime_mut(&mut self) -> &mut GraphRuntime {
        &mut self.runtime
    }

    /// 执行一轮差量刷新（一次 tick）。
    pub fn refresh_once(&mut self) -> Result<DiffRefreshReport> {
        let mut timing = DiffRefreshTiming::default();

        // 1. load checkpoint → poll 或 bootstrap
        let stage = Instant::now();
        let durable_checkpoint = self
            .runtime
            .graph
            .load_diff_refresh_checkpoint()
            .context("load diff refresh checkpoint")?;
        let current_checkpoint = self.effective_checkpoint(durable_checkpoint.as_ref());
        let changeset = match &current_checkpoint {
            Some(checkpoint) => {
                let since = MetaFilesWatermark {
                    active: checkpoint.active.clone(),
                    deleted: checkpoint.deleted.clone(),
                };
                self.source
                    .poll(&since)
                    .context("poll meta files changes")?
            }
            None => self
                .source
                .bootstrap(&self.manifest)
                .context("bootstrap meta files changes")?,
        };
        timing.poll_or_bootstrap_ms = stage.elapsed().as_millis();
        let last_poll_at = now_unix_millis();

        // 2. 空 ChangeSet：不写 graph，但首次 bootstrap 需落 checkpoint
        //    以避免下次 tick 重新全量 bootstrap（P2 修复）
        if changeset.change_count == 0 {
            let coverage = self
                .runtime
                .read_model
                .as_ref()
                .map(|model| model.page_dependency_index.coverage())
                .unwrap_or(crate::query::PageDependencyIndexCoverage::Partial);

            if let Some(pending_checkpoint) = self.pending_checkpoint.clone() {
                // 使用 pending watermark 继续轮询，避免重复消费；
                // 空 poll 仅在 pending 已满阈值或轮次回退时触发持久化。
                self.pending_rounds = self.pending_rounds.saturating_add(1);
                if self.should_persist_pending() {
                    let stage = Instant::now();
                    let persist_report = self
                        .persist_pending()
                        .context("persist pending graph and checkpoint")?;
                    timing.commit_ms = stage.elapsed().as_millis();
                    return Ok(Self::new_machine_report(
                        0,
                        Vec::new(),
                        Vec::new(),
                        Some(pending_checkpoint),
                        last_poll_at,
                        coverage,
                        timing,
                        Some(persist_report),
                        true,
                        0,
                    ));
                }
                return Ok(Self::new_machine_report(
                    0,
                    Vec::new(),
                    Vec::new(),
                    Some(pending_checkpoint),
                    last_poll_at,
                    coverage,
                    timing,
                    None,
                    false,
                    self.pending_dirty_node_ids.len(),
                ));
            }

            // 首次 bootstrap 后空结果：持久化 checkpoint-only commit，
            // 使下一轮走 poll 而非重新 bootstrap；已有 checkpoint 的空 poll 不写盘。
            let (checkpoint, persist_report, persisted, pending_dirty_total) =
                if let Some(checkpoint) = current_checkpoint {
                    (Some(checkpoint), None, false, 0)
                } else {
                    let bootstrap_checkpoint: DiffRefreshCheckpoint =
                        changeset.next_watermark.clone().into();
                    let file_states = self.runtime.graph.load_file_states().unwrap_or_default();
                    let stage = Instant::now();
                    let persist_report = self
                        .runtime
                        .graph
                        .persist_with_checkpoint(&file_states, Some(&bootstrap_checkpoint))
                        .context("persist bootstrap checkpoint")?;
                    timing.commit_ms = stage.elapsed().as_millis();
                    (Some(bootstrap_checkpoint), Some(persist_report), false, 0)
                };
            return Ok(Self::new_machine_report(
                0,
                Vec::new(),
                Vec::new(),
                checkpoint,
                last_poll_at,
                coverage,
                timing,
                persist_report,
                persisted,
                pending_dirty_total,
            ));
        }

        // 3. mirror：失败则 checkpoint 不推进、旧 runtime 保持可用
        let stage = Instant::now();
        apply_changeset_to_mirror(
            &self.session_dir,
            &mut self.manifest,
            &changeset,
            self.provider.as_ref(),
        )
        .context("apply changeset to session mirror")?;
        self.session_manager
            .write_manifest(&self.manifest)
            .context("persist session manifest after mirror")?;
        timing.mirror_ms = stage.elapsed().as_millis();

        // 4. prepare candidate graph（不写盘）
        let stage = Instant::now();
        let prepared =
            ProjectIndexer::prepare(&project_mirror_root(&self.session_dir), &self.graph_db_path)
                .context("prepare candidate graph")?;
        timing.prepare_ms = stage.elapsed().as_millis();

        // 5. prepare replacement read model/cache
        let stage = Instant::now();
        // P0 修复：deleted_node_ids 必须参与失效。
        // 被删节点在新图中不存在（新索引查不到），但旧索引仍登记其原所属页，
        // 旧∪新索引并集计算下合并传入才能正确摘除被删页/依赖已删文件的页的 cache。
        let mut invalidation_ids = prepared.dirty_node_ids.clone();
        {
            let mut seen: std::collections::HashSet<String> =
                invalidation_ids.iter().cloned().collect();
            for node_id in &prepared.deleted_node_ids {
                if seen.insert(node_id.clone()) {
                    invalidation_ids.push(node_id.clone());
                }
            }
        }
        let replacement = self
            .runtime
            .prepare_replacement(&prepared.graph, &invalidation_ids)
            .context("prepare replacement read model")?;
        // re-warm 目标：当前 warm cache 中受影响页面的 (page_id, budget) 条目；
        // 只保留仍存在于候选图中的页面——被删页/改名旧路径页不应被重建 cache
        let mut rewarm_targets = self.collect_rewarm_targets(&replacement.invalidated_pages);
        rewarm_targets.retain(|(page_id, _)| {
            matches!(
                crate::graph_store::GraphReadStore::get_node(&prepared.graph, page_id),
                Ok(Some(_))
            )
        });
        timing.read_model_ms = stage.elapsed().as_millis();

        // 6. 按策略持久化（OneShot 先持久化后切换）
        let next_checkpoint: DiffRefreshCheckpoint = changeset.next_watermark.clone().into();
        let mut candidate = prepared.graph;
        let mut commit = prepared.commit;
        commit.checkpoint = Some(next_checkpoint.clone());
        let mut persisted = false;
        let mut persist_report = None;
        let mut pending_dirty_total = 0usize;
        let invalidated_pages = replacement.invalidated_pages.clone();
        let page_dep_index_coverage = replacement.page_dep_index_coverage;

        if self.persist_mode == DiffRefreshPersistMode::Synchronous {
            let mut stage = Instant::now();
            let report = candidate
                .persist_commit(&commit)
                .context("persist graph and checkpoint commit")?;
            timing.commit_ms = stage.elapsed().as_millis();
            persisted = true;
            persist_report = Some(report);
            stage = Instant::now();
            self.runtime.install_replacement(candidate, replacement);
            timing.swap_ms = stage.elapsed().as_millis();
            self.clear_pending_state();
        } else {
            let mut stage = Instant::now();
            self.runtime.install_replacement(candidate, replacement);
            timing.swap_ms = stage.elapsed().as_millis();
            self.merge_pending_state(
                next_checkpoint.clone(),
                prepared.dirty_node_ids,
                prepared.deleted_node_ids,
                commit,
            );
            if self.should_persist_pending() {
                stage = Instant::now();
                let report = self
                    .persist_pending()
                    .context("persist deferred graph and checkpoint")?;
                timing.commit_ms = stage.elapsed().as_millis();
                persisted = true;
                persist_report = Some(report);
            } else {
                pending_dirty_total = self.pending_dirty_node_ids.len();
            }
        }

        // 7. best-effort batch warm：失败只记 warm_failures，不影响已推进的 checkpoint
        let stage = Instant::now();
        let warm_failures = self.rewarm_best_effort(&rewarm_targets);
        timing.rewarm_ms = stage.elapsed().as_millis();

        Ok(Self::new_machine_report(
            changeset.change_count,
            invalidated_pages,
            warm_failures,
            Some(next_checkpoint),
            last_poll_at,
            page_dep_index_coverage,
            timing,
            persist_report,
            persisted,
            pending_dirty_total,
        ))
    }

    /// 计算当前轮次应使用的 watermark：优先 pending checkpoint，若两者都存在取时间更靠后者。
    fn effective_checkpoint(
        &self,
        durable_checkpoint: Option<&DiffRefreshCheckpoint>,
    ) -> Option<DiffRefreshCheckpoint> {
        match (&self.pending_checkpoint, durable_checkpoint) {
            (None, Some(checkpoint)) => Some(checkpoint.clone()),
            (Some(checkpoint), None) => Some(checkpoint.clone()),
            (Some(pending), Some(durable)) => Some(max_checkpoint(pending, durable)),
            (None, None) => None,
        }
    }

    /// 清理 pending 状态。
    fn clear_pending_state(&mut self) {
        self.pending_checkpoint = None;
        self.pending_dirty_node_ids.clear();
        self.pending_commit = None;
        self.pending_rounds = 0;
    }

    /// 合并本轮脏节点并更新 pending 状态为最新 checkpoint/commit。
    fn merge_pending_state(
        &mut self,
        checkpoint: DiffRefreshCheckpoint,
        dirty_node_ids: Vec<String>,
        deleted_node_ids: Vec<String>,
        commit: IndexCommit,
    ) {
        let mut merged_dirty = HashSet::new();
        for id in self.pending_dirty_node_ids.iter() {
            merged_dirty.insert(id.clone());
        }
        for id in dirty_node_ids {
            merged_dirty.insert(id);
        }
        for id in deleted_node_ids {
            merged_dirty.insert(id);
        }
        self.pending_dirty_node_ids = merged_dirty.into_iter().collect();
        self.pending_dirty_node_ids.sort();
        self.pending_rounds = self.pending_rounds.saturating_add(1);
        self.pending_checkpoint = Some(checkpoint);
        self.pending_commit = Some(commit);
    }

    /// 当前 pending 状态是否达到持久化阈值或轮次回退条件。
    fn should_persist_pending(&self) -> bool {
        self.pending_dirty_node_ids.len() > self.persist_policy.dirty_node_threshold
            || self.pending_rounds >= self.persist_policy.max_pending_rounds
    }

    /// 执行 pending 结果的 durable 持久化；成功后清理 pending 状态用于后续轮次。
    fn persist_pending(&mut self) -> Result<PersistReport> {
        let commit = self
            .pending_commit
            .as_ref()
            .context("missing pending commit for persistence")?;
        let report = (self.persist_fn)(&mut self.runtime.graph, commit)
            .context("persist pending graph and checkpoint")?;
        self.clear_pending_state();
        Ok(report)
    }

    /// 使用标准线程 sleep 执行一次限流 Tick 循环。
    pub fn run_tick_loop(&mut self, max_attempts: usize) -> Result<DiffRefreshTickReport> {
        self.run_tick_loop_with_sleep(max_attempts, |delay_secs| {
            std::thread::sleep(Duration::from_secs(delay_secs))
        })
    }

    /// 使用可注入 sleep 的 Tick 循环，便于 TDD 覆盖退避时序。
    pub fn run_tick_loop_with_sleep<F>(
        &mut self,
        max_attempts: usize,
        mut sleep_fn: F,
    ) -> Result<DiffRefreshTickReport>
    where
        F: FnMut(u64),
    {
        run_tick_loop_with_hooks(
            max_attempts,
            || self.refresh_once(),
            |delay_secs| sleep_fn(delay_secs),
        )
    }

    /// 收集 re-warm 目标：当前 warm cache 中受影响页面的 (page_id, budget)。
    fn collect_rewarm_targets(&self, invalidated_pages: &[String]) -> Vec<(String, String)> {
        let Some(read_model) = self.runtime.read_model.as_ref() else {
            return Vec::new();
        };
        let mut targets: Vec<(String, String)> = read_model
            .page_logic_availability
            .keys()
            .filter_map(|key| {
                let mut parts = key.splitn(2, '\n');
                let page_id = parts.next()?;
                let budget = parts.next()?;
                if invalidated_pages.iter().any(|page| page == page_id) {
                    Some((page_id.to_string(), budget.to_string()))
                } else {
                    None
                }
            })
            .collect();
        targets.sort();
        targets
    }

    /// best-effort re-warm：逐页检查 BatchWarmReport.pages[*].success；
    /// 外层 Err 视为整批失败，均只记入 warm_failures。
    fn rewarm_best_effort(&mut self, targets: &[(String, String)]) -> Vec<String> {
        if targets.is_empty() {
            return Vec::new();
        }
        match (self.warm_fn)(&mut self.runtime, targets) {
            Ok(report) => {
                let mut failures: Vec<String> = report
                    .pages
                    .iter()
                    .filter(|page| !page.success)
                    .map(|page| page.page_id.clone())
                    .collect();
                failures.sort();
                failures
            }
            Err(_) => {
                let mut failures: Vec<String> =
                    targets.iter().map(|(page_id, _)| page_id.clone()).collect();
                failures.sort();
                failures
            }
        }
    }
}

/// 当前 Unix 毫秒时间。
fn now_unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 读取环境变量中的 usize 配置，读取失败时使用回退值。
fn read_env_usize(env_key: &str, fallback: usize) -> usize {
    std::env::var(env_key)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(fallback)
}

fn max_source_cursor(left: &SourceCursor, right: &SourceCursor) -> SourceCursor {
    if left.updated_at_ms > right.updated_at_ms {
        return left.clone();
    }
    if left.updated_at_ms < right.updated_at_ms {
        return right.clone();
    }

    let mut boundary_event_ids = left
        .boundary_event_ids
        .iter()
        .chain(right.boundary_event_ids.iter())
        .cloned()
        .collect::<Vec<_>>();
    boundary_event_ids.sort();
    boundary_event_ids.dedup();
    SourceCursor::new(left.updated_at_ms, boundary_event_ids)
}

fn max_checkpoint(
    left: &DiffRefreshCheckpoint,
    right: &DiffRefreshCheckpoint,
) -> DiffRefreshCheckpoint {
    DiffRefreshCheckpoint {
        active: max_source_cursor(&left.active, &right.active),
        deleted: max_source_cursor(&left.deleted, &right.deleted),
    }
}
