//! M51 性能阶段报告。
//!
//! 该模块把单次 profiling 结果聚合为 CI / 本地都能读取的 JSON 报告。

use crate::graph_store::GraphReadStore;
use crate::perf_profile::PerfProfile;
use crate::query::build_query_page_logic_output_profiled;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// M51 页面逻辑 profiling 场景。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageLogicProfileScenario {
    /// 场景名，用于报告和 CI 日志定位。
    pub name: String,
    /// 页面节点 ID。
    pub page_id: String,
    /// 输出预算，例如 compact / normal / full。
    pub budget: String,
}

impl PageLogicProfileScenario {
    /// 创建一个页面逻辑 profiling 场景。
    pub fn new(
        name: impl Into<String>,
        page_id: impl Into<String>,
        budget: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            page_id: page_id.into(),
            budget: budget.into(),
        }
    }
}

/// 单次场景采样。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLogicProfileSample {
    /// 从 0 开始的采样序号。
    pub sample_index: usize,
    /// 完整阶段 profile。
    pub profile: PerfProfile,
    /// 本次输出 JSON 序列化后的字节数。
    pub output_bytes: usize,
}

/// 阶段耗时汇总。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StageCostSummary {
    /// 阶段名。
    pub name: String,
    /// 参与汇总的样本数。
    pub sample_count: usize,
    /// 最小耗时毫秒。
    pub min_ms: u128,
    /// 最大耗时毫秒。
    pub max_ms: u128,
    /// 平均耗时毫秒。
    pub avg_ms: f64,
}

/// counter 汇总。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CounterSummary {
    /// 参与汇总的样本数。
    pub sample_count: usize,
    /// 最小值。
    pub min: u64,
    /// 最大值。
    pub max: u64,
    /// 平均值。
    pub avg: f64,
}

/// 性能成本模型，用于解释热点目的、成本来源和空间换时间候选方案。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PerformanceCostModel {
    /// 热点阶段或能力名。
    pub hotspot: String,
    /// 该热点在产品语义中的目的。
    pub purpose: String,
    /// 当前实现为什么容易变慢。
    pub why_expensive: String,
    /// 从 profile stage / counter 中提取的成本驱动因子。
    pub cost_drivers: Vec<String>,
    /// 可评估的空间换时间或结构性优化候选。
    pub space_time_candidates: Vec<String>,
    /// 下一步建议动作。
    pub next_action: String,
}

/// 单个场景的 profiling 报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLogicScenarioProfileReport {
    /// 场景定义。
    pub scenario: PageLogicProfileScenario,
    /// 场景采样。
    pub samples: Vec<PageLogicProfileSample>,
    /// 阶段耗时汇总。
    pub stage_summary: Vec<StageCostSummary>,
    /// counter 汇总。
    pub counter_summary: BTreeMap<String, CounterSummary>,
    /// 最近一次输出 JSON 序列化后的字节数。
    pub output_bytes: usize,
    /// 热点成本模型，解释为什么慢以及可选优化方向。
    pub cost_model: Vec<PerformanceCostModel>,
}

/// M51 页面逻辑 profiling 报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageLogicProfileReport {
    /// 报告类型，便于 CI / 后处理脚本识别。
    pub report_kind: String,
    /// 生成时间，Unix 毫秒。
    pub generated_at_unix_ms: u128,
    /// 每个场景的报告。
    pub scenarios: Vec<PageLogicScenarioProfileReport>,
}

/// 非 page-logic 核心慢场景单次采样。
#[cfg(feature = "cli-local")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreProfileSample {
    /// 从 0 开始的采样序号。
    pub sample_index: usize,
    /// 完整阶段 profile。
    pub profile: PerfProfile,
}

/// 非 page-logic 核心慢场景报告。
#[cfg(feature = "cli-local")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreScenarioProfileReport {
    /// 场景名，例如 rebuild / redb / runtime。
    pub scenario: String,
    /// 场景采样。
    pub samples: Vec<CoreProfileSample>,
    /// 阶段耗时汇总。
    pub stage_summary: Vec<StageCostSummary>,
    /// counter 汇总。
    pub counter_summary: BTreeMap<String, CounterSummary>,
    /// 热点成本模型，解释为什么慢以及可选优化方向。
    pub cost_model: Vec<PerformanceCostModel>,
}

/// M51 非 page-logic 核心慢场景 profiling 报告。
#[cfg(feature = "cli-local")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreProfileReport {
    /// 报告类型，便于 CI / 后处理脚本识别。
    pub report_kind: String,
    /// 生成时间，Unix 毫秒。
    pub generated_at_unix_ms: u128,
    /// 每个核心场景的报告。
    pub scenarios: Vec<CoreScenarioProfileReport>,
}

/// 构造页面逻辑阶段 profiling 报告。
pub fn build_page_logic_profile_report(
    graph: &dyn GraphReadStore,
    project_dir: Option<&Path>,
    scenarios: &[PageLogicProfileScenario],
    sample_count: usize,
) -> Result<PageLogicProfileReport> {
    ensure!(sample_count > 0, "sample_count must be greater than 0");
    ensure!(!scenarios.is_empty(), "at least one scenario is required");

    let mut scenario_reports = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        let mut samples = Vec::with_capacity(sample_count);
        for sample_index in 0..sample_count {
            let (output, profile) = build_query_page_logic_output_profiled(
                graph,
                &scenario.page_id,
                project_dir,
                &scenario.budget,
            )
            .with_context(|| {
                format!(
                    "profile scenario {} page {} budget {}",
                    scenario.name, scenario.page_id, scenario.budget
                )
            })?;
            let output_bytes = serde_json::to_vec(&output)
                .context("serialize page logic output for output size")?
                .len();
            samples.push(PageLogicProfileSample {
                sample_index,
                profile,
                output_bytes,
            });
        }

        let output_bytes = samples
            .last()
            .map(|sample| sample.output_bytes)
            .unwrap_or_default();
        let stage_summary = summarize_stages(&samples);
        let counter_summary = summarize_counters(&samples);
        let cost_model =
            build_page_logic_cost_model(&stage_summary, &counter_summary, output_bytes);
        scenario_reports.push(PageLogicScenarioProfileReport {
            scenario: scenario.clone(),
            samples,
            stage_summary,
            counter_summary,
            output_bytes,
            cost_model,
        });
    }

    Ok(PageLogicProfileReport {
        report_kind: "m51_page_logic_profile".to_string(),
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        scenarios: scenario_reports,
    })
}

/// 构造 rebuild / redb / runtime 核心慢场景 profiling 报告。
#[cfg(feature = "cli-local")]
pub fn build_core_profile_report(
    project_dir: &Path,
    sample_count: usize,
) -> Result<CoreProfileReport> {
    ensure!(sample_count > 0, "sample_count must be greater than 0");
    ensure!(
        project_dir.is_dir(),
        "project_dir must be a directory: {}",
        project_dir.display()
    );

    let mut scenarios = Vec::new();
    scenarios.push(build_core_scenario_report(
        "rebuild",
        sample_count,
        |sample_index| profile_rebuild(project_dir, sample_index),
    )?);
    scenarios.push(build_core_scenario_report(
        "redb",
        sample_count,
        |sample_index| profile_redb(project_dir, sample_index),
    )?);
    scenarios.push(build_core_scenario_report(
        "runtime",
        sample_count,
        |sample_index| profile_runtime(project_dir, sample_index),
    )?);

    Ok(CoreProfileReport {
        report_kind: "m51_core_profile".to_string(),
        generated_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        scenarios,
    })
}

fn summarize_stages(samples: &[PageLogicProfileSample]) -> Vec<StageCostSummary> {
    let mut values: BTreeMap<String, Vec<u128>> = BTreeMap::new();
    for sample in samples {
        for stage in &sample.profile.stages {
            values
                .entry(stage.name.clone())
                .or_default()
                .push(stage.duration_ms);
        }
    }
    values
        .into_iter()
        .map(|(name, durations)| {
            let sample_count = durations.len();
            let min_ms = durations.iter().copied().min().unwrap_or_default();
            let max_ms = durations.iter().copied().max().unwrap_or_default();
            let total_ms: u128 = durations.iter().sum();
            StageCostSummary {
                name,
                sample_count,
                min_ms,
                max_ms,
                avg_ms: total_ms as f64 / sample_count as f64,
            }
        })
        .collect()
}

fn summarize_counters(samples: &[PageLogicProfileSample]) -> BTreeMap<String, CounterSummary> {
    let mut values: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for sample in samples {
        for (name, value) in &sample.profile.counters {
            values.entry(name.clone()).or_default().push(*value);
        }
    }
    values
        .into_iter()
        .map(|(name, counters)| {
            let sample_count = counters.len();
            let min = counters.iter().copied().min().unwrap_or_default();
            let max = counters.iter().copied().max().unwrap_or_default();
            let total: u64 = counters.iter().sum();
            (
                name,
                CounterSummary {
                    sample_count,
                    min,
                    max,
                    avg: total as f64 / sample_count as f64,
                },
            )
        })
        .collect()
}

fn build_page_logic_cost_model(
    stage_summary: &[StageCostSummary],
    counter_summary: &BTreeMap<String, CounterSummary>,
    output_bytes: usize,
) -> Vec<PerformanceCostModel> {
    let mut models = Vec::new();

    if has_stage(stage_summary, "key_model_availability")
        || counter_summary.contains_key("availability_models_batched")
    {
        let mut cost_drivers = Vec::new();
        push_stage_driver(&mut cost_drivers, stage_summary, "key_model_availability");
        push_counter_driver(&mut cost_drivers, counter_summary, "key_models");
        push_counter_driver(
            &mut cost_drivers,
            counter_summary,
            "availability_models_batched",
        );
        push_counter_driver(
            &mut cost_drivers,
            counter_summary,
            "availability_condition_groups",
        );
        push_counter_driver(
            &mut cost_drivers,
            counter_summary,
            "availability_fallback_models",
        );
        cost_drivers.push(format!("output_bytes={output_bytes}"));

        models.push(PerformanceCostModel {
            hotspot: "key_model_availability".to_string(),
            purpose: "解释页面关键模型的数据可用性，告诉调用方哪些模型会被 filter、empty gate、join/union 或输入缺失影响。"
                .to_string(),
            why_expensive: "normal/full 必须完整展开页面内关键模型；复杂页面中 key model 数量、condition group 和 dataflow 元数据会一起放大。"
                .to_string(),
            cost_drivers,
            space_time_candidates: vec![
                "runtime/load-time availability fact index keyed by page+model".to_string(),
                "page-scoped dataflow metadata cache reused across page logic and explain".to_string(),
                "precomputed empty-gate/filter bitmap to trade memory for repeated condition scans"
                    .to_string(),
            ],
            next_action:
                "优先评估构建期或 runtime load 期索引；避免每次 query 临时全量扫描 condition facts。"
                    .to_string(),
        });
    }

    if has_stage(stage_summary, "path_summary")
        || counter_summary.keys().any(|name| name.starts_with("path_"))
    {
        let mut cost_drivers = Vec::new();
        push_stage_driver(&mut cost_drivers, stage_summary, "path_summary");
        for name in [
            "path_anchor_extract_ms",
            "path_candidate_search_ms",
            "path_classification_ms",
            // 使用实际的 JSON 物化和跨页上下文计时，不虚构按分类拆分的耗时。
            "path_json_build_ms",
            "path_side_context_ms",
            "path_base_candidates",
            "path_graph_cache_hits",
        ] {
            push_counter_driver(&mut cost_drivers, counter_summary, name);
        }

        models.push(PerformanceCostModel {
            hotspot: "path_summary".to_string(),
            purpose: "把页面组件、字段、模型之间的因果路径归类为 primary、supporting、related 和 rejected，供用户理解字段来源与影响面。"
                .to_string(),
            why_expensive:
                "候选路径生成需要在高 fanout 图上做多轮邻接遍历，再执行分类和 JSON 构造；页面越复杂，候选边界越容易膨胀。"
                    .to_string(),
            cost_drivers,
            space_time_candidates: vec![
                "page-local path anchor index keyed by page+node".to_string(),
                "bounded candidate adjacency cache for BoundedCausalPathFinder".to_string(),
                "precomputed field-to-model reachability summary with invalidation on rebuild"
                    .to_string(),
            ],
            next_action:
                "先用 path_candidate_search_ms 判断是否值得在 BoundedCausalPathFinder 内增加候选剪枝或邻接索引。"
                    .to_string(),
        });
    }

    if has_stage(stage_summary, "prerequisites") {
        let mut cost_drivers = Vec::new();
        push_stage_driver(&mut cost_drivers, stage_summary, "prerequisites");
        for name in [
            "prerequisites_component_scan_ms",
            "prerequisites_action_scan_ms",
            "prerequisites_data_source_scan_ms",
            "prerequisites_sort_ms",
        ] {
            push_counter_driver(&mut cost_drivers, counter_summary, name);
        }
        models.push(PerformanceCostModel {
            hotspot: "prerequisites".to_string(),
            purpose: "汇总页面显示、数据和动作前置条件，解释页面为什么能打开、能查、能执行。".to_string(),
            why_expensive:
                "需要跨组件、模型和动作元数据读取条件；如果重复读取 metadata 或重复构造 JSON，会形成稳定底噪。"
                    .to_string(),
            cost_drivers,
            space_time_candidates: vec![
                "page prerequisites snapshot reused by query_page_logic budgets".to_string(),
                "metadata parse result cache keyed by relative path and file fingerprint".to_string(),
            ],
            next_action: "当前不是最高热点；只有真实 profile 显示超过 path_summary 后再做结构性缓存。"
                .to_string(),
        });
    }

    models
}

#[cfg(feature = "cli-local")]
fn build_core_cost_model(
    scenario: &str,
    stage_summary: &[StageCostSummary],
    counter_summary: &BTreeMap<String, CounterSummary>,
) -> Vec<PerformanceCostModel> {
    let mut drivers = Vec::new();
    for stage in stage_summary {
        drivers.push(format!("stage.{}.avg_ms={:.1}", stage.name, stage.avg_ms));
    }
    for (name, counter) in counter_summary {
        drivers.push(format!("counter.{name}.avg={:.1}", counter.avg));
    }

    let (purpose, why_expensive, space_time_candidates, next_action) = match scenario {
        "rebuild" => (
            "在元数据文件变更后重建图索引，保证 query/explain 使用最新节点、边和文件状态。",
            "cold path 同时包含文件发现、dirty detection、parse、graph apply 和 persist commit；真实项目文件多时 parse 与提交会被放大。",
            vec![
                "parsed metadata artifact cache keyed by file hash".to_string(),
                "delta graph apply with node/edge reuse".to_string(),
                "persist commit batching for file states and graph mutations".to_string(),
            ],
            "先确认真实项目中 parse、graph_apply、persist_commit 哪个最大，再决定做解析缓存还是提交批量化。",
        ),
        "redb" => (
            "提供本地图数据库持久化，让 CLI / runtime 能跨进程复用扫描结果。",
            "open、file state load、node/edge write 与 commit 都可能触发磁盘 IO；大项目下 commit 通常是最贵阶段。",
            vec![
                "write buffer grouped by table and edge type".to_string(),
                "commit batching across node/edge/file-state writes".to_string(),
                "read-side snapshot/index to trade disk space for query load time".to_string(),
            ],
            "M52 不改 schema；若 commit 持续主导，应先做 batch write 设计和 shadow compare。",
        ),
        "runtime" => (
            "模拟长期运行的查询 runtime，衡量 graphdb load、reload check 和 query dispatch 开销。",
            "短生命周期 runtime 每次都要加载 graphdb 并检查 fingerprint；真实插件如果频繁创建 runtime，会把加载成本暴露给交互链路。",
            vec![
                "long-lived runtime with warm GraphDB handle".to_string(),
                "fingerprint sidecar cache for unchanged project".to_string(),
                "query dispatch cache for repeated page/model lookups".to_string(),
            ],
            "优先验证业务环境是否允许 runtime 复用；允许时收益通常高于继续压单次 query JSON 构造。",
        ),
        _ => (
            "记录核心性能场景。",
            "该场景需要结合 stage/counter 判断主要成本。",
            Vec::new(),
            "补充更细 stage 后再决定优化方向。",
        ),
    };

    vec![PerformanceCostModel {
        hotspot: scenario.to_string(),
        purpose: purpose.to_string(),
        why_expensive: why_expensive.to_string(),
        cost_drivers: drivers,
        space_time_candidates,
        next_action: next_action.to_string(),
    }]
}

fn has_stage(stage_summary: &[StageCostSummary], name: &str) -> bool {
    stage_summary.iter().any(|stage| stage.name == name)
}

fn push_stage_driver(drivers: &mut Vec<String>, stage_summary: &[StageCostSummary], name: &str) {
    if let Some(stage) = stage_summary.iter().find(|stage| stage.name == name) {
        drivers.push(format!("stage.{}.avg_ms={:.1}", stage.name, stage.avg_ms));
    }
}

fn push_counter_driver(
    drivers: &mut Vec<String>,
    counter_summary: &BTreeMap<String, CounterSummary>,
    name: &str,
) {
    if let Some(counter) = counter_summary.get(name) {
        drivers.push(format!("counter.{name}.avg={:.1}", counter.avg));
    }
}

#[cfg(feature = "cli-local")]
fn build_core_scenario_report(
    scenario: &str,
    sample_count: usize,
    mut build_sample: impl FnMut(usize) -> Result<PerfProfile>,
) -> Result<CoreScenarioProfileReport> {
    let mut samples = Vec::with_capacity(sample_count);
    for sample_index in 0..sample_count {
        samples.push(CoreProfileSample {
            sample_index,
            profile: build_sample(sample_index).with_context(|| {
                format!("profile core scenario {scenario} sample {sample_index}")
            })?,
        });
    }
    let stage_summary = summarize_core_stages(&samples);
    let counter_summary = summarize_core_counters(&samples);
    let cost_model = build_core_cost_model(scenario, &stage_summary, &counter_summary);
    Ok(CoreScenarioProfileReport {
        scenario: scenario.to_string(),
        samples,
        stage_summary,
        counter_summary,
        cost_model,
    })
}

#[cfg(feature = "cli-local")]
fn summarize_core_stages(samples: &[CoreProfileSample]) -> Vec<StageCostSummary> {
    let mut values: BTreeMap<String, Vec<u128>> = BTreeMap::new();
    for sample in samples {
        for stage in &sample.profile.stages {
            values
                .entry(stage.name.clone())
                .or_default()
                .push(stage.duration_ms);
        }
    }
    values
        .into_iter()
        .map(|(name, durations)| {
            let sample_count = durations.len();
            let min_ms = durations.iter().copied().min().unwrap_or_default();
            let max_ms = durations.iter().copied().max().unwrap_or_default();
            let total_ms: u128 = durations.iter().sum();
            StageCostSummary {
                name,
                sample_count,
                min_ms,
                max_ms,
                avg_ms: total_ms as f64 / sample_count as f64,
            }
        })
        .collect()
}

#[cfg(feature = "cli-local")]
fn summarize_core_counters(samples: &[CoreProfileSample]) -> BTreeMap<String, CounterSummary> {
    let mut values: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for sample in samples {
        for (name, value) in &sample.profile.counters {
            values.entry(name.clone()).or_default().push(*value);
        }
    }
    values
        .into_iter()
        .map(|(name, counters)| {
            let sample_count = counters.len();
            let min = counters.iter().copied().min().unwrap_or_default();
            let max = counters.iter().copied().max().unwrap_or_default();
            let total: u64 = counters.iter().sum();
            (
                name,
                CounterSummary {
                    sample_count,
                    min,
                    max,
                    avg: total as f64 / sample_count as f64,
                },
            )
        })
        .collect()
}

#[cfg(feature = "cli-local")]
fn profile_rebuild(project_dir: &Path, sample_index: usize) -> Result<PerfProfile> {
    use crate::scanner::indexer::ProjectIndexer;
    use crate::storage_provider::LocalStorageProvider;
    use std::time::Instant;

    let workspace = create_profile_workspace("rebuild", project_dir, sample_index)?;
    let db_path = workspace.join("profile.graphdb");
    let mut profile = PerfProfile::new("m51_core_rebuild");
    let provider = LocalStorageProvider;
    let mut graph = crate::graph::GraphDB::open(&db_path).context("profile rebuild open graph")?;

    let started_at = Instant::now();
    let prev_states = graph.load_file_states().unwrap_or_default();
    profile.record_stage("rebuild_load_file_states", started_at.elapsed());

    let started_at = Instant::now();
    let files = ProjectIndexer::discover_files(&workspace).context("profile rebuild discover")?;
    profile.record_stage("rebuild_cold_discover", started_at.elapsed());
    profile.set_counter("rebuild_cold_discovered", files.len() as u64);

    let started_at = Instant::now();
    let plan = ProjectIndexer::diff_file_states(&files, &prev_states, &workspace, &provider)
        .context("profile rebuild dirty detection")?;
    profile.record_stage("rebuild_cold_dirty_detection", started_at.elapsed());
    profile.set_counter("rebuild_cold_dirty", plan.dirty.len() as u64);
    profile.set_counter("rebuild_cold_deleted", plan.deleted.len() as u64);

    let mut new_states = prev_states.clone();
    let started_at = Instant::now();
    ProjectIndexer::apply_deletions(&mut graph, &mut new_states, &plan.deleted)
        .context("profile rebuild apply deletions")?;
    profile.record_stage("rebuild_cold_apply_deletions", started_at.elapsed());

    let started_at = Instant::now();
    let updates =
        ProjectIndexer::parse_dirty_files(&prev_states, &plan.dirty, &workspace, &provider)
            .context("profile rebuild parse dirty files")?;
    profile.record_stage("rebuild_cold_parse", started_at.elapsed());
    profile.set_counter("rebuild_cold_updates", updates.len() as u64);

    let started_at = Instant::now();
    let parsed_nodes = ProjectIndexer::apply_graph_updates(&mut graph, &updates)
        .context("profile rebuild graph apply")?;
    profile.record_stage("rebuild_cold_graph_apply", started_at.elapsed());
    profile.set_counter("rebuild_cold_touched_files", parsed_nodes.len() as u64);

    for update in updates {
        let logical_path = update.logical_path.clone();
        let node_ids = parsed_nodes
            .get(logical_path.as_str())
            .cloned()
            .unwrap_or_default();
        new_states.insert(
            logical_path.clone(),
            crate::graph::FileState {
                file_path: logical_path,
                file_hash: update.file_hash,
                mtime: update.mtime,
                size: update.size,
                node_ids,
            },
        );
    }

    let commit = crate::graph_store::IndexCommit {
        file_states: new_states,
        dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
        deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
        checkpoint: None,
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    let started_at = Instant::now();
    let cold_report = ProjectIndexer::persist_index(&mut graph, commit)
        .context("profile rebuild persist commit")?;
    profile.record_stage("rebuild_cold_persist_commit", started_at.elapsed());
    profile.set_counter("rebuild_cold_unchanged", cold_report.unchanged as u64);

    let started_at = Instant::now();
    let prev_states = graph.load_file_states().unwrap_or_default();
    let files = ProjectIndexer::discover_files(&workspace).context("profile noop discover")?;
    profile.record_stage("rebuild_noop_scan", started_at.elapsed());

    let started_at = Instant::now();
    let noop_plan = ProjectIndexer::diff_file_states(&files, &prev_states, &workspace, &provider)
        .context("profile noop dirty detection")?;
    profile.record_stage("rebuild_noop_dirty_detection", started_at.elapsed());
    profile.set_counter("rebuild_noop_dirty", noop_plan.dirty.len() as u64);
    profile.set_counter("rebuild_noop_deleted", noop_plan.deleted.len() as u64);
    profile.set_counter(
        "rebuild_noop_unchanged",
        noop_plan
            .discovered_count
            .saturating_sub(noop_plan.dirty.len()) as u64,
    );
    profile.finish();
    Ok(profile)
}

#[cfg(feature = "cli-local")]
fn profile_redb(project_dir: &Path, sample_index: usize) -> Result<PerfProfile> {
    use crate::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
    use crate::graph_store::{GraphWriteStore, IndexCommit, IndexStateStore};
    use crate::scanner::indexer::ProjectIndexer;
    use std::collections::HashMap;
    use std::time::Instant;

    let workspace = create_profile_workspace("redb", project_dir, sample_index)?;
    let db_path = workspace.join("profile.graphdb");
    ProjectIndexer::scan(&workspace, &db_path).context("prepare redb profile graph")?;
    let mut profile = PerfProfile::new("m51_core_redb");

    let started_at = Instant::now();
    let graph = GraphDB::open(&db_path).context("profile redb open")?;
    profile.record_stage("redb_open", started_at.elapsed());

    let started_at = Instant::now();
    let file_states = graph
        .load_file_states()
        .context("profile redb load file states")?;
    profile.record_stage("redb_load_file_states", started_at.elapsed());
    profile.set_counter("redb_file_states", file_states.len() as u64);

    let started_at = Instant::now();
    let status = GraphDB::check_graph_db(&db_path);
    profile.record_stage("redb_check_graphdb", started_at.elapsed());
    profile.set_counter("redb_check_diagnostics", status.diagnostics.len() as u64);

    let mut graph = graph;
    let started_at = Instant::now();
    graph
        .upsert_node(Node {
            id: "m52:redb_profile_node_a".to_string(),
            node_type: NodeType::Component,
            path: "m52/redb_profile.spg".to_string(),
            name: "redb_profile_node_a".to_string(),
            meta: None,
        origin_file: None})
        .context("profile redb node write a")?;
    graph
        .upsert_node(Node {
            id: "m52:redb_profile_node_b".to_string(),
            node_type: NodeType::Model,
            path: "m52/redb_profile.tbl".to_string(),
            name: "redb_profile_node_b".to_string(),
            meta: None,
        origin_file: None})
        .context("profile redb node write b")?;
    profile.record_stage("redb_node_write", started_at.elapsed());

    let started_at = Instant::now();
    GraphWriteStore::add_edge(
        &mut graph,
        Edge {
            from: "m52:redb_profile_node_a".to_string(),
            to: "m52:redb_profile_node_b".to_string(),
            edge_type: EdgeType::Reads,
            field_path: None,
            meta: None,
        origin_file: None},
    )
    .context("profile redb edge write")?;
    profile.record_stage("redb_edge_write", started_at.elapsed());

    let mut file_states = HashMap::new();
    file_states.insert(
        "m52/redb_profile.spg".to_string(),
        crate::graph::FileState {
            file_path: "m52/redb_profile.spg".to_string(),
            file_hash: "m52-profile".to_string(),
            mtime: 0,
            size: 0,
            node_ids: vec![
                "m52:redb_profile_node_a".to_string(),
                "m52:redb_profile_node_b".to_string(),
            ],
        },
    );
    let started_at = Instant::now();
    graph
        .persist_index(IndexCommit {
            file_states,
            dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
            deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
            checkpoint: None,
            delta: None,
            scanner_entries: Vec::new(),
            scanner_deleted_paths: Vec::new(),
        })
        .context("profile redb commit")?;
    profile.record_stage("redb_commit", started_at.elapsed());
    profile.finish();
    Ok(profile)
}

#[cfg(feature = "cli-local")]
fn profile_runtime(project_dir: &Path, sample_index: usize) -> Result<PerfProfile> {
    use crate::runtime::{GraphRuntime, RuntimeMode};
    use crate::scanner::indexer::ProjectIndexer;
    use crate::tool_contract::ToolCommand;
    use std::time::Instant;

    let workspace = create_profile_workspace("runtime", project_dir, sample_index)?;
    let db_path = workspace.join("profile.graphdb");
    ProjectIndexer::scan(&workspace, &db_path).context("prepare runtime profile graph")?;
    let mut profile = PerfProfile::new("m51_core_runtime");
    let page_logic_target = choose_runtime_page_logic_target(&workspace);

    let started_at = Instant::now();
    let mut runtime = GraphRuntime::load_with_project_dir(&db_path, Some(&workspace))
        .context("profile runtime load")?;
    let one_shot_load_elapsed = started_at.elapsed();
    let one_shot_load_ms = one_shot_load_elapsed.as_millis() as u64;
    profile.record_stage("runtime_graphdb_load", one_shot_load_elapsed);
    profile.set_counter("runtime_one_shot_load_ms", one_shot_load_ms);
    profile.set_counter("runtime_load_count", runtime.load_count as u64);
    profile.set_counter(
        "runtime_dense_snapshot_build_ms",
        runtime.dense_snapshot_build_ms as u64,
    );
    profile.set_counter(
        "runtime_read_model_build_ms",
        runtime.read_model_build_ms as u64,
    );
    profile.set_counter(
        "runtime_dense_snapshot_nodes",
        runtime
            .dense_snapshot
            .as_ref()
            .map(|snapshot| snapshot.dense_node_count() as u64)
            .unwrap_or_default(),
    );
    profile.set_counter(
        "runtime_dense_snapshot_edges",
        runtime
            .dense_snapshot
            .as_ref()
            .map(|snapshot| snapshot.dense_edge_count() as u64)
            .unwrap_or_default(),
    );

    // 第二次打开 redb 可能改变文件状态；先在尚未再次打开的文件上验证 unchanged 场景。
    let started_at = Instant::now();
    let check_reload = runtime
        .query(lifecycle_request(ToolCommand::CheckReload))
        .context("profile runtime check_reload")?;
    profile.record_stage("runtime_check_reload_unchanged", started_at.elapsed());
    let reloaded = check_reload
        .result
        .get("reloaded")
        .and_then(|value| value.as_bool())
        .context("profile runtime check_reload result.reloaded must be a boolean")?;
    anyhow::ensure!(
        !reloaded
            && check_reload.result.get("error").is_none()
            && check_reload.result["status"]["reload_count"].as_u64() == Some(0),
        "unchanged runtime profile must not reload or fail: {}",
        check_reload.result
    );
    // 诊断条数是结果标记计数（unchanged 场景恒为 GRAPH_UNCHANGED 诊断 + 计时说明），
    // 不是变量指标；报告消费方不应把它当变化信号。
    profile.set_counter(
        "runtime_check_reload_diagnostics",
        check_reload.diagnostics.len() as u64,
    );

    let started_at = Instant::now();
    let mut long_lived_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&workspace),
        RuntimeMode::LongLived,
    )
    .context("profile long-lived runtime load")?;
    let long_lived_load_elapsed = started_at.elapsed();
    let long_lived_load_ms = long_lived_load_elapsed.as_millis() as u64;
    profile.record_stage("runtime_long_lived_load", long_lived_load_elapsed);
    profile.set_counter("runtime_long_lived_load_ms", long_lived_load_ms);
    profile.set_counter(
        "runtime_long_lived_read_model_build_ms",
        long_lived_runtime.read_model_build_ms as u64,
    );
    profile.set_counter(
        "runtime_availability_facts_build_ms",
        long_lived_runtime.availability_facts_build_ms as u64,
    );
    profile.set_counter(
        "runtime_page_dependency_index_build_ms",
        long_lived_runtime.page_dependency_index_build_ms as u64,
    );
    profile.set_counter(
        "runtime_long_lived_dense_nodes",
        long_lived_runtime
            .read_model
            .as_ref()
            .and_then(|model| model.dense_graph.as_ref())
            .map(|dense| dense.dense_node_count() as u64)
            .unwrap_or_default(),
    );
    profile.set_counter(
        "runtime_long_lived_dense_edges",
        long_lived_runtime
            .read_model
            .as_ref()
            .and_then(|model| model.dense_graph.as_ref())
            .map(|dense| dense.dense_edge_count() as u64)
            .unwrap_or_default(),
    );

    let started_at = Instant::now();
    let status = runtime
        .query(lifecycle_request(ToolCommand::Status))
        .context("profile runtime status")?;
    profile.record_stage("runtime_status_dispatch", started_at.elapsed());
    profile.set_counter(
        "runtime_status_diagnostics",
        status.diagnostics.len() as u64,
    );

    let started_at = Instant::now();
    let query_response = runtime
        .query(crate::runtime::RuntimeQueryRequest {
            command: ToolCommand::QueryPageLogic,
            target: page_logic_target.clone(),
            budget: "compact".to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        })
        .context("profile runtime query dispatch")?;
    let one_shot_query_elapsed = started_at.elapsed();
    let one_shot_query_ms = one_shot_query_elapsed.as_millis() as u64;
    profile.record_stage("runtime_query_dispatch", one_shot_query_elapsed);
    profile.set_counter("runtime_one_shot_query_dispatch_ms", one_shot_query_ms);
    profile.set_counter(
        "runtime_query_diagnostics",
        query_response.diagnostics.len() as u64,
    );

    let started_at = Instant::now();
    let (long_lived_query_response, long_lived_query_profile) = long_lived_runtime
        .query_page_logic_profiled(&page_logic_target, "compact")
        .context("profile long-lived runtime query dispatch")?;
    let long_lived_query_elapsed = started_at.elapsed();
    let long_lived_query_ms = long_lived_query_elapsed.as_millis() as u64;
    profile.record_stage(
        "runtime_long_lived_query_dispatch",
        long_lived_query_elapsed,
    );
    profile.set_counter("runtime_long_lived_query_dispatch_ms", long_lived_query_ms);
    profile.set_counter(
        "runtime_long_lived_query_diagnostics",
        long_lived_query_response.diagnostics.len() as u64,
    );
    record_runtime_page_logic_profile_counters(
        &mut profile,
        "runtime_long_lived_query",
        &long_lived_query_profile,
    );

    let started_at = Instant::now();
    let warm_ms = long_lived_runtime
        .warm_page_logic_availability(&page_logic_target, "compact")
        .context("profile long-lived page logic availability warm")?;
    profile.record_stage("runtime_long_lived_availability_warm", started_at.elapsed());
    profile.set_counter("runtime_long_lived_availability_warm_ms", warm_ms as u64);

    let started_at = Instant::now();
    let (warmed_query_response, warmed_query_profile) = long_lived_runtime
        .query_page_logic_profiled(&page_logic_target, "compact")
        .context("profile warmed long-lived runtime query dispatch")?;
    let warmed_query_elapsed = started_at.elapsed();
    let warmed_query_ms = warmed_query_elapsed.as_millis() as u64;
    profile.record_stage(
        "runtime_long_lived_warmed_query_dispatch",
        warmed_query_elapsed,
    );
    profile.set_counter(
        "runtime_long_lived_warmed_query_dispatch_ms",
        warmed_query_ms,
    );
    profile.set_counter(
        "runtime_long_lived_warmed_query_diagnostics",
        warmed_query_response.diagnostics.len() as u64,
    );
    record_runtime_page_logic_profile_counters(
        &mut profile,
        "runtime_long_lived_warmed_query",
        &warmed_query_profile,
    );
    for query_count in [1_u64, 5, 10, 50] {
        profile.set_counter(
            &format!("runtime_one_shot_total_ms_n{query_count}"),
            one_shot_load_ms + one_shot_query_ms.saturating_mul(query_count),
        );
        profile.set_counter(
            &format!("runtime_long_lived_total_ms_n{query_count}"),
            long_lived_load_ms + long_lived_query_ms.saturating_mul(query_count),
        );
        profile.set_counter(
            &format!("runtime_long_lived_warmed_total_ms_n{query_count}"),
            long_lived_load_ms + (warm_ms as u64) + warmed_query_ms.saturating_mul(query_count),
        );
    }

    profile.finish();
    Ok(profile)
}

#[cfg(feature = "cli-local")]
fn record_runtime_page_logic_profile_counters(
    profile: &mut PerfProfile,
    prefix: &str,
    page_logic_profile: &PerfProfile,
) {
    for counter in [
        "availability_materialized_hits",
        "availability_read_model_used",
        "path_dense_adjacency_hits",
        "path_dense_graph_used",
    ] {
        profile.set_counter(
            &format!("{prefix}_{counter}"),
            page_logic_profile.counter(counter),
        );
    }
}

#[cfg(feature = "cli-local")]
fn choose_runtime_page_logic_target(project_dir: &Path) -> String {
    if project_dir.join("app/销售.app/销售/合同协议.spg").exists() {
        return "page:app/销售.app/销售/合同协议.spg".to_string();
    }
    "page:app/page_relations.spg".to_string()
}

#[cfg(feature = "cli-local")]
fn lifecycle_request(
    command: crate::tool_contract::ToolCommand,
) -> crate::runtime::RuntimeQueryRequest {
    crate::runtime::RuntimeQueryRequest {
        command,
        target: String::new(),
        budget: "normal".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    }
}

#[cfg(feature = "cli-local")]
fn create_profile_workspace(
    prefix: &str,
    source_project_dir: &Path,
    sample_index: usize,
) -> Result<std::path::PathBuf> {
    static PROFILE_WORKSPACE_SEQ: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = PROFILE_WORKSPACE_SEQ.fetch_add(1, Ordering::Relaxed);
    let workspace = std::env::temp_dir().join(format!(
        "metadata-checker-m51-core-{prefix}-{}-{sample_index}-{nanos}-{seq}",
        std::process::id()
    ));
    if workspace.exists() {
        std::fs::remove_dir_all(&workspace)
            .with_context(|| format!("remove stale workspace {}", workspace.display()))?;
    }
    copy_dir_recursive(source_project_dir, &workspace)?;
    Ok(workspace)
}

#[cfg(feature = "cli-local")]
fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("create directory {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("read directory {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("read entry in {}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .with_context(|| format!("read file type {}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copy {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CounterSummary, PageLogicProfileSample, StageCostSummary, summarize_counters,
        summarize_stages,
    };
    use crate::perf_profile::PerfProfile;
    use std::time::Duration;

    fn sample(stage_ms: u64, counter_value: u64) -> PageLogicProfileSample {
        let mut profile = PerfProfile::new("test");
        profile.record_stage("stage", Duration::from_millis(stage_ms));
        profile.set_counter("counter", counter_value);
        PageLogicProfileSample {
            sample_index: 0,
            profile,
            output_bytes: 0,
        }
    }

    #[test]
    fn profile_summaries_calculate_exact_stage_and_counter_statistics() {
        let samples = vec![sample(2, 3), sample(8, 9), sample(5, 6)];

        assert_eq!(
            summarize_stages(&samples),
            vec![StageCostSummary {
                name: "stage".to_string(),
                sample_count: 3,
                min_ms: 2,
                max_ms: 8,
                avg_ms: 5.0,
            }]
        );

        let summaries = summarize_counters(&samples);
        assert_eq!(
            summaries.get("counter"),
            Some(&CounterSummary {
                sample_count: 3,
                min: 3,
                max: 9,
                avg: 6.0,
            })
        );
    }
    /// 缺失观测不能填零参与均值；明确记录的零值则必须参与统计。
    #[test]
    fn profile_summaries_distinguish_missing_samples_from_recorded_zero() {
        assert_eq!(summarize_stages(&[]), Vec::new());
        assert_eq!(summarize_counters(&[]), std::collections::BTreeMap::new());
        let mut missing = sample(99, 99);
        missing.profile.stages.clear();
        missing.profile.counters.clear();
        let samples = vec![sample(0, 0), missing, sample(3, 5)];
        assert_eq!(
            summarize_stages(&samples),
            vec![StageCostSummary {
                name: "stage".to_string(),
                sample_count: 2,
                min_ms: 0,
                max_ms: 3,
                avg_ms: 1.5,
            }]
        );
        assert_eq!(
            summarize_counters(&samples).get("counter"),
            Some(&CounterSummary {
                sample_count: 2,
                min: 0,
                max: 5,
                avg: 2.5,
            })
        );
    }
}
