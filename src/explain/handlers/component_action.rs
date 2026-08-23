use crate::explain::evidence::push_relation_evidence;
use crate::explain::importance::{classify_importance, component_type_from_meta};
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
use anyhow::Result;
use serde_json::Value;
use std::io::{self, Write};

use super::super::{find_parent_page, make_ref};

pub(in crate::explain) fn explain_component_graph(
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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
                if let Some(action_neighbors) = graph.get_node_edges(&target.id).ok().flatten() {
                    for edge_view in &action_neighbors.outgoing {
                        let t = &edge_view.node;
                        let e = &edge_view.edge;
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
pub(in crate::explain) fn explain_action_graph(
    graph: &dyn GraphReadStore,
    node: &crate::graph::Node,
    outgoing: Vec<(crate::graph::Node, crate::graph::Edge)>,
    incoming: Vec<(crate::graph::Node, crate::graph::Edge)>,
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
