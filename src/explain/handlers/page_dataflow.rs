use crate::explain::evidence::{push_lineage_evidence, push_relation_evidence};
use crate::explain::importance::classify_importance;
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use anyhow::Result;
use serde_json::Value;
use std::io::{self, Write};

pub(in crate::explain) fn explain_page_graph(
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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
            if let Some(child_neighbors) = graph.get_node_edges(&target.id).ok().flatten() {
                let mut has_action = false;
                let mut child_reads = false;
                let mut child_writes = false;
                for edge_view in &child_neighbors.outgoing {
                    let e = &edge_view.edge;
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
            if let Some(child_neighbors) = graph.get_node_edges(&target.id).ok().flatten() {
                for edge_view in &child_neighbors.outgoing {
                    let t = &edge_view.node;
                    let e = &edge_view.edge;
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
            if let Some(child_neighbors) = graph.get_node_edges(&target.id).ok().flatten() {
                for edge_view in &child_neighbors.outgoing {
                    let action_node = &edge_view.node;
                    let action_edge = &edge_view.edge;
                    if matches!(action_edge.edge_type, crate::graph::EdgeType::Triggers)
                        && let Some(action_neighbors) =
                            graph.get_node_edges(&action_node.id).ok().flatten()
                    {
                        for action_edge_view in &action_neighbors.outgoing {
                            let t = &action_edge_view.node;
                            let e = &action_edge_view.edge;
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
        
                count: None,
                answer_impact: None,
                first_seen_phase: None,
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
    
                count: None,
                answer_impact: None,
                first_seen_phase: None,
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

pub(in crate::explain) fn explain_dataflow_graph(
    _graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
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
                count: None,
                answer_impact: None,
                first_seen_phase: None,
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
        
                count: None,
                answer_impact: None,
                first_seen_phase: None,
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
