use crate::graph_store::{GraphNeighbors, GraphReadStore};
use crate::perf_profile::PerfProfile;
use crate::query::find_candidates;
use anyhow::Result;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::time::Instant;

mod diagnostics;
mod evidence;
mod graph_collect;
mod metadata;
mod path_summary;
mod prerequisites;

/// 提取完整节点 ID 中的组件裸 ID（如 comp:app/a.spg|button1 -> button1）
fn component_short_id(node_id: &str) -> &str {
    node_id.split('|').next_back().unwrap_or(node_id)
}

/// 为 action 构造稳定的近似 json_path
fn action_json_path(component_id: &str, action_id: &str) -> String {
    format!(
        "canvas.components[id='{}'].actions[id='{}']",
        component_short_id(component_id),
        action_id
    )
}

/// 从边元数据提取字符串字段
fn edge_meta_str(edge: &crate::graph::Edge, key: &str) -> Option<String> {
    edge.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

/// 按优先顺序提取对象中的字符串字段，自动跳过 null/非字符串
fn pick_str_field<'a>(obj: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(|v| v.as_str()))
}

/// 记录一个 M51 profiling 阶段。
fn record_profile_stage(profile: &mut Option<&mut PerfProfile>, name: &str, started_at: Instant) {
    let duration = started_at.elapsed();
    #[cfg(feature = "telemetry")]
    {
        let span = crate::telemetry::stage_span(&format!("query_page_logic.{name}"));
        let _guard = span.enter();
        crate::telemetry::record_stage_ok(&span, duration.as_millis() as u64);
    }
    if let Some(profile) = profile.as_deref_mut() {
        profile.record_stage(name, duration);
    }
}

/// 设置一个 M51 profiling counter。
fn set_profile_counter(profile: &mut Option<&mut PerfProfile>, name: &str, value: usize) {
    if let Some(profile) = profile.as_deref_mut() {
        profile.set_counter(name, value as u64);
    }
}

/// 累加一个 M51 profiling counter。
fn add_profile_counter(profile: &mut Option<&mut PerfProfile>, name: &str, value: usize) {
    if let Some(profile) = profile.as_deref_mut() {
        profile.add_counter(name, value as u64);
    }
}

/// 读取节点邻接并在本次 page logic 查询内缓存，避免重复构造 owned neighbor 列表。
fn cached_node_edges(
    graph: &dyn GraphReadStore,
    edge_cache: &mut HashMap<String, Option<GraphNeighbors>>,
    node_id: &str,
) -> Result<Option<GraphNeighbors>> {
    if !edge_cache.contains_key(node_id) {
        edge_cache.insert(node_id.to_string(), graph.get_node_edges(node_id)?);
    }
    Ok(edge_cache.get(node_id).cloned().flatten())
}

/// 从字段引用中提取模型 ID，例如 model:model1.name -> model:model1。
fn model_id_from_reference(reference: &str) -> Option<String> {
    let rest = reference.strip_prefix("model:")?;
    let model_part = rest.split('.').next().unwrap_or(rest).trim();
    if model_part.is_empty() {
        return None;
    }
    Some(format!("model:{}", model_part.trim_start_matches("model:")))
}

/// 将页面内局部模型 ID 转成 page-scoped 查询目标。
fn page_scoped_model_target(page_path: &str, model_id: &str) -> String {
    let model_part = model_id.strip_prefix("model:").unwrap_or(model_id);
    if model_part.contains('|') {
        format!("model:{}", model_part)
    } else {
        format!("model:{}|{}", page_path, model_part)
    }
}

struct KeyModelAvailabilityEntry {
    value: serde_json::Value,
    used_fast_path: bool,
    used_fallback: bool,
    condition_group_count: usize,
}

fn build_key_model_availability_entry(
    graph: &dyn GraphReadStore,
    page_path: &str,
    model_id: &str,
) -> Option<KeyModelAvailabilityEntry> {
    let page_scoped_target = page_scoped_model_target(page_path, model_id);
    let scoped_result = crate::explain::build_explain_availability_fast_output(
        graph,
        &page_scoped_target,
        "compact",
    )
    .ok();
    let scoped_has_facts = scoped_result.as_ref().is_some_and(|result| {
        result
            .get("details")
            .and_then(|d| d.get("answer_facts"))
            .and_then(|a| a.get("availability_facts"))
            .is_some()
    });
    let (resolved_model_target, scope_warning, result, used_fast_path, used_fallback) =
        if scoped_has_facts {
            (
                page_scoped_target.clone(),
                None,
                scoped_result?,
                true,
                false,
            )
        } else {
            let result =
                crate::explain::build_explain_availability_fast_output(graph, model_id, "compact")
                    .ok()?;
            (
                model_id.to_string(),
                Some("page_scoped_target_not_resolved_fallback_to_global_model"),
                result,
                true,
                true,
            )
        };

    let (value, condition_group_count) = key_model_availability_value_from_result(
        model_id,
        &page_scoped_target,
        &resolved_model_target,
        scope_warning,
        &result,
    );

    Some(KeyModelAvailabilityEntry {
        value,
        used_fast_path,
        used_fallback,
        condition_group_count,
    })
}

fn key_model_availability_value_from_result(
    model_id: &str,
    page_scoped_target: &str,
    resolved_model_target: &str,
    scope_warning: Option<&str>,
    result: &serde_json::Value,
) -> (serde_json::Value, usize) {
    let direct_filters: Vec<serde_json::Value> = result
        .get("details")
        .and_then(|d| d.get("data_empty_gates"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("condition_scope").and_then(|v| v.as_str())
                        != Some("dataflow_internal")
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let availability_summary = result
        .get("details")
        .and_then(|d| d.get("answer_facts"))
        .and_then(|a| a.get("availability_facts"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let condition_group_count = result
        .get("details")
        .and_then(|d| d.get("data_empty_gates"))
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0)
        .max(1);
    let truncation = result
        .get("details")
        .and_then(|d| d.get("truncation_guard"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let empty_array = serde_json::Value::Array(Vec::new());
    let source_filters = availability_summary
        .get("source_filters")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let output_filters = availability_summary
        .get("output_filters")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let physical_inputs = availability_summary
        .get("physical_inputs")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let join_rules = availability_summary
        .get("join_rules")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let union_rules = availability_summary
        .get("union_rules")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());

    let mut row_semantics = Vec::new();
    let mut seen_semantics = std::collections::HashSet::new();
    for rule in join_rules
        .as_array()
        .into_iter()
        .flatten()
        .chain(union_rules.as_array().into_iter().flatten())
    {
        if let Some(row_semantic) = rule.get("row_semantic").and_then(|v| v.as_str()) {
            if seen_semantics.insert(row_semantic.to_string()) {
                row_semantics.push(row_semantic.to_string());
            }
        }
    }

    (
        serde_json::json!({
        "model_id": model_id,
        "page_scoped_target": page_scoped_target,
        "resolved_model_target": resolved_model_target,
        "scope_warning": scope_warning,
        "direct_filters": direct_filters,
        "dataflow_table": availability_summary.get("dataflow_table").cloned().unwrap_or(serde_json::Value::Null),
        "source_filters": source_filters,
        "output_filters": output_filters,
        "physical_inputs": physical_inputs,
        "join_rules": join_rules,
        "union_rules": union_rules,
        "row_semantics": row_semantics,
        "availability_summary": availability_summary,
        "truncation_guard": truncation,
        }),
        condition_group_count,
    )
}

fn build_key_model_availability_entry_from_context(
    graph: &dyn GraphReadStore,
    page_path: &str,
    model_id: &str,
    condition_cache: &mut crate::explain::ConditionCollectorCache,
) -> Option<KeyModelAvailabilityEntry> {
    let page_scoped_target = page_scoped_model_target(page_path, model_id);
    let scoped = crate::model_scope::resolve_model_target_in_page(graph, page_path, model_id);
    let (target_node, dataflow_model_id, resolved_model_target, scope_warning, used_fallback) =
        if let Some((target_node, _page_node, dataflow_model_id)) = scoped {
            (
                target_node,
                dataflow_model_id,
                page_scoped_target.clone(),
                None,
                false,
            )
        } else {
            let target_node = graph.get_node(model_id).ok().flatten()?;
            (
                target_node,
                None,
                model_id.to_string(),
                Some("page_scoped_target_not_resolved_fallback_to_global_model"),
                true,
            )
        };

    let mut blocking_conditions: Vec<serde_json::Value> = Vec::new();
    let mut data_empty_gates: Vec<serde_json::Value> = Vec::new();
    let mut supporting_context: Vec<serde_json::Value> = Vec::new();
    let mut related_context: Vec<serde_json::Value> = Vec::new();
    let mut seen_conditions: std::collections::HashSet<String> = std::collections::HashSet::new();

    let conds = condition_cache.collect_for_node(graph, &target_node.id, &mut seen_conditions);
    for cond_obj in conds {
        let source_file = cond_obj
            .get("source_file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let cond_obj =
            crate::explain::annotate_condition_scope(cond_obj, "direct", None, None, None);
        if source_file == page_path {
            let is_model_filter_dependency = cond_obj.get("owner_type").and_then(|v| v.as_str())
                == Some("ModelSource")
                && !crate::explain::condition_owned_by_node(&cond_obj, &target_node.id);
            if is_model_filter_dependency {
                let mut ctx = cond_obj;
                if let Some(obj) = ctx.as_object_mut() {
                    obj.insert(
                        "condition_scope".to_string(),
                        serde_json::json!("referenced_by_model_filter"),
                    );
                    obj.insert(
                        "note".to_string(),
                        serde_json::json!(
                            "该模型过滤条件引用目标组件，但不是目标组件自身或祖先显示条件"
                        ),
                    );
                }
                supporting_context.push(ctx);
                continue;
            }

            match crate::explain::classify_condition(&cond_obj) {
                "blocking" => blocking_conditions.push(cond_obj),
                "data_empty" => data_empty_gates.push(cond_obj),
                _ => supporting_context.push(cond_obj),
            }
        } else {
            let mut rc = cond_obj.clone();
            if let Some(obj) = rc.as_object_mut() {
                obj.insert("note".to_string(), serde_json::json!("非当前页面必要条件"));
            }
            related_context.push(rc);
        }
    }

    if target_node.node_type == crate::graph::NodeType::Model && used_fallback {
        let mut page_counts: HashMap<String, usize> = HashMap::new();
        for rc in &related_context {
            if let Some(file) = rc.get("source_file").and_then(|v| v.as_str()) {
                *page_counts.entry(file.to_string()).or_insert(0) += 1;
            }
        }
        let mut primary_page: Option<String> = None;
        let mut max_count = 0usize;
        for (file, count) in page_counts {
            if count > max_count {
                max_count = count;
                primary_page = Some(file);
            }
        }
        if let Some(primary_page) = primary_page {
            let mut remaining_related: Vec<serde_json::Value> = Vec::new();
            for mut rc in related_context {
                let rc_file = rc.get("source_file").and_then(|v| v.as_str()).unwrap_or("");
                if rc_file == primary_page {
                    if let Some(obj) = rc.as_object_mut() {
                        obj.remove("note");
                    }
                    match crate::explain::classify_condition(&rc) {
                        "blocking" => blocking_conditions.push(rc),
                        "data_empty" => data_empty_gates.push(rc),
                        _ => supporting_context.push(rc),
                    }
                } else {
                    remaining_related.push(rc);
                }
            }
            related_context = remaining_related;
        }
    }

    let expanded_gates = condition_cache.expand_total_row_count_gates(
        graph,
        &blocking_conditions,
        page_path,
        &mut seen_conditions,
    );
    data_empty_gates.extend(expanded_gates);

    if crate::model_scope::is_dataflow_model(&target_node) {
        if let Some(meta) = target_node.meta.as_ref() {
            let dfm = crate::query::DataFlowMeta::from_meta(meta);
            for (idx, projection) in dfm.project_filters().iter().enumerate() {
                let condition_id = format!(
                    "{}|{}|{}|{}",
                    target_node.id, projection.node_alias, projection.role, idx
                );
                let raw_expr = projection
                    .expr
                    .clone()
                    .or_else(|| {
                        Some(format!(
                            "{} {} {}",
                            projection.left.as_deref().unwrap_or(""),
                            projection.operator.as_deref().unwrap_or(""),
                            projection.right.as_deref().unwrap_or("")
                        ))
                    })
                    .unwrap_or_default();
                data_empty_gates.push(serde_json::json!({
                    "condition_id": condition_id,
                    "condition_type": "DataFlowFilter",
                    "condition_scope": "dataflow_internal",
                    "raw_expr": raw_expr,
                    "owner_node_id": target_node.id,
                    "node_alias": projection.node_alias,
                    "node_type": projection.node_type,
                    "role": projection.role,
                    "left": projection.left,
                    "operator": projection.operator,
                    "right": projection.right,
                    "referenced_fields": projection.referenced_fields,
                    "referenced_vars": projection.referenced_vars,
                    "source_file": target_node.path,
                    "json_path": format!("dataFlow.nodes.{}.filters", projection.node_alias),
                }));
            }
        }
    }

    let blocking_conditions = crate::explain::dedupe_conditions(blocking_conditions);
    let data_empty_gates = crate::explain::dedupe_conditions(data_empty_gates);
    let supporting_context = crate::explain::dedupe_conditions(supporting_context);
    let related_context = crate::explain::dedupe_conditions(related_context);
    let dataflow_meta = if crate::model_scope::is_dataflow_model(&target_node) {
        target_node
            .meta
            .as_ref()
            .map(crate::query::DataFlowMeta::from_meta)
    } else {
        None
    };
    let answer_facts = crate::explain::build_answer_facts(
        graph,
        crate::explain::TraversalIntent::Availability,
        &target_node,
        &blocking_conditions,
        &data_empty_gates,
        &None,
        &[],
        "compact",
        dataflow_meta.as_ref(),
        dataflow_model_id.as_deref(),
    );
    let truncation_guard = crate::answer_contract::build_truncation_guard(
        "compact",
        0,
        0,
        0,
        supporting_context.len(),
        related_context.len(),
    );
    let result = serde_json::json!({
        "details": {
            "answer_facts": answer_facts,
            "data_empty_gates": data_empty_gates,
            "truncation_guard": truncation_guard,
        }
    });
    let (value, condition_group_count) = key_model_availability_value_from_result(
        model_id,
        &page_scoped_target,
        &resolved_model_target,
        scope_warning,
        &result,
    );

    Some(KeyModelAvailabilityEntry {
        value,
        used_fast_path: true,
        used_fallback,
        condition_group_count,
    })
}

/// 单次 page logic availability 构造期间共享图读取结果。
struct AvailabilityGraphCache<'a> {
    inner: &'a dyn GraphReadStore,
    nodes: RefCell<HashMap<String, Option<crate::graph::Node>>>,
    edges: RefCell<HashMap<String, Option<GraphNeighbors>>>,
    node_cache_hits: Cell<usize>,
    edge_cache_hits: Cell<usize>,
}

impl<'a> AvailabilityGraphCache<'a> {
    fn new(inner: &'a dyn GraphReadStore) -> Self {
        Self {
            inner,
            nodes: RefCell::new(HashMap::new()),
            edges: RefCell::new(HashMap::new()),
            node_cache_hits: Cell::new(0),
            edge_cache_hits: Cell::new(0),
        }
    }

    fn cache_hits(&self) -> usize {
        self.node_cache_hits.get() + self.edge_cache_hits.get()
    }
}

impl GraphReadStore for AvailabilityGraphCache<'_> {
    fn get_node(
        &self,
        node_id: &str,
    ) -> crate::graph_store::GraphStoreResult<Option<crate::graph::Node>> {
        if let Some(node) = self.nodes.borrow().get(node_id).cloned() {
            self.node_cache_hits.set(self.node_cache_hits.get() + 1);
            return Ok(node);
        }
        let node = self.inner.get_node(node_id)?;
        self.nodes
            .borrow_mut()
            .insert(node_id.to_string(), node.clone());
        Ok(node)
    }

    fn get_node_edges(
        &self,
        node_id: &str,
    ) -> crate::graph_store::GraphStoreResult<Option<GraphNeighbors>> {
        if let Some(neighbors) = self.edges.borrow().get(node_id).cloned() {
            self.edge_cache_hits.set(self.edge_cache_hits.get() + 1);
            return Ok(neighbors);
        }
        let neighbors = self.inner.get_node_edges(node_id)?;
        self.edges
            .borrow_mut()
            .insert(node_id.to_string(), neighbors.clone());
        Ok(neighbors)
    }

    fn node_count(&self) -> crate::graph_store::GraphStoreResult<usize> {
        self.inner.node_count()
    }

    fn edge_count(&self) -> crate::graph_store::GraphStoreResult<usize> {
        self.inner.edge_count()
    }

    fn iter_nodes(
        &self,
    ) -> crate::graph_store::GraphStoreResult<Box<dyn Iterator<Item = crate::graph::Node> + '_>>
    {
        self.inner.iter_nodes()
    }
}

/// 页面内 key model availability 批量构造结果。
#[derive(Clone)]
struct KeyModelAvailabilityBatch {
    entries: Vec<serde_json::Value>,
    fast_path_count: usize,
    fallback_count: usize,
    condition_group_count: usize,
    graph_cache_hits: usize,
}

/// 预热后的页面 availability 读模型缓存。
#[derive(Clone)]
pub struct PageLogicAvailabilityCache {
    page_id: String,
    budget: String,
    batch: KeyModelAvailabilityBatch,
    warm_stages: BTreeMap<String, u128>,
}

impl PageLogicAvailabilityCache {
    /// 判断缓存是否适用于当前 page/budget。
    fn matches(&self, page_id: &str, budget: &str) -> bool {
        self.page_id == page_id && self.budget == budget
    }

    /// 将缓存投影为页面逻辑内部使用的 batch 结构。
    fn project(&self) -> KeyModelAvailabilityBatch {
        self.batch.clone()
    }

    /// 查询 warm 阶段耗时。
    pub fn warm_stage_ms(&self, name: &str) -> Option<u128> {
        self.warm_stages.get(name).copied()
    }
}

/// 页面级 key model availability 派生读模型。
struct PageAvailabilityIndex {
    page_id: String,
    page_path: String,
    key_models: Vec<String>,
    model_facts: Vec<serde_json::Value>,
    fast_path_count: usize,
    fallback_count: usize,
    condition_group_count: usize,
    graph_cache_hits: usize,
}

impl PageAvailabilityIndex {
    /// 从现有 batch fast path 构建第一版 availability 读模型。
    fn build(
        graph: &dyn GraphReadStore,
        page_id: &str,
        page_path: &str,
        key_model_ids: &[String],
        availability_limit: Option<usize>,
    ) -> Self {
        let batch =
            build_key_model_availability_batch(graph, page_path, key_model_ids, availability_limit);
        Self {
            page_id: page_id.to_string(),
            page_path: page_path.to_string(),
            key_models: key_model_ids.to_vec(),
            model_facts: batch.entries,
            fast_path_count: batch.fast_path_count,
            fallback_count: batch.fallback_count,
            condition_group_count: batch.condition_group_count,
            graph_cache_hits: batch.graph_cache_hits,
        }
    }

    /// 将读模型投影回现有 page logic 输出结构。
    fn project(self) -> KeyModelAvailabilityBatch {
        debug_assert!(
            !self.page_id.is_empty(),
            "availability index page id missing"
        );
        debug_assert!(
            !self.page_path.is_empty(),
            "availability index page path missing"
        );
        debug_assert!(
            self.model_facts.len() <= self.key_models.len(),
            "availability projection cannot exceed key model count"
        );
        KeyModelAvailabilityBatch {
            entries: self.model_facts,
            fast_path_count: self.fast_path_count,
            fallback_count: self.fallback_count,
            condition_group_count: self.condition_group_count,
            graph_cache_hits: self.graph_cache_hits,
        }
    }
}

/// 在单次 page logic 调用内批量构造 key model availability，并共享图读取缓存。
fn build_key_model_availability_batch(
    graph: &dyn GraphReadStore,
    page_path: &str,
    key_model_ids: &[String],
    availability_limit: Option<usize>,
) -> KeyModelAvailabilityBatch {
    let cached_graph = AvailabilityGraphCache::new(graph);
    let mut entries = Vec::new();
    let mut fast_path_count = 0usize;
    let mut fallback_count = 0usize;
    let mut condition_group_count = 0usize;
    let mut condition_cache = crate::explain::ConditionCollectorCache::new();

    for model_id in key_model_ids {
        if availability_limit.is_some_and(|limit| entries.len() >= limit) {
            break;
        }
        let entry = build_key_model_availability_entry_from_context(
            &cached_graph,
            page_path,
            model_id,
            &mut condition_cache,
        )
        .or_else(|| build_key_model_availability_entry(&cached_graph, page_path, model_id));
        if let Some(entry) = entry {
            if entry.used_fast_path {
                fast_path_count += 1;
            }
            if entry.used_fallback {
                fallback_count += 1;
            }
            condition_group_count += entry.condition_group_count;
            entries.push(entry.value);
        }
    }

    KeyModelAvailabilityBatch {
        entries,
        fast_path_count,
        fallback_count,
        condition_group_count,
        graph_cache_hits: cached_graph.cache_hits() + condition_cache.cache_hits(),
    }
}

/// 构造 compact key model availability 截断结构，保留完整 key model 总数。
fn compact_key_model_availability_details(
    key_model_availability: &[serde_json::Value],
    total_count: usize,
) -> serde_json::Value {
    let shown_count = key_model_availability.len();
    let remaining_count = total_count.saturating_sub(shown_count);
    serde_json::json!({
        "total_count": total_count,
        "shown_count": shown_count,
        "truncated": remaining_count > 0,
        "remaining_count": remaining_count,
        "items": key_model_availability,
    })
}

/// 查询页面级逻辑摘要
///
/// 输出 page_inputs、data_sources、write_targets、entrypoints、action_flows、visibility_rules、navigation、risk_diagnostics。
pub fn build_query_page_logic_output(
    graph: &dyn GraphReadStore,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<serde_json::Value> {
    build_query_page_logic_output_inner(graph, None, None, page_id, project_dir, budget, None)
}

/// 查询页面级逻辑摘要，并允许复用预构建稠密图快照。
pub fn build_query_page_logic_output_with_dense_snapshot(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<serde_json::Value> {
    build_query_page_logic_output_inner(
        graph,
        dense_snapshot,
        None,
        page_id,
        project_dir,
        budget,
        None,
    )
}

/// 查询页面级逻辑摘要，并允许复用预热 availability 读模型。
pub fn build_query_page_logic_output_with_availability_cache(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    availability_cache: Option<&PageLogicAvailabilityCache>,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<serde_json::Value> {
    build_query_page_logic_output_inner(
        graph,
        dense_snapshot,
        availability_cache,
        page_id,
        project_dir,
        budget,
        None,
    )
}

/// 查询页面级逻辑摘要，并返回 M51 profiling 归因数据。
///
/// 该 API 供 benchmark / profiling runner 使用，不改变默认产品输出。
pub fn build_query_page_logic_output_profiled(
    graph: &dyn GraphReadStore,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<(serde_json::Value, PerfProfile)> {
    build_query_page_logic_output_profiled_with_dense_snapshot(
        graph,
        None,
        page_id,
        project_dir,
        budget,
    )
}

/// 查询页面级逻辑摘要，并允许复用预构建稠密图快照进行 profiling。
///
/// 该 API 只用于 runtime/load 期预构建快照的性能实验；默认产品路径不会在 query-time 构建快照。
pub fn build_query_page_logic_output_profiled_with_dense_snapshot(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<(serde_json::Value, PerfProfile)> {
    let mut profile = PerfProfile::new("query_page_logic");
    let output = build_query_page_logic_output_inner(
        graph,
        dense_snapshot,
        None,
        page_id,
        project_dir,
        budget,
        Some(&mut profile),
    )?;
    profile.finish();
    Ok((output, profile))
}

/// 查询页面级逻辑摘要，并允许复用预热 availability 读模型进行 profiling。
///
/// 该 API 用于验证把 key model availability 从 query-time 前移到 init/warm 阶段后的收益。
pub fn build_query_page_logic_output_profiled_with_availability_cache(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    availability_cache: Option<&PageLogicAvailabilityCache>,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<(serde_json::Value, PerfProfile)> {
    let mut profile = PerfProfile::new("query_page_logic");
    let output = build_query_page_logic_output_inner(
        graph,
        dense_snapshot,
        availability_cache,
        page_id,
        project_dir,
        budget,
        Some(&mut profile),
    )?;
    profile.finish();
    Ok((output, profile))
}

/// 预热页面 availability 读模型缓存。
///
/// 第一版通过现有 page logic 输出提取 availability 投影，用于验证“query-time 构造成本前移”。
pub fn build_page_logic_availability_cache(
    graph: &dyn GraphReadStore,
    page_id: &str,
    _project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<PageLogicAvailabilityCache> {
    let mut warm_stages = BTreeMap::new();
    let target_started = Instant::now();
    let Some(page_node) = graph.get_node(page_id)? else {
        return Ok(PageLogicAvailabilityCache {
            page_id: page_id.to_string(),
            budget: budget.to_string(),
            batch: KeyModelAvailabilityBatch {
                entries: Vec::new(),
                fast_path_count: 0,
                fallback_count: 0,
                condition_group_count: 0,
                graph_cache_hits: 0,
            },
            warm_stages,
        });
    };
    warm_stages.insert(
        "target_lookup".to_string(),
        target_started.elapsed().as_millis(),
    );

    let collect_started = Instant::now();
    let graph_collect::PageLogicNodes {
        child_components,
        child_actions,
    } = graph_collect::collect_page_logic_nodes(graph, page_id)?;
    warm_stages.insert(
        "collect_page_nodes".to_string(),
        collect_started.elapsed().as_millis(),
    );

    let edge_started = Instant::now();
    let (data_sources, write_targets) =
        collect_availability_model_edges(graph, &child_components, &child_actions)?;
    warm_stages.insert(
        "availability_model_edge_scan".to_string(),
        edge_started.elapsed().as_millis(),
    );

    let prerequisites_started = Instant::now();
    let mut ignored_profile = None;
    let prerequisites::PagePrerequisites {
        display_prerequisites,
        data_prerequisites,
        action_prerequisites: _,
    } = prerequisites::collect_page_prerequisites(
        graph,
        &page_node.path,
        &child_components,
        &child_actions,
        &data_sources,
        &mut ignored_profile,
    )?;
    warm_stages.insert(
        "availability_prerequisites".to_string(),
        prerequisites_started.elapsed().as_millis(),
    );

    let key_model_started = Instant::now();
    let key_model_ids = collect_availability_key_model_ids(
        &data_sources,
        &write_targets,
        &display_prerequisites,
        &data_prerequisites,
    );
    warm_stages.insert(
        "availability_key_models".to_string(),
        key_model_started.elapsed().as_millis(),
    );

    let materialize_started = Instant::now();
    let availability_limit = if budget == "compact" { Some(3) } else { None };
    let batch = PageAvailabilityIndex::build(
        graph,
        page_id,
        &page_node.path,
        &key_model_ids,
        availability_limit,
    )
    .project();
    warm_stages.insert(
        "availability_materialize".to_string(),
        materialize_started.elapsed().as_millis(),
    );

    Ok(PageLogicAvailabilityCache {
        page_id: page_id.to_string(),
        budget: budget.to_string(),
        batch,
        warm_stages,
    })
}

fn collect_availability_model_edges(
    graph: &dyn GraphReadStore,
    child_components: &[crate::graph::Node],
    child_actions: &[crate::graph::Node],
) -> Result<(Vec<serde_json::Value>, Vec<serde_json::Value>)> {
    let mut data_sources = Vec::new();
    let mut write_targets = Vec::new();
    for node in child_components.iter().chain(child_actions.iter()) {
        let Some(neighbors) = graph.get_node_edges(&node.id)? else {
            continue;
        };
        for edge_view in neighbors.outgoing {
            let target = edge_view.node;
            let edge = edge_view.edge;
            match edge.edge_type {
                crate::graph::EdgeType::Reads
                | crate::graph::EdgeType::ActionReads
                | crate::graph::EdgeType::ActionValidates
                | crate::graph::EdgeType::ActionLoadsData => {
                    data_sources.push(json!({
                        "target_id": target.id,
                    }));
                }
                crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                    write_targets.push(json!({
                        "target_id": target.id,
                    }));
                }
                _ => {}
            }
        }
    }
    Ok((data_sources, write_targets))
}

fn collect_availability_key_model_ids(
    data_sources: &[serde_json::Value],
    write_targets: &[serde_json::Value],
    display_prerequisites: &[serde_json::Value],
    data_prerequisites: &[serde_json::Value],
) -> Vec<String> {
    let mut key_model_ids = Vec::new();
    let mut seen_models: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ds in data_sources {
        if let Some(target_id) = ds.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    for wt in write_targets {
        if let Some(target_id) = wt.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    for prereq in display_prerequisites
        .iter()
        .chain(data_prerequisites.iter())
    {
        if let Some(depends_on) = prereq.get("depends_on").and_then(|v| v.as_array()) {
            for dep in depends_on {
                let Some(dep) = dep.as_str() else {
                    continue;
                };
                let Some(model_id) = model_id_from_reference(dep) else {
                    continue;
                };
                if seen_models.insert(model_id.clone()) {
                    key_model_ids.push(model_id);
                }
            }
        }
    }
    key_model_ids
}

fn build_query_page_logic_output_inner(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    availability_cache: Option<&PageLogicAvailabilityCache>,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
    mut profile: Option<&mut PerfProfile>,
) -> Result<serde_json::Value> {
    let is_compact = budget == "compact";
    let is_full = budget == "full";
    let stage_started = Instant::now();
    let page_node = match graph.get_node(page_id)? {
        Some(n) => n,
        None => {
            record_profile_stage(&mut profile, "target_lookup", stage_started);
            let candidates = find_candidates(graph, page_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::PageQuery,
                page_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };
    record_profile_stage(&mut profile, "target_lookup", stage_started);
    let mut edge_cache: HashMap<String, Option<GraphNeighbors>> = HashMap::new();

    // ---- 1. 递归收集页面下所有 Component 节点，再收集它们 Triggers 出的 Action 节点 ----
    let stage_started = Instant::now();
    let graph_collect::PageLogicNodes {
        child_components,
        child_actions,
    } = graph_collect::collect_page_logic_nodes(graph, page_id)?;
    record_profile_stage(&mut profile, "collect_page_nodes", stage_started);
    set_profile_counter(&mut profile, "child_components", child_components.len());
    set_profile_counter(&mut profile, "child_actions", child_actions.len());
    let child_component_refs: Vec<&crate::graph::Node> = child_components.iter().collect();
    let child_action_refs: Vec<&crate::graph::Node> = child_actions.iter().collect();

    // ---- 2. 从原始文件读取：递归收集组件元数据、action 元数据、visibility_rules ----
    let stage_started = Instant::now();
    let metadata::PageFileMetadata {
        page_inputs,
        visibility_rules,
        from_file,
        component_json_paths,
        action_meta,
    } = metadata::load_page_file_metadata(project_dir, &page_node.path);
    record_profile_stage(&mut profile, "load_page_metadata", stage_started);
    set_profile_counter(&mut profile, "page_inputs", page_inputs.len());
    set_profile_counter(&mut profile, "visibility_rules", visibility_rules.len());

    // ---- 3. Entrypoints：只包含用户可触发组件（有 action 的 button/link 等） ----
    let stage_started = Instant::now();
    let mut entrypoints: Vec<serde_json::Value> = Vec::new();
    for comp in &child_components {
        if let Some(neighbors) = cached_node_edges(graph, &mut edge_cache, &comp.id)? {
            add_profile_counter(&mut profile, "edges_scanned", neighbors.outgoing.len());
            let has_trigger = neighbors.outgoing.iter().any(|edge_view| {
                matches!(edge_view.edge.edge_type, crate::graph::EdgeType::Triggers)
            });
            if has_trigger {
                let comp_type = comp
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("component_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("component");
                let component_id = component_short_id(&comp.id);
                let json_path = component_json_paths
                    .get(component_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        format!("canvas.components[id='{}']", component_short_id(&comp.id))
                    });
                entrypoints.push(json!({
                    "id": comp.id,
                    "name": comp.name,
                    "type": comp_type,
                    "source_file": comp.path,
                    "node_id": comp.id,
                    "edge_type": "Triggers",
                    "raw_expr": serde_json::Value::Null,
                    "json_path": format!("{}.actions[*]", json_path),
                }));
            }
        }
    }
    record_profile_stage(&mut profile, "entrypoint_scan", stage_started);
    set_profile_counter(&mut profile, "entrypoints", entrypoints.len());

    // ---- 4. 收集所有 Component & Action 的出边，用于 data_sources / write_targets / navigation ----
    let stage_started = Instant::now();
    let mut data_sources: Vec<serde_json::Value> = Vec::new();
    let mut write_targets: Vec<serde_json::Value> = Vec::new();
    let mut navigation: Vec<serde_json::Value> = Vec::new();
    let mut action_flows: Vec<serde_json::Value> = Vec::new();

    let mut all_nodes: Vec<&crate::graph::Node> = Vec::new();
    all_nodes.extend(child_components.iter());
    all_nodes.extend(child_actions.iter());

    for node in &all_nodes {
        if let Some(neighbors) = cached_node_edges(graph, &mut edge_cache, &node.id)? {
            add_profile_counter(&mut profile, "edges_scanned", neighbors.outgoing.len());
            for edge_view in &neighbors.outgoing {
                let target = &edge_view.node;
                let edge = &edge_view.edge;
                match edge.edge_type {
                    crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                        let default_json_path =
                            if matches!(node.node_type, crate::graph::NodeType::Action) {
                                let (atype, aid) = node
                                    .name
                                    .split_once(':')
                                    .map(|(t, i)| (t.to_string(), i.to_string()))
                                    .unwrap_or_else(|| (node.name.clone(), node.name.clone()));
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.{}",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        &aid
                                    ),
                                    if atype == "setParamValue" || atype == "link" {
                                        "params[*].value"
                                    } else {
                                        "fieldValues[*].value"
                                    }
                                )
                            } else {
                                format!(
                                    "canvas.components[id='{}'].<expression>",
                                    component_short_id(&node.id)
                                )
                            };
                        data_sources.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or(default_json_path),
                        }));
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        let default_json_path =
                            if matches!(node.node_type, crate::graph::NodeType::Action) {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.fieldValues[*].value",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            } else {
                                format!(
                                    "canvas.components[id='{}'].submitField",
                                    component_short_id(&node.id)
                                )
                            };
                        write_targets.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or(default_json_path),
                        }));
                    }
                    crate::graph::EdgeType::OpensPage
                    | crate::graph::EdgeType::ActionNavigates
                    | crate::graph::EdgeType::EmbedsPage => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                if matches!(node.node_type, crate::graph::NodeType::Action) {
                                    let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                    let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                    format!(
                                        "{}.path",
                                        action_json_path(
                                            &format!("comp:{}|{}", page_node.path, parent_component),
                                            aid
                                        )
                                    )
                                } else {
                                    format!("canvas.components[id='{}'].resPath", component_short_id(&node.id))
                                }
                            }),
                        }));
                    }
                    crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::PassesParam => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                if matches!(node.node_type, crate::graph::NodeType::Action) {
                                    let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                    let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                    format!(
                                        "{}.params[*].value",
                                        action_json_path(
                                            &format!("comp:{}|{}", page_node.path, parent_component),
                                            aid
                                        )
                                    )
                                } else {
                                    format!(
                                        "canvas.components[id='{}'].params[*].value",
                                        component_short_id(&node.id)
                                    )
                                }
                            }),
                        }));
                    }
                    crate::graph::EdgeType::ActionValidates
                    | crate::graph::EdgeType::ActionLoadsData => {
                        data_sources.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "type": format!("{:?}", edge.edge_type),
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.<derived-target>",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            }),
                        }));
                    }
                    crate::graph::EdgeType::ActionControlsComponent => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.targetComponent[*]",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            }),
                        }));
                    }
                    _ => {}
                }
            }
        }
    }
    record_profile_stage(&mut profile, "edge_scan", stage_started);
    set_profile_counter(&mut profile, "data_sources", data_sources.len());
    set_profile_counter(&mut profile, "write_targets", write_targets.len());
    set_profile_counter(&mut profile, "navigation", navigation.len());

    // ---- 5. Action flows：遍历 Action 节点，聚合 reads/writes/navigation ----
    let stage_started = Instant::now();
    for action in &child_actions {
        let action_neighbors = cached_node_edges(graph, &mut edge_cache, &action.id)?;
        let (action_out, action_in) = match action_neighbors.as_ref() {
            Some(neighbors) => (&neighbors.outgoing, &neighbors.incoming),
            None => {
                action_flows.push(json!({
                    "action_id": action.id,
                    "action_type": action.name,
                    "triggered_by": serde_json::Value::Null,
                    "reads": [],
                    "writes": [],
                    "navigation": []
                }));
                continue;
            }
        };
        add_profile_counter(
            &mut profile,
            "edges_scanned",
            action_out.len() + action_in.len(),
        );

        let (action_type, action_id) = action
            .name
            .split_once(':')
            .map(|(t, i)| (t.to_string(), i.to_string()))
            .unwrap_or_else(|| (action.name.clone(), action.name.clone()));

        let parent_component = action_in.iter().find_map(|edge_view| {
            let src = &edge_view.node;
            let e = &edge_view.edge;
            if matches!(e.edge_type, crate::graph::EdgeType::Triggers)
                && matches!(src.node_type, crate::graph::NodeType::Component)
            {
                Some(src.id.clone())
            } else {
                None
            }
        });

        let mut reads = Vec::new();
        let mut writes = Vec::new();
        let mut nav = Vec::new();
        let mut sets_params = Vec::new();
        let mut passes_params = Vec::new();

        for edge_view in action_out.iter() {
            let target = &edge_view.node;
            let edge = &edge_view.edge;
            match edge.edge_type {
                crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                    writes.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::OpensPage
                | crate::graph::EdgeType::ActionNavigates
                | crate::graph::EdgeType::EmbedsPage => {
                    nav.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "type": format!("{:?}", edge.edge_type),
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::ActionSetsParam => {
                    sets_params.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "field_path": edge.field_path,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::PassesParam => {
                    passes_params.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "field_path": edge.field_path,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionValidates => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "validate": true,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionLoadsData => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "load": true,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionControlsComponent => {
                    nav.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "type": format!("{:?}", edge.edge_type),
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                _ => {}
            }
        }

        // 用 parent_component 提取裸组件名构建 lookup key，避免同名 action 串线
        let lookup_key = parent_component
            .as_deref()
            .and_then(|cid| cid.split('|').next_back())
            .map(|comp_name| format!("{}|{}", comp_name, action_id))
            .unwrap_or_else(|| action_id.clone());
        let trigger_type = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("trigger_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("click")
            .to_string();
        let wait_prev = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("wait_prev"))
            .cloned();
        let action_path_hint = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("json_path"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                action_json_path(parent_component.as_deref().unwrap_or("?"), &action_id)
            });
        let action_source_file = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("source_file"))
            .and_then(|v| v.as_str())
            .unwrap_or(action.path.as_str())
            .to_string();
        let condition_raw = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("condition"))
            .and_then(|v| v.as_str());

        let action_category = crate::action_semantics::classify_action(&action_type);
        let comp_name = parent_component
            .as_deref()
            .and_then(|cid| cid.split('|').next_back())
            .unwrap_or("?");
        let semantic_summary = crate::action_semantics::build_semantic_summary(
            &action_type,
            comp_name,
            &action_id,
            &reads,
            &writes,
            &nav,
        );
        let blocks_on =
            crate::action_semantics::parse_wait_prev(wait_prev.as_ref().and_then(|v| v.as_str()));
        let condition_struct = crate::action_semantics::parse_condition(condition_raw);

        // 执行模型字段推断
        let wait_status = if wait_prev.as_ref().and_then(|v| v.as_str()).is_some() {
            "blocking"
        } else {
            "none"
        };
        let may_interrupt = condition_raw.is_some();
        let failure_behavior = if condition_raw.is_some() {
            "skip_if_condition_fails"
        } else {
            "proceed"
        };

        action_flows.push(json!({
            "action_id": action_id,
            "action_type": action_type,
            "action_category": action_category,
            "semantic_summary": semantic_summary,
            "component_id": parent_component,
            "trigger_type": trigger_type,
            "blocks_on": blocks_on,
            "wait_status": wait_status,
            "condition": condition_struct,
            "may_interrupt": may_interrupt,
            "failure_behavior": failure_behavior,
            "reads": reads,
            "writes": writes,
            "navigation": nav,
            "sets_params": sets_params,
            "passes_params": passes_params,
            "source_file": action_source_file,
            "node_id": action.id,
            "json_path": action_path_hint,
        }));
    }
    record_profile_stage(&mut profile, "action_flow_build", stage_started);
    set_profile_counter(&mut profile, "action_flows", action_flows.len());

    // ---- 5.5 收集页面级条件前置条件（M19） ----
    let stage_started = Instant::now();
    let prerequisites::PagePrerequisites {
        display_prerequisites,
        data_prerequisites,
        action_prerequisites,
    } = prerequisites::collect_page_prerequisites(
        graph,
        &page_node.path,
        &child_components,
        &child_actions,
        &data_sources,
        &mut profile,
    )?;
    record_profile_stage(&mut profile, "prerequisites", stage_started);
    set_profile_counter(
        &mut profile,
        "display_prerequisites",
        display_prerequisites.len(),
    );
    set_profile_counter(&mut profile, "data_prerequisites", data_prerequisites.len());
    set_profile_counter(
        &mut profile,
        "action_prerequisites",
        action_prerequisites.len(),
    );

    // ---- 5.6 主链路抽取（M19.5）—— 使用路径计算领域模型 ----
    let stage_started = Instant::now();
    let path_summary::PageLogicPaths {
        mut primary_paths,
        mut related_context,
        candidate_paths,
        supporting_paths,
        rejected_paths,
        path_selection_diagnostics,
    } = path_summary::build_page_logic_paths(
        graph,
        dense_snapshot,
        page_id,
        &page_node,
        &child_component_refs,
        &child_action_refs,
        &data_sources,
        &write_targets,
        &entrypoints,
        &mut profile,
    );
    record_profile_stage(&mut profile, "path_summary", stage_started);
    set_profile_counter(&mut profile, "primary_paths", primary_paths.len());
    set_profile_counter(&mut profile, "related_context", related_context.len());
    set_profile_counter(&mut profile, "candidate_paths", candidate_paths.len());
    set_profile_counter(&mut profile, "supporting_paths", supporting_paths.len());
    set_profile_counter(&mut profile, "rejected_paths", rejected_paths.len());

    // ---- 6. Risk diagnostics ----
    let stage_started = Instant::now();
    let evidence_sample_limit = diagnostics::EVIDENCE_SAMPLE_LIMIT;
    let diagnostics::PageLogicDiagnostics {
        diagnostics,
        related_context_summary,
    } = {
        let graph_store: &dyn GraphReadStore = graph;
        diagnostics::build_page_logic_diagnostics(
            graph_store,
            page_id,
            &page_node,
            from_file,
            &entrypoints,
            &data_sources,
            &write_targets,
            &navigation,
            &visibility_rules,
            &action_flows,
            &mut primary_paths,
            &mut related_context,
        )
    };
    record_profile_stage(&mut profile, "diagnostics", stage_started);
    set_profile_counter(&mut profile, "diagnostics", diagnostics.len());

    // ---- 7. Summary & page_role ----
    let page_role = if entrypoints.is_empty() {
        "readonly_dashboard"
    } else if !write_targets.is_empty() && !navigation.is_empty() {
        "mixed_interaction_page"
    } else if !write_targets.is_empty() {
        "data_maintenance_page"
    } else if !navigation.is_empty() {
        "navigation_page"
    } else {
        "unknown"
    };

    let what_is_it = format!(
        "页面 {}，{} 个用户入口，读取 {} 个数据源，写入 {} 个目标，{} 个跳转",
        page_node.name,
        entrypoints.len(),
        data_sources.len(),
        write_targets.len(),
        navigation.len()
    );

    // Build top-N lists for brief mode
    let top_entrypoints: Vec<serde_json::Value> = entrypoints.iter().take(3).cloned().collect();
    let top_data_sources: Vec<serde_json::Value> = data_sources.iter().take(3).cloned().collect();
    let top_writes: Vec<serde_json::Value> = write_targets.iter().take(3).cloned().collect();
    let top_navigation: Vec<serde_json::Value> = navigation.iter().take(3).cloned().collect();

    // Build top-N prerequisite lists for readable summary
    let top_display_prerequisites: Vec<serde_json::Value> =
        display_prerequisites.iter().take(3).cloned().collect();
    let top_data_prerequisites: Vec<serde_json::Value> =
        data_prerequisites.iter().take(3).cloned().collect();
    let top_action_prerequisites: Vec<serde_json::Value> =
        action_prerequisites.iter().take(3).cloned().collect();
    // key_primary_paths 从已排序的 primary_paths 中取前 10 条（已按重要性排序）
    let key_primary_paths: Vec<serde_json::Value> =
        primary_paths.iter().take(10).cloned().collect();

    // M35.7: 从 data_sources 和 write_targets 中自动发现关键模型，
    // 并内嵌每个模型的 availability 摘要。
    let availability_stage_started = Instant::now();
    let mut key_model_ids: Vec<String> = Vec::new();
    let mut seen_models: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ds in &data_sources {
        if let Some(target_id) = ds.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    for wt in &write_targets {
        if let Some(target_id) = wt.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    // 同时纳入显示/数据前置条件中引用的模型，避免只看 data_sources 漏掉 totalRowCount__ 类条件。
    for prereq in display_prerequisites
        .iter()
        .chain(data_prerequisites.iter())
    {
        if let Some(depends_on) = prereq.get("depends_on").and_then(|v| v.as_array()) {
            for dep in depends_on {
                let Some(dep) = dep.as_str() else {
                    continue;
                };
                let Some(model_id) = model_id_from_reference(dep) else {
                    continue;
                };
                if seen_models.insert(model_id.clone()) {
                    key_model_ids.push(model_id);
                }
            }
        }
    }

    let key_model_availability_limit = if is_compact { Some(3) } else { None };
    let cached_availability = availability_cache.filter(|cache| cache.matches(page_id, budget));
    let availability_read_model_used = cached_availability.is_some();
    let (availability_batch, availability_index_build_ms, availability_index_projection_ms) =
        if let Some(cache) = cached_availability {
            let projection_started = Instant::now();
            let batch = cache.project();
            (
                batch,
                0,
                (projection_started.elapsed().as_millis() as usize).max(1),
            )
        } else {
            let index_build_started = Instant::now();
            let availability_index = PageAvailabilityIndex::build(
                graph,
                page_id,
                &page_node.path,
                &key_model_ids,
                key_model_availability_limit,
            );
            let build_ms = (index_build_started.elapsed().as_millis() as usize).max(1);
            let projection_started = Instant::now();
            let batch = availability_index.project();
            let projection_ms = (projection_started.elapsed().as_millis() as usize).max(1);
            (batch, build_ms, projection_ms)
        };
    let availability_context_build_ms = availability_index_build_ms;
    let key_model_availability = availability_batch.entries;
    record_profile_stage(
        &mut profile,
        "key_model_availability",
        availability_stage_started,
    );
    set_profile_counter(&mut profile, "key_models", key_model_ids.len());
    set_profile_counter(
        &mut profile,
        "key_model_availability",
        key_model_availability.len(),
    );
    set_profile_counter(
        &mut profile,
        "key_model_availability_fast_path",
        availability_batch.fast_path_count,
    );
    set_profile_counter(
        &mut profile,
        "availability_context_build_ms",
        availability_context_build_ms,
    );
    set_profile_counter(
        &mut profile,
        "availability_index_build_ms",
        availability_index_build_ms,
    );
    set_profile_counter(
        &mut profile,
        "availability_index_projection_ms",
        availability_index_projection_ms,
    );
    set_profile_counter(
        &mut profile,
        "availability_index_fallback_models",
        availability_batch.fallback_count,
    );
    set_profile_counter(
        &mut profile,
        "availability_read_model_used",
        usize::from(availability_read_model_used),
    );
    set_profile_counter(
        &mut profile,
        "availability_models_batched",
        key_model_availability.len(),
    );
    set_profile_counter(
        &mut profile,
        "availability_fallback_models",
        availability_batch.fallback_count,
    );
    set_profile_counter(
        &mut profile,
        "availability_dataflow_meta_cache_hits",
        availability_batch.graph_cache_hits,
    );
    set_profile_counter(
        &mut profile,
        "availability_condition_groups",
        availability_batch.condition_group_count,
    );

    let output_stage_started = Instant::now();
    let summary = serde_json::json!({
        "page_id": page_id,
        "page_name": page_node.name,
        "what_is_it": what_is_it,
        "page_role": page_role,
        "entrypoint_count": entrypoints.len(),
        "data_source_count": data_sources.len(),
        "write_target_count": write_targets.len(),
        "navigation_count": navigation.len(),
        "risk_count": diagnostics.len(),
        "top_entrypoints": top_entrypoints,
        "top_data_sources": top_data_sources,
        "top_writes": top_writes,
        "top_navigation": top_navigation,
        "display_prerequisites_count": display_prerequisites.len(),
        "data_prerequisites_count": data_prerequisites.len(),
        "action_prerequisites_count": action_prerequisites.len(),
        "primary_paths_count": primary_paths.len(),
        "related_context_count": related_context.len(),
        "top_display_prerequisites": top_display_prerequisites,
        "top_data_prerequisites": top_data_prerequisites,
        "top_action_prerequisites": top_action_prerequisites,
        "key_primary_paths": key_primary_paths,
        "key_model_count": key_model_ids.len(),
    });

    let risk_diagnostics: Vec<serde_json::Value> = diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "severity": format!("{:?}", d.severity),
                "code": d.code,
                "message": d.message,
                "location": d.location,
                "suggestion": d.suggestion,
            })
        })
        .collect();

    // M19-FIX: budget 差异输出
    let details = if is_compact {
        serde_json::json!({
            "page_inputs": crate::output::brief::truncated_array(&page_inputs, 5),
            "data_sources": crate::output::brief::truncated_array(&data_sources, 5),
            "write_targets": crate::output::brief::truncated_array(&write_targets, 5),
            "entrypoints": crate::output::brief::truncated_array(&entrypoints, 5),
            "action_flows": crate::output::brief::truncated_array(&action_flows, 5),
            "visibility_rules": crate::output::brief::truncated_array(&visibility_rules, 5),
            "navigation": crate::output::brief::truncated_array(&navigation, 5),
            "risk_diagnostics": crate::output::brief::truncated_array(&risk_diagnostics, 10),
            "display_prerequisites": crate::output::brief::truncated_array(&display_prerequisites, 5),
            "data_prerequisites": crate::output::brief::truncated_array(&data_prerequisites, 5),
            "action_prerequisites": crate::output::brief::truncated_array(&action_prerequisites, 5),
            "primary_paths": crate::output::brief::truncated_array(&primary_paths, 5),
            "related_context": crate::output::brief::truncated_array(&related_context, 3),
            "related_context_summary": related_context_summary.clone(),
            "key_model_availability": compact_key_model_availability_details(&key_model_availability, key_model_ids.len()),
        })
    } else if is_full {
        // full 模式：输出完整路径分类 + rejected + diagnostics
        serde_json::json!({
            "page_inputs": page_inputs.clone(),
            "data_sources": data_sources.clone(),
            "write_targets": write_targets.clone(),
            "entrypoints": entrypoints.clone(),
            "action_flows": action_flows.clone(),
            "visibility_rules": visibility_rules.clone(),
            "navigation": navigation.clone(),
            "risk_diagnostics": risk_diagnostics,
            "display_prerequisites": display_prerequisites.clone(),
            "data_prerequisites": data_prerequisites.clone(),
            "action_prerequisites": action_prerequisites.clone(),
            "primary_paths": primary_paths.clone(),
            "candidate_paths": candidate_paths.clone(),
            "supporting_paths": supporting_paths.clone(),
            "related_context": related_context.clone(),
            "rejected_paths": rejected_paths.clone(),
            "path_selection_diagnostics": path_selection_diagnostics.clone(),
            "related_context_summary": related_context_summary.clone(),
            "key_model_availability": key_model_availability.clone(),
        })
    } else {
        // normal 模式：输出 primary + supporting，不输出 rejected
        serde_json::json!({
            "page_inputs": page_inputs.clone(),
            "data_sources": data_sources.clone(),
            "write_targets": write_targets.clone(),
            "entrypoints": entrypoints.clone(),
            "action_flows": action_flows.clone(),
            "visibility_rules": visibility_rules.clone(),
            "navigation": navigation.clone(),
            "risk_diagnostics": risk_diagnostics,
            "display_prerequisites": display_prerequisites.clone(),
            "data_prerequisites": data_prerequisites.clone(),
            "action_prerequisites": action_prerequisites.clone(),
            "primary_paths": primary_paths.clone(),
            "supporting_paths": supporting_paths.clone(),
            "related_context": related_context.clone(),
            "related_context_summary": related_context_summary.clone(),
            "key_model_availability": key_model_availability.clone(),
        })
    };

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::PageLogic, summary);
    output.query_target = Some(page_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics.clone();
    record_profile_stage(&mut profile, "output_build", output_stage_started);

    let stage_started = Instant::now();
    evidence::populate_page_logic_evidence(
        &mut output,
        page_id,
        &page_node.path,
        &entrypoints,
        &data_sources,
        &write_targets,
        &navigation,
        &visibility_rules,
        &action_flows,
        evidence_sample_limit,
    );

    // Compact mode: add OUTPUT_TRUNCATED diagnostic and evidence_summary
    if is_compact {
        let truncated_arrays = [
            ("data_sources", data_sources.len(), 5),
            ("write_targets", write_targets.len(), 5),
            ("entrypoints", entrypoints.len(), 5),
            ("action_flows", action_flows.len(), 5),
            ("visibility_rules", visibility_rules.len(), 5),
            ("navigation", navigation.len(), 5),
        ];
        let truncated_parts: Vec<String> = truncated_arrays
            .iter()
            .filter(|(_, size, limit)| *size > *limit)
            .map(|(name, size, limit)| format!("{} {}>{}", name, size, limit))
            .collect();
        if !truncated_parts.is_empty() {
            output.diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "OUTPUT_TRUNCATED".to_string(),
                message: format!(
                    "Compact budget: arrays truncated for: {}",
                    truncated_parts.join(", ")
                ),
                location: crate::output::Location {
                    source_file: Some(page_node.path.clone()),
                    node_id: Some(page_id.to_string()),
                    json_path: None,
                },
                suggestion: Some(
                    "Use --budget normal or --budget full to see complete arrays".to_string(),
                ),
            });
        }

        // Add evidence_summary and key_findings to summary
        let evidence_summary =
            crate::output::brief::evidence_summary(&output.evidence, evidence_sample_limit);
        let key_findings =
            crate::output::brief::build_key_findings(&output.summary, &output.diagnostics);
        if let Some(obj) = output.summary.as_object_mut() {
            obj.insert("evidence_summary".to_string(), evidence_summary);
            obj.insert("key_findings".to_string(), serde_json::json!(key_findings));
        }
        // Compact mode: truncate evidence array to limit noise
        output.evidence.truncate(evidence_sample_limit);
    }
    record_profile_stage(&mut profile, "evidence_build", stage_started);
    set_profile_counter(&mut profile, "evidence", output.evidence.len());
    let stage_started = Instant::now();
    let output = output.validate();
    record_profile_stage(&mut profile, "validation", stage_started);
    Ok(serde_json::to_value(output)?)
}

/// 查询页面级逻辑摘要
///
/// CLI 包装器，负责调用 `build_query_page_logic_output` 并按 `human` 参数决定输出格式。
pub fn query_page_logic(
    graph: &dyn GraphReadStore,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    human: bool,
    budget: &str,
) -> Result<()> {
    let output = build_query_page_logic_output(graph, page_id, project_dir, budget)?;
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Page Logic: {} ===", page_id)?;
        if let Some(summary) = output.get("summary").and_then(|s| s.as_object()) {
            if let Some(name) = summary.get("page_name").and_then(|v| v.as_str()) {
                writeln!(out, "Name: {}", name)?;
            }
            if let Some(role) = summary.get("page_role").and_then(|v| v.as_str()) {
                writeln!(out, "Role: {}", role)?;
            }
            if let Some(what) = summary.get("what_is_it").and_then(|v| v.as_str()) {
                writeln!(out, "{}", what)?;
            }
        }
        if let Some(details) = output.get("details").and_then(|d| d.as_object()) {
            if let Some(eps) = details.get("entrypoints").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Entrypoints ({}) ---",
                    eps.len()
                )?;
                for ep in eps {
                    let id = ep.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = ep.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let t = ep.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  [{}] {} ({})", id, name, t)?;
                }
            }
            if let Some(dss) = details.get("data_sources").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Data Sources ({}) ---",
                    dss.len()
                )?;
                for ds in dss {
                    let fp = ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {}", fp)?;
                }
            }
            if let Some(wts) = details.get("write_targets").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Write Targets ({}) ---",
                    wts.len()
                )?;
                for wt in wts {
                    let fp = wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {}", fp)?;
                }
            }
            if let Some(flows) = details.get("action_flows").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Action Flows ({}) ---",
                    flows.len()
                )?;
                for flow in flows {
                    let aid = flow
                        .get("action_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let atype = flow
                        .get("action_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let acat = flow
                        .get("action_category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let summary = flow
                        .get("semantic_summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let cid = flow
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    writeln!(out, "  {} [{} | {}]", aid, atype, acat)?;
                    if !summary.is_empty() {
                        writeln!(out, "    summary: {}", summary)?;
                    }
                    writeln!(out, "    triggered by {}", cid)?;
                    if let Some(raw) = flow
                        .get("blocks_on")
                        .and_then(|b| b.get("raw"))
                        .and_then(|v| v.as_str())
                    {
                        writeln!(out, "    waits for: {}", raw)?;
                    }
                    if let Some(raw) = flow
                        .get("condition")
                        .and_then(|c| c.get("raw_expr"))
                        .and_then(|v| v.as_str())
                    {
                        writeln!(out, "    condition: {}", raw)?;
                    }
                    let writes = flow
                        .get("writes")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    let nav = flow
                        .get("navigation")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    if writes > 0 {
                        writeln!(out, "    writes: {} target(s)", writes)?;
                    }
                    if nav > 0 {
                        writeln!(out, "    navigation: {} target(s)", nav)?;
                    }
                }
            }
            if let Some(nav) = details.get("navigation").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Navigation ({}) ---",
                    nav.len()
                )?;
                for n in nav {
                    let from = n.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                    let to = n.get("to").and_then(|v| v.as_str()).unwrap_or("?");
                    let t = n.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {} -> {} ({})", from, to, t)?;
                }
            }
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
