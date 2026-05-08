use crate::graph::GraphDB;
use crate::output::schema::{format_next_query, format_next_query_multi};
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

pub fn query_model(graph: &GraphDB, model_id: &str, human: bool, budget: &str) -> Result<()> {
    let is_compact = budget == "compact";
    if graph.get_node(model_id).is_none() {
        let candidates = graph.find_candidates(model_id, 5);
        let out = crate::output::schema::build_target_not_found_output(
            crate::output::schema::OutputKind::ModelQuery,
            model_id,
            &candidates,
        );
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
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
        let consumed_by_dataflow_count = consumed_by_dataflows.len();
        let produced_by_count = produced_by.len();
        let dataflow_input_count = dataflow_inputs.len();
        let dataflow_output_count = dataflow_outputs.len();

        let mut what_is_it = format!(
            "模型 {}，被 {} 个节点读取，被 {} 个节点写入",
            model_id,
            readers.len(),
            writers.len()
        );
        if readers.is_empty()
            && writers.is_empty()
            && (dataflow_input_count > 0 || dataflow_output_count > 0)
        {
            what_is_it = format!(
                "模型 {} 主要通过 DataFlow 被消费/产出（输入 {} 个，输出 {} 个）",
                model_id, dataflow_input_count, dataflow_output_count
            );
        }

        let summary = serde_json::json!({
            "model_id": model_id,
            "what_is_it": what_is_it,
            "read_by_count": readers.len(),
            "written_by_count": writers.len(),
            "consumed_by_dataflow_count": consumed_by_dataflow_count,
            "produced_by_count": produced_by_count,
            "dataflow_input_count": dataflow_input_count,
            "dataflow_output_count": dataflow_output_count,
            "dataflow_role": if dataflow_input_count > 0 || dataflow_output_count > 0 { "DataFlowParticipant" } else { "Unknown" },
        });

        let details = if is_compact {
            serde_json::json!({
                "readers": crate::output::brief::truncated_array(&readers, 5),
                "writers": crate::output::brief::truncated_array(&writers, 5),
                "dataflow_inputs": crate::output::brief::truncated_array(&dataflow_inputs, 5),
                "dataflow_outputs": crate::output::brief::truncated_array(&dataflow_outputs, 5),
                "produced_by": crate::output::brief::truncated_array(&produced_by, 5),
                "consumed_by_dataflows": crate::output::brief::truncated_array(&consumed_by_dataflows, 5),
                "upstream_dependencies": crate::output::brief::truncated_array(&upstream, 5),
                "downstream_outputs": crate::output::brief::truncated_array(&downstream, 5),
            })
        } else {
            serde_json::json!({
                "readers": readers,
                "writers": writers,
                "dataflow_inputs": dataflow_inputs,
                "dataflow_outputs": dataflow_outputs,
                "produced_by": produced_by,
                "consumed_by_dataflows": consumed_by_dataflows,
                "upstream_dependencies": upstream,
                "downstream_outputs": downstream,
            })
        };

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
            format_next_query("--explain {} for full semantic summary", model_id),
            format_next_query("--query-dataflow {} for internal subgraph", model_id),
        ];

        // Compact mode: add OUTPUT_TRUNCATED diagnostic and evidence_summary
        if is_compact {
            let truncated_arrays = [
                ("readers", readers.len(), 5),
                ("writers", writers.len(), 5),
                ("dataflow_inputs", dataflow_inputs.len(), 5),
                ("dataflow_outputs", dataflow_outputs.len(), 5),
                ("produced_by", produced_by.len(), 5),
                ("consumed_by_dataflows", consumed_by_dataflows.len(), 5),
                ("upstream_dependencies", upstream.len(), 5),
                ("downstream_outputs", downstream.len(), 5),
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
                        source_file: None,
                        node_id: Some(model_id.to_string()),
                        json_path: None,
                    },
                    suggestion: Some(
                        "Use --budget normal or --budget full to see complete arrays".to_string(),
                    ),
                });
            }

            let evidence_summary = crate::output::brief::evidence_summary(&output.evidence, 5);
            let key_findings =
                crate::output::brief::build_key_findings(&output.summary, &output.diagnostics);
            if let Some(obj) = output.summary.as_object_mut() {
                obj.insert("evidence_summary".to_string(), evidence_summary);
                obj.insert("key_findings".to_string(), serde_json::json!(key_findings));
            }
            output.evidence.truncate(5);
        }

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
            output.next_queries = vec![format_next_query(
                "--query-page-logic {} for page-level logic summary",
                page_id,
            )];

            let output = output.validate();
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
    } else {
        let candidates = graph.find_candidates(page_id, 5);
        let out = crate::output::schema::build_target_not_found_output(
            crate::output::schema::OutputKind::PageQuery,
            page_id,
            &candidates,
        );
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
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
            format_next_query("--query-page {} for page dependencies", page_a),
            format_next_query("--query-page {} for page dependencies", page_b),
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
/// 查询页面级逻辑摘要
///
/// 输出 page_inputs、data_sources、write_targets、entrypoints、action_flows、visibility_rules、navigation、risk_diagnostics。
pub fn query_page_logic(
    graph: &GraphDB,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    human: bool,
    budget: &str,
) -> Result<()> {
    let is_compact = budget == "compact";
    let page_node = match graph.get_node(page_id) {
        Some(n) => n,
        None => {
            let candidates = graph.find_candidates(page_id, 5);
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::PageQuery,
                page_id,
                &candidates,
            );
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
    };

    // ---- 1. 收集页面下所有 Component 节点，再收集它们 Triggers 出的 Action 节点 ----
    let (page_out, _) = graph
        .get_node_edges(page_id)
        .unwrap_or_else(|| (Vec::new(), Vec::new()));

    let mut child_components: Vec<&crate::graph::Node> = Vec::new();
    let mut child_actions: Vec<&crate::graph::Node> = Vec::new();
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (target, edge) in &page_out {
        if matches!(edge.edge_type, crate::graph::EdgeType::Contains)
            && matches!(target.node_type, crate::graph::NodeType::Component)
            && !visited.contains(&target.id)
        {
            visited.insert(target.id.clone());
            child_components.push(target);
            if let Some((comp_out, _)) = graph.get_node_edges(&target.id) {
                for (act, e) in &comp_out {
                    if matches!(e.edge_type, crate::graph::EdgeType::Triggers)
                        && matches!(act.node_type, crate::graph::NodeType::Action)
                        && !visited.contains(&act.id)
                    {
                        visited.insert(act.id.clone());
                        child_actions.push(act);
                    }
                }
            }
        }
    }

    // ---- 2. 从原始文件读取：递归收集组件元数据、action 元数据、visibility_rules ----
    let mut page_inputs: Vec<serde_json::Value> = Vec::new();
    let mut visibility_rules: Vec<serde_json::Value> = Vec::new();
    let mut from_file = false;
    let mut component_json_paths: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // action_id (裸 id，不含前缀) -> { trigger_type, wait_prev, condition }
    let mut action_meta: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();

    if let Some(proj_dir) = project_dir {
        let file_path = proj_dir.join(&page_node.path);
        if file_path.exists()
            && let Ok(content) = std::fs::read_to_string(&file_path)
            && let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&content)
        {
            from_file = true;
            // page_inputs: params
            if let Some(params) = json_val.get("params").and_then(|p| p.as_array()) {
                for p in params {
                    page_inputs.push(json!({
                        "id": p.get("id"),
                        "name": p.get("name"),
                        "type": "page_param",
                    }));
                }
            }

            // 递归收集组件和 action 元数据
            fn collect_components(
                arr: &[serde_json::Value],
                path_prefix: &str,
                source_file: &str,
                component_json_paths: &mut std::collections::HashMap<String, String>,
                visibility_rules: &mut Vec<serde_json::Value>,
                action_meta: &mut std::collections::HashMap<String, serde_json::Value>,
            ) {
                for (index, comp) in arr.iter().enumerate() {
                    let comp_id = comp.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let component_path = format!("{}[{}]", path_prefix, index);
                    if !comp_id.is_empty() {
                        component_json_paths.insert(comp_id.to_string(), component_path.clone());
                    }
                    for prop in ["visible", "hidden", "disabled", "readonly"] {
                        if let Some(val) = comp.get(prop) {
                            let mut expr_struct =
                                crate::action_semantics::build_expression_struct(val.as_str());
                            let mut rule = json!({
                                "component_id": comp_id,
                                "rule": prop,
                                "expression": val,
                                "source_file": source_file,
                                "json_path": format!("{}.{}", component_path, prop),
                            });
                            if let Some(obj) = expr_struct.as_object_mut() {
                                for key in [
                                    "raw_expr",
                                    "refs",
                                    "resolved_refs",
                                    "unresolved_refs",
                                    "ambiguous_refs",
                                    "diagnostics",
                                    "confidence",
                                ] {
                                    rule[key] = obj.remove(key).unwrap_or(serde_json::Value::Null);
                                }
                            }
                            visibility_rules.push(rule);
                        }
                    }
                    // 收集 action 元数据，key 用 comp_id + "|" + action_id 防止同名 action 串线
                    if let Some(actions) = comp.get("actions").and_then(|a| a.as_array()) {
                        for (action_index, act) in actions.iter().enumerate() {
                            if let Some(aid) = act.get("id").and_then(|v| v.as_str()) {
                                let trigger_type = act
                                    .get("triggerType")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("click");
                                let wait_prev = act.get("waitPrev").cloned();
                                // condition: condition 字段或 conditionExp 字段
                                let condition = act
                                    .get("condition")
                                    .or_else(|| act.get("conditionExp"))
                                    .cloned();
                                let action_path =
                                    format!("{}.actions[{}]", component_path, action_index);
                                action_meta.insert(
                                    format!("{}|{}", comp_id, aid),
                                    json!({
                                        "trigger_type": trigger_type,
                                        "wait_prev": wait_prev,
                                        "condition": condition,
                                        "json_path": action_path,
                                        "source_file": source_file,
                                    }),
                                );
                            }
                        }
                    }
                    // 递归嵌套组件
                    for nested_key in ["components", "panels", "steps", "comps"] {
                        if let Some(nested) = comp.get(nested_key).and_then(|v| v.as_array()) {
                            collect_components(
                                nested,
                                &format!("{}.{}", component_path, nested_key),
                                source_file,
                                component_json_paths,
                                visibility_rules,
                                action_meta,
                            );
                        }
                    }
                }
            }

            if let Some(components) = json_val
                .get("canvas")
                .and_then(|c| c.get("components"))
                .and_then(|c| c.as_array())
            {
                collect_components(
                    components,
                    "canvas.components",
                    &page_node.path,
                    &mut component_json_paths,
                    &mut visibility_rules,
                    &mut action_meta,
                );
            }
        }
    }

    // ---- 3. Entrypoints：只包含用户可触发组件（有 action 的 button/link 等） ----
    let mut entrypoints: Vec<serde_json::Value> = Vec::new();
    for comp in &child_components {
        if let Some((comp_out, _)) = graph.get_node_edges(&comp.id) {
            let has_trigger = comp_out
                .iter()
                .any(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::Triggers));
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

    // ---- 4. 收集所有 Component & Action 的出边，用于 data_sources / write_targets / navigation ----
    let mut data_sources: Vec<serde_json::Value> = Vec::new();
    let mut write_targets: Vec<serde_json::Value> = Vec::new();
    let mut navigation: Vec<serde_json::Value> = Vec::new();
    let mut action_flows: Vec<serde_json::Value> = Vec::new();

    let mut all_nodes: Vec<&crate::graph::Node> = Vec::new();
    all_nodes.extend(child_components.iter().copied());
    all_nodes.extend(child_actions.iter().copied());

    for node in &all_nodes {
        if let Some((node_out, _)) = graph.get_node_edges(&node.id) {
            for (target, edge) in &node_out {
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

    // ---- 5. Action flows：遍历 Action 节点，聚合 reads/writes/navigation ----
    for action in &child_actions {
        let (action_out, action_in) = graph
            .get_node_edges(&action.id)
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        let (action_type, action_id) = action
            .name
            .split_once(':')
            .map(|(t, i)| (t.to_string(), i.to_string()))
            .unwrap_or_else(|| (action.name.clone(), action.name.clone()));

        let parent_component = action_in.iter().find_map(|(src, e)| {
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

        for (target, edge) in &action_out {
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

    // ---- 6. Risk diagnostics ----
    let mut diagnostics: Vec<crate::output::Diagnostic> = Vec::new();

    if write_targets.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "NO_WRITE_TARGETS".to_string(),
            message: "Page has no detected write targets".to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Verify if page is read-only or actions are not parsed".to_string()),
        });
    }

    if entrypoints.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "NO_ENTRYPOINTS".to_string(),
            message: "Page has no detected user entrypoints (buttons, links, etc.)".to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: Some("canvas.components[*].actions[*]".to_string()),
            },
            suggestion: Some("Check component action definitions".to_string()),
        });
    }

    if !from_file {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "PAGE_INPUTS_DEFERRED".to_string(),
            message: "Raw page file is unavailable; page_inputs/visibility_rules may be incomplete"
                .to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(
                "Ensure --project-dir points to the project root containing this page file"
                    .to_string(),
            ),
        });
    }

    // UNRESOLVED_PAGE_NAVIGATION：导航目标页面不存在于图中
    for nav in &navigation {
        let to = nav.get("to").and_then(|v| v.as_str()).unwrap_or("");
        if to.starts_with("page:") && graph.get_node(to).is_none() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNRESOLVED_PAGE_NAVIGATION".to_string(),
                message: format!("Navigation target page '{}' not found in graph", to),
                location: crate::output::Location {
                    source_file: nav
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: nav
                        .get("from")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: nav
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Check if target page exists in project".to_string()),
            });
        }
    }

    // UNRESOLVED_MODEL_WRITE：写入目标模型不存在于图中
    for wt in &write_targets {
        let target_id = wt.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if target_id.starts_with("model:") && graph.get_node(target_id).is_none() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNRESOLVED_MODEL_WRITE".to_string(),
                message: format!("Write target model '{}' not found in graph", target_id),
                location: crate::output::Location {
                    source_file: wt
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: pick_str_field(wt, &["source_action", "source_component"])
                        .map(ToString::to_string),
                    json_path: wt
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Check if target model exists in project sources".to_string()),
            });
        }
    }

    // VISIBILITY_RULE_UNRESOLVED：visibility 规则中的表达式包含未解析或多义引用
    for rule in &visibility_rules {
        let has_unresolved = rule
            .get("unresolved_refs")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        let has_unresolved_diag = rule
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|d| {
                    d.get("code").and_then(|v| v.as_str()) == Some("EXPR_UNRESOLVED_REF")
                        || d.get("code").and_then(|v| v.as_str()) == Some("EXPR_AMBIGUOUS_REF")
                })
            })
            .unwrap_or(false);
        if has_unresolved || has_unresolved_diag {
            let expr = rule
                .get("raw_expr")
                .and_then(|v| v.as_str())
                .or_else(|| rule.get("expression").and_then(|v| v.as_str()))
                .unwrap_or("<non-string expression>");
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "VISIBILITY_RULE_UNRESOLVED".to_string(),
                message: format!(
                    "Visibility rule '{}' contains unresolved or ambiguous references",
                    expr
                ),
                location: crate::output::Location {
                    source_file: rule
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: rule
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: rule
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some(
                    "Review the expression and verify each referenced component/model exists"
                        .to_string(),
                ),
            });
        }
    }

    // ACTION_FLOW_INCOMPLETE：仅对“通常应产生副作用”的动作类型发出提示
    for flow in &action_flows {
        let reads = flow
            .get("reads")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let writes = flow
            .get("writes")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let nav_count = flow
            .get("navigation")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let action_type = flow
            .get("action_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let action_category = flow
            .get("action_category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let is_query_refresh_like = matches!(
            action_type,
            "loadData" | "resetData" | "refreshData" | "refreshModels" | "newData" | "validateData"
        ) || matches!(
            action_category,
            "data_read" | "data_refresh" | "data_initialization" | "validation"
        );
        let expects_side_effect = matches!(action_category, "data_write" | "param_mutation")
            || matches!(
                action_type,
                "submitData" | "insertData" | "updateData" | "deleteData" | "setParamValue"
            );
        if reads > 0
            && writes == 0
            && nav_count == 0
            && expects_side_effect
            && !is_query_refresh_like
        {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "ACTION_FLOW_INCOMPLETE".to_string(),
                message: format!(
                    "Action {} reads but does not write; may be a query-only action",
                    flow.get("action_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ),
                location: crate::output::Location {
                    source_file: flow
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: flow
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: flow
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Verify if this action should produce a write target".to_string()),
            });
        }
    }

    // UNKNOWN_ACTION_TYPE：存在未识别的 action 类型
    for flow in &action_flows {
        if flow.get("action_category").and_then(|v| v.as_str()) == Some("unknown") {
            let atype = flow
                .get("action_type")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNKNOWN_ACTION_TYPE".to_string(),
                message: format!("Unknown action type '{}' encountered", atype),
                location: crate::output::Location {
                    source_file: flow
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: flow
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: flow
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some(
                    "Check if this action type is supported by metadata-checker".to_string(),
                ),
            });
        }
    }

    // EVIDENCE_SAMPLED：明细数量大于 evidence 展开上限时提示 evidence 非全集
    let evidence_sample_limit = 5usize;
    let sampling_categories = [
        ("entrypoints", entrypoints.len()),
        ("data_sources", data_sources.len()),
        ("write_targets", write_targets.len()),
        ("navigation", navigation.len()),
        ("visibility_rules", visibility_rules.len()),
        ("action_flows", action_flows.len()),
    ];
    let sampled_parts: Vec<String> = sampling_categories
        .iter()
        .filter(|(_, size)| *size > evidence_sample_limit)
        .map(|(name, size)| format!("{name} {size}>{evidence_sample_limit}"))
        .collect();
    if !sampled_parts.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "EVIDENCE_SAMPLED".to_string(),
            message: format!(
                "Evidence includes only a sample for: {}",
                sampled_parts.join(", ")
            ),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(
                "Use details arrays for full coverage; evidence is intentionally low-noise sampled"
                    .to_string(),
            ),
        });
    }

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
        })
    } else {
        serde_json::json!({
            "page_inputs": page_inputs.clone(),
            "data_sources": data_sources.clone(),
            "write_targets": write_targets.clone(),
            "entrypoints": entrypoints.clone(),
            "action_flows": action_flows.clone(),
            "visibility_rules": visibility_rules.clone(),
            "navigation": navigation.clone(),
            "risk_diagnostics": risk_diagnostics,
        })
    };

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::PageLogic, summary);
    output.query_target = Some(page_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics.clone();

    output.evidence.push(
        crate::output::Evidence::new(
            format!("Page {} has {} entrypoints", page_id, entrypoints.len()),
            "Graph traversal: child components with Triggers edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(page_id)
        .with_source_file(&page_node.path)
        .with_json_path("canvas.components[*].actions[*]")
        .with_edge_type("Triggers"),
    );
    for ep in entrypoints.iter().take(evidence_sample_limit) {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Entrypoint {} ({}) can trigger page logic",
                    ep.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                    ep.get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("component")
                ),
                "Component has action definitions in page metadata",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_source_file(
                ep.get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&page_node.path),
            )
            .with_node_id(ep.get("id").and_then(|v| v.as_str()).unwrap_or("?"))
            .with_edge_type(
                ep.get("edge_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Triggers"),
            )
            .with_json_path(
                ep.get("json_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("canvas.components[*].actions[*]"),
            ),
        );
    }
    if !data_sources.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} reads from {} data sources",
                    page_id,
                    data_sources.len()
                ),
                "Graph traversal: Reads edges from page children",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Reads/ActionReads"),
        );
        for ds in data_sources.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Read {} via {}",
                        ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?"),
                        pick_str_field(ds, &["source_action", "source_component"]).unwrap_or("?")
                    ),
                    "Relation extracted from graph edge under page scope",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    ds.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    pick_str_field(ds, &["source_action", "source_component"]).unwrap_or("?"),
                )
                .with_edge_type(
                    ds.get("edge_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Reads"),
                )
                .with_raw_expr(
                    ds.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .unwrap_or(ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?")),
                )
                .with_json_path(
                    ds.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].<expression>"),
                ),
            );
        }
    }
    if !write_targets.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} writes to {} targets", page_id, write_targets.len()),
                "Graph traversal: Writes/ActionWrites edges from page children",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Writes/ActionWrites"),
        );
        for wt in write_targets.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Write {} via {}",
                        wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?"),
                        pick_str_field(wt, &["source_action", "source_component"]).unwrap_or("?")
                    ),
                    "Write target inferred from action/submit binding edge",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    wt.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    pick_str_field(wt, &["source_action", "source_component"]).unwrap_or("?"),
                )
                .with_edge_type(
                    wt.get("edge_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Writes"),
                )
                .with_raw_expr(
                    wt.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .unwrap_or(wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?")),
                )
                .with_json_path(
                    wt.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*].fieldValues[*].value"),
                ),
            );
        }
    }
    if !navigation.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} has {} navigation relations",
                    page_id,
                    navigation.len()
                ),
                "Graph traversal: OpensPage/EmbedsPage/Param transfer edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Navigation"),
        );
        for nav_item in navigation.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Navigation {} -> {} ({})",
                        nav_item.get("from").and_then(|v| v.as_str()).unwrap_or("?"),
                        nav_item.get("to").and_then(|v| v.as_str()).unwrap_or("?"),
                        nav_item.get("type").and_then(|v| v.as_str()).unwrap_or("?")
                    ),
                    "Navigation relation extracted from action/component edges",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    nav_item
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(nav_item.get("from").and_then(|v| v.as_str()).unwrap_or("?"))
                .with_edge_type(
                    nav_item
                        .get("edge_type")
                        .or_else(|| nav_item.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Navigation"),
                )
                .with_raw_expr(
                    nav_item.get("raw_expr").and_then(|v| v.as_str()).unwrap_or(
                        nav_item
                            .get("field_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                    ),
                )
                .with_json_path(
                    nav_item
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*].path"),
                ),
            );
        }
    }
    if !visibility_rules.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} has {} visibility-related rules",
                    page_id,
                    visibility_rules.len()
                ),
                "Visibility conditions are collected recursively from component tree",
            )
            .with_confidence(crate::output::Confidence::Medium)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("VisibilityRule"),
        );
        for rule in visibility_rules.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Visibility rule {}.{}",
                        rule.get("component_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        rule.get("rule").and_then(|v| v.as_str()).unwrap_or("?")
                    ),
                    "Rule originates from component metadata field",
                )
                .with_confidence(match rule.get("confidence").and_then(|v| v.as_str()) {
                    Some("high") => crate::output::Confidence::High,
                    Some("low") => crate::output::Confidence::Low,
                    _ => crate::output::Confidence::Medium,
                })
                .with_source_file(
                    rule.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    rule.get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?"),
                )
                .with_edge_type("VisibilityRule")
                .with_raw_expr(
                    rule.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .or_else(|| rule.get("expression").and_then(|v| v.as_str()))
                        .unwrap_or("<non-string expression>"),
                )
                .with_json_path(
                    rule.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].visible"),
                ),
            );
        }
    }
    if !action_flows.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} has {} action flows", page_id, action_flows.len()),
                "Graph traversal: Action nodes under page",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Triggers"),
        );
        for flow in action_flows.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Action flow {} ({})",
                        flow.get("action_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        flow.get("action_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    ),
                    "Action flow assembled from action node edges plus raw action metadata",
                )
                .with_confidence(
                    if flow.get("action_category").and_then(|v| v.as_str()) == Some("unknown") {
                        crate::output::Confidence::Low
                    } else {
                        crate::output::Confidence::High
                    },
                )
                .with_source_file(
                    flow.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(flow.get("node_id").and_then(|v| v.as_str()).unwrap_or("?"))
                .with_edge_type("Triggers")
                .with_raw_expr(
                    flow.get("condition")
                        .and_then(|v| v.get("raw_expr"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            flow.get("blocks_on")
                                .and_then(|v| v.get("raw"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("action execution metadata"),
                )
                .with_json_path(
                    flow.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*]"),
                ),
            );
        }
    }

    let mut nq = vec![
        format_next_query("--explain {} for page semantic summary", page_id),
        format_next_query_multi(
            "--context {} --depth 2 --budget normal for surrounding context",
            &[page_id, "2"],
        ),
    ];
    let mut model_names: Vec<String> = write_targets
        .iter()
        .filter_map(|wt| wt.get("target_id").and_then(|v| v.as_str()))
        .filter(|id| id.starts_with("model:"))
        .map(|id| id.strip_prefix("model:").unwrap_or(id).to_string())
        .collect();
    model_names.sort();
    model_names.dedup();
    for m in model_names.iter().take(3) {
        nq.push(format_next_query("--query-model {} for model details", m));
    }
    output.next_queries = nq;

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
    let output = output.validate();

    // ---- 8. Human 模式 ----
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Page Logic: {} ===", page_id)?;
        writeln!(out, "Name: {} | Role: {}", page_node.name, page_role)?;
        writeln!(out, "{}", what_is_it)?;
        writeln!(out, "\n--- Entrypoints ({}) ---", entrypoints.len())?;
        for ep in &entrypoints {
            let name = ep.get("name").and_then(|v| v.as_str()).unwrap_or("?");
            let ep_type = ep.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            writeln!(
                out,
                "  [{}] {} ({})",
                ep.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                name,
                ep_type
            )?;
        }
        writeln!(out, "\n--- Data Sources ({}) ---", data_sources.len())?;
        for ds in &data_sources {
            let fp = ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
            // field_path 已包含 model 前缀（如 model1.fieldB），避免重复输出 model.model.field
            let model = ds.get("model").and_then(|v| v.as_str()).unwrap_or("?");
            if fp.starts_with(model) {
                writeln!(out, "  {}", fp)?;
            } else {
                writeln!(out, "  {}.{}", model, fp)?;
            }
        }
        writeln!(out, "\n--- Write Targets ({}) ---", write_targets.len())?;
        for wt in &write_targets {
            let fp = wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
            let model = wt.get("model").and_then(|v| v.as_str()).unwrap_or("?");
            if fp.starts_with(model) {
                writeln!(out, "  {}", fp)?;
            } else {
                writeln!(out, "  {}.{}", model, fp)?;
            }
        }
        writeln!(out, "\n--- Action Flows ({}) ---", action_flows.len())?;
        for flow in &action_flows {
            let aid = flow
                .get("action_id")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let atype = flow
                .get("action_type")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let cid = flow
                .get("component_id")
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
        writeln!(out, "\n--- Navigation ({}) ---", navigation.len())?;
        for nav in &navigation {
            let from = nav.get("from").and_then(|v| v.as_str()).unwrap_or("?");
            let to = nav.get("to").and_then(|v| v.as_str()).unwrap_or("?");
            let nav_type = nav.get("type").and_then(|v| v.as_str()).unwrap_or("?");
            writeln!(out, "  {} -> {} ({})", from, to, nav_type)?;
        }
        if !visibility_rules.is_empty() {
            writeln!(
                out,
                "\n--- Visibility Rules ({}) ---",
                visibility_rules.len()
            )?;
            for rule in &visibility_rules {
                let cid = rule
                    .get("component_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let r = rule.get("rule").and_then(|v| v.as_str()).unwrap_or("?");
                writeln!(out, "  {}: {}", cid, r)?;
            }
        }
        if !output.diagnostics.is_empty() {
            writeln!(out, "\n⚠️  Risk Diagnostics:")?;
            for diag in &output.diagnostics {
                writeln!(out, "  [{}] {}", diag.code, diag.message)?;
            }
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

/// 按关键词搜索图节点
///
/// 返回匹配节点列表，按名称相似度排序。
pub fn find_nodes(
    graph: &GraphDB,
    keyword: &str,
    node_type_filter: Option<&str>,
    limit: usize,
) -> crate::output::schema::AiOutput {
    let keyword_lower = keyword.to_lowercase();
    let mut matches = Vec::new();

    for (_, idx) in &graph.node_indices {
        if let Some(node) = graph.graph.node_weight(*idx) {
            let node_type_str = format!("{:?}", node.node_type).to_lowercase();
            if let Some(filter) = node_type_filter {
                if !node_type_str.contains(filter) {
                    continue;
                }
            }
            let score = if node.id.to_lowercase() == keyword_lower
                || node.name.to_lowercase() == keyword_lower
            {
                100.0
            } else if node.id.to_lowercase().contains(&keyword_lower)
                || node.name.to_lowercase().contains(&keyword_lower)
            {
                80.0
            } else if node.path.to_lowercase().contains(&keyword_lower) {
                50.0
            } else {
                0.0
            };
            if score > 0.0 {
                matches.push((node.clone(), score));
            }
        }
    }

    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let matches: Vec<_> = matches
        .into_iter()
        .filter(|(n, _)| seen.insert(n.id.clone()))
        .take(limit)
        .collect();

    let match_count = matches.len();
    let match_details: Vec<serde_json::Value> = matches
        .iter()
        .map(|(n, score)| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "node_type": format!("{:?}", n.node_type),
                "source_file": n.path,
                "match_score": score,
                "match_reason": if *score >= 100.0 {
                    "exact match"
                } else if *score >= 80.0 {
                    "name/id substring match"
                } else {
                    "path match"
                },
            })
        })
        .collect();

    let kind = match node_type_filter {
        Some("page") => crate::output::schema::OutputKind::PageQuery,
        Some("model") => crate::output::schema::OutputKind::ModelQuery,
        Some("component") => crate::output::schema::OutputKind::ComponentQuery,
        _ => crate::output::schema::OutputKind::PageQuery,
    };

    let summary = serde_json::json!({
        "keyword": keyword,
        "match_count": match_count,
        "what_is_it": format!("Found {} nodes matching '{}'", match_count, keyword),
    });

    let mut output = crate::output::schema::AiOutput::new(kind, summary);
    output.query_target = Some(keyword.to_string());
    output.details = Some(serde_json::json!({
        "matches": match_details,
    }));

    for (n, score) in &matches {
        output.evidence.push(
            crate::output::schema::Evidence::new(
                format!("Node {} ({}) matches with score {}", n.id, n.name, score),
                "Keyword search against graph node index",
            )
            .with_confidence(if *score >= 100.0 {
                crate::output::schema::Confidence::High
            } else if *score >= 80.0 {
                crate::output::schema::Confidence::Medium
            } else {
                crate::output::schema::Confidence::Low
            })
            .with_node_id(&n.id)
            .with_source_file(&n.path),
        );
    }

    if match_count > 0 {
        let top = &matches[0].0;
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--explain {} for semantic summary",
                &top.id,
            ));
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--context {} --depth 2 for surrounding context",
                &top.id,
            ));
    } else {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Info,
            code: "NO_MATCHES_FOUND".to_string(),
            message: format!("No nodes found matching keyword '{}'", keyword),
            location: crate::output::schema::Location::default(),
            suggestion: Some("Try a broader keyword or verify spelling".to_string()),
        });
    }

    output.validate()
}

/// 在页面作用域内解析局部模型 ID
///
/// 从页面的数据源、组件绑定、action reads/writes、DataFlow 输入输出中搜索与 local_model_id 匹配的模型。
pub fn resolve_model_in_page(
    graph: &GraphDB,
    page_id: &str,
    local_model_id: &str,
) -> crate::output::schema::AiOutput {
    let page_node = match graph.get_node(page_id) {
        Some(n) => n,
        None => {
            let mut out = crate::output::schema::AiOutput::new(
                crate::output::schema::OutputKind::ModelQuery,
                serde_json::json!({
                    "page_id": page_id,
                    "local_model_id": local_model_id,
                    "resolved_count": 0,
                    "what_is_it": format!("Page {} not found", page_id),
                }),
            );
            out.diagnostics.push(crate::output::schema::Diagnostic {
                severity: crate::output::schema::DiagnosticSeverity::Error,
                code: "TARGET_NOT_FOUND".to_string(),
                message: format!("Page '{}' not found in graph", page_id),
                location: crate::output::schema::Location::default(),
                suggestion: Some("Verify page ID or use --find-page to search".to_string()),
            });
            out.next_queries
                .push(crate::output::schema::format_next_query(
                    "--find-page {} to search for similar pages",
                    page_id,
                ));
            return out.validate();
        }
    };

    let mut candidates: Vec<(crate::graph::Node, f64, String)> = Vec::new();
    let local_lower = local_model_id.to_lowercase();

    // 从页面出边收集关联模型
    if let Some((outgoing, _)) = graph.get_node_edges(page_id) {
        for (target, edge) in &outgoing {
            let score = if target.id.to_lowercase() == local_lower {
                100.0
            } else if target.id.to_lowercase().contains(&local_lower)
                || target.name.to_lowercase().contains(&local_lower)
            {
                80.0
            } else {
                0.0
            };
            if score > 0.0 {
                candidates.push(((*target).clone(), score, format!("{:?}", edge.edge_type)));
            }
        }
    }

    // 全局搜索裸名匹配
    let bare = local_model_id
        .strip_prefix("model:")
        .unwrap_or(local_model_id);
    for (_, idx) in &graph.node_indices {
        if let Some(node) = graph.graph.node_weight(*idx) {
            if node.id.starts_with("model:") {
                let node_bare = node.id.strip_prefix("model:").unwrap_or(&node.id);
                if node_bare.eq_ignore_ascii_case(bare) {
                    candidates.push((node.clone(), 60.0, "global model match".to_string()));
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|(n, _, _)| seen.insert(n.id.clone()))
        .collect();

    let resolved_count = candidates.len();
    let ambiguous = resolved_count > 1;

    let candidate_details: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(n, score, reason)| {
            serde_json::json!({
                "model_id": n.id,
                "model_name": n.name,
                "confidence": score,
                "match_reason": reason,
            })
        })
        .collect();

    let summary = serde_json::json!({
        "page_id": page_id,
        "local_model_id": local_model_id,
        "resolved_count": resolved_count,
        "ambiguous": ambiguous,
        "what_is_it": if resolved_count == 0 {
            format!("No model resolved for '{}' in page {}", local_model_id, page_id)
        } else if ambiguous {
            format!("Multiple models match '{}' in page {}", local_model_id, page_id)
        } else {
            format!("Model {} resolved in page {}", candidates[0].0.id, page_id)
        },
    });

    let mut output = crate::output::schema::AiOutput::new(
        crate::output::schema::OutputKind::ModelQuery,
        summary,
    );
    output.query_target = Some(format!("{}|{}", page_id, local_model_id));
    output.details = Some(serde_json::json!({
        "candidates": candidate_details,
        "page_source_file": page_node.path,
    }));

    for (n, score, reason) in &candidates {
        output.evidence.push(
            crate::output::schema::Evidence::new(
                format!("Model {} matched via {} (score {})", n.id, reason, score),
                "Page-scoped model resolution",
            )
            .with_confidence(if *score >= 80.0 {
                crate::output::schema::Confidence::High
            } else {
                crate::output::schema::Confidence::Medium
            })
            .with_node_id(&n.id)
            .with_source_file(&page_node.path),
        );
    }

    if resolved_count == 0 {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Warning,
            code: "MODEL_UNRESOLVED".to_string(),
            message: format!(
                "No model matching '{}' found in page {}",
                local_model_id, page_id
            ),
            location: crate::output::schema::Location {
                source_file: Some(page_node.path),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Use --find-model to search globally".to_string()),
        });
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--find-model {} to search globally",
                local_model_id,
            ));
    } else if ambiguous {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Info,
            code: "AMBIGUOUS_RESOLUTION".to_string(),
            message: format!(
                "Multiple models match '{}'; candidate count: {}",
                local_model_id, resolved_count
            ),
            location: crate::output::schema::Location {
                source_file: Some(page_node.path),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Use --explain <MODEL_ID> to verify specific model".to_string()),
        });
        for (n, _, _) in &candidates {
            output
                .next_queries
                .push(crate::output::schema::format_next_query(
                    "--explain {} for semantic summary",
                    &n.id,
                ));
        }
    } else {
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--explain {} for semantic summary",
                &candidates[0].0.id,
            ));
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--query-model {} for model dependencies",
                &candidates[0].0.id,
            ));
    }

    output.validate()
}
