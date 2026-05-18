pub use crate::answer_contract::TraversalIntent;
use crate::answer_contract::{
    build_answer_contract, build_required_followups, build_thinking_frame, build_truncation_guard,
};
use crate::dependency::DependencyGraph;
use crate::explain::condition_facts::{
    annotate_condition_scope, build_answer_facts, build_context_summary,
    build_primary_reason_for_intent, build_traversal_policy, build_value_source_context,
    classify_condition, collect_conditions_for_node, collect_inherited_conditions_by_json_path,
    component_ancestor_chain, component_json_paths, condition_owned_by_node, dedupe_conditions,
    expand_total_row_count_gates, partition_paths_for_intent,
};
use crate::explain::evidence::push_relation_evidence;
use crate::explain::handlers::{
    explain_action_graph, explain_component_graph, explain_condition_graph, explain_dataflow_graph,
    explain_field_graph, explain_model_graph, explain_page_graph,
};
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

mod condition_facts;
mod evidence;
mod handlers;
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

    let answer_contract = build_answer_contract(
        effective_intent,
        &target_node,
        primary_path.len(),
        candidate_paths.len(),
        rejected_paths.len(),
        _budget,
        is_page_scoped_target,
        dataflow_model_id.as_deref(),
        dataflow_model_id.as_deref(),
    );
    let thinking_frame = build_thinking_frame(
        effective_intent,
        &target_node,
        primary_path.len(),
        candidate_paths.len(),
        &value_source_context,
    );
    let truncation_guard = build_truncation_guard(
        _budget,
        primary_path.len(),
        candidate_paths.len(),
        rejected_paths.len(),
        supporting_context.len(),
        related_context.len(),
    );
    let required_followups = build_required_followups(
        effective_intent,
        target_id,
        _budget,
        primary_path.len(),
        candidate_paths.len(),
        &value_source_context,
        is_page_scoped_target,
        dataflow_model_id.as_deref(),
        truncation_guard
            .get("safe_to_answer_full_relationships")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
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
        "answer_contract": answer_contract,
        "thinking_frame": thinking_frame,
        "truncation_guard": truncation_guard,
        "required_followups": required_followups,
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
