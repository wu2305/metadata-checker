use super::{find_candidates, find_parent_page};
use crate::graph::{
    find_consumed_by_dataflows, find_dataflow_inputs, find_dataflow_outputs, find_produced_by,
    find_readers, find_writers,
};
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use anyhow::Result;
use std::io::{self, Write};

/// 构建 query_model JSON 输出，不直接打印。
pub fn build_query_model_output(
    graph: &dyn GraphReadStore,
    model_id: &str,
    budget: &str,
) -> Result<serde_json::Value> {
    let is_compact = budget == "compact";
    if graph.get_node(model_id)?.is_none() {
        let candidates = find_candidates(graph, model_id, 5)?;
        let out = crate::output::schema::build_target_not_found_output(
            crate::output::schema::OutputKind::ModelQuery,
            model_id,
            &candidates,
        );
        return Ok(serde_json::to_value(out)?);
    }
    let readers: Vec<_> = find_readers(graph, model_id)?
        .into_iter()
        .map(|(n, e)| {
            let page = find_parent_page(graph, &n.id).unwrap_or(None);
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
    let writers: Vec<_> = find_writers(graph, model_id)?
        .into_iter()
        .map(|(n, e)| {
            let page = find_parent_page(graph, &n.id).unwrap_or(None);
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
    let dataflow_inputs = find_dataflow_inputs(graph, model_id)?
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
    let dataflow_outputs = find_dataflow_outputs(graph, model_id)?
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
    let produced_by = find_produced_by(graph, model_id)?
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
    let consumed_by_dataflows = find_consumed_by_dataflows(graph, model_id)?
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
    let upstream = find_dataflow_inputs(graph, model_id)?
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
    let downstream = find_dataflow_outputs(graph, model_id)?
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

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::ModelQuery, summary);
    output.query_target = Some(model_id.to_string());
    output.details = Some(details);
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
    Ok(serde_json::to_value(output)?)
}

/// 查询某个模型被哪些节点读取/写入。
pub fn query_model(
    graph: &dyn GraphReadStore,
    model_id: &str,
    human: bool,
    budget: &str,
) -> Result<()> {
    if graph.get_node(model_id)?.is_none() {
        let candidates = find_candidates(graph, model_id, 5)?;
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

        let readers = find_readers(graph, model_id)?;
        writeln!(
            out,
            "
--- Read By ({} nodes) ---",
            readers.len()
        )?;
        for (node, edge) in readers {
            let page = find_parent_page(graph, &node.id)?;
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

        let writers = find_writers(graph, model_id)?;
        writeln!(
            out,
            "
--- Written By ({} nodes) ---",
            writers.len()
        )?;
        for (node, edge) in writers {
            let page = find_parent_page(graph, &node.id)?;
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
        let output = build_query_model_output(graph, model_id, budget)?;
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
