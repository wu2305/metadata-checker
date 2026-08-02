use crate::graph_store::GraphReadStore;
use std::collections::{HashMap, HashSet};

/// load-time 物化的条件事实（输出边界再转 JSON）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionFact {
    pub condition_id: String,
    pub condition_type: String,
    pub effect_type: String,
    pub subject_type: String,
    pub owner_type: String,
    pub raw_expr: String,
    pub normalized_expr: String,
    pub json_path: String,
    pub source_file: String,
    pub referenced_symbols: Vec<String>,
}

impl ConditionFact {
    /// 序列化为既有 condition JSON 契约。
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "condition_id": self.condition_id,
            "condition_type": self.condition_type,
            "effect_type": self.effect_type,
            "subject_type": self.subject_type,
            "owner_type": self.owner_type,
            "raw_expr": self.raw_expr,
            "normalized_expr": self.normalized_expr,
            "json_path": self.json_path,
            "source_file": self.source_file,
            "referenced_symbols": self.referenced_symbols,
        })
    }
}

/// 从条件节点构建 typed 事实。
pub(crate) fn build_condition_fact(source: &crate::graph::Node) -> ConditionFact {
    let meta = source.meta.as_ref().unwrap_or(&serde_json::Value::Null);
    let condition_type = meta
        .get("condition_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let effect_type = meta
        .get("effect_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let raw_expr = meta.get("raw_expr").and_then(|v| v.as_str()).unwrap_or("");
    let normalized_expr = meta
        .get("normalized_expr")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let json_path = meta.get("json_path").and_then(|v| v.as_str()).unwrap_or("");
    let owner_type = meta
        .get("owner_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let subject_type = meta
        .get("subject_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let referenced_symbols: Vec<String> = meta
        .get("referenced_symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    ConditionFact {
        condition_id: source.id.clone(),
        condition_type: condition_type.to_string(),
        effect_type: effect_type.to_string(),
        subject_type: subject_type.to_string(),
        owner_type: owner_type.to_string(),
        raw_expr: raw_expr.to_string(),
        normalized_expr: normalized_expr.to_string(),
        json_path: json_path.to_string(),
        source_file: source.path.clone(),
        referenced_symbols,
    }
}

/// 辅助：从条件节点构建条件 JSON（query/explain 边界）。
fn build_cond_obj(source: &crate::graph::Node) -> serde_json::Value {
    build_condition_fact(source).to_json()
}

/// 从条件对象推断条件所属节点，用于区分直接条件、继承条件和去重来源
fn condition_owner_node_id(cond_obj: &serde_json::Value) -> Option<String> {
    let condition_id = cond_obj.get("condition_id").and_then(|v| v.as_str())?;
    let source_file = cond_obj
        .get("source_file")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let owner_type = cond_obj
        .get("owner_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let owner_token = condition_id
        .split('|')
        .nth(1)
        .and_then(|s| s.split('#').next())?;

    match owner_type {
        "Component" | "FieldDefault" => Some(format!("comp:{}|{}", source_file, owner_token)),
        "ModelSource" => Some(format!("model:{}", owner_token)),
        "Page" => Some(format!("page:{}", source_file)),
        // 动作条件 id 形如 `cond:<文件>|<组件>#<动作>#conditionExp`，
        // 而动作节点 id 是 `action:<文件>|<组件>|<动作>`。此前只取到组件段，
        // 拼出的 owner_node_id 指向一个不存在的节点，模型照着查必然落空。
        "Action" => {
            let action_token = condition_id
                .split('|')
                .nth(1)
                .and_then(|s| s.split('#').nth(1));
            match action_token {
                Some(action_token) if !action_token.is_empty() => Some(format!(
                    "action:{}|{}|{}",
                    source_file, owner_token, action_token
                )),
                _ => Some(format!("action:{}|{}", source_file, owner_token)),
            }
        }
        _ => Some(owner_token.to_string()),
    }
}

/// 给条件打上面向解释的作用域标签
pub(crate) fn annotate_condition_scope(
    mut cond_obj: serde_json::Value,
    scope: &str,
    inherited_from: Option<&str>,
    ancestor_distance: Option<usize>,
    expanded_from: Option<&str>,
) -> serde_json::Value {
    let owner_node_id = condition_owner_node_id(&cond_obj);
    if let Some(obj) = cond_obj.as_object_mut() {
        obj.insert("condition_scope".to_string(), serde_json::json!(scope));
        if let Some(owner_node_id) = owner_node_id {
            obj.insert(
                "owner_node_id".to_string(),
                serde_json::json!(owner_node_id),
            );
        }
        if let Some(inherited_from) = inherited_from {
            obj.insert(
                "inherited_from".to_string(),
                serde_json::json!(inherited_from),
            );
        }
        if let Some(ancestor_distance) = ancestor_distance {
            obj.insert(
                "ancestor_distance".to_string(),
                serde_json::json!(ancestor_distance),
            );
        }
        if let Some(expanded_from) = expanded_from {
            obj.insert(
                "expanded_from_condition".to_string(),
                serde_json::json!(expanded_from),
            );
            obj.insert(
                "expansion_reason".to_string(),
                serde_json::json!("totalRowCount__ 显示门控需要展开当前页面模型过滤条件"),
            );
        }
    }
    cond_obj
}

pub(crate) fn condition_owned_by_node(cond_obj: &serde_json::Value, node_id: &str) -> bool {
    cond_obj
        .get("owner_node_id")
        .and_then(|v| v.as_str())
        .map_or(false, |owner| owner == node_id)
}

/// 生成条件去重 key：同一页面、同一作用、同一归一化表达式和同一引用集合视为等价
fn condition_dedupe_key(cond_obj: &serde_json::Value) -> String {
    let condition_type = cond_obj
        .get("condition_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let effect_type = cond_obj
        .get("effect_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let source_file = cond_obj
        .get("source_file")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expr = cond_obj
        .get("normalized_expr")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| cond_obj.get("raw_expr").and_then(|v| v.as_str()))
        .unwrap_or("");
    let mut refs: Vec<String> = cond_obj
        .get("referenced_symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    refs.sort();
    refs.dedup();
    format!(
        "{}|{}|{}|{}|{}",
        source_file,
        condition_type,
        effect_type,
        expr,
        refs.join(",")
    )
}

/// 对条件数组去重，同时保留重复声明的证据来源
pub(crate) fn dedupe_conditions(conditions: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut index_by_key: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut result: Vec<serde_json::Value> = Vec::new();

    for cond in conditions {
        let key = condition_dedupe_key(&cond);
        if let Some(idx) = index_by_key.get(&key).copied() {
            let duplicate_id = cond
                .get("condition_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let duplicate_scope = cond
                .get("condition_scope")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let duplicate_owner = cond
                .get("owner_node_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(obj) = result[idx].as_object_mut() {
                let ids = obj
                    .entry("deduped_condition_ids".to_string())
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(arr) = ids.as_array_mut() {
                    if !duplicate_id.is_empty()
                        && !arr.iter().any(|v| v.as_str() == Some(&duplicate_id))
                    {
                        arr.push(serde_json::json!(duplicate_id));
                    }
                }

                let scopes = obj
                    .entry("deduped_scopes".to_string())
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(arr) = scopes.as_array_mut() {
                    if !duplicate_scope.is_empty()
                        && !arr.iter().any(|v| v.as_str() == Some(&duplicate_scope))
                    {
                        arr.push(serde_json::json!(duplicate_scope));
                    }
                }

                let owners = obj
                    .entry("deduped_owner_node_ids".to_string())
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(arr) = owners.as_array_mut() {
                    if !duplicate_owner.is_empty()
                        && !arr.iter().any(|v| v.as_str() == Some(&duplicate_owner))
                    {
                        arr.push(serde_json::json!(duplicate_owner));
                    }
                }
            }
        } else {
            let idx = result.len();
            index_by_key.insert(key.clone(), idx);
            let mut cond = cond;
            if let Some(obj) = cond.as_object_mut() {
                obj.insert("condition_key".to_string(), serde_json::json!(key));
            }
            result.push(cond);
        }
    }

    result
}

/// 沿 Contains 入边查找组件祖先，返回从近到远的祖先组件
pub(in crate::explain) fn component_ancestor_chain(
    graph: &dyn GraphReadStore,
    component_id: &str,
) -> Vec<(String, usize)> {
    let mut ancestors = Vec::new();
    let mut current = component_id.to_string();
    let mut seen = std::collections::HashSet::new();
    let mut distance = 0usize;

    while seen.insert(current.clone()) {
        let Some(neighbors) = graph.get_node_edges(&current).ok().flatten() else {
            break;
        };
        let parent = neighbors
            .incoming
            .iter()
            .find(|edge_view| {
                let source = &edge_view.node;
                let edge = &edge_view.edge;
                matches!(edge.edge_type, crate::graph::EdgeType::Contains)
                    && matches!(
                        source.node_type,
                        crate::graph::NodeType::Component | crate::graph::NodeType::Page
                    )
            })
            .map(|edge_view| edge_view.node.clone());

        let Some(parent) = parent else {
            break;
        };
        if parent.node_type == crate::graph::NodeType::Page {
            break;
        }

        distance += 1;
        ancestors.push((parent.id.clone(), distance));
        current = parent.id;
    }

    ancestors
}

/// 从条件引用中抽取 `modelX.totalRowCount__` 的模型名
fn total_row_count_models(cond_obj: &serde_json::Value) -> Vec<String> {
    let mut models = Vec::new();
    if let Some(refs) = cond_obj
        .get("referenced_symbols")
        .and_then(|v| v.as_array())
    {
        for r in refs.iter().filter_map(|v| v.as_str()) {
            if let Some(rest) = r.strip_prefix("model:") {
                if let Some(model) = rest.strip_suffix(".totalRowCount__") {
                    models.push(model.to_string());
                }
            }
        }
    }

    let raw_expr = cond_obj
        .get("raw_expr")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if models.is_empty() && raw_expr.contains("totalRowCount__") {
        for token in raw_expr.split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '>' | '<' | '=' | '!' | '(' | ')' | '+' | '-' | '*' | '/' | ',' | '\'' | '"'
                )
        }) {
            if let Some(model) = token.strip_suffix(".totalRowCount__") {
                if !model.is_empty() {
                    models.push(model.to_string());
                }
            }
        }
    }

    models.sort();
    models.dedup();
    models
}

/// 收集组件自身表达式所在 JSON path，用于通过 JSON 层级推断祖先容器
pub(in crate::explain) fn component_json_paths(
    graph: &dyn GraphReadStore,
    component_id: &str,
) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(neighbors) = graph.get_node_edges(component_id).ok().flatten() {
        for edge_view in &neighbors.incoming {
            let source = &edge_view.node;
            let edge = &edge_view.edge;
            if matches!(source.node_type, crate::graph::NodeType::Condition)
                && matches!(edge.edge_type, crate::graph::EdgeType::DependsOn)
            {
                if let Some(path) = source
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("json_path"))
                    .and_then(|v| v.as_str())
                {
                    paths.push(path.to_string());
                }
            }
        }
        for edge_view in &neighbors.outgoing {
            let edge = &edge_view.edge;
            if let Some(path) = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
            {
                paths.push(path.to_string());
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn condition_component_json_path(cond_obj: &serde_json::Value) -> Option<String> {
    let json_path = cond_obj.get("json_path").and_then(|v| v.as_str())?;
    json_path
        .rsplit_once('.')
        .map(|(prefix, _)| prefix.to_string())
}

pub(in crate::explain::condition_facts) fn is_ancestor_json_path(
    ancestor_component_path: &str,
    target_json_path: &str,
) -> bool {
    target_json_path.starts_with(&format!("{}.components[", ancestor_component_path))
}

/// 低代码组件图当前只保证 page -> component Contains；嵌套父子关系用 json_path 前缀补足
pub(in crate::explain) fn collect_inherited_conditions_by_json_path(
    graph: &dyn GraphReadStore,
    page_path: &str,
    target_paths: &[String],
    seen_conditions: &mut std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let mut inherited = Vec::new();
    let nodes = match graph.iter_nodes() {
        Ok(nodes) => nodes,
        Err(_) => return inherited,
    };
    for node in nodes {
        if !matches!(node.node_type, crate::graph::NodeType::Condition) || node.path != page_path {
            continue;
        }
        let cond_obj = build_cond_obj(&node);
        if classify_condition(&cond_obj) != "blocking" {
            continue;
        }
        let Some(component_path) = condition_component_json_path(&cond_obj) else {
            continue;
        };
        if !target_paths
            .iter()
            .any(|target_path| is_ancestor_json_path(&component_path, target_path))
        {
            continue;
        }
        if !seen_conditions.insert(node.id.clone()) {
            continue;
        }
        let inherited_from = condition_owner_node_id(&cond_obj);
        inherited.push(annotate_condition_scope(
            cond_obj,
            "inherited",
            inherited_from.as_deref(),
            None,
            None,
        ));
    }
    inherited
}

/// 对 `model.totalRowCount__` 门控展开当前页面模型 filter
pub(crate) fn expand_total_row_count_gates(
    graph: &dyn GraphReadStore,
    blocking_conditions: &[serde_json::Value],
    page_path: &str,
    seen_conditions: &mut std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let mut expanded = Vec::new();
    for blocking in blocking_conditions {
        let from_condition = blocking
            .get("condition_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        for model in total_row_count_models(blocking) {
            let model_id = format!("model:{}", model);
            let conds = collect_conditions_for_node(graph, &model_id, page_path, seen_conditions);
            for cond_obj in conds {
                let source_file = cond_obj
                    .get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if source_file != page_path {
                    continue;
                }
                if cond_obj.get("owner_type").and_then(|v| v.as_str()) != Some("ModelSource") {
                    continue;
                }
                if classify_condition(&cond_obj) != "data_empty" {
                    continue;
                }
                expanded.push(annotate_condition_scope(
                    cond_obj,
                    "expanded_from_total_row_count",
                    Some(&model_id),
                    None,
                    Some(from_condition),
                ));
            }
        }
    }
    expanded
}

/// 辅助：分类条件
pub(crate) fn classify_condition(cond_obj: &serde_json::Value) -> &'static str {
    let condition_type = cond_obj
        .get("condition_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let raw_expr = cond_obj
        .get("raw_expr")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(
        condition_type,
        "VisibleCondition"
            | "DisableCondition"
            | "EnableCondition"
            | "ActionCondition"
            | "ActionConditionExp"
    ) {
        "blocking"
    } else if condition_type.contains("Filter") || raw_expr.contains("totalRowCount__") {
        "data_empty"
    } else {
        "supporting"
    }
}

/// 辅助：收集节点的 incoming condition 边
pub(crate) fn collect_conditions_for_node(
    graph: &dyn GraphReadStore,
    node_id: &str,
    _page_path: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    collect_condition_objects_for_node(graph, node_id, seen)
}

/// 缓存节点条件对象，供 page logic 批量 availability 构造复用。
pub(crate) struct ConditionCollectorCache {
    by_node: HashMap<String, Vec<serde_json::Value>>,
    /// load-time typed 索引；命中后再投影为 JSON。
    prefilled: Option<std::sync::Arc<HashMap<String, Vec<ConditionFact>>>>,
    cache_hits: usize,
    materialized_hits: usize,
}

impl ConditionCollectorCache {
    pub(crate) fn new() -> Self {
        Self {
            by_node: HashMap::new(),
            prefilled: None,
            cache_hits: 0,
            materialized_hits: 0,
        }
    }

    /// 从物化索引种子化缓存，避免 query-time 重复扫描 condition 边。
    pub(crate) fn from_prefilled(
        prefilled: std::sync::Arc<HashMap<String, Vec<ConditionFact>>>,
    ) -> Self {
        Self {
            by_node: HashMap::new(),
            prefilled: Some(prefilled),
            cache_hits: 0,
            materialized_hits: 0,
        }
    }

    pub(crate) fn cache_hits(&self) -> usize {
        self.cache_hits
    }

    pub(crate) fn materialized_hits(&self) -> usize {
        self.materialized_hits
    }

    pub(crate) fn collect_for_node(
        &mut self,
        graph: &dyn GraphReadStore,
        node_id: &str,
        seen: &mut HashSet<String>,
    ) -> Vec<serde_json::Value> {
        if let Some(prefilled) = &self.prefilled {
            if let Some(cached) = prefilled.get(node_id) {
                self.materialized_hits += 1;
                self.cache_hits += 1;
                return filter_seen_conditions(cached.iter().map(ConditionFact::to_json), seen);
            }
        }
        if let Some(cached) = self.by_node.get(node_id) {
            self.cache_hits += 1;
            return filter_seen_conditions(cached.iter().cloned(), seen);
        }
        let raw = collect_condition_objects_for_node(graph, node_id, &mut HashSet::new());
        let result = filter_seen_conditions(raw.iter().cloned(), seen);
        self.by_node.insert(node_id.to_string(), raw);
        result
    }

    pub(crate) fn expand_total_row_count_gates(
        &mut self,
        graph: &dyn GraphReadStore,
        blocking_conditions: &[serde_json::Value],
        page_path: &str,
        seen_conditions: &mut HashSet<String>,
    ) -> Vec<serde_json::Value> {
        let mut expanded = Vec::new();
        for blocking in blocking_conditions {
            let from_condition = blocking
                .get("condition_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            for model in total_row_count_models(blocking) {
                let model_id = format!("model:{}", model);
                let conds = self.collect_for_node(graph, &model_id, seen_conditions);
                for cond_obj in conds {
                    let source_file = cond_obj
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if source_file != page_path {
                        continue;
                    }
                    if cond_obj.get("owner_type").and_then(|v| v.as_str()) != Some("ModelSource") {
                        continue;
                    }
                    if classify_condition(&cond_obj) != "data_empty" {
                        continue;
                    }
                    expanded.push(annotate_condition_scope(
                        cond_obj,
                        "expanded_from_total_row_count",
                        Some(&model_id),
                        None,
                        Some(from_condition),
                    ));
                }
            }
        }
        expanded
    }
}

fn filter_seen_conditions(
    conditions: impl Iterator<Item = serde_json::Value>,
    seen: &mut HashSet<String>,
) -> Vec<serde_json::Value> {
    conditions
        .filter(|cond| {
            cond.get("condition_id")
                .and_then(|v| v.as_str())
                .is_some_and(|id| seen.insert(id.to_string()))
        })
        .collect()
}

/// 在 graph load 阶段一次性预收集所有节点的 typed condition 事实。
pub(crate) fn precollect_all_node_conditions(
    graph: &dyn GraphReadStore,
) -> crate::graph_store::GraphStoreResult<HashMap<String, Vec<ConditionFact>>> {
    let mut map = HashMap::new();
    for node in graph.iter_nodes()? {
        let raw = collect_condition_facts_for_node(graph, &node.id, &mut HashSet::new());
        if !raw.is_empty() {
            map.insert(node.id.clone(), raw);
        }
    }
    Ok(map)
}

/// 收集指向目标节点的 condition 依赖并构建 typed facts。
pub(crate) fn collect_condition_facts_for_node(
    graph: &dyn GraphReadStore,
    node_id: &str,
    seen: &mut HashSet<String>,
) -> Vec<ConditionFact> {
    let mut results = Vec::new();
    if let Some(neighbors) = graph.get_node_edges(node_id).ok().flatten() {
        for edge_view in &neighbors.incoming {
            let source = &edge_view.node;
            let edge = &edge_view.edge;
            if !matches!(source.node_type, crate::graph::NodeType::Condition) {
                continue;
            }
            if !matches!(edge.edge_type, crate::graph::EdgeType::DependsOn) {
                continue;
            }
            if !seen.insert(source.id.clone()) {
                continue;
            }
            results.push(build_condition_fact(source));
        }
    }
    results
}

fn collect_condition_objects_for_node(
    graph: &dyn GraphReadStore,
    node_id: &str,
    seen: &mut HashSet<String>,
) -> Vec<serde_json::Value> {
    collect_condition_facts_for_node(graph, node_id, seen)
        .into_iter()
        .map(|fact| fact.to_json())
        .collect()
}
