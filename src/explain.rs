use crate::dependency::DependencyGraph;
use crate::graph::GraphDB;
use crate::superpage::{RefType, SuperPageMetadata};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, Write};

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

    let diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];
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
    output.diagnostics = diagnostics;
    output.next_queries = vec![
        format!("--context {} --depth 2 for surrounding closure", target_id),
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

/// 判断模型节点是否为 DataFlow
fn is_dataflow_model(node: &crate::graph::Node) -> bool {
    node.meta
        .as_ref()
        .and_then(|m| m.get("modelType"))
        .and_then(|v| v.as_str())
        == Some("DataFlow")
}

/// 辅助：构建带页面信息的引用对象
fn make_ref(target: &crate::graph::Node, edge: &crate::graph::Edge) -> serde_json::Value {
    serde_json::json!({
        "id": target.id,
        "name": target.name,
        "type": format!("{:?}", target.node_type),
        "edge_type": format!("{:?}", edge.edge_type),
        "field_path": edge.field_path,
        "source_file": target.path,
    })
}

/// 辅助：按重要性分类
fn classify_importance(
    has_nav: bool,
    has_write: bool,
    has_read: bool,
    node_type: &crate::graph::NodeType,
) -> String {
    match node_type {
        crate::graph::NodeType::Page => {
            if has_nav || has_write {
                "entrypoint".to_string()
            } else {
                "container".to_string()
            }
        }
        crate::graph::NodeType::Component | crate::graph::NodeType::Action => {
            if has_nav {
                "navigation".to_string()
            } else if has_write {
                "write_target".to_string()
            } else if has_read {
                "data_source".to_string()
            } else {
                "calculated_display".to_string()
            }
        }
        crate::graph::NodeType::Model | crate::graph::NodeType::Field => {
            if has_write {
                "write_target".to_string()
            } else if has_read {
                "data_source".to_string()
            } else {
                "unknown".to_string()
            }
        }
    }
}

/// 解释项目图中的节点
///
/// 支持完整 ID，例如 --explain model:physical_x。
/// 按 NodeType 分发，生成语义化的 summary、details、evidence。
pub fn explain_node_graph(graph: &GraphDB, node_id: &str, human: bool) -> Result<()> {
    let node = graph
        .get_node(node_id)
        .ok_or_else(|| anyhow::anyhow!("Node '{}' not found in graph", node_id))?;

    let (outgoing, incoming) = graph
        .get_node_edges(node_id)
        .unwrap_or_else(|| (Vec::new(), Vec::new()));

    match node.node_type {
        crate::graph::NodeType::Component => {
            explain_component_graph(graph, &node, outgoing, incoming, human)?
        }
        crate::graph::NodeType::Action => {
            explain_action_graph(graph, &node, outgoing, incoming, human)?
        }
        crate::graph::NodeType::Model => {
            if is_dataflow_model(&node) {
                explain_dataflow_graph(graph, &node, outgoing, incoming, human)?
            } else {
                explain_model_graph(graph, &node, outgoing, incoming, human)?
            }
        }
        crate::graph::NodeType::Field => {
            explain_field_graph(graph, &node, outgoing, incoming, human)?
        }
        crate::graph::NodeType::Page => {
            explain_page_graph(graph, &node, outgoing, incoming, human)?
        }
    }
    Ok(())
}

fn explain_component_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
    let parent_page = find_parent_page(graph, &node.id);
    let mut reads = Vec::new();
    let mut writes = Vec::new();
    let mut triggered_by = Vec::new();
    let mut affects = Vec::new();
    let mut action_count = 0usize;
    let mut has_nav = false;
    let mut has_write = false;
    let mut has_read = false;

    for (target, edge) in &outgoing {
        match edge.edge_type {
            crate::graph::EdgeType::Reads => {
                reads.push(make_ref(target, edge));
                has_read = true;
            }
            crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                writes.push(make_ref(target, edge));
                has_write = true;
            }
            crate::graph::EdgeType::Triggers => {
                action_count += 1;
                affects.push(serde_json::json!({
                    "id": target.id,
                    "name": target.name,
                    "type": format!("{:?}", target.node_type),
                    "edge_type": "Triggers",
                    "source_file": target.path,
                }));
            }
            crate::graph::EdgeType::OpensPage => {
                has_nav = true;
                affects.push(make_ref(target, edge));
            }
            crate::graph::EdgeType::SetsParam => {
                affects.push(make_ref(target, edge));
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
            crate::graph::EdgeType::Contains => {
                if matches!(source.node_type, crate::graph::NodeType::Page) {
                    triggered_by.push(serde_json::json!({
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
    let action_info = if action_count > 0 {
        format!("，具有 {} 个动作", action_count)
    } else {
        String::new()
    };
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

    let what = format!(
        "组件 {}{}{}{}{}",
        node.name, page_info, action_info, read_info, write_info
    );

    let mut importance = classify_importance(has_nav, has_write, has_read, &node.node_type);
    if action_count > 0 && importance == "calculated_display" {
        importance = "entrypoint".to_string();
    }

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "component",
        "type_detail": node.name,
        "importance": importance,
        "page": parent_page.as_ref().map(|p| p.name.clone()),
        "page_id": parent_page.as_ref().map(|p| p.id.clone()),
        "action_count": action_count,
        "read_count": reads.len(),
        "write_count": writes.len(),
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": reads,
        "writes": writes,
        "triggered_by": triggered_by,
        "affects": affects,
        "lineage": lineage,
    });

    let diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];

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
    if !writes.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Component {} writes to {} targets", node.id, writes.len()),
                "Outgoing Writes/ActionWrites edges from component",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&node.id),
        );
    }
    output.next_queries = vec![
        format!("--context {} --depth 2 for surrounding closure", node.id),
        format!(
            "--query-page-logic {} for page-level logic",
            parent_page
                .as_ref()
                .map(|p| p.id.clone())
                .unwrap_or_default()
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
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn explain_action_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
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
    let mut affects = Vec::new();
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
                reads.push(make_ref(target, edge));
                has_read = true;
            }
            crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                writes.push(make_ref(target, edge));
                has_write = true;
            }
            crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::ActionNavigates => {
                has_nav = true;
                affects.push(make_ref(target, edge));
            }
            crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::ActionSetsParam => {
                affects.push(make_ref(target, edge));
                has_write = true;
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
    let importance = classify_importance(has_nav, has_write, has_read, &node.node_type);

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "action",
        "type_detail": action_type,
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
        "affects": affects,
        "lineage": lineage,
    });

    let diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];

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
    output.next_queries = vec![
        format!("--context {} --depth 2 for surrounding closure", node.id),
        format!(
            "--explain {} for parent component",
            parent_comp
                .as_ref()
                .map(|c| c.id.clone())
                .unwrap_or_default()
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
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn explain_model_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    _outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    _incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
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
    let importance = classify_importance(has_nav, has_write, has_read, &node.node_type);

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

    let diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];

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
    output.next_queries = vec![
        format!("--query-model {} for full model dependencies", node.id),
        format!("--context {} --depth 2 for surrounding closure", node.id),
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
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn explain_field_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    _outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
    // Find parent model
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
                writers.push(serde_json::json!({
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
            crate::graph::EdgeType::DataflowInput => {
                produced_by.push(serde_json::json!({
                    "id": source.id,
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": "DataflowInput",
                    "source_file": source.path,
                }));
            }
            _ => {}
        }
    }

    let read_count = readers.len();
    let write_count = writers.len();
    let has_read = read_count > 0;
    let has_write = write_count > 0;
    let has_nav = false;

    let model_name = parent_model
        .as_ref()
        .map(|m| m.name.as_str())
        .unwrap_or("?");
    let what = format!(
        "模型 {} 的字段 {}，被 {} 个组件/动作读取，被 {} 个组件/动作写入",
        model_name, node.name, read_count, write_count
    );
    let importance = classify_importance(has_nav, has_write, has_read, &node.node_type);

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
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": readers,
        "writes": writers,
        "triggered_by": writers.clone(),
        "affects": readers.clone(),
        "lineage": lineage,
        "produced_by": produced_by,
    });

    let diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];

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
    output.next_queries = vec![
        format!(
            "--explain {} for parent model summary",
            parent_model
                .as_ref()
                .map(|m| m.id.clone())
                .unwrap_or_default()
        ),
        format!("--context {} --depth 2 for surrounding closure", node.id),
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
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn explain_page_graph(
    graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
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
                            has_action = true;
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
    let importance = classify_importance(has_nav, has_write, has_read, &node.node_type);

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
        "reads": data_sources,
        "writes": write_targets,
        "triggered_by": opened_by,
        "affects": entrypoints,
        "lineage": lineage,
        "navigation": navigation,
        "entrypoints": entrypoints,
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
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
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
    output.next_queries = vec![
        format!("--query-page-logic {} for detailed page logic", node.id),
        format!("--query-page {} for page dependencies", node.id),
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
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

fn explain_dataflow_graph(
    _graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<()> {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut internal_nodes = Vec::new();

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
                internal_nodes.push(serde_json::json!({
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
    let internal_count = internal_nodes.len();
    let _has_read = input_count > 0;
    let _has_write = output_count > 0;

    let what = format!(
        "DataFlow {}，输入 {} 个源，输出 {} 个目标，内部 {} 个节点",
        node.name, input_count, output_count, internal_count
    );

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "dataflow",
        "type_detail": node.name,
        "importance": "data_source",
        "input_count": input_count,
        "output_count": output_count,
        "internal_node_count": internal_count,
    });

    let lineage: Vec<serde_json::Value> = Vec::new();
    let details = serde_json::json!({
        "reads": inputs.clone(),
        "writes": outputs.clone(),
        "triggered_by": inputs.clone(),
        "affects": outputs.clone(),
        "lineage": lineage,
        "inputs": inputs,
        "outputs": outputs,
        "internal_nodes": internal_nodes,
    });

    let mut diagnostics = vec![crate::output::Diagnostic {
        severity: crate::output::DiagnosticSeverity::Info,
        code: "LINEAGE_DEFERRED_TO_M6".to_string(),
        message: "Field-level lineage not yet implemented".to_string(),
        location: crate::output::Location::new(),
        suggestion: Some("Use --context or wait for M6 milestone".to_string()),
    }];
    if internal_count == 0 {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "DATAFLOW_INTERNAL_NODES_EMPTY".to_string(),
            message: "Could not enumerate internal DataFlow nodes".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some("Check if DataFlow metadata is complete".to_string()),
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
    output.next_queries = vec![
        format!("--query-dataflow {} for full subgraph", node.name),
        format!("--context {} --depth 2 for surrounding closure", node.id),
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
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
