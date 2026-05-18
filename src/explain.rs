pub use crate::answer_contract::TraversalIntent;
use crate::answer_contract::{AnswerFactKind, answer_fact_enabled};
use crate::dependency::DependencyGraph;
use crate::explain::evidence::{push_lineage_evidence, push_relation_evidence};
use crate::explain::importance::{classify_importance, component_type_from_meta};
use crate::graph::GraphDB;
use crate::model_scope::{
    is_dataflow_model, parse_scoped_model_target, resolve_model_target_in_page,
};
use crate::output::schema::format_next_query;
use crate::path::PathFinder;
use crate::path::PathSelector;
use crate::superpage::{RefType, SuperPageMetadata};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, Write};

mod evidence;
mod importance;

/// 解释单文件 .spg 中的组件
///
/// 对单文件 .spg 支持组件 ID 简写，例如 --explain input1。
pub fn explain_component_spg(spg: &SuperPageMetadata, target_id: &str, human: bool) -> Result<()> {
    let graph = DependencyGraph::new(spg);
    let comp = spg
        .components
        .iter()
        .find(|c| c.id == target_id)
        .ok_or_else(|| anyhow::anyhow!("Component '{}' not found", target_id))?;

    let comp_exprs: Vec<_> = spg
        .expressions
        .iter()
        .filter(|e| e.component_id == target_id)
        .collect();

    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut triggered_by = Vec::new();
    let mut affects = Vec::new();

    for expr in &comp_exprs {
        for ref_type in &expr.refs {
            match ref_type {
                RefType::ModelField(model, field) => {
                    reads.push(json!({
                        "type": "model_field",
                        "model": model,
                        "field": field,
                        "source_expr": expr.raw_expr,
                    }));
                }
                RefType::Param(id) => {
                    reads.push(json!({
                        "type": "param",
                        "id": id,
                        "source_expr": expr.raw_expr,
                    }));
                }
                RefType::ComponentValue(id) => {
                    reads.push(json!({
                        "type": "component_value",
                        "id": id,
                        "source_expr": expr.raw_expr,
                    }));
                }
                _ => {}
            }
        }
    }

    for action in &comp.actions {
        match action.action_type.as_str() {
            "submitData" | "insertData" | "updateData" | "deleteData" => {
                if let Some(ref model) = action.data_set {
                    writes.push(json!({
                        "type": "model_write",
                        "model": model,
                        "action_type": action.action_type,
                        "trigger_type": action.trigger_type,
                    }));
                } else if action.action_type == "submitData" {
                    // Infer model from submitComponent bindings
                    for target_comp_id in &action.submit_component {
                        if let Some(target_comp) =
                            spg.components.iter().find(|c| c.id == *target_comp_id)
                            && let Some(submit_field) = target_comp.properties.get("submitField")
                        {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if !parts.is_empty() {
                                writes.push(json!({
                                    "type": "model_write",
                                    "model": parts[0],
                                    "field": parts.get(1).unwrap_or(&""),
                                    "action_type": action.action_type,
                                    "trigger_type": action.trigger_type,
                                    "inferred_from": target_comp_id,
                                }));
                            }
                        }
                    }
                }
            }
            "setParamValue" => {
                for (param_name, _) in &action.params {
                    writes.push(json!({
                        "type": "param_set",
                        "param": param_name,
                        "action_type": action.action_type,
                    }));
                }
            }
            "link" => {
                if let Some(ref path) = action.path {
                    writes.push(json!({
                        "type": "navigation",
                        "target": path,
                        "action_type": action.action_type,
                    }));
                }
            }
            _ => {}
        }
    }

    if comp.actions.is_empty() {
        triggered_by.push(json!("expression_calculation"));
    } else {
        for action in &comp.actions {
            triggered_by.push(json!({
                "action_type": action.action_type,
                "trigger_type": action.trigger_type,
            }));
        }
    }

    if let Some(downstream) = graph.reverse_deps.get(target_id) {
        for dep_id in downstream {
            if let Some(dep_comp) = spg.components.iter().find(|c| c.id == *dep_id) {
                affects.push(json!({
                    "id": dep_id,
                    "type": dep_comp.component_type,
                    "relationship": "expression_reference",
                }));
            }
        }
    }

    let summary = json!({
        "what_is_it": format!("{} component {}", comp.component_type, comp.id),
        "type": "component",
        "type_detail": comp.component_type,
        "importance": if comp.actions.is_empty() { "calculated_display" } else { "entrypoint" },
    });

    let details = json!({
        "reads": reads,
        "writes": writes,
        "triggered_by": triggered_by,
        "affects": affects,
        "lineage": [],
        "properties": comp.properties,
        "expressions": comp_exprs.iter().map(|e| json!({
            "field": e.field,
            "raw_expr": e.raw_expr,
        })).collect::<Vec<Value>>(),
    });

    let diagnostics: Vec<crate::output::Diagnostic> = Vec::new();
    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(target_id.to_string());
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Component {} is a {}", comp.id, comp.component_type),
            "Direct component definition in parsed metadata",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&comp.id),
    );
    push_relation_evidence(&mut output, "reads", &reads);
    push_relation_evidence(&mut output, "writes", &writes);
    push_relation_evidence(&mut output, "triggered_by", &triggered_by);
    push_relation_evidence(&mut output, "affects", &affects);
    output.diagnostics = diagnostics;
    output.next_queries = vec![
        format_next_query("--context {} --depth 2 for surrounding closure", target_id),
        "--project-dir <DIR> --explain model:<MODEL> for cross-file context".to_string(),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", target_id)?;
        writeln!(out, "What: {} component {}", comp.component_type, comp.id)?;
        writeln!(out, "Type: {}", comp.component_type)?;
        writeln!(
            out,
            "Importance: {}",
            if comp.actions.is_empty() {
                "display"
            } else {
                "entrypoint"
            }
        )?;
        if !reads.is_empty() {
            writeln!(out, "\n--- Reads ({}) ---", reads.len())?;
            for r in &reads {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !writes.is_empty() {
            writeln!(out, "\n--- Writes ({}) ---", writes.len())?;
            for w in &writes {
                writeln!(out, "  {:?}", w)?;
            }
        }
        if !triggered_by.is_empty() {
            writeln!(out, "\n--- Triggered By ({}) ---", triggered_by.len())?;
            for t in &triggered_by {
                writeln!(out, "  {:?}", t)?;
            }
        }
        if !affects.is_empty() {
            writeln!(out, "\n--- Affects ({}) ---", affects.len())?;
            for a in &affects {
                writeln!(out, "  {:?}", a)?;
            }
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

/// 解释项目图中的节点
/// 查找节点的父页面（通过 Contains 或 Triggers 边递归）
fn find_parent_page(graph: &GraphDB, node_id: &str) -> Option<crate::graph::Node> {
    if let Some((_, incoming)) = graph.get_node_edges(node_id) {
        for (parent, edge) in incoming {
            if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
                if matches!(parent.node_type, crate::graph::NodeType::Page) {
                    return Some(parent.clone());
                }
                if let Some(page) = find_parent_page(graph, &parent.id) {
                    return Some(page);
                }
            }
            if matches!(edge.edge_type, crate::graph::EdgeType::Triggers)
                && let Some(page) = find_parent_page(graph, &parent.id)
            {
                return Some(page);
            }
        }
    }
    None
}

/// 辅助：构建带页面信息的引用对象
fn make_ref(
    target: &crate::graph::Node,
    edge: &crate::graph::Edge,
    source_path: Option<&str>,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "id": target.id,
        "name": target.name,
        "type": format!("{:?}", target.node_type),
        "edge_type": format!("{:?}", edge.edge_type),
        "field_path": edge.field_path,
        "source_file": edge.meta.as_ref().and_then(|m| m.get("source_file")).and_then(|v| v.as_str()).or(source_path),
    });
    if let Some(expr) = edge
        .meta
        .as_ref()
        .and_then(|m| m.get("source_expr"))
        .and_then(|v| v.as_str())
    {
        obj["raw_expr"] = serde_json::json!(expr);
    }
    if let Some(path) = edge
        .meta
        .as_ref()
        .and_then(|m| m.get("json_path"))
        .and_then(|v| v.as_str())
    {
        obj["json_path"] = serde_json::json!(path);
    }
    obj
}

/// 解释项目图中的节点
///
/// 支持完整 ID，例如 --explain model:physical_x。
/// 按 NodeType 分发，生成语义化的 summary、details、evidence。
/// 解释目标节点为什么具有当前状态（为什么不显示/为什么不可用/为什么数据为空）
///
/// 支持的目标格式：
/// - `comp:PAGE|ID` — 解释组件为什么不显示或为什么不可用
/// - `model:PAGE|MODEL` — 在指定页面作用域内解释模型可用性
/// - `model:ID` — 解释模型为什么可能为空
/// - `field:MODEL.FIELD` — 解释字段值来源或为什么为空
///
/// 辅助：从条件节点构建条件对象
fn build_cond_obj(source: &crate::graph::Node) -> serde_json::Value {
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
    serde_json::json!({
        "condition_id": source.id.clone(),
        "condition_type": condition_type,
        "effect_type": effect_type,
        "subject_type": subject_type,
        "owner_type": owner_type,
        "raw_expr": raw_expr,
        "normalized_expr": normalized_expr,
        "json_path": json_path,
        "source_file": source.path,
        "referenced_symbols": referenced_symbols,
    })
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
        "Action" => Some(format!("action:{}|{}", source_file, owner_token)),
        _ => Some(owner_token.to_string()),
    }
}

/// 给条件打上面向解释的作用域标签
fn annotate_condition_scope(
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

fn condition_owned_by_node(cond_obj: &serde_json::Value, node_id: &str) -> bool {
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
fn dedupe_conditions(conditions: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
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
fn component_ancestor_chain(graph: &GraphDB, component_id: &str) -> Vec<(String, usize)> {
    let mut ancestors = Vec::new();
    let mut current = component_id.to_string();
    let mut seen = std::collections::HashSet::new();
    let mut distance = 0usize;

    while seen.insert(current.clone()) {
        let Some((_outgoing, incoming)) = graph.get_node_edges(&current) else {
            break;
        };
        let parent = incoming
            .iter()
            .find(|(source, edge)| {
                matches!(edge.edge_type, crate::graph::EdgeType::Contains)
                    && matches!(
                        source.node_type,
                        crate::graph::NodeType::Component | crate::graph::NodeType::Page
                    )
            })
            .map(|(source, _)| (*source).clone());

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
fn component_json_paths(graph: &GraphDB, component_id: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some((outgoing, incoming)) = graph.get_node_edges(component_id) {
        for (source, edge) in &incoming {
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
        for (_target, edge) in &outgoing {
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

fn is_ancestor_json_path(ancestor_component_path: &str, target_json_path: &str) -> bool {
    target_json_path.starts_with(&format!("{}.components[", ancestor_component_path))
}

/// 低代码组件图当前只保证 page -> component Contains；嵌套父子关系用 json_path 前缀补足
fn collect_inherited_conditions_by_json_path(
    graph: &GraphDB,
    page_path: &str,
    target_paths: &[String],
    seen_conditions: &mut std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let mut inherited = Vec::new();
    for (_id, idx) in &graph.node_indices {
        let Some(node) = graph.graph.node_weight(*idx) else {
            continue;
        };
        if !matches!(node.node_type, crate::graph::NodeType::Condition) || node.path != page_path {
            continue;
        }
        let cond_obj = build_cond_obj(node);
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

fn component_property<'a>(node: &'a crate::graph::Node, key: &str) -> Option<&'a str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get("properties"))
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .or_else(|| {
            node.meta
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
        })
}

fn component_meta_string<'a>(node: &'a crate::graph::Node, key: &str) -> Option<&'a str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
}

/// 识别 `${FIELD}` 这种依赖当前行数据上下文的裸字段引用
fn extract_single_bare_symbol(raw_expr: &str) -> Option<String> {
    let trimmed = raw_expr.trim();
    let inner = trimmed
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))?
        .trim();
    if inner.is_empty() || inner.contains('.') {
        return None;
    }
    if inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(inner.to_string())
    } else {
        None
    }
}

fn nearest_data_context_component(
    graph: &GraphDB,
    page_path: &str,
    target_json_path: &str,
) -> Option<crate::graph::Node> {
    let mut best: Option<(usize, crate::graph::Node)> = None;
    for (_id, idx) in &graph.node_indices {
        let Some(node) = graph.graph.node_weight(*idx) else {
            continue;
        };
        if !matches!(node.node_type, crate::graph::NodeType::Component) || node.path != page_path {
            continue;
        }
        let has_data_context = component_meta_string(node, "dataSet").is_some()
            || component_meta_string(node, "source").is_some();
        if !has_data_context {
            continue;
        }
        let Some(ctx_path) = component_meta_string(node, "json_path") else {
            continue;
        };
        if !is_ancestor_json_path(ctx_path, target_json_path) {
            continue;
        }
        let score = ctx_path.len();
        if best
            .as_ref()
            .map_or(true, |(best_score, _)| score > *best_score)
        {
            best = Some((score, node.clone()));
        }
    }
    best.map(|(_, node)| node)
}

fn model_from_table_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
}

fn dataflow_field_origin(
    dataflow_node: &crate::graph::Node,
    bare_symbol: &str,
) -> Option<serde_json::Value> {
    let meta = dataflow_node.meta.as_ref()?;
    let dfm = crate::query::DataFlowMeta::from_meta(meta);
    let projection = crate::query::project_output_field_origin(&dfm, bare_symbol);
    Some(projection.to_json())
}

fn build_value_source_context_from_derived_edge(
    graph: &GraphDB,
    target_node: &crate::graph::Node,
) -> Option<serde_json::Value> {
    let (outgoing, _) = graph.get_node_edges(&target_node.id)?;
    let (field_node, edge) = outgoing
        .iter()
        .find(|(node, edge)| {
            let Some(meta) = edge.meta.as_ref() else {
                return false;
            };
            if !matches!(edge.edge_type, crate::graph::EdgeType::Reads)
                || !node.id.starts_with("field:")
                || meta.get("resolution").and_then(|v| v.as_str())
                    != Some("inherited_container_data_context")
            {
                return false;
            }
            let data_set = meta
                .get("target_model")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let bare_symbol = meta
                .get("bare_symbol")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            node.id == format!("field:{}.{}", data_set, bare_symbol)
        })
        .or_else(|| {
            outgoing.iter().find(|(node, edge)| {
                matches!(edge.edge_type, crate::graph::EdgeType::Reads)
                    && node.id.starts_with("field:")
                    && edge
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("resolution"))
                        .and_then(|v| v.as_str())
                        == Some("inherited_container_data_context")
            })
        })?;
    let meta = edge.meta.as_ref()?;
    let raw_expr = meta.get("source_expr").and_then(|v| v.as_str())?;
    let bare_symbol = meta.get("bare_symbol").and_then(|v| v.as_str())?;
    let data_set = meta.get("target_model").and_then(|v| v.as_str())?;
    let context_component_id = meta
        .get("data_context_component_id")
        .and_then(|v| v.as_str())?;
    let context_node_id = format!("comp:{}|{}", target_node.path, context_component_id);
    let context_node = graph.get_node(&context_node_id);
    let data_set_model_id = format!("model:{}", data_set);
    let data_set_model = graph.get_node(&data_set_model_id);
    let data_set_model_path = meta
        .get("target_model_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| data_set_model.as_ref().map(|n| n.path.clone()));
    let dataflow_model = data_set_model_path
        .as_deref()
        .and_then(model_from_table_path)
        .map(|model| format!("model:{}", model))
        .and_then(|model_id| graph.get_node(&model_id));
    let dataflow_field_origin = dataflow_model
        .as_ref()
        .and_then(|node| dataflow_field_origin(node, bare_symbol));
    let dataflow_inputs = dataflow_model
        .as_ref()
        .map(|node| {
            graph
                .find_dataflow_inputs(&node.id)
                .into_iter()
                .map(|(input, edge)| {
                    serde_json::json!({
                        "node_id": input.id,
                        "name": input.name,
                        "source_file": input.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "field_path": edge.field_path,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(serde_json::json!({
        "source_component": target_node.id,
        "source_field": meta.get("source_field").and_then(|v| v.as_str()).unwrap_or("value"),
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "resolution": "inherited_container_data_context",
        "graph_edge": {
            "edge_type": format!("{:?}", edge.edge_type),
            "field_path": edge.field_path,
            "target_id": field_node.id,
        },
        "nearest_data_context": {
            "node_id": context_node.as_ref().map(|node| node.id.clone()),
            "component_id": context_component_id,
            "component_type": context_node.as_ref().and_then(|node| component_meta_string(node, "component_type")),
            "json_path": meta.get("data_context_json_path").and_then(|v| v.as_str()),
            "source": meta.get("data_context_source").and_then(|v| v.as_str()),
            "dataSet": data_set,
        },
        "data_set_model": data_set_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": data_set_model_path.as_deref().unwrap_or(&node.path),
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "field_path": format!("{}.{}", data_set, bare_symbol),
        "table_source_path": data_set_model_path,
        "dataflow_model": dataflow_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "dataflow_field_origin": dataflow_field_origin,
        "dataflow_inputs": dataflow_inputs,
    }))
}

fn build_value_source_context(
    graph: &GraphDB,
    target_node: &crate::graph::Node,
    page_path: &str,
) -> Option<serde_json::Value> {
    if !matches!(target_node.node_type, crate::graph::NodeType::Component) {
        return None;
    }
    if let Some(ctx) = build_value_source_context_from_derived_edge(graph, target_node) {
        return Some(ctx);
    }
    let raw_expr = component_property(target_node, "value")?;
    let bare_symbol = extract_single_bare_symbol(raw_expr)?;
    let target_json_path = component_meta_string(target_node, "json_path")
        .map(|path| format!("{}.value", path))
        .or_else(|| {
            component_json_paths(graph, &target_node.id)
                .into_iter()
                .next()
        })?;
    let context_node = nearest_data_context_component(graph, page_path, &target_json_path)?;
    let data_set = component_meta_string(&context_node, "dataSet")
        .or_else(|| component_property(&context_node, "dataSet"))?;
    let data_set_model_id = format!("model:{}", data_set);
    let data_set_model = graph.get_node(&data_set_model_id);
    let data_set_model_path = data_set_model.as_ref().map(|n| n.path.clone());

    let dataflow_model = data_set_model_path
        .as_deref()
        .and_then(model_from_table_path)
        .map(|model| format!("model:{}", model))
        .and_then(|model_id| graph.get_node(&model_id));
    let dataflow_field_origin = dataflow_model
        .as_ref()
        .and_then(|node| dataflow_field_origin(node, &bare_symbol));
    let dataflow_inputs = dataflow_model
        .as_ref()
        .map(|node| {
            graph
                .find_dataflow_inputs(&node.id)
                .into_iter()
                .map(|(input, edge)| {
                    serde_json::json!({
                        "node_id": input.id,
                        "name": input.name,
                        "source_file": input.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "field_path": edge.field_path,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(serde_json::json!({
        "source_component": target_node.id,
        "source_field": "value",
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "resolution": "nearest_data_context_container",
        "nearest_data_context": {
            "node_id": context_node.id,
            "component_id": context_node.name,
            "component_type": component_meta_string(&context_node, "component_type"),
            "json_path": component_meta_string(&context_node, "json_path"),
            "source": component_meta_string(&context_node, "source"),
            "dataSet": data_set,
        },
        "data_set_model": data_set_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "field_path": format!("{}.{}", data_set, bare_symbol),
        "table_source_path": data_set_model_path,
        "dataflow_model": dataflow_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "dataflow_field_origin": dataflow_field_origin,
        "dataflow_inputs": dataflow_inputs,
    }))
}

fn answer_path_step(
    step: usize,
    node_id: impl Into<String>,
    node_type: impl Into<String>,
    edge_type: impl Into<String>,
    direction: impl Into<String>,
    field_path: Option<&str>,
    source_file: Option<&str>,
    json_path: Option<&str>,
    why_included: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({
        "step": step,
        "node_id": node_id.into(),
        "node_type": node_type.into(),
        "edge_type": edge_type.into(),
        "direction": direction.into(),
        "field_path": field_path,
        "source_file": source_file,
        "json_path": json_path,
        "why_included": why_included.into(),
    })
}

fn evidence_ref_from_condition(cond: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "node_id": cond.get("condition_id").and_then(|v| v.as_str()),
        "source_file": cond.get("source_file").and_then(|v| v.as_str()),
        "json_path": cond.get("json_path").and_then(|v| v.as_str()),
    })
}

fn compact_condition_fact(cond: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "condition_id": cond.get("condition_id").and_then(|v| v.as_str()),
        "condition_type": cond.get("condition_type").and_then(|v| v.as_str()),
        "condition_scope": cond.get("condition_scope").and_then(|v| v.as_str()),
        "raw_expr": cond.get("raw_expr").and_then(|v| v.as_str()),
        "owner_node_id": cond.get("owner_node_id").and_then(|v| v.as_str()),
        "inherited_from": cond.get("inherited_from").and_then(|v| v.as_str()),
        "expanded_from_condition": cond.get("expanded_from_condition").and_then(|v| v.as_str()),
        "source_file": cond.get("source_file").and_then(|v| v.as_str()),
        "json_path": cond.get("json_path").and_then(|v| v.as_str()),
    })
}

fn build_display_facts(
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
) -> serde_json::Value {
    let direct_conditions: Vec<_> = blocking_conditions
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("direct"))
        .map(compact_condition_fact)
        .collect();
    let inherited_conditions: Vec<_> = blocking_conditions
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited"))
        .map(compact_condition_fact)
        .collect();
    let expanded_data_gates: Vec<_> = data_empty_gates
        .iter()
        .filter(|c| {
            c.get("condition_scope").and_then(|v| v.as_str())
                == Some("expanded_from_total_row_count")
        })
        .map(compact_condition_fact)
        .collect();

    let mut evidence_refs: Vec<serde_json::Value> = blocking_conditions
        .iter()
        .chain(data_empty_gates.iter())
        .map(evidence_ref_from_condition)
        .collect();
    evidence_refs.truncate(8);

    let mut paths = Vec::new();
    for cond in blocking_conditions.iter().take(3) {
        paths.push(serde_json::json!({
            "intent": "display",
            "result": cond.get("raw_expr").and_then(|v| v.as_str()),
            "confidence": "high",
            "why_complete": "display gate condition found",
            "stop_condition_hit": "display_condition_found",
            "evidence_refs": [evidence_ref_from_condition(cond)],
            "steps": [
                answer_path_step(
                    1,
                    cond.get("condition_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "Condition",
                    "DependsOn",
                    "incoming",
                    None,
                    cond.get("source_file").and_then(|v| v.as_str()),
                    cond.get("json_path").and_then(|v| v.as_str()),
                    "condition controls target display or inherited container display",
                )
            ]
        }));
    }

    serde_json::json!({
        "result": if !direct_conditions.is_empty() || !inherited_conditions.is_empty() {
            "display_conditions_found"
        } else {
            "no_display_condition_found"
        },
        "confidence": if !direct_conditions.is_empty() || !inherited_conditions.is_empty() {
            "high"
        } else {
            "medium"
        },
        "target": target_node.id,
        "has_direct_condition": !direct_conditions.is_empty(),
        "direct_conditions": direct_conditions,
        "inherited_conditions": inherited_conditions,
        "expanded_data_gates": expanded_data_gates,
        "evidence_refs": evidence_refs,
        "paths": paths,
        "missing_evidence": if blocking_conditions.is_empty() {
            serde_json::json!(["no direct or inherited display gate found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_value_source_facts(value_source_context: &Option<serde_json::Value>) -> serde_json::Value {
    let Some(ctx) = value_source_context.as_ref() else {
        return serde_json::json!({
            "result": null,
            "confidence": "low",
            "paths": [],
            "evidence_refs": [],
            "missing_evidence": ["value_source_context not available"],
        });
    };

    let dataflow_origin = ctx.get("dataflow_field_origin");
    let dataflow_table = ctx
        .get("table_source_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let dataflow_output_field = dataflow_origin
        .and_then(|origin| origin.get("dataflow_output_field").and_then(|v| v.as_str()))
        .unwrap_or("");

    let physical_source_fields: Vec<String> = dataflow_origin
        .and_then(|origin| {
            origin
                .get("physical_source_fields")
                .and_then(|v| v.as_array())
        })
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let via = dataflow_origin
        .and_then(|origin| origin.get("via").and_then(|v| v.as_str()))
        .unwrap_or("");
    let original_node =
        dataflow_origin.and_then(|origin| origin.get("original_node").and_then(|v| v.as_str()));
    let original_field =
        dataflow_origin.and_then(|origin| origin.get("original_field").and_then(|v| v.as_str()));

    let candidate_inputs: Vec<String> = ctx
        .get("dataflow_inputs")
        .and_then(|inputs| inputs.as_array())
        .map(|inputs| {
            inputs
                .iter()
                .filter_map(|item| {
                    item.get("node_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("name").and_then(|v| v.as_str()))
                        .or_else(|| item.get("source_file").and_then(|v| v.as_str()))
                        .map(ToString::to_string)
                })
                .collect()
        })
        .unwrap_or_default();

    let origin_path = if !physical_source_fields.is_empty() {
        Some(physical_source_fields[0].as_str())
    } else {
        None
    };
    let table_source_path = ctx.get("table_source_path").and_then(|v| v.as_str());
    let result = origin_path.or(table_source_path);
    let raw_expr = ctx.get("raw_expr").and_then(|v| v.as_str());
    let bare_symbol = ctx.get("bare_symbol").and_then(|v| v.as_str());
    let field_path = ctx.get("field_path").and_then(|v| v.as_str());
    let context_component = ctx
        .get("nearest_data_context")
        .and_then(|v| v.get("component_id"))
        .and_then(|v| v.as_str());
    let data_set = ctx
        .get("nearest_data_context")
        .and_then(|v| v.get("dataSet"))
        .and_then(|v| v.as_str());
    let graph_target = ctx
        .get("graph_edge")
        .and_then(|v| v.get("target_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let graph_edge_type = ctx
        .get("graph_edge")
        .and_then(|v| v.get("edge_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("Reads");

    let mut steps = Vec::new();
    steps.push(answer_path_step(
        1,
        ctx.get("source_component")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "Component",
        graph_edge_type,
        "outgoing",
        field_path,
        None,
        ctx.get("nearest_data_context")
            .and_then(|v| v.get("json_path"))
            .and_then(|v| v.as_str()),
        "target value expression reads a field",
    ));
    if let Some(component) = context_component {
        steps.push(answer_path_step(
            2,
            component,
            "Component",
            "inherited_container_data_context",
            "ancestor",
            field_path,
            None,
            ctx.get("nearest_data_context")
                .and_then(|v| v.get("json_path"))
                .and_then(|v| v.as_str()),
            "bare field resolved through nearest data context container",
        ));
    }
    if !graph_target.is_empty() {
        steps.push(answer_path_step(
            3,
            graph_target,
            "Field",
            graph_edge_type,
            "outgoing",
            field_path,
            table_source_path,
            None,
            "resolved dataSet field",
        ));
    }
    if let Some(origin) = origin_path {
        steps.push(answer_path_step(
            4,
            origin,
            "Model",
            "dataflow_field_origin",
            "upstream",
            field_path,
            Some(origin),
            None,
            "DataFlow field origin matched original input table",
        ));
    }

    serde_json::json!({
        "result": result,
        "confidence": if origin_path.is_some() { "high" } else if table_source_path.is_some() { "medium" } else { "low" },
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "dataflow_table": if dataflow_table.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_table)
        },
        "dataflow_output_field": if dataflow_output_field.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_output_field.to_string())
        },
        "physical_source_fields": physical_source_fields,
        "candidate_inputs": candidate_inputs,
        "via": via,
        "original_node": original_node,
        "original_field": original_field,
        "nearest_data_context": context_component,
        "data_set": data_set,
        "field_path": field_path,
        "table_source_path": table_source_path,
        "proven_physical_input": origin_path,
        "paths": [{
            "intent": "value-source",
            "result": result,
            "confidence": if origin_path.is_some() { "high" } else if table_source_path.is_some() { "medium" } else { "low" },
            "why_complete": if origin_path.is_some() {
                "field-level DataFlow origin found"
            } else {
                "stopped at page source table path"
            },
            "stop_condition_hit": if origin_path.is_some() {
                "proven_physical_input_found"
            } else {
                "table_source_path_found"
            },
            "evidence_refs": [{
                "node_id": graph_target,
                "field_path": field_path,
                "raw_expr": raw_expr,
            }],
            "steps": steps,
        }],
        "evidence_refs": [{
            "node_id": graph_target,
            "field_path": field_path,
            "raw_expr": raw_expr,
        }],
        "missing_evidence": if result.is_none() {
            serde_json::json!(["no table source path or DataFlow field origin found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_writer_facts(primary_path: &[serde_json::Value]) -> serde_json::Value {
    let writer_paths: Vec<_> = primary_path
        .iter()
        .filter(|p| {
            let path_text = p.to_string();
            path_text.contains("ActionWrites")
                || path_text.contains("FieldWrite")
                || path_text.contains("Writes")
        })
        .take(5)
        .cloned()
        .collect();

    let paths: Vec<_> = writer_paths
        .iter()
        .enumerate()
        .map(|(idx, path)| {
            let path_id = path.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            let steps = path
                .get("segments")
                .and_then(|v| v.as_array())
                .map(|segments| {
                    segments
                        .iter()
                        .enumerate()
                        .map(|(step_idx, segment)| {
                            let to_node = segment.get("to").unwrap_or(&serde_json::Value::Null);
                            let edge = segment.get("edge").unwrap_or(&serde_json::Value::Null);
                            answer_path_step(
                                step_idx + 1,
                                to_node
                                    .get("node_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(path_id),
                                to_node
                                    .get("node_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Node"),
                                edge.get("edge_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown"),
                                "causal",
                                edge.get("field_path").and_then(|v| v.as_str()),
                                to_node.get("path").and_then(|v| v.as_str()),
                                edge.get("json_path").and_then(|v| v.as_str()),
                                "selected primary path segment contributes to writer lineage",
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|steps| !steps.is_empty())
                .unwrap_or_else(|| {
                    vec![answer_path_step(
                        idx + 1,
                        path_id,
                        "Path",
                        "Writes/ActionWrites/FieldWrite",
                        "incoming",
                        None,
                        None,
                        None,
                        "primary path contains a writer edge",
                    )]
                });
            serde_json::json!({
                "intent": "writer",
                "result": path_id,
                "confidence": "high",
                "why_complete": "writer edge found in selected primary path",
                "stop_condition_hit": "writer_path_found",
                "evidence_refs": [{
                    "path_id": path_id,
                }],
                "steps": steps,
            })
        })
        .collect();

    serde_json::json!({
        "result": if writer_paths.is_empty() { serde_json::Value::Null } else { serde_json::json!("writer_paths_found") },
        "confidence": if writer_paths.is_empty() { "low" } else { "high" },
        "paths": paths,
        "evidence_refs": writer_paths.iter().map(|p| serde_json::json!({
            "path_id": p.get("path_id").and_then(|v| v.as_str()),
        })).collect::<Vec<_>>(),
        "missing_evidence": if writer_paths.is_empty() {
            serde_json::json!(["no writer path selected"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_availability_facts(
    data_empty_gates: &[serde_json::Value],
    dataflow_table: &str,
    dataflow_input_paths: &[String],
    dataflow_meta: Option<&crate::query::DataFlowMeta>,
) -> serde_json::Value {
    let gates: Vec<_> = data_empty_gates
        .iter()
        .map(compact_condition_fact)
        .collect();

    let source_filters: Vec<serde_json::Value> = data_empty_gates
        .iter()
        .filter(|g| g.get("role").and_then(|v| v.as_str()) == Some("source_filter"))
        .map(|g| serde_json::json!({
            "node_alias": g.get("node_alias").and_then(|v| v.as_str()),
            "raw_expr": g.get("raw_expr").and_then(|v| v.as_str()),
            "left": g.get("left").and_then(|v| v.as_str()),
            "operator": g.get("operator").and_then(|v| v.as_str()),
            "right": g.get("right").and_then(|v| v.as_str()),
            "referenced_fields": g.get("referenced_fields").cloned().unwrap_or(serde_json::Value::Null),
            "referenced_vars": g.get("referenced_vars").cloned().unwrap_or(serde_json::Value::Null),
        }))
        .collect();

    let output_filters: Vec<serde_json::Value> = data_empty_gates
        .iter()
        .filter(|g| g.get("role").and_then(|v| v.as_str()) == Some("output_filter"))
        .map(|g| serde_json::json!({
            "node_alias": g.get("node_alias").and_then(|v| v.as_str()),
            "raw_expr": g.get("raw_expr").and_then(|v| v.as_str()),
            "left": g.get("left").and_then(|v| v.as_str()),
            "operator": g.get("operator").and_then(|v| v.as_str()),
            "right": g.get("right").and_then(|v| v.as_str()),
            "referenced_fields": g.get("referenced_fields").cloned().unwrap_or(serde_json::Value::Null),
            "referenced_vars": g.get("referenced_vars").cloned().unwrap_or(serde_json::Value::Null),
        }))
        .collect();

    let mut referenced_vars: Vec<String> = Vec::new();
    let mut seen_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    for g in data_empty_gates
        .iter()
        .filter(|g| g.get("condition_type").and_then(|v| v.as_str()) == Some("DataFlowFilter"))
    {
        if let Some(arr) = g.get("referenced_vars").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if seen_vars.insert(s.to_string()) {
                        referenced_vars.push(s.to_string());
                    }
                }
            }
        }
    }

    let has_dataflow_filters = data_empty_gates
        .iter()
        .any(|g| g.get("condition_type").and_then(|v| v.as_str()) == Some("DataFlowFilter"));

    let mut physical_inputs: Vec<String> = if !dataflow_input_paths.is_empty() {
        dataflow_input_paths.to_vec()
    } else {
        dataflow_meta
            .map(|dfm| dfm.get_model_table_paths())
            .unwrap_or_default()
    };
    physical_inputs.sort();
    physical_inputs.dedup();

    let mut join_rules: Vec<serde_json::Value> = Vec::new();
    let mut union_rules: Vec<serde_json::Value> = Vec::new();
    if let Some(dfm) = dataflow_meta {
        for (node_id, conditions) in &dfm.node_join_conditions {
            let alias = dfm
                .get_alias(node_id)
                .map(|s| s.as_str())
                .unwrap_or(node_id.as_str());
            for jc in conditions {
                let row_semantic = match jc.join_type.as_str() {
                    "LeftJoin" | "Left Outer Join" => "left_rows_preserved_right_fields_nullable",
                    "RightJoin" | "Right Outer Join" => "right_rows_preserved_left_fields_nullable",
                    "InnerJoin" | "Inner Join" => "both_tables_must_match",
                    "FullJoin" | "Full Outer Join" => "all_rows_preserved_nullable",
                    _ => "join_rows_filtered_by_condition",
                };
                let clauses_json: Vec<serde_json::Value> = jc
                    .clauses
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "left": c.left_exp,
                            "operator": c.operator,
                            "right": c.right_exp,
                        })
                    })
                    .collect();
                join_rules.push(serde_json::json!({
                    "node_alias": alias,
                    "join_type": &jc.join_type,
                    "left_table": &jc.left_table,
                    "right_table": &jc.right_table,
                    "row_semantic": row_semantic,
                    "clauses": clauses_json,
                }));
            }
        }
        for (node_id, entries) in &dfm.node_union_maps {
            let alias = dfm
                .get_alias(node_id)
                .map(|s| s.as_str())
                .unwrap_or(node_id.as_str());
            let field_mappings: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "fields": e.values,
                        "visible": e.visible,
                    })
                })
                .collect();
            union_rules.push(serde_json::json!({
                "node_alias": alias,
                "row_semantic": "any_branch_can_output",
                "field_mappings": field_mappings,
            }));
        }
    }

    serde_json::json!({
        "result": if gates.is_empty() { "no_data_gate_found" } else { "data_gates_found" },
        "confidence": if gates.is_empty() { "medium" } else { "high" },
        "gates": gates,
        "dataflow_table": if dataflow_table.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_table.to_string())
        },
        "physical_inputs": physical_inputs,
        "dataflow_availability": if has_dataflow_filters { "dataflow_filters_present" } else { "no_dataflow_filters" },
        "source_filters": source_filters,
        "output_filters": output_filters,
        "join_rules": join_rules,
        "union_rules": union_rules,
        "referenced_vars": referenced_vars,
        "paths": data_empty_gates.iter().take(3).map(|gate| serde_json::json!({
            "intent": "availability",
            "result": gate.get("raw_expr").and_then(|v| v.as_str()),
            "confidence": "high",
            "why_complete": "data availability gate found",
            "stop_condition_hit": "filter_or_total_row_count_gate_found",
            "evidence_refs": [evidence_ref_from_condition(gate)],
            "steps": [
                answer_path_step(
                    1,
                    gate.get("condition_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "Condition",
                    "DependsOn",
                    "incoming",
                    None,
                    gate.get("source_file").and_then(|v| v.as_str()),
                    gate.get("json_path").and_then(|v| v.as_str()),
                    "filter or totalRowCount condition controls data availability",
                )
            ]
        })).collect::<Vec<_>>(),
        "evidence_refs": data_empty_gates.iter().map(evidence_ref_from_condition).collect::<Vec<_>>(),
        "missing_evidence": if data_empty_gates.is_empty() {
            serde_json::json!(["no data availability gate found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn graph_edge_ref(node: &crate::graph::Node, edge: &crate::graph::Edge) -> serde_json::Value {
    serde_json::json!({
        "node_id": node.id,
        "node_type": format!("{:?}", node.node_type),
        "name": node.name,
        "source_file": edge.meta.as_ref().and_then(|m| m.get("source_file")).and_then(|v| v.as_str()).or(Some(node.path.as_str())),
        "edge_type": format!("{:?}", edge.edge_type),
        "field_path": edge.field_path.clone(),
        "json_path": edge.meta.as_ref().and_then(|m| m.get("json_path")).and_then(|v| v.as_str()),
    })
}

fn build_model_io_facts(graph: &GraphDB, target_node: &crate::graph::Node) -> serde_json::Value {
    if target_node.node_type != crate::graph::NodeType::Model {
        return serde_json::json!({
            "result": null,
            "confidence": "none",
            "reads": [],
            "writes": [],
            "paths": [],
            "evidence_refs": [],
            "missing_evidence": ["model_io_facts only applies to model targets"],
        });
    }

    let mut reads = Vec::new();
    let mut writes = Vec::new();
    if let Some((outgoing, incoming)) = graph.get_node_edges(&target_node.id) {
        for (node, edge) in outgoing.iter().chain(incoming.iter()) {
            match edge.edge_type {
                crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                    reads.push(graph_edge_ref(node, edge));
                }
                crate::graph::EdgeType::Writes
                | crate::graph::EdgeType::ActionWrites
                | crate::graph::EdgeType::FieldWrite => {
                    writes.push(graph_edge_ref(node, edge));
                }
                _ => {}
            }
        }
    }
    reads.truncate(5);
    writes.truncate(5);
    let has_io = !reads.is_empty() || !writes.is_empty();

    serde_json::json!({
        "result": if has_io {
            "model_io_summary_found"
        } else {
            "no_model_io_edges_found"
        },
        "confidence": if has_io { "medium" } else { "low" },
        "reads": reads,
        "writes": writes,
        "paths": [],
        "evidence_refs": [],
        "missing_evidence": if has_io {
            serde_json::json!([])
        } else {
            serde_json::json!(["no read/write graph edges found for model target"])
        },
    })
}

fn build_primary_reason_for_intent(
    intent: TraversalIntent,
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
    primary_path: &[serde_json::Value],
    value_source_context: &Option<serde_json::Value>,
    answer_facts: &serde_json::Value,
) -> String {
    match intent {
        TraversalIntent::Writer => {
            let writer_paths = answer_facts
                .get("writer_facts")
                .and_then(|v| v.get("paths"))
                .and_then(|v| v.as_array())
                .map(|items| items.len())
                .unwrap_or(0);
            if writer_paths > 0 {
                return format!(
                    "目标 {} 的写入/生成链路已找到，共 {} 条 writer 路径",
                    target_node.id, writer_paths
                );
            }
            return format!("目标 {} 未发现明确 writer 链路", target_node.id);
        }
        TraversalIntent::ValueSource => {
            if let Some(result) = answer_facts
                .get("value_source_facts")
                .and_then(|v| v.get("result"))
                .and_then(|v| v.as_str())
            {
                return format!("目标 {} 的值来源已解析到 {}", target_node.id, result);
            }
            return format!("目标 {} 未发现明确值来源", target_node.id);
        }
        TraversalIntent::Display => {
            if !blocking_conditions.is_empty() {
                return format!(
                    "目标 {} 受 {} 个显示/启用条件影响",
                    target_node.id,
                    blocking_conditions.len()
                );
            }
            if !data_empty_gates.is_empty() {
                return format!(
                    "目标 {} 的显示受 {} 个数据门控展开条件影响",
                    target_node.id,
                    data_empty_gates.len()
                );
            }
            return format!("目标 {} 未发现明确显示条件", target_node.id);
        }
        TraversalIntent::Availability => {
            if !data_empty_gates.is_empty() {
                return format!(
                    "目标 {} 受 {} 个数据门控影响（filter/totalRowCount__）",
                    target_node.id,
                    data_empty_gates.len()
                );
            }
            return format!("目标 {} 未发现明确数据可用性门控", target_node.id);
        }
        TraversalIntent::Context | TraversalIntent::Auto => {}
    }

    if target_node.node_type == crate::graph::NodeType::Page {
        if !blocking_conditions.is_empty() {
            format!(
                "页面 {} 有 {} 个阻塞条件（visible/disable/action condition）",
                target_node.name,
                blocking_conditions.len()
            )
        } else if !data_empty_gates.is_empty() {
            format!(
                "页面 {} 有 {} 个数据门控（filter/totalRowCount__）",
                target_node.name,
                data_empty_gates.len()
            )
        } else {
            format!("页面 {} 未发现明确的阻塞条件或数据门控", target_node.name)
        }
    } else if !blocking_conditions.is_empty() {
        format!(
            "目标 {} 受 {} 个阻塞条件影响（visible/disable/action condition）",
            target_node.id,
            blocking_conditions.len()
        )
    } else if !data_empty_gates.is_empty() {
        format!(
            "目标 {} 受 {} 个数据门控影响（filter/totalRowCount__）",
            target_node.id,
            data_empty_gates.len()
        )
    } else if !primary_path.is_empty() {
        format!(
            "目标 {} 的数据链路已找到，共 {} 条主路径",
            target_node.id,
            primary_path.len()
        )
    } else if value_source_context.is_some() {
        format!("目标 {} 的裸字段值来源已解析到最近数据容器", target_node.id)
    } else {
        format!("目标 {} 未发现明确的阻塞条件或数据链路", target_node.id)
    }
}

fn build_traversal_policy(intent: TraversalIntent, budget: &str) -> serde_json::Value {
    let (max_paths, max_steps_per_path) = match budget {
        "compact" => (3, 6),
        "full" => (10, 12),
        _ => (5, 8),
    };
    serde_json::json!({
        "intent": intent.as_str(),
        "max_paths": max_paths,
        "max_steps_per_path": max_steps_per_path,
        "allowed_edge_types": match intent {
            TraversalIntent::Display => serde_json::json!(["DependsOn", "Contains"]),
            TraversalIntent::ValueSource => serde_json::json!(["Reads", "FieldAlias", "DataflowOutput", "DataflowInput"]),
            TraversalIntent::Writer => serde_json::json!(["Reads(target bridge)", "Writes", "ActionWrites", "FieldWrite", "FieldAlias"]),
            TraversalIntent::Availability => serde_json::json!(["DependsOn", "DataflowInput"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["Reads", "Writes", "ActionWrites", "FieldWrite", "DependsOn", "Contains"]),
        },
        "directions": match intent {
            TraversalIntent::Writer => serde_json::json!(["incoming", "reverse_alias"]),
            TraversalIntent::Display => serde_json::json!(["incoming", "ancestor"]),
            TraversalIntent::ValueSource => serde_json::json!(["outgoing", "upstream"]),
            TraversalIntent::Availability => serde_json::json!(["incoming", "filter_refs"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["incoming", "outgoing"]),
        },
        "stop_conditions": match intent {
            TraversalIntent::Display => serde_json::json!(["display_condition_found", "expanded_model_filter_found"]),
            TraversalIntent::ValueSource => serde_json::json!(["proven_physical_input_found", "table_source_path_found", "field_origin_unprovable"]),
            TraversalIntent::Writer => serde_json::json!(["writer_path_found"]),
            TraversalIntent::Availability => serde_json::json!(["filter_or_total_row_count_gate_found"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["path_budget_exhausted"]),
        },
        "rank_rules": ["field-level paths before model-level paths", "proven paths before candidates", "paths with evidence before inferred paths"],
    })
}

fn build_answer_facts(
    graph: &GraphDB,
    intent: TraversalIntent,
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
    value_source_context: &Option<serde_json::Value>,
    primary_path: &[serde_json::Value],
    budget: &str,
    dataflow_meta: Option<&crate::query::DataFlowMeta>,
    dataflow_model_id: Option<&str>,
) -> serde_json::Value {
    let traversal_policy = build_traversal_policy(intent, budget);
    let mut facts = serde_json::Map::new();
    facts.insert("intent".to_string(), serde_json::json!(intent.as_str()));

    if answer_fact_enabled(intent, target_node, AnswerFactKind::Display) {
        facts.insert(
            "display_facts".to_string(),
            build_display_facts(target_node, blocking_conditions, data_empty_gates),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::ValueSource) {
        facts.insert(
            "value_source_facts".to_string(),
            build_value_source_facts(value_source_context),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Writer) {
        facts.insert("writer_facts".to_string(), build_writer_facts(primary_path));
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Availability) {
        facts.insert(
            "availability_facts".to_string(),
            build_availability_facts(
                data_empty_gates,
                &target_node.path,
                &collect_dataflow_input_paths(graph, dataflow_model_id),
                dataflow_meta,
            ),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Context) {
        facts.insert(
            "context_facts".to_string(),
            serde_json::json!({
                "result": "use_context_command_for_broad_neighbors",
                "confidence": "medium",
                "paths": [],
                "evidence_refs": [],
                "missing_evidence": ["use --context for broad surrounding context"],
            }),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::ModelIo) {
        facts.insert(
            "model_io_facts".to_string(),
            build_model_io_facts(graph, target_node),
        );
    }

    facts.insert("traversal_policy".to_string(), traversal_policy);
    serde_json::Value::Object(facts)
}

fn collect_dataflow_input_paths(graph: &GraphDB, target_model_id: Option<&str>) -> Vec<String> {
    let target_model_id = match target_model_id {
        Some(id) => id,
        None => return Vec::new(),
    };

    let mut paths = Vec::new();
    if let Some((outgoing, _)) = graph.get_node_edges(target_model_id) {
        for (_, edge) in outgoing {
            if !matches!(edge.edge_type, crate::graph::EdgeType::DataflowInput) {
                continue;
            }
            if let Some(path) = edge.field_path.as_deref() {
                if !path.is_empty() && !paths.contains(&path.to_string()) {
                    paths.push(path.to_string());
                }
                continue;
            }
            if let Some(path) = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_file"))
                .and_then(|v| v.as_str())
            {
                if !path.is_empty() && !paths.contains(&path.to_string()) {
                    paths.push(path.to_string());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// 构建上下文数组摘要，用于 compact 模式隐藏大体量旁路明细
fn build_context_summary(
    items: &[serde_json::Value],
    emitted_count: usize,
    hidden: bool,
    note: &str,
) -> serde_json::Value {
    let mut by_scope: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut by_source_file: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for item in items {
        let scope = item
            .get("condition_scope")
            .or_else(|| item.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        *by_scope.entry(scope.to_string()).or_insert(0) += 1;

        if let Some(source_file) = item.get("source_file").and_then(|v| v.as_str()) {
            *by_source_file.entry(source_file.to_string()).or_insert(0) += 1;
        }
    }

    let source_file_distinct_count = by_source_file.len();
    let by_source_file = if hidden {
        std::collections::BTreeMap::new()
    } else {
        by_source_file
    };

    serde_json::json!({
        "total_count": items.len(),
        "emitted_count": emitted_count,
        "hidden": hidden,
        "note": note,
        "by_scope": by_scope,
        "by_source_file": by_source_file,
        "source_file_distinct_count": source_file_distinct_count,
    })
}

fn path_edge_types(path: &serde_json::Value) -> Vec<String> {
    path.get("segments")
        .and_then(|v| v.as_array())
        .map(|segments| {
            segments
                .iter()
                .filter_map(|segment| {
                    segment
                        .get("edge")
                        .and_then(|edge| edge.get("edge_type"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn path_has_any_edge(path: &serde_json::Value, edge_types: &[&str]) -> bool {
    let edges = path_edge_types(path);
    edges
        .iter()
        .any(|edge| edge_types.iter().any(|allowed| edge == allowed))
}

fn path_reject_reason_for_intent(
    path: &serde_json::Value,
    intent: TraversalIntent,
) -> Option<&'static str> {
    let edges = path_edge_types(path);
    if edges.is_empty() {
        return Some("missing_path_segments");
    }

    match intent {
        TraversalIntent::Display => {
            if edges
                .iter()
                .all(|edge| matches!(edge.as_str(), "DependsOn" | "Contains"))
            {
                None
            } else {
                Some("edge_type_not_allowed_for_display_intent")
            }
        }
        TraversalIntent::ValueSource => {
            if edges.iter().all(|edge| {
                matches!(
                    edge.as_str(),
                    "Reads" | "ActionReads" | "FieldAlias" | "DataflowOutput" | "DataflowInput"
                )
            }) {
                None
            } else {
                Some("edge_type_not_allowed_for_value_source_intent")
            }
        }
        TraversalIntent::Writer => {
            let has_writer_edge =
                path_has_any_edge(path, &["Writes", "ActionWrites", "FieldWrite"]);
            let only_writer_chain_edges = edges.iter().all(|edge| {
                matches!(
                    edge.as_str(),
                    "Reads" | "FieldAlias" | "Writes" | "ActionWrites" | "FieldWrite"
                )
            });
            if has_writer_edge && only_writer_chain_edges {
                None
            } else if !has_writer_edge {
                Some("writer_intent_requires_write_edge")
            } else {
                Some("edge_type_not_allowed_for_writer_intent")
            }
        }
        TraversalIntent::Availability => {
            if edges
                .iter()
                .all(|edge| matches!(edge.as_str(), "DependsOn" | "DataflowInput"))
            {
                None
            } else {
                Some("edge_type_not_allowed_for_availability_intent")
            }
        }
        TraversalIntent::Context | TraversalIntent::Auto => None,
    }
}

fn rejected_path(path: serde_json::Value, reject_reason: &str) -> serde_json::Value {
    let mut value = path;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "reject_reason".to_string(),
            serde_json::json!(reject_reason),
        );
    }
    value
}

/// 将路径选择结果按当前 intent 分为证明、候选、拒绝三类
fn partition_paths_for_intent(
    selection: crate::path::PathSelectionResult,
    intent: TraversalIntent,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
) {
    let mut proven_paths = Vec::new();
    let mut candidate_paths = Vec::new();
    let mut rejected_paths = Vec::new();

    for path in selection.primary_paths {
        let path_json = path.to_json();
        if let Some(reason) = path_reject_reason_for_intent(&path_json, intent) {
            rejected_paths.push(rejected_path(path_json, reason));
        } else {
            proven_paths.push(path_json);
        }
    }

    for path in selection
        .candidate_paths
        .into_iter()
        .chain(selection.supporting_paths)
        .chain(selection.related_context)
    {
        let path_json = path.to_json();
        if let Some(reason) = path_reject_reason_for_intent(&path_json, intent) {
            rejected_paths.push(rejected_path(path_json, reason));
        } else {
            candidate_paths.push(path_json);
        }
    }

    for path in selection.rejected_paths {
        rejected_paths.push(rejected_path(path.to_json(), "path_selector_rejected"));
    }

    (proven_paths, candidate_paths, rejected_paths)
}

/// 对 `model.totalRowCount__` 门控展开当前页面模型 filter
fn expand_total_row_count_gates(
    graph: &GraphDB,
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
fn classify_condition(cond_obj: &serde_json::Value) -> &'static str {
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
fn collect_conditions_for_node(
    graph: &GraphDB,
    node_id: &str,
    _page_path: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    if let Some((_out, incoming)) = graph.get_node_edges(node_id) {
        for (source, edge) in &incoming {
            if !matches!(source.node_type, crate::graph::NodeType::Condition) {
                continue;
            }
            if !matches!(edge.edge_type, crate::graph::EdgeType::DependsOn) {
                continue;
            }
            if !seen.insert(source.id.clone()) {
                continue;
            }
            let cond_obj = build_cond_obj(source);
            results.push(cond_obj);
        }
    }
    results
}

/// 解释目标节点为什么具有当前状态（为什么不显示/为什么不可用/为什么数据为空）
///
/// 支持的目标格式：
/// - `comp:PAGE|ID` — 解释组件为什么不显示或为什么不可用
/// - `model:ID` — 解释模型为什么可能为空
/// - `field:MODEL.FIELD` — 解释字段值来源或为什么为空
/// - `page:PATH` — 解释页面主要条件门控和数据链路
///
/// 将 explain-condition JSON 结果渲染为人类可读摘要文本
///
/// 供 runtime 在 human 模式下复用，不直接打印到 stdout
pub fn render_explain_condition_human(result: &serde_json::Value, target_id: &str) -> String {
    let obj = match result.as_object() {
        Some(o) => o,
        None => return format!("Invalid explain output for {}", target_id),
    };
    let summary = obj.get("summary").and_then(|v| v.as_object());
    let details = obj.get("details").and_then(|v| v.as_object());
    let target = details
        .and_then(|d| d.get("target"))
        .and_then(|t| t.get("node_id"))
        .and_then(|v| v.as_str())
        .unwrap_or(target_id);
    let primary_reason = summary
        .and_then(|s| s.get("primary_reason").and_then(|v| v.as_str()))
        .unwrap_or("");
    let empty_arr: Vec<serde_json::Value> = Vec::new();
    let blocking = details
        .and_then(|d| d.get("blocking_conditions").and_then(|v| v.as_array()))
        .unwrap_or(&empty_arr);
    let gates = details
        .and_then(|d| d.get("data_empty_gates").and_then(|v| v.as_array()))
        .unwrap_or(&empty_arr);
    let paths = details
        .and_then(|d| d.get("primary_path").and_then(|v| v.as_array()))
        .unwrap_or(&empty_arr);

    let mut lines = Vec::new();
    lines.push(format!("=== Why: {} ===", target));
    lines.push(format!("Primary reason: {}", primary_reason));
    if !blocking.is_empty() {
        lines.push("Blocking conditions:".to_string());
        for c in blocking {
            lines.push(format!(
                "  - {}: {}",
                c.get("condition_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                c.get("raw_expr").and_then(|v| v.as_str()).unwrap_or("?"),
            ));
        }
    }
    if !gates.is_empty() {
        lines.push("Data empty gates:".to_string());
        for c in gates {
            lines.push(format!(
                "  - {}: {}",
                c.get("condition_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                c.get("raw_expr").and_then(|v| v.as_str()).unwrap_or("?"),
            ));
        }
    }
    if !paths.is_empty() {
        lines.push("Primary paths:".to_string());
        for p in paths {
            lines.push(format!(
                "  {}",
                serde_json::to_string(p).unwrap_or_default()
            ));
        }
    }
    lines.join(
        "
",
    )
}

/// CLI 包装：explain-condition 输出到 stdout
///
/// 内部调用 build_explain_condition_output 获取结构化结果，再按 human/JSON 格式打印
pub fn explain_condition_target(
    graph: &GraphDB,
    target_id: &str,
    human: bool,
    budget: &str,
    intent: TraversalIntent,
) -> Result<()> {
    let result = build_explain_condition_output_with_intent(graph, target_id, budget, intent)?;

    if human {
        let text = render_explain_condition_human(&result, target_id);
        println!("{}", text);
    } else {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }

    Ok(())
}

/// 构建 explain-condition 结构化 JSON 输出，不直接打印
///
/// 返回纯 JSON Value，供 CLI 包装或 runtime 复用
pub fn build_explain_condition_output(
    graph: &GraphDB,
    target_id: &str,
    _budget: &str,
) -> Result<serde_json::Value> {
    build_explain_condition_output_with_intent(graph, target_id, _budget, TraversalIntent::Auto)
}

/// 构建带 M33 intent 的 explain-condition 结构化 JSON 输出，不直接打印
pub fn build_explain_condition_output_with_intent(
    graph: &GraphDB,
    target_id: &str,
    _budget: &str,
    intent: TraversalIntent,
) -> Result<serde_json::Value> {
    let (target_node, scoped_page_node, dataflow_model_id) =
        if let Some((page_ref, local_model_id)) = parse_scoped_model_target(target_id) {
            if let Some((scoped_model, page_node, scoped_df_model_id)) =
                resolve_model_target_in_page(graph, &page_ref, &local_model_id)
            {
                (scoped_model, Some(page_node), scoped_df_model_id)
            } else {
                let candidates = find_local_candidates(graph, target_id);
                let out = crate::output::schema::build_target_not_found_output(
                    crate::output::schema::OutputKind::Explain,
                    target_id,
                    &candidates,
                );
                return Ok(serde_json::to_value(out)?);
            }
        } else if let Some(node) = graph.get_node(target_id) {
            (node, None, None)
        } else {
            let candidates = find_local_candidates(graph, target_id);
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::Explain,
                target_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        };

    let effective_intent = intent;
    let is_page_scoped_target = scoped_page_node.is_some();

    let page_node = if let Some(page_node) = scoped_page_node {
        page_node
    } else {
        match target_node.node_type {
            crate::graph::NodeType::Page => target_node.clone(),
            _ => find_parent_page(graph, &target_node.id).unwrap_or(target_node.clone()),
        }
    };
    let page_path = page_node.path.clone();

    let mut blocking_conditions: Vec<serde_json::Value> = Vec::new();
    let mut data_empty_gates: Vec<serde_json::Value> = Vec::new();
    let mut supporting_context: Vec<serde_json::Value> = Vec::new();
    let mut related_context: Vec<serde_json::Value> = Vec::new();
    let mut seen_conditions: std::collections::HashSet<String> = std::collections::HashSet::new();

    let target_node_ids: Vec<String> = if target_node.node_type == crate::graph::NodeType::Page {
        let mut ids = vec![target_node.id.clone()];
        if let Some((outgoing, _incoming)) = graph.get_node_edges(&target_node.id) {
            for (child, edge) in &outgoing {
                if matches!(
                    edge.edge_type,
                    crate::graph::EdgeType::Contains | crate::graph::EdgeType::Triggers
                ) {
                    ids.push(child.id.clone());
                    if let Some((child_out, _)) = graph.get_node_edges(&child.id) {
                        for (grandchild, gedge) in &child_out {
                            if matches!(
                                gedge.edge_type,
                                crate::graph::EdgeType::Contains | crate::graph::EdgeType::Triggers
                            ) {
                                ids.push(grandchild.id.clone());
                            }
                        }
                    }
                }
            }
        }
        ids
    } else {
        let mut ids = vec![target_node.id.clone()];
        if target_node.node_type == crate::graph::NodeType::Component {
            if let Some((outgoing, _incoming)) = graph.get_node_edges(&target_node.id) {
                for (child, edge) in &outgoing {
                    if matches!(edge.edge_type, crate::graph::EdgeType::Triggers) {
                        ids.push(child.id.clone());
                    }
                }
            }
        }
        ids
    };

    for node_id in &target_node_ids {
        let conds = collect_conditions_for_node(graph, node_id, &page_path, &mut seen_conditions);
        for cond_obj in conds {
            let source_file = cond_obj
                .get("source_file")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let cond_obj = annotate_condition_scope(cond_obj, "direct", None, None, None);
            if source_file == page_path {
                let is_model_filter_dependency =
                    cond_obj.get("owner_type").and_then(|v| v.as_str()) == Some("ModelSource")
                        && !condition_owned_by_node(&cond_obj, node_id);
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

                match classify_condition(&cond_obj) {
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
    }

    if target_node.node_type == crate::graph::NodeType::Component {
        for (ancestor_id, distance) in component_ancestor_chain(graph, &target_node.id) {
            let conds =
                collect_conditions_for_node(graph, &ancestor_id, &page_path, &mut seen_conditions);
            for cond_obj in conds {
                let source_file = cond_obj
                    .get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let cond_obj = annotate_condition_scope(
                    cond_obj,
                    "inherited",
                    Some(&ancestor_id),
                    Some(distance),
                    None,
                );
                if source_file == page_path {
                    match classify_condition(&cond_obj) {
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
        }

        let target_paths = component_json_paths(graph, &target_node.id);
        for cond_obj in collect_inherited_conditions_by_json_path(
            graph,
            &page_path,
            &target_paths,
            &mut seen_conditions,
        ) {
            match classify_condition(&cond_obj) {
                "blocking" => blocking_conditions.push(cond_obj),
                "data_empty" => data_empty_gates.push(cond_obj),
                _ => supporting_context.push(cond_obj),
            }
        }
    }

    if target_node.node_type == crate::graph::NodeType::Model && !is_page_scoped_target {
        let mut page_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
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
                    match classify_condition(&rc) {
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

    let expanded_gates = expand_total_row_count_gates(
        graph,
        &blocking_conditions,
        &page_path,
        &mut seen_conditions,
    );
    data_empty_gates.extend(expanded_gates);

    // M34: DataFlow availability projection — inject internal filters into data_empty_gates
    if is_dataflow_model(&target_node) {
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

    let value_source_context = build_value_source_context(graph, &target_node, &page_path);
    blocking_conditions = dedupe_conditions(blocking_conditions);
    data_empty_gates = dedupe_conditions(data_empty_gates);
    supporting_context = dedupe_conditions(supporting_context);

    let mut primary_path: Vec<serde_json::Value> = Vec::new();
    let mut candidate_paths: Vec<serde_json::Value> = Vec::new();
    let mut rejected_paths: Vec<serde_json::Value> = Vec::new();
    if target_node.id.starts_with("field:") || target_node.id.starts_with("comp:") {
        let path_query = crate::path::PathQuery {
            page_id: format!("page:{}", page_path),
            page_path: page_path.clone(),
            target_anchors: vec![target_node.id.clone()],
            source_anchors: Vec::new(),
            sink_anchors: Vec::new(),
            bridge_anchors: Vec::new(),
            excluded_anchors: Vec::new(),
            budget: _budget.to_string(),
        };
        let finder = crate::path::BoundedCausalPathFinder::default();
        let mut candidates = finder.find_candidates(graph, &path_query);

        if target_node.id.starts_with("comp:") {
            if let Some((outgoing, _incoming)) = graph.get_node_edges(&target_node.id) {
                for (target, edge) in &outgoing {
                    if matches!(
                        edge.edge_type,
                        crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads
                    ) && edge.field_path.is_some()
                    {
                        let data_source = serde_json::json!({
                            "source_component": target_node.id,
                            "target_id": target.id,
                            "field_path": edge.field_path,
                            "raw_expr": edge.meta.as_ref().and_then(|m| m.get("source_expr")).and_then(|v| v.as_str()),
                            "json_path": edge.meta.as_ref().and_then(|m| m.get("json_path")).and_then(|v| v.as_str()),
                        });
                        let field_candidates =
                            crate::path::build_field_causal_paths_for_data_source(
                                graph,
                                &page_node,
                                &data_source,
                            );
                        candidates.extend(field_candidates);
                    }
                }
            }
        } else if target_node.id.starts_with("field:") {
            let parts: Vec<&str> = target_node.id.split('.').collect();
            if parts.len() >= 2 {
                let model_id = parts[0];
                let field_name = parts[1..].join(".");
                let data_source = serde_json::json!({
                    "source_component": target_node.id,
                    "target_id": target_node.id,
                    "field_path": format!("{}.{}", model_id, field_name),
                    "raw_expr": format!("${{{}}}", target_node.id),
                });
                let field_candidates = crate::path::build_field_causal_paths_for_data_source(
                    graph,
                    &page_node,
                    &data_source,
                );
                candidates.extend(field_candidates);
            }
        }

        {
            let mut seen = std::collections::HashSet::new();
            candidates.retain(|c| seen.insert(c.path_id.clone()));
        }

        let selector = crate::path::RuleBasedPathSelector;
        let selection = selector.select(&path_query, candidates);
        let (proven, candidates, rejected) =
            partition_paths_for_intent(selection, effective_intent);
        primary_path = proven;
        candidate_paths = candidates;
        rejected_paths = rejected;
    }

    let dataflow_meta = if is_dataflow_model(&target_node) {
        target_node
            .meta
            .as_ref()
            .map(crate::query::DataFlowMeta::from_meta)
    } else {
        None
    };
    let answer_facts = build_answer_facts(
        graph,
        effective_intent,
        &target_node,
        &blocking_conditions,
        &data_empty_gates,
        &value_source_context,
        &primary_path,
        _budget,
        dataflow_meta.as_ref(),
        dataflow_model_id.as_deref(),
    );
    let traversal_policy = answer_facts
        .get("traversal_policy")
        .cloned()
        .unwrap_or_else(|| build_traversal_policy(effective_intent, _budget));

    let primary_reason = build_primary_reason_for_intent(
        effective_intent,
        &target_node,
        &blocking_conditions,
        &data_empty_gates,
        &primary_path,
        &value_source_context,
        &answer_facts,
    );

    let compact_budget = _budget == "compact";
    let details_primary_path = if compact_budget {
        Vec::new()
    } else {
        primary_path.clone()
    };
    let details_proven_paths = if compact_budget {
        Vec::new()
    } else {
        primary_path.clone()
    };
    let details_candidate_paths = if compact_budget {
        Vec::new()
    } else {
        candidate_paths.clone()
    };
    let details_rejected_paths = if compact_budget {
        Vec::new()
    } else {
        rejected_paths.clone()
    };
    let primary_path_summary = build_context_summary(
        &primary_path,
        details_primary_path.len(),
        compact_budget && !primary_path.is_empty(),
        if compact_budget {
            "compact 模式隐藏 primary_path 明细；answer_facts.paths 已保留短证据"
        } else {
            "primary_path 明细已展开"
        },
    );
    let candidate_path_summary = build_context_summary(
        &candidate_paths,
        details_candidate_paths.len(),
        compact_budget && !candidate_paths.is_empty(),
        if compact_budget {
            "compact 模式隐藏 candidate_paths 明细；需要核查时使用 --budget normal 或 full"
        } else {
            "candidate_paths 明细已展开"
        },
    );
    let rejected_path_summary = build_context_summary(
        &rejected_paths,
        details_rejected_paths.len(),
        compact_budget && !rejected_paths.is_empty(),
        if compact_budget {
            "compact 模式隐藏 rejected_paths 明细；需要排查路径拒绝原因时使用 --budget normal 或 full"
        } else {
            "rejected_paths 明细已展开"
        },
    );
    let details_value_source_context = if compact_budget {
        serde_json::Value::Null
    } else {
        value_source_context
            .clone()
            .unwrap_or(serde_json::Value::Null)
    };
    let details_supporting_context = if compact_budget {
        Vec::new()
    } else {
        supporting_context.clone()
    };
    let details_related_context = if compact_budget {
        Vec::new()
    } else {
        related_context.clone()
    };
    let supporting_context_summary = build_context_summary(
        &supporting_context,
        details_supporting_context.len(),
        compact_budget && !supporting_context.is_empty(),
        if compact_budget {
            "compact 模式隐藏 supporting_context 明细；需要核查时使用 --budget normal 或 full"
        } else {
            "supporting_context 明细已展开"
        },
    );
    let related_context_summary = build_context_summary(
        &related_context,
        details_related_context.len(),
        compact_budget && !related_context.is_empty(),
        if compact_budget {
            "compact 模式隐藏 related_context 明细；它们是相关但非必要上下文"
        } else {
            "related_context 是相关但非必要上下文"
        },
    );

    let summary = serde_json::json!({
        "what_is_it": format!("Why 解释: {}", target_node.id),
        "target_id": target_node.id,
        "target_type": format!("{:?}", target_node.node_type),
        "target_name": target_node.name,
        "intent": effective_intent.as_str(),
        "primary_reason": primary_reason,
        "answer_facts_count": answer_facts
            .as_object()
            .map(|obj| {
                obj.keys()
                    .filter(|k| k.ends_with("_facts") && k.as_str() != "inactive_facts")
                    .count()
            })
            .unwrap_or(0),
        "blocking_conditions_count": blocking_conditions.len(),
        "data_empty_gates_count": data_empty_gates.len(),
        "primary_paths_count": primary_path.len(),
        "candidate_paths_count": candidate_paths.len(),
        "rejected_paths_count": rejected_paths.len(),
        "value_source_context_count": usize::from(value_source_context.is_some()),
        "supporting_context_count": supporting_context.len(),
        "related_context_count": related_context.len(),
    });

    let details = serde_json::json!({
        "target": {
            "node_id": target_node.id,
            "node_type": format!("{:?}", target_node.node_type),
            "name": target_node.name,
            "path": target_node.path,
        },
        "primary_path": details_primary_path,
        "primary_path_summary": primary_path_summary,
        "answer_facts": answer_facts,
        "traversal_policy": traversal_policy,
        "proven_paths": details_proven_paths,
        "candidate_paths": details_candidate_paths,
        "candidate_paths_summary": candidate_path_summary,
        "rejected_paths": details_rejected_paths,
        "rejected_paths_summary": rejected_path_summary,
        "value_source_context": details_value_source_context,
        "blocking_conditions": blocking_conditions,
        "data_empty_gates": data_empty_gates,
        "supporting_context": details_supporting_context,
        "supporting_context_summary": supporting_context_summary,
        "related_context": details_related_context,
        "related_context_summary": related_context_summary,
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(target_id.to_string());
    output.details = Some(details);

    Ok(serde_json::to_value(output)?)
}

/// 辅助：在页面范围内查找近似候选目标
/// 计算两个字符串的最长公共前缀长度
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars()
        .zip(b.chars())
        .take_while(|(ca, cb)| ca == cb)
        .count()
}

/// 判断字符串是否全为数字
fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn find_local_candidates(graph: &GraphDB, target_id: &str) -> Vec<(crate::graph::Node, String)> {
    let mut candidates = Vec::new();
    let target_lower = target_id.to_lowercase();

    let target_prefix = if target_id.starts_with("model:") {
        Some("model")
    } else if target_id.starts_with("page:") {
        Some("page")
    } else if target_id.starts_with("comp:") {
        Some("comp")
    } else if target_id.starts_with("action:") {
        Some("action")
    } else if target_id.starts_with("field:") {
        Some("field")
    } else {
        None
    };

    let target_bare = target_id
        .strip_prefix("model:")
        .or_else(|| target_id.strip_prefix("page:"))
        .or_else(|| target_id.strip_prefix("comp:"))
        .or_else(|| target_id.strip_prefix("action:"))
        .or_else(|| target_id.strip_prefix("field:"))
        .unwrap_or(target_id);

    let target_page = if target_prefix == Some("comp") || target_prefix == Some("action") {
        target_bare.split('|').next().map(|s| s.to_string())
    } else {
        None
    };
    let target_id_part = if target_prefix == Some("comp") || target_prefix == Some("action") {
        target_bare.split('|').next_back().map(|s| s.to_string())
    } else {
        Some(target_bare.to_string())
    };

    for idx in graph.node_indices.values() {
        if let Some(node) = graph.graph.node_weight(*idx) {
            if node.id == target_id || node.id.trim().is_empty() || node.name.trim().is_empty() {
                continue;
            }
            let node_bare = node
                .id
                .strip_prefix("model:")
                .or_else(|| node.id.strip_prefix("page:"))
                .or_else(|| node.id.strip_prefix("comp:"))
                .or_else(|| node.id.strip_prefix("action:"))
                .or_else(|| node.id.strip_prefix("field:"))
                .unwrap_or(&node.id);

            let mut score = 0.0;
            let mut reason = "substring match";

            // 同页面组件优先
            if let Some(ref page) = target_page
                && node_bare.starts_with(page)
            {
                if let Some(ref id_part) = target_id_part {
                    let node_name_lower = node.name.to_lowercase();
                    let target_id_lower = id_part.to_lowercase();

                    // 双向子串匹配：input3 匹配 input33（前缀），input3 匹配 input13（包含子串）
                    if node_name_lower.contains(&target_id_lower)
                        || target_id_lower.contains(&node_name_lower)
                    {
                        score = 3.0;
                        reason = "same page component name match";
                    }

                    // 公共前缀 + 数字后缀近似：input33 ~ input3 / input13 / input23
                    if score == 0.0 {
                        let prefix_len = common_prefix_len(&node_name_lower, &target_id_lower);
                        if prefix_len >= 3 {
                            let node_suffix = &node_name_lower[prefix_len..];
                            let target_suffix = &target_id_lower[prefix_len..];
                            if is_all_digits(node_suffix) && is_all_digits(target_suffix) {
                                score = 2.5;
                                reason = "same prefix numeric suffix match";
                            }
                        }
                    }
                }
            }

            // 裸名精确匹配（不同前缀）
            if score == 0.0 && !target_bare.is_empty() && node_bare == target_bare {
                score = 5.0;
                reason = "bare name match with different prefix";
            }

            // 子串匹配
            if score == 0.0
                && (node.id.to_lowercase().contains(&target_lower)
                    || node.name.to_lowercase().contains(&target_lower))
            {
                score = 1.0;
            }

            if score > 0.0 {
                candidates.push((node.clone(), score, reason.to_string()));
            }
        }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    candidates
        .into_iter()
        .take(5)
        .map(|(n, _, r)| (n, r))
        .collect()
}

/// 构建 explain JSON 输出
///
/// 返回 Value，由外层调用者决定输出格式。
pub fn build_explain_output(graph: &GraphDB, node_id: &str) -> Result<Value> {
    let node = match graph.get_node(node_id) {
        Some(n) => n,
        None => {
            let candidates = graph.find_candidates(node_id, 5);
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::Explain,
                node_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };

    let (outgoing, incoming) = graph
        .get_node_edges(node_id)
        .unwrap_or_else(|| (Vec::new(), Vec::new()));

    let value = match node.node_type {
        crate::graph::NodeType::Component => {
            explain_component_graph(graph, &node, outgoing, incoming, false)?
        }
        crate::graph::NodeType::Action => {
            explain_action_graph(graph, &node, outgoing, incoming, false)?
        }
        crate::graph::NodeType::Model => {
            if is_dataflow_model(&node) {
                explain_dataflow_graph(graph, &node, outgoing, incoming, false)?
            } else {
                explain_model_graph(graph, &node, outgoing, incoming, false)?
            }
        }
        crate::graph::NodeType::Field => {
            explain_field_graph(graph, &node, outgoing, incoming, false)?
        }
        crate::graph::NodeType::Page => {
            explain_page_graph(graph, &node, outgoing, incoming, false)?
        }
        crate::graph::NodeType::Condition => {
            explain_condition_graph(graph, &node, outgoing, incoming, false)?
        }
    };
    Ok(value)
}

/// 解释图节点
///
/// CLI 包装器，负责调用 `build_explain_output` 并按 `human` 参数决定输出格式。
pub fn explain_node_graph(graph: &GraphDB, node_id: &str, human: bool) -> Result<()> {
    if human {
        let node = match graph.get_node(node_id) {
            Some(n) => n,
            None => {
                let candidates = graph.find_candidates(node_id, 5);
                let out = crate::output::schema::build_target_not_found_output(
                    crate::output::schema::OutputKind::Explain,
                    node_id,
                    &candidates,
                );
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
        };

        let (outgoing, incoming) = graph
            .get_node_edges(node_id)
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        match node.node_type {
            crate::graph::NodeType::Component => {
                let _ = explain_component_graph(graph, &node, outgoing, incoming, true)?;
            }
            crate::graph::NodeType::Action => {
                let _ = explain_action_graph(graph, &node, outgoing, incoming, true)?;
            }
            crate::graph::NodeType::Model => {
                if is_dataflow_model(&node) {
                    let _ = explain_dataflow_graph(graph, &node, outgoing, incoming, true)?;
                } else {
                    let _ = explain_model_graph(graph, &node, outgoing, incoming, true)?;
                }
            }
            crate::graph::NodeType::Field => {
                let _ = explain_field_graph(graph, &node, outgoing, incoming, true)?;
            }
            crate::graph::NodeType::Page => {
                let _ = explain_page_graph(graph, &node, outgoing, incoming, true)?;
            }
            crate::graph::NodeType::Condition => {
                let _ = explain_condition_graph(graph, &node, outgoing, incoming, true)?;
            }
        }
        Ok(())
    } else {
        let value = build_explain_output(graph, node_id)?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        Ok(())
    }
}

/// 解释条件节点：输出完整条件依赖路径，包含上游依赖、下游影响与每段 evidence
fn explain_condition_graph(
    _graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    // 从节点 meta 提取条件核心信息
    let meta = node.meta.as_ref().unwrap_or(&serde_json::Value::Null);
    let cond_id = node.id.split('|').next_back().unwrap_or(&node.id);
    let condition_type = meta
        .get("condition_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
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
    let owner_id = meta.get("owner_id").and_then(|v| v.as_str()).unwrap_or("");
    let effect_type = meta
        .get("effect_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
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

    // 分类 outgoing 边：owner 边（operation="Conditions"） vs upstream 边（operation="DependsOn"）
    let mut upstream_deps: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();
    let mut downstream_owners: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();
    let mut downstream_effects: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();

    for (target, edge) in &outgoing {
        let is_owner_edge = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("operation"))
            .and_then(|v| v.as_str())
            == Some("Conditions");
        if is_owner_edge {
            downstream_owners.push((target, edge));
        } else if target.name == "totalRowCount__" {
            downstream_effects.push((target, edge));
        } else {
            upstream_deps.push((target, edge));
        }
    }

    // 构建上游依赖详情
    let upstream_dependencies: Vec<serde_json::Value> = upstream_deps
        .iter()
        .map(|(target, edge)| {
            let sym = edge.field_path.as_deref().unwrap_or("?");
            let edge_raw_expr = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .unwrap_or(raw_expr);
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            let reason = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "symbol": sym,
                "target_node_id": target.id,
                "target_node_type": format!("{:?}", target.node_type),
                "target_name": target.name,
                "raw_expr": edge_raw_expr,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": reason,
            })
        })
        .collect();

    // 构建下游 owner 详情
    let downstream_owner_details: Vec<serde_json::Value> = downstream_owners
        .iter()
        .map(|(target, edge)| {
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            let reason = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "owner_node_id": target.id,
                "owner_node_type": format!("{:?}", target.node_type),
                "owner_name": target.name,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": reason,
            })
        })
        .collect();

    // 构建下游 effect 详情（totalRowCount__ 等）
    let downstream_effect_details: Vec<serde_json::Value> = downstream_effects
        .iter()
        .map(|(target, edge)| {
            let edge_raw_expr = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .unwrap_or(raw_expr);
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            serde_json::json!({
                "effect_type": "totalRowCount__",
                "target_node_id": target.id,
                "target_node_type": format!("{:?}", target.node_type),
                "target_name": target.name,
                "raw_expr": edge_raw_expr,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": "模型过滤条件决定命中行数",
            })
        })
        .collect();

    // 处理 incoming 边（当前 graph 中较少有指向 condition 的边，为健壮性保留）
    let incoming_nodes: Vec<serde_json::Value> = incoming
        .iter()
        .map(|(source, edge)| {
            serde_json::json!({
                "node_id": source.id,
                "node_type": format!("{:?}", source.node_type),
                "name": source.name,
                "edge_type": format!("{:?}", edge.edge_type),
                "field_path": edge.field_path,
                "source_file": source.path,
            })
        })
        .collect();

    let what = format!(
        "条件 {} ({}) 作用于 {}，表达式 '{}', 引用 {} 个上游符号，影响 {} 个下游对象",
        cond_id,
        condition_type,
        owner_id,
        normalized_expr,
        upstream_dependencies.len(),
        downstream_owner_details.len() + downstream_effect_details.len()
    );

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "condition",
        "condition_id": cond_id,
        "condition_type": condition_type,
        "effect_type": effect_type,
        "subject_type": subject_type,
        "owner_type": owner_type,
        "owner_id": owner_id,
        "raw_expr": raw_expr,
        "normalized_expr": normalized_expr,
        "json_path": json_path,
        "source_file": node.path,
        "referenced_symbols_count": referenced_symbols.len(),
        "upstream_dependency_count": upstream_dependencies.len(),
        "downstream_owner_count": downstream_owner_details.len(),
        "downstream_effect_count": downstream_effect_details.len(),
    });

    let details = serde_json::json!({
        "condition": {
            "condition_id": cond_id,
            "condition_type": condition_type,
            "effect_type": effect_type,
            "subject_type": subject_type,
            "raw_expr": raw_expr,
            "normalized_expr": normalized_expr,
            "json_path": json_path,
            "owner_type": owner_type,
            "owner_id": owner_id,
            "referenced_symbols": referenced_symbols,
            "source_file": node.path,
        },
        "upstream_dependencies": upstream_dependencies,
        "downstream_owners": downstream_owner_details,
        "downstream_effects": downstream_effect_details,
        "incoming_nodes": incoming_nodes,
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);

    // 条件自身证据
    output.evidence.push(
        crate::output::Evidence::new(
            format!(
                "Condition {} (type={}, effect={}, subject={}) defined at {} with {} referenced symbols",
                cond_id, condition_type, effect_type, subject_type, node.path, referenced_symbols.len()
            ),
            "Graph condition node with full metadata from scanner",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path)
        .with_json_path(json_path)
        .with_raw_expr(raw_expr),
    );

    // 上游依赖 evidence
    for (target, edge) in &upstream_deps {
        let sym = edge.field_path.as_deref().unwrap_or("?");
        let edge_raw_expr = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("source_expr"))
            .and_then(|v| v.as_str())
            .unwrap_or(raw_expr);
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Condition depends on {} via expression '{}'",
                    sym, edge_raw_expr
                ),
                "Condition references upstream symbol in raw expression",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_raw_expr(edge_raw_expr)
            .with_edge_type("DependsOn"),
        );
    }

    // 下游 owner evidence
    for (target, edge) in &downstream_owners {
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Condition {} controls {} (type={:?})",
                    cond_id, target.name, target.node_type
                ),
                "Condition determines owner component/action visibility or execution",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_edge_type("DependsOn"),
        );
    }

    // 下游 effect evidence（totalRowCount__ 等）
    for (target, edge) in &downstream_effects {
        let edge_raw_expr = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("source_expr"))
            .and_then(|v| v.as_str())
            .unwrap_or(raw_expr);
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Filter condition determines {} ({}), affecting row count gating",
                    target.name, edge_raw_expr
                ),
                "Model filter condition implicitly determines totalRowCount__ gating",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_raw_expr(edge_raw_expr)
            .with_edge_type("DependsOn"),
        );
    }

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        writeln!(out, "Condition Type: {}", condition_type)?;
        writeln!(out, "Effect: {}", effect_type)?;
        writeln!(out, "Raw Expr: {}", raw_expr)?;
        if !upstream_dependencies.is_empty() {
            writeln!(
                out,
                "
--- Upstream Dependencies ({}) ---",
                upstream_dependencies.len()
            )?;
            for d in &upstream_dependencies {
                writeln!(out, "  {:?}", d)?;
            }
        }
        if !downstream_owner_details.is_empty() {
            writeln!(
                out,
                "
--- Downstream Owners ({}) ---",
                downstream_owner_details.len()
            )?;
            for o in &downstream_owner_details {
                writeln!(out, "  {:?}", o)?;
            }
        }
        if !downstream_effect_details.is_empty() {
            writeln!(
                out,
                "
--- Downstream Effects ({}) ---",
                downstream_effect_details.len()
            )?;
            for e in &downstream_effect_details {
                writeln!(out, "  {:?}", e)?;
            }
        }
    }
    Ok(serde_json::to_value(output)?)
}

fn explain_component_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    let parent_page = find_parent_page(graph, &node.id);
    let comp_type = component_type_from_meta(node);

    let mut reads = Vec::new();
    let mut writes_models = Vec::new();
    let mut navigates_to = Vec::new();
    let mut affects_components = Vec::new();
    let mut triggers = Vec::new();
    let mut located_in = Vec::new();
    let mut triggered_by = Vec::new();
    let mut action_count = 0usize;
    let mut has_nav = false;
    let mut has_write = false;
    let mut has_read = false;
    let mut action_summaries: Vec<String> = Vec::new();

    for (target, edge) in &outgoing {
        match edge.edge_type {
            crate::graph::EdgeType::Reads => {
                reads.push(make_ref(target, edge, Some(&node.path)));
                has_read = true;
            }
            crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                writes_models.push(make_ref(target, edge, Some(&node.path)));
                has_write = true;
            }
            crate::graph::EdgeType::Triggers => {
                action_count += 1;
                triggers.push(serde_json::json!({
                    "id": target.id,
                    "name": target.name,
                    "type": format!("{:?}", target.node_type),
                    "edge_type": "Triggers",
                    "source_file": target.path,
                }));
                // Aggregate action reads/writes/navigation for semantic summary and component writes
                let mut action_reads = Vec::new();
                let mut action_writes = Vec::new();
                let mut action_nav = Vec::new();
                if let Some((action_out, _)) = graph.get_node_edges(&target.id) {
                    for (t, e) in &action_out {
                        match e.edge_type {
                            crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                                reads.push(make_ref(t, e, Some(&target.path)));
                                action_reads.push(make_ref(t, e, Some(&target.path)));
                                has_read = true;
                            }
                            crate::graph::EdgeType::Writes
                            | crate::graph::EdgeType::ActionWrites => {
                                writes_models.push(make_ref(t, e, Some(&target.path)));
                                action_writes.push(make_ref(t, e, Some(&target.path)));
                                has_write = true;
                            }
                            crate::graph::EdgeType::OpensPage
                            | crate::graph::EdgeType::ActionNavigates => {
                                has_nav = true;
                                navigates_to.push(make_ref(t, e, Some(&target.path)));
                                action_nav.push(make_ref(t, e, Some(&target.path)));
                            }
                            crate::graph::EdgeType::SetsParam
                            | crate::graph::EdgeType::ActionSetsParam => {
                                affects_components.push(make_ref(t, e, Some(&target.path)));
                                has_write = true;
                            }
                            crate::graph::EdgeType::ActionControlsComponent => {
                                affects_components.push(make_ref(t, e, Some(&target.path)));
                            }
                            crate::graph::EdgeType::ActionValidates => {
                                reads.push(make_ref(t, e, Some(&target.path)));
                                action_reads.push(make_ref(t, e, Some(&target.path)));
                                has_read = true;
                            }
                            crate::graph::EdgeType::ActionLoadsData => {
                                reads.push(make_ref(t, e, Some(&target.path)));
                                action_reads.push(make_ref(t, e, Some(&target.path)));
                                has_read = true;
                            }
                            _ => {}
                        }
                    }
                }
                let action_type = if let Some(pos) = target.name.find(':') {
                    target.name[..pos].to_string()
                } else {
                    "unknown".to_string()
                };
                let summary = crate::action_semantics::build_semantic_summary(
                    &action_type,
                    &node.name,
                    &target.name,
                    &action_reads,
                    &action_writes,
                    &action_nav,
                );
                action_summaries.push(summary);
            }
            crate::graph::EdgeType::OpensPage => {
                has_nav = true;
                navigates_to.push(make_ref(target, edge, Some(&node.path)));
            }
            crate::graph::EdgeType::SetsParam => {
                affects_components.push(make_ref(target, edge, Some(&node.path)));
            }
            crate::graph::EdgeType::ActionControlsComponent => {
                affects_components.push(make_ref(target, edge, Some(&node.path)));
            }
            _ => {}
        }
    }

    for (source, edge) in &incoming {
        match edge.edge_type {
            crate::graph::EdgeType::Reads => {
                triggered_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": "Reads",
                    "direction": "incoming",
                    "source_file": source.path,
                }));
            }
            crate::graph::EdgeType::Contains => {
                if matches!(source.node_type, crate::graph::NodeType::Page) {
                    located_in.push(serde_json::json!({
                        "id": source.id,
                        "name": source.name,
                        "type": "Page",
                        "edge_type": "Contains",
                        "direction": "incoming",
                        "source_file": source.path,
                    }));
                }
            }
            _ => {}
        }
    }

    let page_info = parent_page
        .as_ref()
        .map(|p| format!("，位于页面 {}", p.name))
        .unwrap_or_default();

    let what = if !action_summaries.is_empty() {
        format!(
            "{} {}，{}",
            comp_type,
            node.name,
            action_summaries.join("；")
        )
    } else if !reads.is_empty() {
        let first = reads[0].get("name").and_then(|v| v.as_str()).unwrap_or("?");
        format!("{} {}，读取 {}", comp_type, node.name, first)
    } else {
        format!("{} {}{}", comp_type, node.name, page_info)
    };

    let importance = classify_importance(
        has_nav,
        has_write,
        has_read,
        action_count > 0,
        &comp_type,
        &node.node_type,
    );

    let semantic_summary = action_summaries.join("；");

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "component",
        "type_detail": comp_type,
        "importance": importance,
        "semantic_summary": semantic_summary,
        "page": parent_page.as_ref().map(|p| p.name.clone()),
        "page_id": parent_page.as_ref().map(|p| p.id.clone()),
        "action_count": action_count,
        "read_count": reads.len(),
        "write_count": writes_models.len(),
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": reads,
        "writes": writes_models,
        "writes_models": writes_models,
        "located_in": located_in,
        "triggers": triggers,
        "navigates_to": navigates_to,
        "affects_components": affects_components,
        "triggered_by": triggered_by,
        "affects": {
            "components": affects_components,
            "models": writes_models,
            "pages": navigates_to,
        },
        "lineage": lineage,
    });

    let mut diagnostics: Vec<crate::output::Diagnostic> = Vec::new();
    // Only emit LINEAGE_SOURCE_MISSING for components that actually need
    // field-level lineage tracing: form inputs or components with model writes
    let needs_lineage = !writes_models.is_empty();
    if needs_lineage && lineage.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "LINEAGE_SOURCE_MISSING".to_string(),
            message: "Field-level lineage source could not be determined".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some(
                "Check --context or --query-dataflow for upstream relationships".to_string(),
            ),
        });
    }

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Component {} is defined in {}", node.id, node.path),
            "Graph node with Contains parent and expression edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if !reads.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Component {} reads from {} targets", node.id, reads.len()),
                "Outgoing Reads edges from component",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !writes_models.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Component {} writes to {} targets",
                    node.id,
                    writes_models.len()
                ),
                "Outgoing Writes/ActionWrites edges from component",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    push_relation_evidence(&mut output, "reads", &reads);
    push_relation_evidence(&mut output, "writes_models", &writes_models);
    push_relation_evidence(&mut output, "located_in", &located_in);
    push_relation_evidence(&mut output, "triggers", &triggers);
    push_relation_evidence(&mut output, "navigates_to", &navigates_to);
    push_relation_evidence(&mut output, "affects_components", &affects_components);
    output.next_queries = vec![
        format_next_query("--context {} --depth 2 for surrounding closure", &node.id),
        format_next_query(
            "--query-page-logic {} for page-level logic",
            parent_page
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_default()
                .as_str(),
        ),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !reads.is_empty() {
            writeln!(
                out,
                "
--- Reads ({}) ---",
                reads.len()
            )?;
            for r in &reads {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !writes_models.is_empty() {
            writeln!(
                out,
                "
--- Writes ({}) ---",
                writes_models.len()
            )?;
            for w in &writes_models {
                writeln!(out, "  {:?}", w)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}
fn explain_action_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    let parent_comp = incoming.iter().find_map(|(source, edge)| {
        if matches!(edge.edge_type, crate::graph::EdgeType::Triggers)
            && matches!(source.node_type, crate::graph::NodeType::Component)
        {
            Some((*source).clone())
        } else {
            None
        }
    });
    let parent_page = find_parent_page(graph, &node.id);

    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut triggered_by = Vec::new();
    let mut navigates_to = Vec::new();
    let mut affects_components = Vec::new();
    let mut has_nav = false;
    let mut has_write = false;
    let mut has_read = false;
    let mut action_type = "unknown".to_string();

    // Extract action type from name like "link:action1"
    if let Some(pos) = node.name.find(':') {
        action_type = node.name[..pos].to_string();
    }

    for (target, edge) in &outgoing {
        match edge.edge_type {
            crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                reads.push(make_ref(target, edge, Some(&node.path)));
                has_read = true;
            }
            crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                writes.push(make_ref(target, edge, Some(&node.path)));
                has_write = true;
            }
            crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::ActionNavigates => {
                has_nav = true;
                navigates_to.push(make_ref(target, edge, Some(&node.path)));
            }
            crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::ActionSetsParam => {
                affects_components.push(make_ref(target, edge, Some(&node.path)));
                has_write = true;
            }
            crate::graph::EdgeType::ActionControlsComponent => {
                affects_components.push(make_ref(target, edge, Some(&node.path)));
            }
            crate::graph::EdgeType::ActionValidates => {
                reads.push(make_ref(target, edge, Some(&node.path)));
                has_read = true;
            }
            crate::graph::EdgeType::ActionLoadsData => {
                reads.push(make_ref(target, edge, Some(&node.path)));
                has_read = true;
            }
            _ => {}
        }
    }

    for (source, edge) in &incoming {
        match edge.edge_type {
            crate::graph::EdgeType::Triggers => {
                triggered_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": "Triggers",
                    "direction": "incoming",
                    "source_file": source.path,
                }));
            }
            crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                triggered_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "direction": "incoming",
                    "source_file": source.path,
                }));
            }
            _ => {}
        }
    }

    let comp_info = parent_comp
        .as_ref()
        .map(|c| format!("由 {} 触发", c.name))
        .unwrap_or_else(|| "触发源未知".to_string());
    let read_info = if !reads.is_empty() {
        let first = reads[0].get("name").and_then(|v| v.as_str()).unwrap_or("?");
        format!("，读取 {}", first)
    } else {
        String::new()
    };
    let write_info = if !writes.is_empty() {
        let first = writes[0]
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        format!("，写入 {}", first)
    } else {
        String::new()
    };
    let _nav_info = if has_nav {
        "，并导航到新页面"
    } else {
        ""
    };

    let what = format!(
        "{} 动作 {}，{}{}{}",
        action_type, node.name, comp_info, read_info, write_info
    );
    let importance = classify_importance(has_nav, has_write, has_read, false, "", &node.node_type);

    // 从 graph 节点元数据提取原始 action 属性
    let meta_wait_prev = node
        .meta
        .as_ref()
        .and_then(|m| m.get("waitPrev"))
        .and_then(|v| v.as_str());
    let meta_condition = node
        .meta
        .as_ref()
        .and_then(|m| m.get("condition"))
        .and_then(|v| v.as_str());
    let meta_condition_exp = node
        .meta
        .as_ref()
        .and_then(|m| m.get("conditionExp"))
        .and_then(|v| v.as_str());
    let meta_trigger_type = node
        .meta
        .as_ref()
        .and_then(|m| m.get("triggerType"))
        .and_then(|v| v.as_str());

    let action_category = crate::action_semantics::classify_action(&action_type);
    let comp_name = parent_comp
        .as_ref()
        .map(|c| c.name.clone())
        .unwrap_or_else(|| "?".to_string());
    let semantic_summary = crate::action_semantics::build_semantic_summary(
        &action_type,
        &comp_name,
        &node.name,
        &reads,
        &writes,
        &navigates_to,
    );
    let blocks_on = crate::action_semantics::parse_wait_prev(meta_wait_prev);
    let condition_struct =
        crate::action_semantics::parse_condition(meta_condition.or(meta_condition_exp));

    let wait_status = if meta_wait_prev.is_some() {
        "blocking"
    } else {
        "none"
    };
    let may_interrupt = meta_condition.is_some() || meta_condition_exp.is_some();
    let failure_behavior = if meta_condition.is_some() || meta_condition_exp.is_some() {
        "skip_if_condition_fails"
    } else {
        "proceed"
    };

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "action",
        "type_detail": action_type,
        "action_category": action_category,
        "semantic_summary": semantic_summary,
        "importance": importance,
        "parent_component": parent_comp.as_ref().map(|c| c.name.clone()),
        "parent_component_id": parent_comp.as_ref().map(|c| c.id.clone()),
        "page": parent_page.as_ref().map(|p| p.name.clone()),
        "page_id": parent_page.as_ref().map(|p| p.id.clone()),
        "read_count": reads.len(),
        "write_count": writes.len(),
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": reads,
        "writes": writes,
        "triggered_by": triggered_by,
        "navigates_to": navigates_to,
        "affects_components": affects_components,
        "affects": {
            "components": affects_components,
            "pages": navigates_to,
        },
        "lineage": lineage,
        "action_category": action_category,
        "semantic_summary": semantic_summary,
        "blocks_on": blocks_on,
        "wait_status": wait_status,
        "condition": condition_struct,
        "may_interrupt": may_interrupt,
        "failure_behavior": failure_behavior,
        "trigger_type": meta_trigger_type.unwrap_or("click"),
    });

    let diagnostics: Vec<crate::output::Diagnostic> = Vec::new();

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Action {} is defined in {}", node.id, node.path),
            "Graph Action node with Triggers edge from parent component",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if !reads.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Action {} reads from {} targets", node.id, reads.len()),
                "Outgoing Reads/ActionReads edges from action",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !writes.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Action {} writes to {} targets", node.id, writes.len()),
                "Outgoing Writes/ActionWrites edges from action",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if let Some(cond) = meta_condition.or(meta_condition_exp) {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Action {} has execution condition", node.id),
                "Condition is defined in action metadata",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_source_file(&node.path)
            .with_node_id(&node.id)
            .with_edge_type("ActionCondition")
            .with_raw_expr(cond)
            .with_json_path("actions[].condition / actions[].conditionExp"),
        );
    }
    push_relation_evidence(&mut output, "reads", &reads);
    push_relation_evidence(&mut output, "writes", &writes);
    push_relation_evidence(&mut output, "triggered_by", &triggered_by);
    push_relation_evidence(&mut output, "navigates_to", &navigates_to);
    push_relation_evidence(&mut output, "affects_components", &affects_components);
    output.next_queries = vec![
        format_next_query("--context {} --depth 2 for surrounding closure", &node.id),
        format_next_query(
            "--explain {} for parent component",
            parent_comp
                .as_ref()
                .map(|c| c.id.clone())
                .unwrap_or_default()
                .as_str(),
        ),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Category: {}", action_category)?;
        writeln!(out, "Summary: {}", semantic_summary)?;
        writeln!(out, "Path: {}", node.path)?;
        if meta_trigger_type.is_some() {
            writeln!(out, "Trigger: {}", meta_trigger_type.unwrap_or("click"))?;
        }
        if let Some(raw) = meta_wait_prev {
            writeln!(out, "Waits for: {}", raw)?;
        }
        if meta_condition.is_some() || meta_condition_exp.is_some() {
            writeln!(
                out,
                "Condition: {}",
                meta_condition.or(meta_condition_exp).unwrap_or("")
            )?;
        }
        if !reads.is_empty() {
            writeln!(
                out,
                "
--- Reads ({}) ---",
                reads.len()
            )?;
            for r in &reads {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !writes.is_empty() {
            writeln!(
                out,
                "
--- Writes ({}) ---",
                writes.len()
            )?;
            for w in &writes {
                writeln!(out, "  {:?}", w)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}

fn explain_model_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    _outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    _incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    // Use existing graph query helpers for richer semantics
    let readers = graph.find_readers(&node.id);
    let writers = graph.find_writers(&node.id);
    let dataflow_inputs = graph.find_dataflow_inputs(&node.id);
    let dataflow_outputs = graph.find_dataflow_outputs(&node.id);
    let produced_by = graph.find_produced_by(&node.id);
    let consumed_by_dataflows = graph.find_consumed_by_dataflows(&node.id);

    let read_count = readers.len();
    let write_count = writers.len();
    let has_read = read_count > 0;
    let has_write = write_count > 0;
    let has_nav = false;

    let reader_refs: Vec<serde_json::Value> = readers
        .iter()
        .map(|(n, e)| {
            let page = find_parent_page(graph, &n.id);
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "type": format!("{:?}", n.node_type),
                "edge_type": "Reads",
                "field_path": e.field_path,
                "source_file": n.path,
                "page": page.as_ref().map(|p| p.name.clone()),
                "page_id": page.as_ref().map(|p| p.id.clone()),
            })
        })
        .collect();

    let writer_refs: Vec<serde_json::Value> = writers
        .iter()
        .map(|(n, e)| {
            let page = find_parent_page(graph, &n.id);
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "type": format!("{:?}", n.node_type),
                "edge_type": format!("{:?}", e.edge_type),
                "field_path": e.field_path,
                "source_file": n.path,
                "page": page.as_ref().map(|p| p.name.clone()),
                "page_id": page.as_ref().map(|p| p.id.clone()),
            })
        })
        .collect();

    let what = format!(
        "数据模型 {}，被 {} 个组件/动作读取，被 {} 个组件/动作写入，参与 {} 个 DataFlow",
        node.name,
        read_count,
        write_count,
        dataflow_inputs.len() + dataflow_outputs.len()
    );
    let importance = classify_importance(has_nav, has_write, has_read, false, "", &node.node_type);

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "model",
        "type_detail": node.name,
        "importance": importance,
        "read_by_count": read_count,
        "written_by_count": write_count,
        "dataflow_input_count": dataflow_inputs.len(),
        "dataflow_output_count": dataflow_outputs.len(),
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": reader_refs,
        "writes": writer_refs,
        "triggered_by": writer_refs.clone(),
        "affects": reader_refs.clone(),
        "lineage": lineage,
        "dataflow_inputs": dataflow_inputs.iter().map(|(n, e)| serde_json::json!({
            "id": n.id, "name": n.name, "field_path": e.field_path,
        })).collect::<Vec<_>>(),
        "dataflow_outputs": dataflow_outputs.iter().map(|(n, e)| serde_json::json!({
            "id": n.id, "name": n.name, "field_path": e.field_path,
        })).collect::<Vec<_>>(),
        "produced_by": produced_by.iter().map(|(n, e)| serde_json::json!({
            "id": n.id, "name": n.name, "field_path": e.field_path,
        })).collect::<Vec<_>>(),
        "consumed_by_dataflows": consumed_by_dataflows.iter().map(|(n, e)| serde_json::json!({
            "id": n.id, "name": n.name, "field_path": e.field_path,
        })).collect::<Vec<_>>(),
    });

    let diagnostics: Vec<crate::output::Diagnostic> = Vec::new();

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Model {} is defined in {}", node.id, node.path),
            "Graph Model node with dimension fields",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if read_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Model {} is read by {} components/actions",
                    node.id, read_count
                ),
                "Graph traversal: find_readers",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if write_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Model {} is written by {} components/actions",
                    node.id, write_count
                ),
                "Graph traversal: find_writers",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !dataflow_inputs.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Model {} is input to {} dataflows",
                    node.id,
                    dataflow_inputs.len()
                ),
                "Graph traversal: find_dataflow_inputs",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !dataflow_outputs.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Model {} is output from {} dataflows",
                    node.id,
                    dataflow_outputs.len()
                ),
                "Graph traversal: find_dataflow_outputs",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    push_relation_evidence(&mut output, "reads", &reader_refs);
    push_relation_evidence(&mut output, "writes", &writer_refs);
    output.next_queries = vec![
        format_next_query("--query-model {} for full model dependencies", &node.name),
        format_next_query("--context {} --depth 2 for surrounding closure", &node.id),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !reader_refs.is_empty() {
            writeln!(
                out,
                "
--- Read By ({}) ---",
                reader_refs.len()
            )?;
            for r in &reader_refs {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !writer_refs.is_empty() {
            writeln!(
                out,
                "
--- Written By ({}) ---",
                writer_refs.len()
            )?;
            for w in &writer_refs {
                writeln!(out, "  {:?}", w)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}

/// 解释参数节点：展示哪些组件/条件依赖此参数，以及哪些动作设置了此参数
fn explain_param_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    _outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    let mut dependents = Vec::new();
    let mut setters = Vec::new();

    for (source, edge) in &incoming {
        match edge.edge_type {
            crate::graph::EdgeType::DependsOn => {
                let page = find_parent_page(graph, &source.id);
                dependents.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                }));
            }
            crate::graph::EdgeType::ActionSetsParam => {
                let page = find_parent_page(graph, &source.id);
                setters.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                }));
            }
            _ => {}
        }
    }

    let dep_count = dependents.len();
    let set_count = setters.len();
    let what = format!(
        "页面参数 {}，被 {} 个组件/条件依赖，被 {} 个动作设置",
        node.name, dep_count, set_count
    );

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "param",
        "type_detail": node.name,
        "dependent_count": dep_count,
        "setter_count": set_count,
    });

    let details = serde_json::json!({
        "dependents": dependents.clone(),
        "setters": setters.clone(),
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new(
            format!(
                "Param {} has {} dependents and {} setters",
                node.id, dep_count, set_count
            ),
            "Graph param node with DependsOn and ActionSetsParam edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !dependents.is_empty() {
            writeln!(
                out,
                "
--- Dependents ({}) ---",
                dependents.len()
            )?;
            for d in &dependents {
                writeln!(out, "  {:?}", d)?;
            }
        }
        if !setters.is_empty() {
            writeln!(
                out,
                "
--- Setters ({}) ---",
                setters.len()
            )?;
            for s in &setters {
                writeln!(out, "  {:?}", s)?;
            }
        }
    }
    Ok(serde_json::to_value(output)?)
}

fn explain_field_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    _outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    // 参数节点走独立分支
    if node.id.starts_with("param:") {
        return explain_param_graph(graph, node, _outgoing, incoming, human);
    }

    let parent_model = incoming.iter().find_map(|(source, edge)| {
        if matches!(edge.edge_type, crate::graph::EdgeType::Contains)
            && matches!(source.node_type, crate::graph::NodeType::Model)
        {
            Some((*source).clone())
        } else {
            None
        }
    });

    let mut readers = Vec::new();
    let mut writers = Vec::new();
    let mut produced_by = Vec::new();
    let mut determined_by = Vec::new();

    for (source, edge) in &incoming {
        match edge.edge_type {
            crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                let page = find_parent_page(graph, &source.id);
                readers.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                }));
            }
            crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                let page = find_parent_page(graph, &source.id);
                let source_expr = edge
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("source_expr"))
                    .and_then(|v| v.as_str());

                writers.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "source_expr": source_expr,
                }));
            }
            crate::graph::EdgeType::DataflowInput => {
                produced_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": "DataflowInput",
                    "source_file": source.path,
                }));
            }
            crate::graph::EdgeType::DependsOn => {
                let page = find_parent_page(graph, &source.id);
                determined_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                }));
            }
            _ => {}
        }
    }

    let read_count = readers.len();
    let det_count = determined_by.len();
    if let Some(ref model) = parent_model
        && let Some((_model_out, model_in)) = graph.get_node_edges(&model.id)
    {
        let field_name = node.name.as_str();
        for (source, edge) in &model_in {
            if matches!(
                edge.edge_type,
                crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads
            ) && edge
                .field_path
                .as_ref()
                .map(|fp| {
                    let parts: Vec<&str> = fp.split('.').collect();
                    parts.last() == Some(&field_name)
                })
                .unwrap_or(false)
            {
                let page = find_parent_page(graph, &source.id);
                readers.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                }));
            }
            if matches!(
                edge.edge_type,
                crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites
            ) && edge
                .field_path
                .as_ref()
                .map(|fp| {
                    let parts: Vec<&str> = fp.split('.').collect();
                    parts.last() == Some(&field_name)
                })
                .unwrap_or(false)
            {
                let page = find_parent_page(graph, &source.id);
                let source_expr = edge
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("source_expr"))
                    .and_then(|v| v.as_str());

                writers.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                    "source_file": source.path,
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "source_expr": source_expr,
                }));
            }
        }
    }

    let write_count = writers.len();
    let has_read = read_count > 0;
    let has_write = write_count > 0;
    let has_nav = false;

    let model_name = parent_model
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("?");

    let mut lineage: Vec<serde_json::Value> = Vec::new();
    let mut diagnostics: Vec<crate::output::Diagnostic> = Vec::new();

    let field_meta = node.meta.as_ref();
    let source_input_field = field_meta
        .and_then(|m| m.get("source_input_field"))
        .and_then(|v| v.as_str());
    let source_expr = field_meta
        .and_then(|m| m.get("source_expr"))
        .and_then(|v| v.as_str());
    let source_expr_models = field_meta
        .and_then(|m| m.get("source_expr_models"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();

    if let Some(input_field) = source_input_field {
        let via_model = parent_model
            .as_ref()
            .map(|m| m.id.clone())
            .unwrap_or_default();
        lineage.push(serde_json::json!({
            "target_field": node.id,
            "source_fields": [format!("field:{}.{}", model_name, input_field)],
            "source_expr": null,
            "transform": "inputField mapping",
            "via_node": via_model,
            "confidence": "high",
            "evidence": {
                "source_file": node.path,
                "node_id": node.id,
                "edge_type": "Contains",
                "raw_expr": null,
                "json_path": "dimensions[].inputField",
            }
        }));
    }

    if let Some(expr) = source_expr {
        let mut source_fields: Vec<String> = Vec::new();
        for model_ref in &source_expr_models {
            source_fields.push(format!("model:{}", model_ref));
        }
        if source_fields.is_empty() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "LINEAGE_EXPR_UNPARSED".to_string(),
                message: format!(
                    "Expression '{}' could not be resolved to specific source fields",
                    expr
                ),
                location: crate::output::Location::new(),
                suggestion: Some("Check if expression parser supports this syntax".to_string()),
            });
        }
        lineage.push(serde_json::json!({
            "target_field": node.id,
            "source_fields": source_fields,
            "source_expr": expr,
            "transform": "expression calculation",
            "via_node": parent_model.as_ref().map(|m| m.id.clone()).unwrap_or_default(),
            "confidence": if source_fields.is_empty() { "low" } else { "medium" },
            "evidence": {
                "source_file": node.path,
                "node_id": node.id,
                "edge_type": "Contains",
                "raw_expr": expr,
                "json_path": "dimensions[].exp",
            }
        }));
    }

    if !produced_by.is_empty() {
        for producer in &produced_by {
            let producer_id = producer.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let producer_node = graph.get_node(producer_id);
            // Try to resolve field-level mapping from producer node meta (dimensions)
            let producer_dims: Vec<serde_json::Value> = producer_node
                .as_ref()
                .and_then(|n| n.meta.as_ref())
                .and_then(|m| m.get("dimensions"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let field_name = node.name.as_str();
            let mut mapped_source_field: Option<String> = None;
            let mut mapped_transform = "DataFlow input";
            for dim in &producer_dims {
                let dim_name = dim.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if dim_name == field_name {
                    if let Some(input) = dim.get("inputField").and_then(|v| v.as_str()) {
                        mapped_source_field = Some(input.to_string());
                        mapped_transform = "DataFlow inputField mapping";
                    } else if let Some(exp) = dim.get("exp").and_then(|v| v.as_str()) {
                        mapped_source_field = Some(exp.to_string());
                        mapped_transform = "DataFlow expression";
                    }
                    break;
                }
            }
            if let Some((producer_out, _producer_in)) = graph.get_node_edges(producer_id) {
                for (upstream_model, edge) in &producer_out {
                    if matches!(edge.edge_type, crate::graph::EdgeType::DataflowInput) {
                        let source_field = mapped_source_field
                            .as_ref()
                            .map(|sf| format!("field:{}.{}", upstream_model.name, sf));
                        let source_fields: Vec<String> = source_field.into_iter().collect();
                        lineage.push(serde_json::json!({
                            "target_field": node.id,
                            "source_fields": source_fields,
                            "source_expr": mapped_source_field.as_ref(),
                            "transform": mapped_transform,
                            "via_node": producer_id.to_string(),
                            "confidence": if mapped_source_field.is_some() { "high" } else { "medium" },
                            "evidence": {
                                "source_file": upstream_model.path.clone(),
                                "node_id": upstream_model.id.clone(),
                                "edge_type": "DataflowInput",
                                "raw_expr": mapped_source_field.as_ref(),
                                "json_path": "dataFlow.nodes[].fields[] / dimensions[]",
                            }
                        }));
                    }
                }
            }
        }
    }

    for writer in &writers {
        let writer_id = writer.get("id").and_then(|v| v.as_str()).unwrap_or("?");
        let field_path = writer
            .get("field_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let source_expr = writer.get("source_expr").and_then(|v| v.as_str());
        let mut source_fields: Vec<String> = Vec::new();
        let mut resolved_refs: Vec<serde_json::Value> = Vec::new();
        let mut has_unresolved = false;
        if let Some(expr) = source_expr {
            let refs = crate::superpage::parse_expression_refs(expr);
            if refs.is_empty() {
                has_unresolved = true;
            }
            for r in refs {
                let (ref_type_str, ref_id, confidence) = match &r {
                    crate::superpage::RefType::ModelField(m, f) => {
                        let fid = format!("field:{}.{}", m, f);
                        source_fields.push(fid.clone());
                        ("ModelField", fid, "high")
                    }
                    crate::superpage::RefType::ComponentValue(c) => {
                        ("ComponentValue", format!("comp:{}", c), "high")
                    }
                    crate::superpage::RefType::ComponentProperty(c, p) => {
                        ("ComponentProperty", format!("comp:{}.{}", c, p), "high")
                    }
                    crate::superpage::RefType::Param(p) => {
                        ("Param", format!("param:{}", p), "high")
                    }
                    crate::superpage::RefType::UserProperty(p) => {
                        ("UserProperty", format!("user:{}", p), "medium")
                    }
                    crate::superpage::RefType::SystemVar(v) => ("SystemVar", v.clone(), "high"),
                    crate::superpage::RefType::Other(o) => {
                        has_unresolved = true;
                        ("Other", o.clone(), "low")
                    }
                };
                resolved_refs.push(serde_json::json!({
                    "ref_type": ref_type_str,
                    "ref_id": ref_id,
                    "confidence": confidence,
                }));
            }
        }
        let confidence = if source_expr.is_some() && has_unresolved {
            "medium"
        } else if source_expr.is_some() && resolved_refs.is_empty() {
            "low"
        } else {
            "high"
        };
        lineage.push(serde_json::json!({
            "target_field": node.id,
            "source_fields": source_fields,
            "source_expr": source_expr,
            "resolved_refs": resolved_refs,
            "transform": "page action write",
            "via_node": writer_id.to_string(),
            "confidence": confidence,
            "evidence": {
                "source_file": writer.get("source_file").and_then(|v| v.as_str()).unwrap_or(""),
                "node_id": writer_id,
                "edge_type": writer.get("edge_type").and_then(|v| v.as_str()).unwrap_or(""),
                "raw_expr": field_path,
                "json_path": "actions[].fieldValues[]",
            }
        }));
    }

    // Chain DataFlow lineage: if no direct source, try to trace through DataFlow inputs
    if lineage.is_empty()
        && let Some(ref model) = parent_model
    {
        // Check if parent is a DataFlow with input models
        let dataflow_inputs = graph.find_dataflow_inputs(&model.id);
        let field_name = node.name.as_str();
        for (input_model, _edge) in dataflow_inputs {
            let input_dims: Vec<serde_json::Value> = input_model
                .meta
                .as_ref()
                .and_then(|m| m.get("dimensions"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for dim in &input_dims {
                let dim_name = dim.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let dbfield = dim.get("dbfield").and_then(|v| v.as_str()).unwrap_or("");
                if dim_name == field_name || dbfield == field_name {
                    let source_field = format!("field:{}.{}", input_model.name, dim_name);
                    lineage.push(serde_json::json!({
                        "target_field": node.id,
                        "source_fields": [source_field],
                        "source_expr": null,
                        "transform": "DataFlow chain (input model field match)",
                        "via_node": model.id.clone(),
                        "confidence": "medium",
                        "evidence": {
                            "source_file": input_model.path.clone(),
                            "node_id": input_model.id.clone(),
                            "edge_type": "DataflowInput",
                            "raw_expr": null,
                            "json_path": format!("dimensions[].name='{}'", dim_name),
                        }
                    }));
                    break;
                }
            }
        }
        // Check if parent is produced by a DataFlow (physical table case)
        let producers = graph.find_produced_by(&model.id);
        for (producer, _edge) in producers {
            let producer_dims: Vec<serde_json::Value> = producer
                .meta
                .as_ref()
                .and_then(|m| m.get("dimensions"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for dim in &producer_dims {
                let dim_name = dim.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let dbfield = dim.get("dbfield").and_then(|v| v.as_str()).unwrap_or("");
                if dim_name == field_name || dbfield == field_name {
                    let source_field = format!("field:{}.{}", producer.name, dim_name);
                    lineage.push(serde_json::json!({
                        "target_field": node.id,
                        "source_fields": [source_field],
                        "source_expr": null,
                        "transform": "DataFlow chain (producer field match)",
                        "via_node": producer.id.clone(),
                        "confidence": "medium",
                        "evidence": {
                            "source_file": producer.path.clone(),
                            "node_id": producer.id.clone(),
                            "edge_type": "OutputsTo",
                            "raw_expr": null,
                            "json_path": format!("dimensions[].name='{}'", dim_name),
                        }
                    }));
                    break;
                }
            }
        }
    }

    if lineage.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "LINEAGE_SOURCE_MISSING".to_string(),
            message: format!("Field {} has no traceable source lineage", node.id),
            location: crate::output::Location::new(),
            suggestion: Some(
                "Check dimensions[].inputField or dimensions[].exp metadata".to_string(),
            ),
        });
    }

    let what = if det_count > 0 {
        format!(
            "模型 {} 的字段 {}，被 {} 个组件/动作读取，被 {} 个条件决定，lineage {} 条",
            model_name,
            node.name,
            read_count,
            det_count,
            lineage.len()
        )
    } else {
        format!(
            "模型 {} 的字段 {}，被 {} 个组件/动作读取，被 {} 个组件/动作写入，lineage {} 条",
            model_name,
            node.name,
            read_count,
            write_count,
            lineage.len()
        )
    };
    let importance = classify_importance(has_nav, has_write, has_read, false, "", &node.node_type);

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "field",
        "type_detail": node.name,
        "importance": importance,
        "parent_model": model_name,
        "parent_model_id": parent_model.as_ref().map(|m| m.id.clone()),
        "read_by_count": read_count,
        "written_by_count": write_count,
        "produced_by_dataflow_count": produced_by.len(),
        "lineage_count": lineage.len(),
        "determined_by_count": det_count,
    });

    let details = serde_json::json!({
        "reads": readers.clone(),
        "writes": writers.clone(),
        "triggered_by": writers.clone(),
        "affects": readers.clone(),
        "lineage": lineage.clone(),
        "produced_by": produced_by.clone(),
        "determined_by": determined_by.clone(),
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Field {} belongs to model {}", node.id, model_name),
            "Graph Field node with Contains edge from parent Model",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if read_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Field {} is read by {} components/actions",
                    node.id, read_count
                ),
                "Incoming Reads/ActionReads edges to field",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if write_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Field {} is written by {} components/actions",
                    node.id, write_count
                ),
                "Incoming Writes/ActionWrites edges to field",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !produced_by.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Field {} is produced by {} dataflows",
                    node.id,
                    produced_by.len()
                ),
                "Incoming DataflowInput edges to field",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    push_relation_evidence(&mut output, "reads", &readers);
    push_relation_evidence(&mut output, "writes", &writers);
    push_relation_evidence(&mut output, "produced_by", &produced_by);
    push_lineage_evidence(&mut output, &lineage);
    output.next_queries = vec![
        format_next_query(
            "--explain {} for parent model summary",
            parent_model
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_default()
                .as_str(),
        ),
        format_next_query("--context {} --depth 2 for surrounding closure", &node.id),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !readers.is_empty() {
            writeln!(
                out,
                "
--- Read By ({}) ---",
                readers.len()
            )?;
            for r in &readers {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !writers.is_empty() {
            writeln!(
                out,
                "
--- Written By ({}) ---",
                writers.len()
            )?;
            for w in &writers {
                writeln!(out, "  {:?}", w)?;
            }
        }
        if !output.diagnostics.is_empty() {
            writeln!(
                out,
                "
⚠️  Diagnostics:"
            )?;
            for diag in &output.diagnostics {
                writeln!(out, "  [{}] {}", diag.code, diag.message)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}

fn explain_page_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    let mut entrypoints = Vec::new();
    let mut data_sources = Vec::new();
    let mut write_targets = Vec::new();
    let mut navigation = Vec::new();
    let mut child_count = 0usize;
    let mut has_nav = false;
    let mut has_write = false;
    let mut has_read = false;

    for (target, edge) in &outgoing {
        if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
            child_count += 1;
            // Check if this child has actions (entrypoint)
            if let Some((child_out, _)) = graph.get_node_edges(&target.id) {
                let mut has_action = false;
                let mut child_reads = false;
                let mut child_writes = false;
                for (_, e) in &child_out {
                    match e.edge_type {
                        crate::graph::EdgeType::Triggers => {
                            has_action = true;
                        }
                        crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                            child_writes = true;
                        }
                        crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::SetsParam => {
                            has_action = true;
                        }
                        crate::graph::EdgeType::Reads => {
                            child_reads = true;
                        }
                        _ => {}
                    }
                }
                if has_action {
                    entrypoints.push(serde_json::json!({
                        "id": target.id,
                        "name": target.name,
                        "type": format!("{:?}", target.node_type),
                        "has_reads": child_reads,
                        "has_writes": child_writes,
                    }));
                }
            }
        }
    }

    // For data_sources, write_targets, navigation: look at child component edges
    // and follow Triggers edges to their actions
    for (target, edge) in &outgoing {
        if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
            // Check component's own edges
            if let Some((child_out, _)) = graph.get_node_edges(&target.id) {
                for (t, e) in &child_out {
                    if matches!(e.edge_type, crate::graph::EdgeType::OpensPage) {
                        navigation.push(serde_json::json!({
                            "from": target.id,
                            "from_name": target.name,
                            "to": t.id,
                            "to_name": t.name,
                            "type": "OpensPage",
                        }));
                        has_nav = true;
                    }
                }
            }
            // Check action edges (via Triggers)
            if let Some((child_out, _)) = graph.get_node_edges(&target.id) {
                for (action_node, action_edge) in &child_out {
                    if matches!(action_edge.edge_type, crate::graph::EdgeType::Triggers)
                        && let Some((action_out, _)) = graph.get_node_edges(&action_node.id)
                    {
                        for (t, e) in &action_out {
                            match e.edge_type {
                                crate::graph::EdgeType::Reads
                                | crate::graph::EdgeType::ActionReads => {
                                    data_sources.push(serde_json::json!({
                                        "component_id": target.id,
                                        "component_name": target.name,
                                        "action": action_node.name,
                                        "action_id": action_node.id,
                                        "model": t.name,
                                        "model_id": t.id,
                                        "field_path": e.field_path,
                                    }));
                                    has_read = true;
                                }
                                crate::graph::EdgeType::Writes
                                | crate::graph::EdgeType::ActionWrites => {
                                    write_targets.push(serde_json::json!({
                                        "component_id": target.id,
                                        "component_name": target.name,
                                        "action": action_node.name,
                                        "action_id": action_node.id,
                                        "model": t.name,
                                        "model_id": t.id,
                                        "field_path": e.field_path,
                                    }));
                                    has_write = true;
                                }
                                crate::graph::EdgeType::OpensPage
                                | crate::graph::EdgeType::ActionNavigates => {
                                    navigation.push(serde_json::json!({
                                        "from": target.id,
                                        "from_name": target.name,
                                        "action": action_node.name,
                                        "action_id": action_node.id,
                                        "to": t.id,
                                        "to_name": t.name,
                                        "type": format!("{:?}", e.edge_type),
                                    }));
                                    has_nav = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
    }

    // Pages that open this page
    // Pages that open this page
    let mut opened_by = Vec::new();
    for (source, edge) in &incoming {
        if matches!(edge.edge_type, crate::graph::EdgeType::OpensPage) {
            opened_by.push(serde_json::json!({
                "id": source.id,
                "name": source.name,
                "type": format!("{:?}", source.node_type),
                "edge_type": "OpensPage",
                "source_file": source.path,
            }));
        }
    }

    let entry_count = entrypoints.len();
    let what = format!(
        "页面 {}，包含 {} 个组件，{} 个入口点，{} 个数据源，{} 个写入目标",
        node.name,
        child_count,
        entry_count,
        data_sources.len(),
        write_targets.len()
    );
    let importance = classify_importance(has_nav, has_write, has_read, false, "", &node.node_type);

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "page",
        "type_detail": node.name,
        "importance": importance,
        "child_count": child_count,
        "entrypoint_count": entry_count,
        "data_source_count": data_sources.len(),
        "write_target_count": write_targets.len(),
        "navigation_count": navigation.len(),
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": data_sources.clone(),
        "writes": write_targets.clone(),
        "triggered_by": opened_by.clone(),
        "affects": entrypoints.clone(),
        "lineage": lineage.clone(),
        "navigation": navigation.clone(),
        "entrypoints": entrypoints.clone(),
    });

    let mut diagnostics = Vec::new();
    if entry_count == 0 {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "NO_ENTRYPOINTS".to_string(),
            message: "Page has no detected user entrypoints".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some("Page may be read-only or actions not yet parsed".to_string()),
        });
    }
    diagnostics.push(crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_SOURCE_MISSING".to_string(),
        message: "Field-level lineage source could not be determined".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some(
            "Check --context or --query-dataflow for upstream relationships".to_string(),
        ),
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Page {} contains {} components", node.id, child_count),
            "Graph Page node with Contains edges to child components",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if entry_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} has {} entrypoints", node.id, entry_count),
                "Child components with outgoing Triggers edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !data_sources.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} reads from {} data sources",
                    node.id,
                    data_sources.len()
                ),
                "Child components with outgoing Reads edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if !write_targets.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} writes to {} targets", node.id, write_targets.len()),
                "Child components with outgoing Writes/ActionWrites edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    push_relation_evidence(&mut output, "reads", &data_sources);
    push_relation_evidence(&mut output, "writes", &write_targets);
    push_relation_evidence(&mut output, "triggered_by", &opened_by);
    push_relation_evidence(&mut output, "affects", &entrypoints);
    push_relation_evidence(&mut output, "navigation", &navigation);
    output.next_queries = vec![
        format_next_query("--query-page-logic {} for detailed page logic", &node.id),
        format_next_query("--query-page {} for page dependencies", &node.id),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !data_sources.is_empty() {
            writeln!(
                out,
                "
--- Data Sources ({}) ---",
                data_sources.len()
            )?;
            for r in &data_sources {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !write_targets.is_empty() {
            writeln!(
                out,
                "
--- Write Targets ({}) ---",
                write_targets.len()
            )?;
            for w in &write_targets {
                writeln!(out, "  {:?}", w)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}

fn explain_dataflow_graph(
    _graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();

    for (target, edge) in &outgoing {
        match edge.edge_type {
            crate::graph::EdgeType::OutputsTo => {
                outputs.push(serde_json::json!({
                    "id": target.id,
                    "name": target.name,
                    "type": format!("{:?}", target.node_type),
                    "field_path": edge.field_path,
                    "source_file": target.path,
                }));
            }
            crate::graph::EdgeType::DataflowInput => {
                inputs.push(serde_json::json!({
                    "id": target.id,
                    "name": target.name,
                    "type": format!("{:?}", target.node_type),
                    "edge_type": "DataflowInput",
                    "source_file": target.path,
                }));
            }
            _ => {}
        }
    }

    for (source, edge) in &incoming {
        if edge.edge_type == crate::graph::EdgeType::DataflowInput {
            inputs.push(serde_json::json!({
                "id": source.id,
                "name": source.name,
                "type": format!("{:?}", source.node_type),
                "field_path": edge.field_path,
                "source_file": source.path,
            }));
        }
    }
    let input_count = inputs.len();
    let output_count = outputs.len();

    let _has_read = input_count > 0;
    let _has_write = output_count > 0;

    // M6: Parse DataFlow internal metadata
    let raw_meta = node.meta.as_ref();
    let internal_deps: std::collections::HashMap<String, Vec<String>> = raw_meta
        .and_then(|m| m.get("internalDeps"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let alias_map: std::collections::HashMap<String, String> = raw_meta
        .and_then(|m| m.get("aliasMap"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let node_types: std::collections::HashMap<String, String> = raw_meta
        .and_then(|m| m.get("nodeTypes"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let dimensions: Vec<serde_json::Value> = raw_meta
        .and_then(|m| m.get("dimensions"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let internal_node_count = alias_map.len();

    // Build internal topology
    let mut internal_nodes = Vec::new();
    let mut internal_edges = Vec::new();
    for (alias, node_id) in &alias_map {
        let ntype = node_types
            .get(node_id)
            .map(|s| s.as_str())
            .unwrap_or("Unknown");
        internal_nodes.push(serde_json::json!({
            "id": node_id,
            "alias": alias,
            "type": ntype,
        }));
        if let Some(deps) = internal_deps.get(node_id) {
            for dep in deps {
                internal_edges.push(serde_json::json!({
                    "from": dep,
                    "to": node_id,
                }));
            }
        }
    }

    // Build field-level lineage from dimensions
    let mut lineage: Vec<serde_json::Value> = Vec::new();
    let mut diagnostics: Vec<crate::output::Diagnostic> = Vec::new();
    for dim in &dimensions {
        let dim_name = dim.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let _dbfield = dim.get("dbfield").and_then(|v| v.as_str());
        let input_field = dim.get("inputField").and_then(|v| v.as_str());
        let exp = dim.get("exp").and_then(|v| v.as_str());

        if let Some(input) = input_field {
            lineage.push(serde_json::json!({
                "target_field": format!("field:{}.{}", node.name, dim_name),
                "source_fields": [format!("field:{}.{}", node.name, input)],
                "source_expr": null,
                "transform": "inputField mapping",
                "via_node": node.id.clone(),
                "confidence": "high",
                "evidence": {
                    "source_file": node.path.clone(),
                    "node_id": format!("field:{}.{}", node.name, dim_name),
                    "edge_type": "Contains",
                    "raw_expr": null,
                    "json_path": format!("dimensions[].inputField for {}", dim_name),
                }
            }));
        } else if let Some(expr) = exp {
            let refs = crate::superpage::parse_expression_refs(expr);
            let source_fields: Vec<String> = refs
                .iter()
                .filter_map(|r| match r {
                    crate::superpage::RefType::ModelField(m, f) => Some(format!("{}.{}", m, f)),
                    _ => None,
                })
                .collect();
            if source_fields.is_empty() {
                diagnostics.push(crate::output::Diagnostic {
                    severity: crate::output::DiagnosticSeverity::Info,
                    code: "LINEAGE_EXPR_UNPARSED".to_string(),
                    message: format!(
                        "Dimension '{}' expression could not be resolved: {}",
                        dim_name, expr
                    ),
                    location: crate::output::Location::new(),
                    suggestion: Some("Expression parser may not support this syntax".to_string()),
                });
            }
            lineage.push(serde_json::json!({
                "target_field": format!("field:{}.{}", node.name, dim_name),
                "source_fields": source_fields.iter().map(|s| format!("field:{}", s)).collect::<Vec<_>>(),
                "source_expr": expr,
                "transform": "expression calculation",
                "via_node": node.id.clone(),
                "confidence": if source_fields.is_empty() { "low" } else { "medium" },
                "evidence": {
                    "source_file": node.path.clone(),
                    "node_id": format!("field:{}.{}", node.name, dim_name),
                    "edge_type": "Contains",
                    "raw_expr": expr,
                    "json_path": format!("dimensions[].exp for {}", dim_name),
                }
            }));
        } else {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "LINEAGE_SOURCE_MISSING".to_string(),
                message: format!("Dimension '{}' has no inputField or exp", dim_name),
                location: crate::output::Location::new(),
                suggestion: Some("Add inputField or exp to dimension metadata".to_string()),
            });
        }
    }

    let what = format!(
        "DataFlow {}，输入 {} 个源，输出 {} 个目标，内部 {} 个节点",
        node.name, input_count, output_count, internal_node_count
    );

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "dataflow",
        "type_detail": node.name,
        "importance": "data_source",
        "input_count": input_count,
        "output_count": output_count,
        "internal_node_count": internal_node_count,
    });

    let details = serde_json::json!({
        "reads": inputs.clone(),
        "writes": outputs.clone(),
        "triggered_by": inputs.clone(),
        "affects": outputs.clone(),
        "lineage": lineage.clone(),
        "inputs": inputs.clone(),
        "outputs": outputs.clone(),
        "internal_topology": {
            "nodes": internal_nodes,
            "edges": internal_edges,
        },
    });

    if internal_node_count == 0 {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "LINEAGE_SOURCE_MISSING".to_string(),
            message: "DataFlow internal topology not available".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some("Check if dataFlow.nodes exists in .tbl metadata".to_string()),
        });
    }

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("DataFlow {} is defined in {}", node.id, node.path),
            "Graph Model node with modelType=DataFlow",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path),
    );
    if input_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("DataFlow {} has {} inputs", node.id, input_count),
                "Incoming DataflowInput edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    if output_count > 0 {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("DataFlow {} has {} outputs", node.id, output_count),
                "Outgoing OutputsTo edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    push_relation_evidence(&mut output, "reads", &inputs);
    push_relation_evidence(&mut output, "writes", &outputs);
    push_lineage_evidence(&mut output, &lineage);
    output.next_queries = vec![
        format_next_query("--query-dataflow {} for full subgraph", &node.name),
        format_next_query("--context {} --depth 2 for surrounding closure", &node.id),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        if !inputs.is_empty() {
            writeln!(
                out,
                "
--- Inputs ({}) ---",
                inputs.len()
            )?;
            for r in &inputs {
                writeln!(out, "  {:?}", r)?;
            }
        }
        if !outputs.is_empty() {
            writeln!(
                out,
                "
--- Outputs ({}) ---",
                outputs.len()
            )?;
            for w in &outputs {
                writeln!(out, "  {:?}", w)?;
            }
        }
        out.flush()?;
    }
    Ok(serde_json::to_value(output)?)
}
