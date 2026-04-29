use crate::graph::GraphDB;
use anyhow::Result;
use serde_json::json;
use std::io::{self, Write};

/// 图查询模块
///
/// 提供面向用户的查询接口，支持 human 可读格式和 JSON 格式：
///   - query_model：查询某个模型被哪些页面读取/写入
///   - query_page：查询某个页面的出边/入边关系
///   - query_cross：查询两个页面之间的跨文件关系
///   - query_dataflow：展开 DataFlow 的子图，做字段级来源追溯
///
/// 追溯节点所属的页面（通过 Contains 边）
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

pub fn query_model(graph: &GraphDB, model_id: &str, human: bool) -> Result<()> {
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Model: {} ===", model_id)?;

        let readers = graph.find_readers(model_id);
        writeln!(
            out,
            "
--- Read By ({} nodes) ---",
            readers.len()
        )?;
        for (node, edge) in readers {
            let page = find_parent_page(graph, &node.id);
            let page_info = page
                .as_ref()
                .map(|p| format!(" (page: {})", p.name))
                .unwrap_or_default();
            writeln!(
                out,
                "  {} [{}] {}{} via {}",
                node.name,
                node.id,
                format!("{:?}", node.node_type).to_lowercase(),
                page_info,
                edge.field_path.as_deref().unwrap_or("-")
            )?;
        }

        let writers = graph.find_writers(model_id);
        writeln!(
            out,
            "
--- Written By ({} nodes) ---",
            writers.len()
        )?;
        for (node, edge) in writers {
            let page = find_parent_page(graph, &node.id);
            let page_info = page
                .as_ref()
                .map(|p| format!(" (page: {})", p.name))
                .unwrap_or_default();
            writeln!(
                out,
                "  {} [{}] {}{} via {}",
                node.name,
                node.id,
                format!("{:?}", node.node_type).to_lowercase(),
                page_info,
                edge.field_path.as_deref().unwrap_or("-")
            )?;
        }
    } else {
        let readers: Vec<_> = graph
            .find_readers(model_id)
            .into_iter()
            .map(|(n, e)| {
                let page = find_parent_page(graph, &n.id);
                serde_json::json!({
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "component_or_action": n.name,
                    "node_id": n.id,
                    "node_type": format!("{:?}", n.node_type),
                    "edge_type": "Reads",
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "meta": e.meta,
                })
            })
            .collect();
        let writers: Vec<_> = graph
            .find_writers(model_id)
            .into_iter()
            .map(|(n, e)| {
                let page = find_parent_page(graph, &n.id);
                serde_json::json!({
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "component_or_action": n.name,
                    "node_id": n.id,
                    "node_type": format!("{:?}", n.node_type),
                    "edge_type": format!("{:?}", e.edge_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "meta": e.meta,
                })
            })
            .collect();
        let dataflow_inputs = graph
            .find_dataflow_inputs(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        let dataflow_outputs = graph
            .find_dataflow_outputs(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        let produced_by = graph
            .find_produced_by(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        let consumed_by_dataflows = graph
            .find_consumed_by_dataflows(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        // 兼容性保留字段
        let upstream = graph
            .find_upstream_dependencies(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        let downstream = graph
            .find_downstream_outputs(model_id)
            .into_iter()
            .map(|(n, e)| {
                serde_json::json!({
                    "node_id": n.id,
                    "name": n.name,
                    "node_type": format!("{:?}", n.node_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "edge_type": format!("{:?}", e.edge_type),
                })
            })
            .collect::<Vec<_>>();
        let summary = serde_json::json!({
            "model_id": model_id,
            "read_by_count": readers.len(),
            "written_by_count": writers.len(),
            "dataflow_role": "Unknown",
        });

        let details = serde_json::json!({
            "readers": readers,
            "writers": writers,
            "dataflow_inputs": dataflow_inputs,
            "dataflow_outputs": dataflow_outputs,
            "produced_by": produced_by,
            "consumed_by_dataflows": consumed_by_dataflows,
            "upstream_dependencies": upstream,
            "downstream_outputs": downstream,
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::ModelQuery, summary);
        output.query_target = Some(model_id.to_string());
        output.details = Some(details);
        // 为 summary 中每个计数和 details 中每个主要数组提供独立 evidence
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Model {} has {} readers", model_id, readers.len()),
                "Graph traversal: find_readers",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(model_id),
        );
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Model {} has {} writers", model_id, writers.len()),
                "Graph traversal: find_writers",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(model_id),
        );
        if !dataflow_inputs.is_empty() {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Model {} is input to {} dataflows",
                        model_id,
                        dataflow_inputs.len()
                    ),
                    "Graph traversal: find_dataflow_inputs",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(model_id),
            );
        }
        if !dataflow_outputs.is_empty() {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Model {} is output from {} dataflows",
                        model_id,
                        dataflow_outputs.len()
                    ),
                    "Graph traversal: find_dataflow_outputs",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(model_id),
            );
        }
        if !produced_by.is_empty() {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Model {} is produced by {} nodes",
                        model_id,
                        produced_by.len()
                    ),
                    "Graph traversal: find_produced_by",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(model_id),
            );
        }
        if !consumed_by_dataflows.is_empty() {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Model {} is consumed by {} dataflows",
                        model_id,
                        consumed_by_dataflows.len()
                    ),
                    "Graph traversal: find_consumed_by_dataflows",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(model_id),
            );
        }
        output.next_queries = vec![
            format!("--explain {} for full semantic summary", model_id),
            format!("--query-dataflow {} for internal subgraph", model_id),
        ];

        let output = output.validate();
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

/// 查询页面的跨文件依赖关系
pub fn query_page(graph: &GraphDB, page_id: &str, human: bool) -> Result<()> {
    if let Some((outgoing, incoming)) = graph.get_node_edges(page_id) {
        if human {
            let mut out = io::stdout();
            writeln!(out, "=== Page: {} ===", page_id)?;
            writeln!(out, "\n--- Outgoing Edges ({}): ---", outgoing.len())?;
            for (node, edge) in outgoing {
                writeln!(out, "  -> {} ({:?})", node.name, edge.edge_type)?;
            }
            writeln!(out, "\n--- Incoming Edges ({}): ---", incoming.len())?;
            for (node, edge) in incoming {
                writeln!(out, "  <- {} ({:?})", node.name, edge.edge_type)?;
            }
        } else {
            let summary = serde_json::json!({
                "page_id": page_id,
                "outgoing_count": outgoing.len(),
                "incoming_count": incoming.len(),
                "relation_types": outgoing.iter().map(|(_, e)| format!("{:?}", e.edge_type)).collect::<std::collections::HashSet<String>>().into_iter().collect::<Vec<String>>(),
            });

            let details = serde_json::json!({
                "outgoing": outgoing.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
                "incoming": incoming.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
            });

            let mut output =
                crate::output::AiOutput::new(crate::output::OutputKind::PageQuery, summary);
            output.query_target = Some(page_id.to_string());
            output.details = Some(details);
            output.evidence.push(
                crate::output::Evidence::new(
                    format!("Page {} has {} outgoing edges", page_id, outgoing.len()),
                    "Graph traversal: outgoing edges from page node",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(page_id),
            );
            output.evidence.push(
                crate::output::Evidence::new(
                    format!("Page {} has {} incoming edges", page_id, incoming.len()),
                    "Graph traversal: incoming edges to page node",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_node_id(page_id),
            );
            output.next_queries = vec![format!(
                "--query-page-logic {} for page-level logic summary",
                page_id
            )];

            let output = output.validate();
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    } else {
        anyhow::bail!("Page {} not found in graph", page_id);
    }
    Ok(())
}

/// 查询两个页面之间的直接或间接关系
pub fn query_cross(graph: &GraphDB, page_a: &str, page_b: &str, human: bool) -> Result<()> {
    let paths = graph.find_cross_relations(page_a, page_b);
    if human {
        let mut out = io::stdout();
        writeln!(
            out,
            "=== Cross-File Relations: {} <-> {} ===",
            page_a, page_b
        )?;
        writeln!(out, "\nDirect Connections: {}", paths.len())?;
        for (i, path) in paths.iter().enumerate() {
            writeln!(out, "\n[Path {}]", i + 1)?;
            for (node, edge) in path {
                writeln!(
                    out,
                    "  {} ({:?}) -> {:?}",
                    node.name, node.node_type, edge.edge_type
                )?;
            }
        }
    } else {
        let json_paths: Vec<_> = paths
            .iter()
            .map(|path| {
                path.iter()
                    .map(|(n, e)| {
                        serde_json::json!({
                            "name": n.name,
                            "type": format!("{:?}", n.node_type),
                            "edge": format!("{:?}", e.edge_type),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        let summary = serde_json::json!({
            "page_a": page_a,
            "page_b": page_b,
            "path_count": paths.len(),
        });

        let details = serde_json::json!({
            "paths": json_paths,
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::CrossPageQuery, summary);
        output.query_target = Some(format!("{} <-> {}", page_a, page_b));
        output.details = Some(details);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Cross-page query found {} paths between {} and {}",
                    paths.len(),
                    page_a,
                    page_b
                ),
                "Graph traversal: find_cross_relations",
            )
            .with_confidence(crate::output::Confidence::High),
        );
        for (i, path) in paths.iter().take(5).enumerate() {
            let nodes: Vec<String> = path.iter().map(|(n, _)| n.name.clone()).collect();
            output.evidence.push(
                crate::output::Evidence::new(
                    format!("Path {}: {}", i + 1, nodes.join(" -> ")),
                    "Graph traversal: individual cross-page path",
                )
                .with_confidence(crate::output::Confidence::High),
            );
        }
        output.next_queries = vec![
            format!("--query-page {} for page dependencies", page_a),
            format!("--query-page {} for page dependencies", page_b),
        ];

        let output = output.validate();
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

mod dataflow;
pub use dataflow::query_dataflow;

/// 查询页面级逻辑摘要
///
/// 输出 page_inputs、data_sources、write_targets、entrypoints、action_flows、visibility_rules、navigation、risk_diagnostics。
pub fn query_page_logic(graph: &GraphDB, page_id: &str, human: bool) -> Result<()> {
    let page_node = graph
        .get_node(page_id)
        .ok_or_else(|| anyhow::anyhow!("Page '{}' not found in graph", page_id))?;

    let (outgoing, _incoming) = graph
        .get_node_edges(page_id)
        .unwrap_or_else(|| (Vec::new(), Vec::new()));

    let page_inputs: Vec<serde_json::Value> = Vec::new();
    let mut data_sources: Vec<serde_json::Value> = Vec::new();
    let mut write_targets: Vec<serde_json::Value> = Vec::new();
    let mut entrypoints: Vec<serde_json::Value> = Vec::new();
    let action_flows: Vec<serde_json::Value> = Vec::new();
    let visibility_rules: Vec<serde_json::Value> = Vec::new();
    let mut navigation: Vec<serde_json::Value> = Vec::new();
    let risk_diagnostics: Vec<serde_json::Value> = Vec::new();

    // Collect child components via Contains edges
    let mut child_nodes = Vec::new();
    for (target, edge) in &outgoing {
        if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
            child_nodes.push((*target).clone());
        }
    }

    for child in &child_nodes {
        let child_type = format!("{:?}", child.node_type).to_lowercase();
        if child_type.contains("component") || child_type.contains("action") {
            // Check if this component has actions (entrypoints)
            if let Some((child_out, _)) = graph.get_node_edges(&child.id) {
                let mut has_action = false;
                for (target, edge) in &child_out {
                    match edge.edge_type {
                        crate::graph::EdgeType::Reads => {
                            data_sources.push(json!({
                                "component_id": child.id,
                                "model": target.name,
                                "field_path": edge.field_path,
                            }));
                        }
                        crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                            write_targets.push(json!({
                                "component_id": child.id,
                                "model": target.name,
                                "field_path": edge.field_path,
                            }));
                            has_action = true;
                        }
                        crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::SetsParam => {
                            navigation.push(json!({
                                "from": child.id,
                                "to": target.id,
                                "type": format!("{:?}", edge.edge_type),
                            }));
                            has_action = true;
                        }
                        _ => {}
                    }
                }

                if has_action {
                    entrypoints.push(json!({
                        "id": child.id,
                        "name": child.name,
                        "type": child_type,
                    }));
                }
            }
        }
    }

    // Simple risk diagnostics
    let mut diagnostics = Vec::new();
    if write_targets.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "NO_WRITE_TARGETS".to_string(),
            message: "Page has no detected write targets".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some("Verify if page is read-only or actions are not parsed".to_string()),
        });
    }

    if entrypoints.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "NO_ENTRYPOINTS".to_string(),
            message: "Page has no detected user entrypoints (buttons, links, etc.)".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some("Check component action definitions".to_string()),
        });
    }

    let summary = serde_json::json!({
        "page_id": page_id,
        "page_name": page_node.name,
        "entrypoint_count": entrypoints.len(),
        "data_source_count": data_sources.len(),
        "write_target_count": write_targets.len(),
        "risk_count": diagnostics.len(),
    });

    let details = serde_json::json!({
        "page_inputs": page_inputs,
        "data_sources": data_sources,
        "write_targets": write_targets,
        "entrypoints": entrypoints,
        "action_flows": action_flows,
        "visibility_rules": visibility_rules,
        "navigation": navigation,
        "risk_diagnostics": diagnostics.iter().map(|d| serde_json::json!({
            "severity": format!("{:?}", d.severity),
            "code": d.code,
            "message": d.message,
            "suggestion": d.suggestion,
        })).collect::<Vec<serde_json::Value>>(),
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::PageLogic, summary);
    output.query_target = Some(page_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Page {} has {} entrypoints", page_id, entrypoints.len()),
            "Graph traversal: child components with actions",
        )
        .with_confidence(crate::output::Confidence::Medium)
        .with_node_id(page_id)
        .with_source_file(&page_node.path),
    );
    if !data_sources.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} reads from {} data sources",
                    page_id,
                    data_sources.len()
                ),
                "Graph traversal: Reads edges from child components",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path),
        );
    }
    if !write_targets.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} writes to {} targets", page_id, write_targets.len()),
                "Graph traversal: Writes/ActionWrites edges from child components",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path),
        );
    }
    output.next_queries = vec![
        format!("--query-page {} for page dependencies", page_id),
        format!("--explain {} for page semantic summary", page_id),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Page Logic: {} ===", page_id)?;
        writeln!(out, "Name: {}", page_node.name)?;
        writeln!(out, "\nEntrypoints: {}", entrypoints.len())?;
        for ep in &entrypoints {
            writeln!(out, "  {:?}", ep)?;
        }
        writeln!(out, "\nData Sources: {}", data_sources.len())?;
        for ds in &data_sources {
            writeln!(out, "  {:?}", ds)?;
        }
        writeln!(out, "\nWrite Targets: {}", write_targets.len())?;
        for wt in &write_targets {
            writeln!(out, "  {:?}", wt)?;
        }
        writeln!(out, "\nNavigation: {}", navigation.len())?;
        for nav in &navigation {
            writeln!(out, "  {:?}", nav)?;
        }
        if !risk_diagnostics.is_empty() {
            writeln!(out, "\n⚠️  Risk Diagnostics:")?;
            for risk in &risk_diagnostics {
                writeln!(out, "  {:?}", risk)?;
            }
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
