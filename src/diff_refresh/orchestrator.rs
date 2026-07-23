//! M54 差量刷新编排器
//!
//! `DiffRefreshOrchestrator` 把 Tasks 2-6 的组件串成一轮原子刷新：
//! load checkpoint →（缺失则 `bootstrap`，否则 `poll`）→ mirror →
//! prepare candidate graph → prepare replacement read model →
//! persist graph+checkpoint（单 write transaction）→ install replacement →
//! best-effort batch warm。
//!
//! 生命周期假设：
//! - orchestrator 独占持有 `GraphRuntime`（LongLived）与内存 `SessionManifest`；
//!   manifest 变更后通过 `SessionManager::write_manifest` 落盘。
//! - `source` / `provider` 以 trait object 注入；fixture 与 BI 真源共用同一编排。
//! - 单轮语义为「一次 tick」：退避/定时循环由上层（M55）负责。
//! - 失败语义：mirror/prepare/commit 任一失败直接返回 Err，checkpoint 不推进、
//!   旧 runtime 保持可用；re-warm 是 best-effort，失败只记入 `warm_failures`。

use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};

use super::mirror::apply_changeset_to_mirror;
use super::source::MetaFilesChangeSource;
use super::types::{DiffRefreshCheckpoint, MetaFilesWatermark};
use crate::graph_store::IndexStateStore;
use crate::runtime::{BatchWarmReport, GraphRuntime};
use crate::scanner::indexer::ProjectIndexer;
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
    /// 本轮消费的变更事件数（等于 ChangeSet.change_count）。
    pub change_count: usize,
    /// 本轮失效的页面（旧、新 PageDependencyIndex 并集，已排序）。
    pub invalidated_pages: Vec<String>,
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

/// re-warm 执行器：默认委托 `GraphRuntime::warm_page_logic_batch`，
/// 测试可注入失败以覆盖 warm-failure cold fallback 语义。
type WarmFn = Box<dyn FnMut(&mut GraphRuntime, &[(String, String)]) -> Result<BatchWarmReport>>;

/// 差量刷新编排器（持有方见模块注释的生命周期假设）。
pub struct DiffRefreshOrchestrator {
    session_manager: SessionManager,
    session_dir: PathBuf,
    manifest: SessionManifest,
    graph_db_path: PathBuf,
    source: Box<dyn MetaFilesChangeSource>,
    provider: Box<dyn RemoteSessionProvider>,
    runtime: GraphRuntime,
    warm_fn: WarmFn,
}

impl DiffRefreshOrchestrator {
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
            warm_fn: Box::new(|runtime, targets| runtime.warm_page_logic_batch(targets)),
        }
    }

    /// 注入自定义 re-warm 执行器（测试用）。
    pub fn set_warm_fn(&mut self, warm_fn: WarmFn) {
        self.warm_fn = warm_fn;
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
        let current_checkpoint = self
            .runtime
            .graph
            .load_diff_refresh_checkpoint()
            .context("load diff refresh checkpoint")?;
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

            // 首次 bootstrap 后空结果：持久化 checkpoint-only commit
            // 使下一轮走 poll 而非重新 bootstrap
            let checkpoint = if current_checkpoint.is_none() {
                let bootstrap_checkpoint: DiffRefreshCheckpoint =
                    changeset.next_watermark.clone().into();
                let stage = Instant::now();
                let file_states = self.runtime.graph.load_file_states().unwrap_or_default();
                self.runtime
                    .graph
                    .persist_with_checkpoint(&file_states, Some(&bootstrap_checkpoint))
                    .context("persist bootstrap checkpoint")?;
                timing.commit_ms = stage.elapsed().as_millis();
                Some(bootstrap_checkpoint)
            } else {
                current_checkpoint
            };

            return Ok(DiffRefreshReport {
                change_count: 0,
                invalidated_pages: Vec::new(),
                warm_failures: Vec::new(),
                checkpoint,
                last_poll_at,
                page_dep_index_coverage: coverage,
                timing,
            });
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

        // 6. persist graph+checkpoint（单 write transaction）
        let stage = Instant::now();
        let next_checkpoint: DiffRefreshCheckpoint = changeset.next_watermark.clone().into();
        let mut candidate = prepared.graph;
        let mut commit = prepared.commit;
        commit.checkpoint = Some(next_checkpoint.clone());
        IndexStateStore::persist_index(&mut candidate, commit)
            .context("persist graph and checkpoint commit")?;
        timing.commit_ms = stage.elapsed().as_millis();

        // 7. install replacement（只 move 已准备对象）
        let stage = Instant::now();
        let invalidated_pages = replacement.invalidated_pages.clone();
        let page_dep_index_coverage = replacement.page_dep_index_coverage;
        self.runtime.install_replacement(candidate, replacement);
        timing.swap_ms = stage.elapsed().as_millis();

        // 8. best-effort batch warm：失败只记 warm_failures，不影响已推进的 checkpoint
        let stage = Instant::now();
        let warm_failures = self.rewarm_best_effort(&rewarm_targets);
        timing.rewarm_ms = stage.elapsed().as_millis();

        Ok(DiffRefreshReport {
            change_count: changeset.change_count,
            invalidated_pages,
            warm_failures,
            checkpoint: Some(next_checkpoint),
            last_poll_at,
            page_dep_index_coverage,
            timing,
        })
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
