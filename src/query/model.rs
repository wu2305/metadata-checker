use super::{find_candidates, find_parent_page};
use crate::graph::{
    Node, find_consumed_by_dataflows, find_dataflow_inputs, find_dataflow_outputs,
    find_produced_by, find_readers, find_writers,
};
use crate::graph_identity::{NodeIdKind, resolve_node_target};
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use anyhow::Result;
use std::io::{self, Write};

/// M59-2 A1 接线：裸旧 target 在页面局部身份启用后的解析结果。
///
/// A1 把页面局部模型/字段编码为 `<kind>:<PAGE>|<local>`，物理表仍是全局
/// `<kind>:<name>`。旧 target（`model:orders`）因此可能落在
/// 「唯一的局部节点」「多个同名节点」或「完全不存在」三种状态上，而精确
/// `get_node` 只能区分「存在 / 不存在」——它会静默命中同名的物理模型，
/// 或在只有局部节点时直接报 missing。approved plan 要求显式解析并报歧义。
pub(crate) enum LegacyModelTarget {
    /// 精确 id 命中（scoped 或物理全局 id），按原样继续。
    Exact(String),
    /// 裸名唯一命中一个页面局部节点：改用该节点的真实 id 继续，并如实回报。
    Resolved(String),
    /// 裸名匹配多个节点：交回全部候选，调用方必须报 AMBIGUOUS_TARGET。
    Ambiguous(Vec<Node>),
    /// 无候选。
    Missing,
}

/// 解析 `model:` / `field:` 旧 target。
///
/// 只在 target 带 `model:` / `field:` 前缀时生效，其余 target 原样返回
/// `Exact`（不改变既有 surface 的行为）。scoped id（含 `|`）走精确查表。
pub(crate) fn resolve_legacy_model_target(
    graph: &dyn GraphReadStore,
    target: &str,
) -> Result<LegacyModelTarget> {
    let kind = if let Some(rest) = target.strip_prefix("model:") {
        if rest.contains('|') {
            return Ok(LegacyModelTarget::Exact(target.to_string()));
        }
        NodeIdKind::Model
    } else if let Some(rest) = target.strip_prefix("field:") {
        if rest.contains('|') {
            return Ok(LegacyModelTarget::Exact(target.to_string()));
        }
        NodeIdKind::Field
    } else {
        return Ok(LegacyModelTarget::Exact(target.to_string()));
    };

    match resolve_node_target(graph, kind, target)? {
        crate::graph_identity::TargetResolution::Unique(node) => {
            if node.id == target {
                Ok(LegacyModelTarget::Exact(target.to_string()))
            } else {
                Ok(LegacyModelTarget::Resolved(node.id))
            }
        }
        crate::graph_identity::TargetResolution::Ambiguous(nodes) => {
            Ok(LegacyModelTarget::Ambiguous(nodes))
        }
        crate::graph_identity::TargetResolution::Missing => Ok(LegacyModelTarget::Missing),
    }
}

/// 构造歧义输出：交回全部候选并要求调用方从 `next_queries` 里挑一条重试。
///
/// `kind` 由调用方给出（ModelQuery / Explain …），`next_query_template` 是
/// 该 surface 的重试命令模板，保证 `next_queries` 可直接执行。
pub(crate) fn build_ambiguous_target_output(
    target: &str,
    nodes: &[Node],
    kind: crate::output::OutputKind,
    next_query_template: &str,
) -> Result<serde_json::Value> {
    let candidates: Vec<serde_json::Value> = nodes
        .iter()
        .map(|node| {
            serde_json::json!({
                "id": node.id,
                "name": node.name,
                "node_type": format!("{:?}", node.node_type),
                "reason": "旧 target 匹配到多个节点：页面局部身份启用后同名局部节点与物理节点并存",
            })
        })
        .collect();
    let next_queries: Vec<String> = nodes
        .iter()
        .map(|node| format!("{} {}", next_query_template, node.id))
        .collect();
    let summary = serde_json::json!({
        "target_id": target,
        "resolved_count": 0,
        "what_is_it": format!("Target '{}' matches {} nodes", target, candidates.len()),
        "candidate_count": candidates.len(),
    });
    let mut out = crate::output::AiOutput::new(kind, summary);
    out.query_target = Some(target.to_string());
    out.details = Some(serde_json::json!({
        "candidate_targets": candidates,
    }));
    out.next_queries = next_queries;
    out.diagnostics.push(crate::diagnostics::envelope_diagnostic(
        "AMBIGUOUS_TARGET",
        candidates.len(),
        crate::output::Location::default(),
        format!(
            "'{target}' 匹配到 {} 个节点（页面局部身份启用后同名局部节点与物理节点并存）。从 next_queries 里挑一条完整写法重试，不要重复这一条命令。",
            candidates.len()
        ),
    ));
    Ok(serde_json::to_value(out.validate())?)
}

/// 构建 query_model JSON 输出，不直接打印。
pub fn build_query_model_output(
    graph: &dyn GraphReadStore,
    model_id: &str,
    budget: &str,
) -> Result<serde_json::Value> {
    let is_compact = budget == "compact";
    // M59-2 A1 接线：旧 target 先显式解析，再交给精确查表。
    // 不解析的话，页面局部身份启用后 `model:orders` 会静默命中同名的物理
    // 模型（事实被吞且不报歧义），或在只有局部节点时直接报 missing。
    let model_id = &match resolve_legacy_model_target(graph, model_id)? {
        LegacyModelTarget::Exact(id) | LegacyModelTarget::Resolved(id) => id,
        LegacyModelTarget::Ambiguous(nodes) => {
            return build_ambiguous_target_output(
                model_id,
                &nodes,
                crate::output::OutputKind::ModelQuery,
                "--query-model",
            );
        }
        LegacyModelTarget::Missing => {
            let candidates = find_candidates(graph, model_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::ModelQuery,
                model_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };
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
    // 模型级关系答不了「这个字段是从哪一路传过来的」——那是 `--explain field:X.y`。
    // 此前的 next_queries 里没有任何指向字段的入口，链式血缘的问题（df_a -> physical_x
    // -> df_b）走到第二步就只能靠模型自己猜出 `field:` 这个前缀存在。已经出现在边上的
    // 字段名是确定的事实，直接拼成可执行命令交回去。
    let model_name = model_id.strip_prefix("model:").unwrap_or(model_id);
    let mut field_names: Vec<String> = Vec::new();
    for entry in readers.iter().chain(writers.iter()) {
        let Some(path) = entry.get("field_path").and_then(|v| v.as_str()) else {
            continue;
        };
        let leaf = path.rsplit('.').next().unwrap_or(path);
        if !leaf.is_empty() && !field_names.contains(&leaf.to_string()) {
            field_names.push(leaf.to_string());
        }
    }
    // 顺序即建议。模型几乎总是照抄第一条，而这次调用刚刚回答完模型级关系——再来一条
    // `--explain model:X` 基本是重复。评测里 `dataflow_chain_trace` 12 次有 10 次栽在这里：
    // 第一步走对了，第二步照抄第一条 next_query 回到模型级，再也走不到字段。
    // 还不知道的是字段级来源，就把它排在最前面。
    output.next_queries = Vec::new();
    for field in field_names.iter().take(3) {
        output.next_queries.push(format_next_query(
            "--explain {} for field-level lineage (chain across DataFlows)",
            &format!("field:{model_name}.{field}"),
        ));
    }
    output.next_queries.extend([
        format_next_query("--query-dataflow {} for internal subgraph", model_id),
        format_next_query("--explain {} for full semantic summary", model_id),
    ]);
    if field_names.is_empty() {
        output.next_queries.push(format!(
            "--explain 'field:{model_name}.<字段名>'（单个字段的上游来源与下游去向，跨 DataFlow 的链式传递要逐字段查）"
        ));
    }

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
            let mut diag = crate::diagnostics::envelope_diagnostic(
                "OUTPUT_TRUNCATED",
                1,
                crate::output::Location {
                    source_file: None,
                    node_id: Some(model_id.to_string()),
                    json_path: None,
                },
                format!(
                    "Compact budget: arrays truncated for: {}",
                    truncated_parts.join(", ")
                ),
            );
            diag.severity = crate::output::DiagnosticSeverity::Info;
            diag.suggestion =
                Some("Use --budget normal or --budget full to see complete arrays".to_string());
            output.diagnostics.push(diag);
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
    // 与 `build_query_model_output` 同一套解析：CLI 的 human 分支不得绕过歧义诊断。
    let resolved = match resolve_legacy_model_target(graph, model_id)? {
        LegacyModelTarget::Exact(id) | LegacyModelTarget::Resolved(id) => id,
        LegacyModelTarget::Ambiguous(nodes) => {
            let out = build_ambiguous_target_output(
                model_id,
                &nodes,
                crate::output::OutputKind::ModelQuery,
                "--query-model",
            )?;
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
        LegacyModelTarget::Missing => {
            let candidates = find_candidates(graph, model_id, 5)?;
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::ModelQuery,
                model_id,
                &candidates,
            );
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
    };
    let model_id = &resolved;
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
