use crate::explain::evidence::{push_lineage_evidence, push_relation_evidence};
use crate::explain::importance::classify_importance;
use crate::graph::{
    find_consumed_by_dataflows, find_dataflow_inputs, find_dataflow_outputs, find_produced_by,
    find_readers, find_writers,
};
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use anyhow::Result;
use serde_json::Value;
use std::io::{self, Write};

use super::super::find_parent_page;

/// 把字段的血缘归纳成一句话：这个字段的值是从哪些字段、经过哪一段传过来的。
///
/// M58.3 评测里 `dataflow_chain_trace` 问的是 df_a -> physical_x -> df_b 的链式传递。
/// 工具算得出来——`details.lineage` 里两条记录写着 `source_fields: ["field:physical_x.id"]`
/// 和 `["field:df_a.id"]`，`transform` 都是 "DataFlow chain"——但 summary 只给一个
/// `lineage_count: 2`，同时 `primary_reason` 还写着「未发现明确的阻塞条件或数据链路」。
/// 模型读到的是「没有链路」，于是答「没有链式传递」。
///
/// 数出 2 条和说出「df_b.id 来自 physical_x.id 和 df_a.id，经 DataFlow 链式传递」之间那步
/// 推理是确定的，正是应该留在工具里的部分。
fn lineage_statement(field_id: &str, lineage: &[Value]) -> Option<String> {
    let mut sources: Vec<String> = Vec::new();
    let mut via_dataflow = false;
    for entry in lineage {
        if entry
            .get("transform")
            .and_then(Value::as_str)
            .is_some_and(|transform| transform.contains("DataFlow chain"))
        {
            via_dataflow = true;
        }
        let Some(fields) = entry.get("source_fields").and_then(Value::as_array) else {
            continue;
        };
        for source in fields.iter().filter_map(Value::as_str) {
            if !sources.iter().any(|known| known == source) {
                sources.push(source.to_string());
            }
        }
    }
    if sources.is_empty() {
        return None;
    }
    let how = if via_dataflow {
        "经 DataFlow 链式传递（lineage）"
    } else {
        "经字段传递（lineage）"
    };
    Some(format!(
        "字段 {field_id} 的值{how}来自：{}。逐条来源、经过哪个节点、依据哪条边，见 details.lineage 的 source_fields / via_node / evidence。",
        sources.join("、")
    ))
}

pub(in crate::explain) fn explain_model_graph(
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    _outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    _incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    // Use existing graph query helpers for richer semantics
    let readers = find_readers(graph, &node.id)?;
    let writers = find_writers(graph, &node.id)?;
    let dataflow_inputs = find_dataflow_inputs(graph, &node.id)?;
    let dataflow_outputs = find_dataflow_outputs(graph, &node.id)?;
    let produced_by = find_produced_by(graph, &node.id)?;
    let consumed_by_dataflows = find_consumed_by_dataflows(graph, &node.id)?;

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
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    _outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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

pub(in crate::explain) fn explain_field_graph(
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    _outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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
        && let Some(model_neighbors) = graph.get_node_edges(&model.id).ok().flatten()
    {
        let field_name = node.name.as_str();
        for edge_view in &model_neighbors.incoming {
            let source = &edge_view.node;
            let edge = &edge_view.edge;
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
                count: None,
                answer_impact: None,
                first_seen_phase: None,
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
            let producer_node = graph.get_node(producer_id).ok().flatten();
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
            if let Some(producer_neighbors) = graph.get_node_edges(producer_id).ok().flatten() {
                for edge_view in &producer_neighbors.outgoing {
                    let upstream_model = &edge_view.node;
                    let edge = &edge_view.edge;
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
        let dataflow_inputs = find_dataflow_inputs(graph, &model.id).unwrap_or_default();
        let field_name = node.name.as_str();
        for (input_model, _edge) in dataflow_inputs {
            let input_dims: Vec<serde_json::Value> = input_model
                .meta
                .as_ref()
                .and_then(|m| m.get("dimensions"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut matched_input_field = false;
            for dim in &input_dims {
                let dim_name = dim.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let dbfield = dim.get("dbfield").and_then(|v| v.as_str()).unwrap_or("");
                if dim_name == field_name || dbfield == field_name {
                    matched_input_field = true;
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
            if !matched_input_field {
                let source_field = format!("field:{}.{}", input_model.name, field_name);
                // depends 先创建的占位模型可能没有 dimensions；字段节点仍可作为血缘兜底。
                if graph.get_node(&source_field).ok().flatten().is_some() {
                    lineage.push(serde_json::json!({
                        "target_field": node.id,
                        "source_fields": [source_field],
                        "source_expr": null,
                        "transform": "DataFlow chain (input model field node match)",
                        "via_node": model.id.clone(),
                        "confidence": "medium",
                        "evidence": {
                            "source_file": input_model.path.clone(),
                            "node_id": input_model.id.clone(),
                            "edge_type": "DataflowInput",
                            "raw_expr": null,
                            "json_path": format!("field node '{}'", field_name),
                        }
                    }));
                }
            }
        }
        // Check if parent is produced by a DataFlow (physical table case)
        let producers = find_produced_by(graph, &model.id).unwrap_or_default();
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
            suggestion: Some(,
            count: None,
            answer_impact: None,
            first_seen_phase: None,
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
        "lineage_statement": lineage_statement(&node.id, &lineage),
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
