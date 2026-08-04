use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

#[cfg(feature = "cli-local")]
std::thread_local! {
    static PAGE_LOGIC_CACHE_WARM_STRUCTURAL_WRITES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PAGE_LOGIC_CACHE_MAP_CLONE_ELEMENTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

use crate::dense_graph::DenseGraphSnapshot;
use crate::graph::GraphDB;
use crate::response_processor::ResponseProcessor;
pub use crate::response_processor::{RuntimeQueryResponse, RuntimeTiming};

/// Runtime 使用模式，用于区分一次性 CLI 查询和长生命周期服务。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeMode {
    /// 一次性查询路径，避免把初始化成本强行前移。
    OneShot,
    /// 长生命周期路径，允许在初始化阶段构建可复用读模型。
    LongLived,
}

/// 单页 page logic warm 结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchWarmPageResult {
    /// 页面节点 ID。
    pub page_id: String,
    /// 输出预算。
    pub budget: String,
    /// 是否成功写入 warm cache。
    pub success: bool,
    /// 单页 warm 耗时（毫秒）。
    pub warm_ms: u128,
    /// 失败时的错误信息。
    pub error: Option<String>,
}

/// 批量 page logic warm 报告。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BatchWarmReport {
    /// 各页 warm 结果。
    pub pages: Vec<BatchWarmPageResult>,
    /// 整批总耗时（毫秒）。
    pub total_ms: u128,
    /// 本次批量 warm 的结构性写入次数（应为 1）。
    pub structural_writes: usize,
}

/// Runtime 初始化阶段构建的只读派生模型。
pub struct RuntimeReadModel {
    /// 稠密 ID + CSR 风格只读图快照；build 失败时为 `None`。
    pub dense_graph: Option<Arc<DenseGraphSnapshot>>,
    /// M53：load-time 物化 availability condition facts。
    pub availability_facts: Arc<crate::query::MaterializedAvailabilityFactsIndex>,
    /// 预热后的页面 availability 缓存。
    pub page_logic_availability: HashMap<String, crate::query::PageLogicAvailabilityCache>,
    /// M53：page logic 节点到页面的反向依赖索引。
    pub page_dependency_index: Arc<crate::query::PageDependencyIndex>,
}

impl RuntimeReadModel {
    /// 构造 page availability 缓存 key。
    fn page_logic_availability_key(page_id: &str, budget: &str) -> String {
        format!("{page_id}\n{budget}")
    }

    /// 读取预热后的 page availability 缓存。
    pub fn page_logic_availability(
        &self,
        page_id: &str,
        budget: &str,
    ) -> Option<&crate::query::PageLogicAvailabilityCache> {
        self.page_logic_availability
            .get(&Self::page_logic_availability_key(page_id, budget))
    }

    /// 判断是否已经预热指定 page/budget。
    pub fn has_page_logic_availability(&self, page_id: &str, budget: &str) -> bool {
        self.page_logic_availability(page_id, budget).is_some()
    }
}

/// 运行时派生模型更新模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadModelUpdateMode {
    /// 三个关键索引均走增量更新路径。
    Incremental,
    /// 至少一个索引走增量、至少一个索引走回退全量。
    Mixed,
    /// 三个关键索引全部回退全量重建。
    Full,
}

/// M54：replacement read model — 基于候选图预先构建的只读派生模型。
///
/// `prepare_replacement` 的产物；`install_replacement` 只 move 已准备对象。
pub struct PreparedRuntimeReadModel {
    /// 候选 read model：dense graph / availability facts / PageDependencyIndex
    /// 已基于候选图重建，warm cache 只保留未受影响页面
    pub read_model: RuntimeReadModel,
    /// 候选稠密快照（与 read_model.dense_graph 同源）
    pub dense_snapshot: Option<Arc<DenseGraphSnapshot>>,
    /// 是否启用稠密快照
    pub dense_snapshot_enabled: bool,
    /// 稠密快照构建耗时（毫秒）
    pub dense_snapshot_build_ms: u128,
    /// availability facts 构建耗时（毫秒）
    pub availability_facts_build_ms: u128,
    /// PageDependencyIndex 构建耗时（毫秒）
    pub page_dependency_index_build_ms: u128,
    /// read model 构建总耗时（毫秒）
    pub read_model_build_ms: u128,
    /// 当前候选 read model 的更新策略。
    pub read_model_update_mode: ReadModelUpdateMode,
    /// 旧、新 PageDependencyIndex 计算的受影响页面并集（已排序）
    pub invalidated_pages: Vec<String>,
    /// 页面依赖索引覆盖度：旧、新索引任一为 Partial 即 Partial
    ///（覆盖无法证明时 invalidated_pages 已保守扩大到全部 warm 页面）
    pub page_dep_index_coverage: crate::query::PageDependencyIndexCoverage,
}

/// Hot Graph Runtime：在同一进程内复用已加载的 GraphDB
///
/// M23 目标：把"加载图"和"执行查询"从 CLI 分支中解耦，
/// 证明同一个 runtime 连续执行多次查询时，第二次不再全量加载 graphdb。
pub struct GraphRuntime {
    /// 内存中的图数据库（全量加载）
    pub graph: GraphDB,
    /// graphdb 文件路径
    pub graph_db_path: PathBuf,
    /// 项目目录路径（从 graph_db_path 推导）
    pub project_dir: Option<PathBuf>,
    /// 加载时间戳
    pub loaded_at: SystemTime,
    /// graphdb 文件 mtime（用于后续增量检测）
    pub graph_file_mtime: Option<SystemTime>,
    /// graphdb 文件大小
    pub graph_file_size: u64,
    /// 累计加载次数（热查询应保持为 1）
    pub load_count: usize,
    /// 首次 graph 加载耗时（毫秒）
    pub graph_load_ms: u128,
    /// 累计 reload 次数
    pub reload_count: usize,
    /// 上次 reload 错误信息
    pub last_reload_error: Option<String>,
    /// graphdb 文件指纹
    pub graph_fingerprint: GraphFingerprint,
    /// 运行期预构建的稠密只读快照。
    pub dense_snapshot: Option<Arc<DenseGraphSnapshot>>,
    /// 稠密快照构建耗时（毫秒）。
    pub dense_snapshot_build_ms: u128,
    /// availability facts 物化索引构建耗时（毫秒）。
    pub availability_facts_build_ms: u128,
    /// page 反向依赖索引构建耗时（毫秒）。
    pub page_dependency_index_build_ms: u128,
    /// 是否在 runtime load/reload 阶段构建稠密快照。
    pub dense_snapshot_enabled: bool,
    /// Runtime 模式。
    pub runtime_mode: RuntimeMode,
    /// 长生命周期 runtime 的派生只读模型。
    pub read_model: Option<Arc<RuntimeReadModel>>,
    /// 派生只读模型构建耗时（毫秒）。
    pub read_model_build_ms: u128,
}

/// Runtime 查询命令枚举
///
/// M38 统一为 ToolCommand，消除 CLI / stdio / MCP 分叉。
pub type RuntimeQueryCommand = crate::tool_contract::ToolCommand;

/// Runtime 查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeQueryRequest {
    pub command: RuntimeQueryCommand,
    pub target: String,
    pub budget: String,
    pub human: bool,
    #[serde(default)]
    pub intent: Option<String>,
    pub page_scope: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub check_reload: bool,
}

/// GraphDB 文件指纹，用于变更检测
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphFingerprint {
    /// graphdb 文件路径
    pub path: PathBuf,
    /// 文件修改时间
    pub mtime: Option<SystemTime>,
    /// 文件大小
    pub size: u64,
    /// 文件前 4096 字节的内容 hash
    pub content_prefix_hash: u64,
}

/// 计算文件前 prefix_len 字节的内容 hash
fn compute_prefix_hash(path: &std::path::Path, prefix_len: usize) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    if let Ok(data) = std::fs::read(path) {
        data.iter()
            .take(prefix_len)
            .for_each(|b| b.hash(&mut hasher));
    }
    hasher.finish()
}

/// reload_if_changed 结果枚举
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReloadResult {
    /// 文件未变更，未执行 reload
    Unchanged,
    /// 文件已变更，reload 成功
    Reloaded,
    /// 文件已变更，reload 失败
    ReloadFailed { error: String },
}

/// Runtime 状态快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// graphdb 路径
    pub graph_db_path: PathBuf,
    /// 加载时间戳（Unix 时间戳秒数）
    pub loaded_at: u64,
    /// 首次加载次数
    pub load_count: usize,
    /// 累计 reload 次数
    pub reload_count: usize,
    /// 图中节点数量
    pub node_count: usize,
    /// 图中边数量
    pub edge_count: usize,
    /// 文件修改时间
    pub graph_file_mtime: Option<SystemTime>,
    /// 文件大小
    pub graph_file_size: u64,
    /// 上次 reload 错误
    pub last_reload_error: Option<String>,
    /// Runtime 模式（stdio 产品路径应为 LongLived）。
    pub runtime_mode: RuntimeMode,
    /// 是否已构建 LongLived 派生读模型。
    pub read_model_ready: bool,
}

impl GraphRuntime {
    /// 加载 graphdb 并构建 Runtime
    ///
    /// 记录文件 metadata，初始化 load_count = 1。
    /// 当显式提供 `project_dir` 时优先使用，否则从 `graph_db_path.parent()` 推导。
    pub fn load_with_project_dir(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        Self::load_with_project_dir_and_mode(graph_db_path, project_dir, RuntimeMode::OneShot)
    }

    /// 加载 graphdb 并显式构建稠密只读快照。
    pub fn load_with_project_dir_and_dense_snapshot(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        Self::load_with_project_dir_and_mode(graph_db_path, project_dir, RuntimeMode::LongLived)
    }

    /// 按指定 Runtime 模式加载 graphdb。
    pub fn load_with_project_dir_and_mode(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
        runtime_mode: RuntimeMode,
    ) -> Result<Self> {
        Self::load_with_project_dir_internal(graph_db_path, project_dir, runtime_mode)
    }

    fn load_with_project_dir_internal(
        graph_db_path: impl AsRef<Path>,
        project_dir: Option<impl AsRef<Path>>,
        runtime_mode: RuntimeMode,
    ) -> Result<Self> {
        let path = graph_db_path.as_ref().to_path_buf();
        #[cfg(feature = "telemetry")]
        let graph_load_span = crate::telemetry::graph_load_span(&path.to_string_lossy());
        #[cfg(feature = "telemetry")]
        let _graph_load_guard = graph_load_span.enter();

        let start = Instant::now();
        let graph = GraphDB::open_or_diagnostic(&path).map_err(|e| {
            anyhow::anyhow!(
                "GraphDB open failed: {}",
                serde_json::to_string(&e).unwrap_or_default()
            )
        })?;
        let graph_load_ms = start.elapsed().as_millis();

        let (graph_file_mtime, graph_file_size) = std::fs::metadata(&path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));

        let mut diagnostics = Vec::new();
        diagnostics.push(format!("Graph loaded in {} ms", graph_load_ms));

        let prefix_hash = compute_prefix_hash(&path, 4096);
        let fingerprint = GraphFingerprint {
            path: path.clone(),
            mtime: graph_file_mtime,
            size: graph_file_size,
            content_prefix_hash: prefix_hash,
        };
        let build_read_model = runtime_mode == RuntimeMode::LongLived;
        let mut dense_snapshot_build_ms = 0_u128;
        let mut availability_facts_build_ms = 0_u128;
        let mut page_dependency_index_build_ms = 0_u128;
        let read_model = if build_read_model {
            let dense_started = Instant::now();
            let dense_graph = DenseGraphSnapshot::from_graph(&graph).ok().map(Arc::new);
            dense_snapshot_build_ms = dense_started.elapsed().as_millis();
            if dense_graph.is_none() {
                diagnostics.push(
                    "DenseGraphSnapshot build failed; long-lived dense path disabled".to_string(),
                );
            }
            let facts_started = Instant::now();
            let availability_facts = match crate::query::MaterializedAvailabilityFactsIndex::build(
                &graph,
            ) {
                Ok(index) => Arc::new(index),
                Err(error) => {
                    diagnostics.push(format!(
                        "MaterializedAvailabilityFactsIndex build failed: {error}; using empty index"
                    ));
                    Arc::new(crate::query::MaterializedAvailabilityFactsIndex::empty())
                }
            };
            availability_facts_build_ms = facts_started.elapsed().as_millis();
            if dense_graph.is_some() || availability_facts.node_count > 0 {
                let page_dep_started = Instant::now();
                let page_dependency_index = match crate::query::PageDependencyIndex::build(&graph) {
                    Ok(index) => Arc::new(index),
                    Err(error) => {
                        diagnostics.push(format!(
                            "PageDependencyIndex build failed: {error}; using empty index"
                        ));
                        Arc::new(crate::query::PageDependencyIndex::empty())
                    }
                };
                page_dependency_index_build_ms = page_dep_started.elapsed().as_millis();
                Some(Arc::new(RuntimeReadModel {
                    dense_graph,
                    availability_facts,
                    page_logic_availability: HashMap::new(),
                    page_dependency_index,
                }))
            } else {
                diagnostics.push(
                    "Long-lived read model skipped: dense snapshot and materialized index both unavailable"
                        .to_string(),
                );
                None
            }
        } else {
            None
        };
        let read_model_build_ms = if build_read_model {
            dense_snapshot_build_ms + availability_facts_build_ms + page_dependency_index_build_ms
        } else {
            0
        };
        let dense_snapshot = read_model
            .as_ref()
            .and_then(|model| model.dense_graph.as_ref().map(Arc::clone));
        let dense_snapshot_enabled = dense_snapshot.is_some();

        let project_dir = project_dir
            .map(|p| p.as_ref().to_path_buf())
            .or_else(|| path.parent().map(|p| p.to_path_buf()));

        #[cfg(feature = "telemetry")]
        crate::telemetry::record_graph_load(
            &graph_load_span,
            graph_load_ms,
            graph.graph.node_count(),
            graph.graph.edge_count(),
        );

        Ok(GraphRuntime {
            graph,
            graph_db_path: path,
            project_dir,
            loaded_at: SystemTime::now(),
            graph_file_mtime,
            graph_file_size,
            load_count: 1,
            graph_load_ms,
            reload_count: 0,
            last_reload_error: None,
            graph_fingerprint: fingerprint,
            dense_snapshot,
            dense_snapshot_build_ms,
            availability_facts_build_ms,
            page_dependency_index_build_ms,
            dense_snapshot_enabled,
            runtime_mode,
            read_model,
            read_model_build_ms,
        })
    }

    /// 兼容旧签名：从 graph_db_path.parent() 推导 project_dir
    pub fn load(graph_db_path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_project_dir(graph_db_path, None::<&Path>)
    }

    /// 在长生命周期 runtime 的初始化阶段预热 page logic availability。
    pub fn warm_page_logic_availability(&mut self, page_id: &str, budget: &str) -> Result<u128> {
        let started_at = Instant::now();
        let cache = self.build_page_logic_cache_entry(page_id, budget)?;
        self.insert_page_logic_cache_entry(page_id, budget, cache)?;
        Ok(started_at.elapsed().as_millis())
    }

    /// 批量预热 page logic availability，整批只做一次结构性写入。
    pub fn warm_page_logic_batch(
        &mut self,
        targets: &[(String, String)],
    ) -> Result<BatchWarmReport> {
        let started_at = Instant::now();
        let mut pages = Vec::with_capacity(targets.len());
        let mut pending: Vec<(String, crate::query::PageLogicAvailabilityCache)> =
            Vec::with_capacity(targets.len());

        for (page_id, budget) in targets {
            let page_started = Instant::now();
            match self.build_page_logic_cache_entry(page_id, budget) {
                Ok(cache) => {
                    pending.push((
                        RuntimeReadModel::page_logic_availability_key(page_id, budget),
                        cache,
                    ));
                    pages.push(BatchWarmPageResult {
                        page_id: page_id.clone(),
                        budget: budget.clone(),
                        success: true,
                        warm_ms: page_started.elapsed().as_millis(),
                        error: None,
                    });
                }
                Err(error) => {
                    pages.push(BatchWarmPageResult {
                        page_id: page_id.clone(),
                        budget: budget.clone(),
                        success: false,
                        warm_ms: page_started.elapsed().as_millis(),
                        error: Some(error.to_string()),
                    });
                }
            }
        }

        let structural_writes = if pending.is_empty() {
            0
        } else {
            self.insert_page_logic_cache_entries(pending)?;
            1
        };

        Ok(BatchWarmReport {
            pages,
            total_ms: started_at.elapsed().as_millis(),
            structural_writes,
        })
    }

    /// 根据 dirty node 列表摘除 warm cache 中受影响的页面条目。
    ///
    /// 只摘除缓存，不自动重新 warm，也不判断是否应该触发 reload。
    pub fn invalidate_pages_for_dirty_nodes(
        &mut self,
        dirty_node_ids: &[String],
    ) -> Result<Vec<String>> {
        let Some(read_model) = self.read_model.as_mut() else {
            return Err(anyhow::anyhow!(
                "page logic cache invalidation requires long-lived runtime read model"
            ));
        };
        let affected = read_model
            .page_dependency_index
            .affected_pages(dirty_node_ids);
        if affected.is_empty() {
            return Ok(Vec::new());
        }

        let model = Self::unique_read_model_mut(read_model)?;
        let mut removed_pages = Vec::new();
        model.page_logic_availability.retain(|key, _| {
            let page_id = key.split('\n').next().unwrap_or(key.as_str());
            if affected.contains(page_id) {
                if !removed_pages.iter().any(|existing| existing == page_id) {
                    removed_pages.push(page_id.to_string());
                }
                false
            } else {
                true
            }
        });
        removed_pages.sort();
        Ok(removed_pages)
    }

    /// M54：基于候选图预构建 replacement read model（不改动当前 runtime）。
    ///
    /// `dirty_ids` 由调用方传入 dirty ∪ deleted 节点集合。稳定 node set 时
    /// 增量更新 dense graph、availability facts、PageDependencyIndex；
    /// 新增/删除节点时统一回退 full rebuild。
    /// 受影响页面取旧、新索引计算结果的并集；warm cache 只复制未受影响页面。
    pub fn prepare_replacement(
        &self,
        candidate: &GraphDB,
        dirty_ids: &[String],
    ) -> Result<PreparedRuntimeReadModel> {
        let Some(current_model) = self.read_model.as_ref() else {
            return Err(anyhow::anyhow!(
                "prepare replacement requires long-lived runtime read model"
            ));
        };

        let mut dense_snapshot_update_by_incremental = false;
        let mut availability_update_by_incremental = false;
        let mut page_dep_update_by_incremental = false;

        let dense_started = Instant::now();
        let dense_graph = if let Some(current_dense_graph) = current_model.dense_graph.as_ref() {
            match current_dense_graph.try_update_incremental(candidate, dirty_ids) {
                Ok(Some(next_dense_graph)) => {
                    dense_snapshot_update_by_incremental = true;
                    Some(Arc::new(next_dense_graph))
                }
                Ok(None) | Err(_) => DenseGraphSnapshot::from_graph(candidate).ok().map(Arc::new),
            }
        } else {
            DenseGraphSnapshot::from_graph(candidate).ok().map(Arc::new)
        };
        let dense_snapshot_build_ms = dense_started.elapsed().as_millis();
        // Dense snapshot 的 node-set 校验是三个派生索引共享的安全闸门：
        // 新增/删除节点时所有索引都必须走 full fallback，避免 Facts/PageDep
        // 在缺少完整节点目录的情况下误报 incremental。
        let stable_node_set = dense_snapshot_update_by_incremental;

        let facts_started = Instant::now();
        let availability_facts = if stable_node_set {
            match current_model
                .availability_facts
                .try_update_incremental(candidate, dirty_ids)
            {
                Ok(Some(next_availability_facts)) => {
                    availability_update_by_incremental = true;
                    Arc::new(next_availability_facts)
                }
                Ok(None) | Err(_) => {
                    match crate::query::MaterializedAvailabilityFactsIndex::build(candidate) {
                        Ok(index) => Arc::new(index),
                        Err(_) => {
                            Arc::new(crate::query::MaterializedAvailabilityFactsIndex::empty())
                        }
                    }
                }
            }
        } else {
            match crate::query::MaterializedAvailabilityFactsIndex::build(candidate) {
                Ok(index) => Arc::new(index),
                Err(_) => Arc::new(crate::query::MaterializedAvailabilityFactsIndex::empty()),
            }
        };
        let availability_facts_build_ms = facts_started.elapsed().as_millis();

        let page_dep_started = Instant::now();
        let page_dependency_index = if stable_node_set {
            match current_model
                .page_dependency_index
                .try_update_incremental(candidate, dirty_ids)
            {
                Ok(Some(next_page_dep_index)) => {
                    page_dep_update_by_incremental = true;
                    Arc::new(next_page_dep_index)
                }
                Ok(None) | Err(_) => match crate::query::PageDependencyIndex::build(candidate) {
                    Ok(index) => Arc::new(index),
                    Err(_) => Arc::new(crate::query::PageDependencyIndex::empty()),
                },
            }
        } else {
            match crate::query::PageDependencyIndex::build(candidate) {
                Ok(index) => Arc::new(index),
                Err(_) => Arc::new(crate::query::PageDependencyIndex::empty()),
            }
        };
        let page_dependency_index_build_ms = page_dep_started.elapsed().as_millis();

        let read_model_update_mode = match (
            dense_snapshot_update_by_incremental,
            availability_update_by_incremental,
            page_dep_update_by_incremental,
        ) {
            (true, true, true) => ReadModelUpdateMode::Incremental,
            (false, false, false) => ReadModelUpdateMode::Full,
            _ => ReadModelUpdateMode::Mixed,
        };

        // 旧、新索引计算的受影响页面取并集，避免索引差异漏失效
        let mut affected = current_model
            .page_dependency_index
            .affected_pages(dirty_ids);
        affected.extend(page_dependency_index.affected_pages(dirty_ids));

        // M55：旧、新索引任一覆盖度为 Partial 时无法证明未受影响页面，
        // 保守扩大 invalidation 到当前 warm cache 的全部页面
        let page_dep_index_coverage = match (
            current_model.page_dependency_index.coverage(),
            page_dependency_index.coverage(),
        ) {
            (
                crate::query::PageDependencyIndexCoverage::Full,
                crate::query::PageDependencyIndexCoverage::Full,
            ) => crate::query::PageDependencyIndexCoverage::Full,
            _ => {
                for key in current_model.page_logic_availability.keys() {
                    let page_id = key.split('\n').next().unwrap_or(key.as_str());
                    affected.insert(page_id.to_string());
                }
                crate::query::PageDependencyIndexCoverage::Partial
            }
        };

        // warm cache 只复制未受影响页面
        let page_logic_availability = current_model
            .page_logic_availability
            .iter()
            .filter(|(key, _)| {
                let page_id = key.split('\n').next().unwrap_or(key.as_str());
                !affected.contains(page_id)
            })
            .map(|(key, cache)| (key.clone(), cache.clone()))
            .collect();

        let mut invalidated_pages: Vec<String> = affected.into_iter().collect();
        invalidated_pages.sort();
        let dense_snapshot_enabled = dense_graph.is_some();
        let read_model_build_ms =
            dense_snapshot_build_ms + availability_facts_build_ms + page_dependency_index_build_ms;
        Ok(PreparedRuntimeReadModel {
            read_model: RuntimeReadModel {
                dense_graph: dense_graph.clone(),
                availability_facts,
                page_logic_availability,
                page_dependency_index,
            },
            dense_snapshot: dense_graph,
            dense_snapshot_enabled,
            dense_snapshot_build_ms,
            availability_facts_build_ms,
            page_dependency_index_build_ms,
            read_model_build_ms,
            read_model_update_mode,
            invalidated_pages,
            page_dep_index_coverage,
        })
    }

    /// M54：安装候选图与已准备的 read model；只 move 已准备对象，不返回 Result。
    ///
    /// 调用方必须先完成 graph+checkpoint 原子提交再调用本方法；
    /// 本方法不做任何可能失败的图构建，只更新运行时持有对象与文件指纹。
    pub fn install_replacement(&mut self, candidate: GraphDB, prepared: PreparedRuntimeReadModel) {
        self.graph = candidate;
        self.loaded_at = SystemTime::now();
        let (graph_file_mtime, graph_file_size) = std::fs::metadata(&self.graph_db_path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));
        self.graph_file_mtime = graph_file_mtime;
        self.graph_file_size = graph_file_size;
        if let Ok(fingerprint) = self.current_fingerprint() {
            self.graph_fingerprint = fingerprint;
        }
        self.read_model = Some(Arc::new(prepared.read_model));
        self.dense_snapshot = prepared.dense_snapshot;
        self.dense_snapshot_enabled = prepared.dense_snapshot_enabled;
        self.dense_snapshot_build_ms = prepared.dense_snapshot_build_ms;
        self.availability_facts_build_ms = prepared.availability_facts_build_ms;
        self.page_dependency_index_build_ms = prepared.page_dependency_index_build_ms;
        self.read_model_build_ms = prepared.read_model_build_ms;
        self.reload_count += 1;
        self.last_reload_error = None;
    }

    /// 构建单条 page logic warm cache 条目（不写回 read model）。
    fn build_page_logic_cache_entry(
        &self,
        page_id: &str,
        budget: &str,
    ) -> Result<crate::query::PageLogicAvailabilityCache> {
        let Some(read_model) = self.read_model.as_ref() else {
            return Err(anyhow::anyhow!(
                "page logic availability warm requires long-lived runtime read model"
            ));
        };
        crate::query::build_page_logic_availability_cache(
            &self.graph,
            read_model.dense_graph.as_deref(),
            page_id,
            self.project_dir.as_deref(),
            budget,
            Some(read_model.availability_facts.as_ref()),
        )
    }

    /// 写入单条 page logic warm cache（原地更新，避免深拷贝整个 map）。
    fn insert_page_logic_cache_entry(
        &mut self,
        page_id: &str,
        budget: &str,
        cache: crate::query::PageLogicAvailabilityCache,
    ) -> Result<()> {
        let key = RuntimeReadModel::page_logic_availability_key(page_id, budget);
        self.insert_page_logic_cache_entries(vec![(key, cache)])
    }

    /// 批量写入 page logic warm cache（一次结构性写入）。
    fn insert_page_logic_cache_entries(
        &mut self,
        entries: Vec<(String, crate::query::PageLogicAvailabilityCache)>,
    ) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let Some(read_model) = self.read_model.as_mut() else {
            return Err(anyhow::anyhow!(
                "page logic availability warm requires long-lived runtime read model"
            ));
        };
        let model = Self::unique_read_model_mut(read_model)?;
        for (key, cache) in entries {
            model.page_logic_availability.insert(key, cache);
        }
        record_page_logic_cache_structural_write();
        Ok(())
    }

    /// 获取唯一持有的 `RuntimeReadModel` 可变引用；若存在额外 Arc 持有者则仅克隆 warm cache map。
    fn unique_read_model_mut(
        read_model: &mut Arc<RuntimeReadModel>,
    ) -> Result<&mut RuntimeReadModel> {
        if Arc::strong_count(read_model) == 1 {
            return Ok(Arc::get_mut(read_model)
                .expect("unique Arc<RuntimeReadModel> must allow in-place mutation"));
        }
        let snapshot = Arc::clone(read_model);
        let page_logic_availability = snapshot.page_logic_availability.clone();
        #[cfg(feature = "cli-local")]
        PAGE_LOGIC_CACHE_MAP_CLONE_ELEMENTS.with(|counter| {
            counter.set(counter.get() + page_logic_availability.len());
        });
        *read_model = Arc::new(RuntimeReadModel {
            dense_graph: snapshot.dense_graph.clone(),
            availability_facts: Arc::clone(&snapshot.availability_facts),
            page_dependency_index: Arc::clone(&snapshot.page_dependency_index),
            page_logic_availability,
        });
        Ok(Arc::get_mut(read_model).expect("fresh Arc must be uniquely owned"))
    }

    /// 执行查询，复用内存中的 graph
    ///
    /// M38 统一入口：支持所有已加载 graphdb 后的运行期工具命令。
    pub fn query(&mut self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse> {
        use crate::tool_contract::ToolCommand;
        let total_start = Instant::now();
        let mut diagnostics = Vec::new();
        #[cfg(feature = "telemetry")]
        let command_label = format!("{:?}", request.command);
        #[cfg(feature = "telemetry")]
        let query_span = crate::telemetry::runtime_query_span(
            &command_label,
            &request.target,
            &request.budget,
            request.intent.as_deref(),
            request.human,
            self.graph.graph.node_count(),
            self.graph.graph.edge_count(),
        );
        #[cfg(feature = "telemetry")]
        let _query_guard = query_span.enter();

        // ReloadGraph / CheckReload 需要可变借用 self，在 match 之前单独处理
        if request.command == ToolCommand::ReloadGraph {
            let result = match self.reload() {
                Ok(()) => {
                    diagnostics.push("GRAPH_RELOADED".to_string());
                    serde_json::to_value(self.status())?
                }
                Err(e) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "ok": false,
                        "error": format!("{}", e),
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
            };
            let query_compute_ms = total_start.elapsed().as_millis();
            diagnostics.push(format!(
                "Query compute: {} ms before response processing",
                query_compute_ms
            ));
            let response = ResponseProcessor::runtime_response(
                result,
                diagnostics,
                0,
                query_compute_ms,
                total_start,
            )?;
            #[cfg(feature = "telemetry")]
            crate::telemetry::record_runtime_response(
                &query_span,
                &command_label,
                &request.budget,
                &response.timing,
            );
            return Ok(response);
        }

        if request.command == ToolCommand::CheckReload {
            let result = match self.reload_if_changed() {
                Ok(crate::runtime::ReloadResult::Reloaded) => {
                    diagnostics.push("GRAPH_RELOADED".to_string());
                    serde_json::json!({
                        "reloaded": true,
                        "status": self.status(),
                        "diagnostics": vec!["GRAPH_RELOADED"],
                    })
                }
                Ok(crate::runtime::ReloadResult::Unchanged) => {
                    diagnostics.push("GRAPH_UNCHANGED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "status": self.status(),
                        "diagnostics": vec!["GRAPH_UNCHANGED"],
                    })
                }
                Ok(crate::runtime::ReloadResult::ReloadFailed { error }) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "error": error,
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
                Err(e) => {
                    diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                    serde_json::json!({
                        "reloaded": false,
                        "error": format!("{}", e),
                        "diagnostics": vec!["GRAPH_RELOAD_FAILED"],
                    })
                }
            };
            let query_compute_ms = total_start.elapsed().as_millis();
            diagnostics.push(format!(
                "Query compute: {} ms before response processing",
                query_compute_ms
            ));
            let response = ResponseProcessor::runtime_response(
                result,
                diagnostics,
                0,
                query_compute_ms,
                total_start,
            )?;
            #[cfg(feature = "telemetry")]
            crate::telemetry::record_runtime_response(
                &query_span,
                &command_label,
                &request.budget,
                &response.timing,
            );
            return Ok(response);
        }

        let query_start = Instant::now();
        #[cfg(feature = "telemetry")]
        let query_compute_span = tracing::info_span!(
            "metadata_checker.stage.query_compute",
            "metadata_checker.command" = %command_label,
            "metadata_checker.target" = %request.target,
        );
        #[cfg(feature = "telemetry")]
        let query_compute_guard = query_compute_span.enter();
        let mut result = match request.command {
            ToolCommand::DiffRefresh => {
                // diff_refresh 不经 RuntimeQueryRequest 通路执行（stdio handler
                // 与 CLI one-shot 已先行拦截），到达此处一律视为协议错误。
                return Err(anyhow::anyhow!(
                    "DIFF_REFRESH_CONTEXT_REQUIRED: diff_refresh is not a runtime query command"
                ));
            }
            ToolCommand::AdviseQuery => {
                let question_kind = request.intent.as_deref().unwrap_or("auto");
                let page_scope = request.page_scope.as_deref();
                crate::answer_contract::build_advise_query_output(
                    &request.target,
                    page_scope,
                    question_kind,
                    &request.budget,
                )
            }
            ToolCommand::ExplainCondition => {
                let intent = crate::explain::TraversalIntent::parse(
                    request.intent.as_deref().unwrap_or("auto"),
                )?;
                crate::explain::build_explain_condition_output_with_intent(
                    &self.graph,
                    &request.target,
                    &request.budget,
                    intent,
                )?
            }
            ToolCommand::QueryModel => crate::query::build_query_model_output(
                &self.graph,
                &request.target,
                &request.budget,
            )?,
            ToolCommand::QueryPageLogic => {
                let project_dir = self.project_dir.as_deref();
                let availability_cache = self.read_model.as_ref().and_then(|model| {
                    model.page_logic_availability(&request.target, &request.budget)
                });
                let materialized_availability = self
                    .read_model
                    .as_ref()
                    .map(|model| model.availability_facts.as_ref());
                crate::query::build_query_page_logic_output_with_availability_cache(
                    &self.graph,
                    self.read_model
                        .as_ref()
                        .and_then(|model| model.dense_graph.as_deref())
                        .or(self.dense_snapshot.as_deref()),
                    availability_cache,
                    materialized_availability,
                    &request.target,
                    project_dir,
                    &request.budget,
                )?
            }
            ToolCommand::Explain => {
                crate::explain::build_explain_output(&self.graph, &request.target)?
            }
            ToolCommand::Context => {
                let depth = request.depth.unwrap_or(1);
                crate::context::build_context_output(
                    &self.graph,
                    &request.target,
                    depth,
                    &request.budget,
                )?
            }
            ToolCommand::Find => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                None,
                20,
            )?)?,
            ToolCommand::FindPage => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("page"),
                20,
            )?)?,
            ToolCommand::FindModel => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("model"),
                20,
            )?)?,
            ToolCommand::FindComponent => serde_json::to_value(crate::query::find_nodes(
                &self.graph,
                &request.target,
                Some("component"),
                20,
            )?)?,
            ToolCommand::QueryPage => {
                crate::query::build_query_page_output(&self.graph, &request.target)?
            }
            ToolCommand::QueryCross => {
                let (page_a, page_b) =
                    crate::tool_contract::parse_query_cross_target(&request.target)?;
                crate::query::build_query_cross_output(&self.graph, &page_a, &page_b)?
            }
            ToolCommand::QueryDataflow => {
                crate::query::build_query_dataflow_output(&self.graph, &request.target)?
            }
            ToolCommand::ReloadGraph | ToolCommand::CheckReload => {
                unreachable!("ReloadGraph and CheckReload handled before match")
            }
            ToolCommand::Status => {
                let status = self.status();
                serde_json::to_value(status)?
            }
        };
        #[cfg(feature = "telemetry")]
        drop(query_compute_guard);
        let query_compute_ms = query_start.elapsed().as_millis();

        if request.human {
            match request.command {
                ToolCommand::ExplainCondition => {
                    let human_text =
                        crate::explain::render_explain_condition_human(&result, &request.target);
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert(
                            "human_summary".to_string(),
                            serde_json::Value::String(human_text),
                        );
                    }
                }
                _ => {
                    diagnostics.push("HUMAN_MODE_NOT_SUPPORTED".to_string());
                }
            }
        }

        diagnostics.push(format!(
            "Query compute: {} ms before response processing",
            query_compute_ms
        ));

        let mut response = ResponseProcessor::runtime_response(
            result,
            diagnostics,
            0,
            query_compute_ms,
            total_start,
        )?;
        response.diagnostics.push(format!(
            "Response processed: serialize {} ms, total {} ms",
            response.timing.serialize_ms, response.timing.total_ms
        ));
        #[cfg(feature = "telemetry")]
        crate::telemetry::record_runtime_response(
            &query_span,
            &command_label,
            &request.budget,
            &response.timing,
        );
        Ok(response)
    }

    /// profiling 专用：执行 QueryPageLogic 并返回 page logic 归因 counters。
    ///
    /// 不改变默认 `query()` 契约；仅供 benchmark / perf_report 验证 B1/B2 集成。
    #[cfg(feature = "cli-local")]
    pub fn query_page_logic_profiled(
        &mut self,
        target: &str,
        budget: &str,
    ) -> Result<(
        crate::response_processor::RuntimeQueryResponse,
        crate::perf_profile::PerfProfile,
    )> {
        let total_start = Instant::now();
        let query_start = Instant::now();
        let availability_cache = self
            .read_model
            .as_ref()
            .and_then(|model| model.page_logic_availability(target, budget));
        let materialized_availability = self
            .read_model
            .as_ref()
            .map(|model| model.availability_facts.as_ref());
        let dense_snapshot = self
            .read_model
            .as_ref()
            .and_then(|model| model.dense_graph.as_deref())
            .or(self.dense_snapshot.as_deref());
        let (result, profile) =
            crate::query::build_query_page_logic_output_profiled_with_availability_cache(
                &self.graph,
                dense_snapshot,
                availability_cache,
                materialized_availability,
                target,
                self.project_dir.as_deref(),
                budget,
            )?;
        let query_compute_ms = query_start.elapsed().as_millis();
        let response = ResponseProcessor::runtime_response(
            result,
            Vec::new(),
            0,
            query_compute_ms,
            total_start,
        )?;
        Ok((response, profile))
    }

    /// 获取当前 graphdb 文件指纹
    pub fn current_fingerprint(&self) -> Result<GraphFingerprint> {
        let path = &self.graph_db_path;
        let (mtime, size) = std::fs::metadata(path)
            .map(|m| (m.modified().ok(), m.len()))
            .unwrap_or((None, 0));
        let prefix_hash = compute_prefix_hash(path, 4096);
        Ok(GraphFingerprint {
            path: path.clone(),
            mtime,
            size,
            content_prefix_hash: prefix_hash,
        })
    }

    /// 检查 graphdb 文件是否发生变化
    pub fn is_graph_changed(&self) -> Result<bool> {
        let current = self.current_fingerprint()?;
        let changed = current.size != self.graph_fingerprint.size
            || current.mtime != self.graph_fingerprint.mtime
            || current.content_prefix_hash != self.graph_fingerprint.content_prefix_hash;
        Ok(changed)
    }

    /// 如果 graphdb 发生变化则 reload，返回明确的结果枚举
    pub fn reload_if_changed(&mut self) -> Result<ReloadResult> {
        if self.is_graph_changed()? {
            match self.reload() {
                Ok(()) => Ok(ReloadResult::Reloaded),
                Err(e) => {
                    let err = format!("{}", e);
                    self.last_reload_error = Some(err.clone());
                    Ok(ReloadResult::ReloadFailed { error: err })
                }
            }
        } else {
            Ok(ReloadResult::Unchanged)
        }
    }

    /// 安全 reload：先加载新图，成功后再替换旧图
    pub fn reload(&mut self) -> Result<()> {
        let project_dir = self.project_dir.clone();
        match Self::load_with_project_dir_internal(
            &self.graph_db_path,
            project_dir.as_deref(),
            self.runtime_mode,
        ) {
            Ok(new_runtime) => {
                self.graph = new_runtime.graph;
                self.loaded_at = new_runtime.loaded_at;
                self.graph_file_mtime = new_runtime.graph_file_mtime;
                self.graph_file_size = new_runtime.graph_file_size;
                self.load_count = new_runtime.load_count;
                self.graph_load_ms = new_runtime.graph_load_ms;
                self.graph_fingerprint = new_runtime.graph_fingerprint;
                self.project_dir = new_runtime.project_dir;
                self.dense_snapshot = new_runtime.dense_snapshot;
                self.dense_snapshot_build_ms = new_runtime.dense_snapshot_build_ms;
                self.availability_facts_build_ms = new_runtime.availability_facts_build_ms;
                self.page_dependency_index_build_ms = new_runtime.page_dependency_index_build_ms;
                self.dense_snapshot_enabled = new_runtime.dense_snapshot_enabled;
                self.runtime_mode = new_runtime.runtime_mode;
                self.read_model = new_runtime.read_model;
                self.read_model_build_ms = new_runtime.read_model_build_ms;
                self.reload_count += 1;
                self.last_reload_error = None;
                Ok(())
            }
            Err(e) => {
                let err_msg = format!("reload failed: {}", e);
                self.last_reload_error = Some(err_msg.clone());
                Err(anyhow::anyhow!("{}", err_msg))
            }
        }
    }

    /// 获取当前 runtime 状态快照
    pub fn status(&self) -> RuntimeStatus {
        let loaded_at_secs = self
            .loaded_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        RuntimeStatus {
            graph_db_path: self.graph_db_path.clone(),
            loaded_at: loaded_at_secs,
            load_count: self.load_count,
            reload_count: self.reload_count,
            node_count: self.graph.graph.node_count(),
            edge_count: self.graph.graph.edge_count(),
            graph_file_mtime: self.graph_file_mtime,
            graph_file_size: self.graph_file_size,
            last_reload_error: self.last_reload_error.clone(),
            runtime_mode: self.runtime_mode,
            read_model_ready: self.read_model.is_some(),
        }
    }
}

/// 记录 page logic warm 结构性写入。
fn record_page_logic_cache_structural_write() {
    #[cfg(feature = "cli-local")]
    PAGE_LOGIC_CACHE_WARM_STRUCTURAL_WRITES.with(|counter| {
        counter.set(counter.get() + 1);
    });
}

/// 重置 warm cache 观测计数器（测试用）。
#[cfg(feature = "cli-local")]
pub fn reset_page_logic_cache_write_counters() {
    PAGE_LOGIC_CACHE_WARM_STRUCTURAL_WRITES.with(|counter| counter.set(0));
    PAGE_LOGIC_CACHE_MAP_CLONE_ELEMENTS.with(|counter| counter.set(0));
}

/// 读取 warm 结构性写入次数（测试用）。
#[cfg(feature = "cli-local")]
pub fn page_logic_cache_warm_structural_writes() -> usize {
    PAGE_LOGIC_CACHE_WARM_STRUCTURAL_WRITES.with(|counter| counter.get())
}

/// 读取 warm cache HashMap 深拷贝元素累计（测试用，应为 0）。
#[cfg(feature = "cli-local")]
pub fn page_logic_cache_map_clone_elements() -> usize {
    PAGE_LOGIC_CACHE_MAP_CLONE_ELEMENTS.with(|counter| counter.get())
}
