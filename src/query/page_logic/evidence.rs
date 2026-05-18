use crate::output::schema::{format_next_query, format_next_query_multi};

use super::pick_str_field;

/// 向 PageLogic 输出写入 evidence 采样和 next_queries
pub(super) fn populate_page_logic_evidence(
    output: &mut crate::output::AiOutput,
    page_id: &str,
    page_path: &str,
    entrypoints: &[serde_json::Value],
    data_sources: &[serde_json::Value],
    write_targets: &[serde_json::Value],
    navigation: &[serde_json::Value],
    visibility_rules: &[serde_json::Value],
    action_flows: &[serde_json::Value],
    evidence_sample_limit: usize,
) {
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Page {} has {} entrypoints", page_id, entrypoints.len()),
            "Graph traversal: child components with Triggers edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(page_id)
        .with_source_file(page_path)
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
                    .unwrap_or(page_path),
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
            .with_source_file(page_path)
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
                        .unwrap_or(page_path),
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
            .with_source_file(page_path)
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
                        .unwrap_or(page_path),
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
            .with_source_file(page_path)
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
                        .unwrap_or(page_path),
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
            .with_source_file(page_path)
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
                        .unwrap_or(page_path),
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
            .with_source_file(page_path)
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
                        .unwrap_or(page_path),
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
}
