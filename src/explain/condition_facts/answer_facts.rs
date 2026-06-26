use crate::answer_contract::{AnswerFactKind, TraversalIntent, answer_fact_enabled};
use crate::graph_store::GraphReadStore;

fn answer_path_step(
    step: usize,
    node_id: impl Into<String>,
    node_type: impl Into<String>,
    edge_type: impl Into<String>,
    direction: impl Into<String>,
    field_path: Option<&str>,
    source_file: Option<&str>,
    json_path: Option<&str>,
    why_included: impl Into<String>,
) -> serde_json::Value {
    serde_json::json!({
        "step": step,
        "node_id": node_id.into(),
        "node_type": node_type.into(),
        "edge_type": edge_type.into(),
        "direction": direction.into(),
        "field_path": field_path,
        "source_file": source_file,
        "json_path": json_path,
        "why_included": why_included.into(),
    })
}

fn evidence_ref_from_condition(cond: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "node_id": cond.get("condition_id").and_then(|v| v.as_str()),
        "source_file": cond.get("source_file").and_then(|v| v.as_str()),
        "json_path": cond.get("json_path").and_then(|v| v.as_str()),
    })
}

fn compact_condition_fact(cond: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "condition_id": cond.get("condition_id").and_then(|v| v.as_str()),
        "condition_type": cond.get("condition_type").and_then(|v| v.as_str()),
        "condition_scope": cond.get("condition_scope").and_then(|v| v.as_str()),
        "raw_expr": cond.get("raw_expr").and_then(|v| v.as_str()),
        "owner_node_id": cond.get("owner_node_id").and_then(|v| v.as_str()),
        "inherited_from": cond.get("inherited_from").and_then(|v| v.as_str()),
        "expanded_from_condition": cond.get("expanded_from_condition").and_then(|v| v.as_str()),
        "source_file": cond.get("source_file").and_then(|v| v.as_str()),
        "json_path": cond.get("json_path").and_then(|v| v.as_str()),
    })
}

fn compact_condition_fact_for_budget(cond: &serde_json::Value, budget: &str) -> serde_json::Value {
    if budget == "compact" {
        return serde_json::json!({
            "condition_id": cond.get("condition_id").and_then(|v| v.as_str()),
            "condition_scope": cond.get("condition_scope").and_then(|v| v.as_str()),
            "raw_expr": cond.get("raw_expr").and_then(|v| v.as_str()),
            "source_file": cond.get("source_file").and_then(|v| v.as_str()),
            "json_path": cond.get("json_path").and_then(|v| v.as_str()),
        });
    }
    compact_condition_fact(cond)
}

fn build_display_facts(
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
    budget: &str,
) -> serde_json::Value {
    let direct_conditions: Vec<_> = blocking_conditions
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("direct"))
        .map(|c| compact_condition_fact_for_budget(c, budget))
        .collect();
    let inherited_conditions: Vec<_> = blocking_conditions
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited"))
        .map(|c| compact_condition_fact_for_budget(c, budget))
        .collect();
    let expanded_data_gates: Vec<_> = data_empty_gates
        .iter()
        .filter(|c| {
            c.get("condition_scope").and_then(|v| v.as_str())
                == Some("expanded_from_total_row_count")
        })
        .map(|c| compact_condition_fact_for_budget(c, budget))
        .collect();

    let mut evidence_refs: Vec<serde_json::Value> = blocking_conditions
        .iter()
        .chain(data_empty_gates.iter())
        .map(evidence_ref_from_condition)
        .collect();
    evidence_refs.truncate(8);

    let mut paths = Vec::new();
    for cond in blocking_conditions.iter().take(3) {
        if budget == "compact" {
            paths.push(serde_json::json!({
                "intent": "display",
                "result": cond.get("raw_expr").and_then(|v| v.as_str()),
                "confidence": "high",
                "stop_condition_hit": "display_condition_found",
                "evidence_refs": [evidence_ref_from_condition(cond)],
            }));
        } else {
            paths.push(serde_json::json!({
                "intent": "display",
                "result": cond.get("raw_expr").and_then(|v| v.as_str()),
                "confidence": "high",
                "why_complete": "display gate condition found",
                "stop_condition_hit": "display_condition_found",
                "evidence_refs": [evidence_ref_from_condition(cond)],
                "steps": [
                    answer_path_step(
                        1,
                        cond.get("condition_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "Condition",
                        "DependsOn",
                        "incoming",
                        None,
                        cond.get("source_file").and_then(|v| v.as_str()),
                        cond.get("json_path").and_then(|v| v.as_str()),
                        "condition controls target display or inherited container display",
                    )
                ]
            }));
        }
    }

    serde_json::json!({
        "result": if !direct_conditions.is_empty() || !inherited_conditions.is_empty() {
            "display_conditions_found"
        } else {
            "no_display_condition_found"
        },
        "confidence": if !direct_conditions.is_empty() || !inherited_conditions.is_empty() {
            "high"
        } else {
            "medium"
        },
        "target": target_node.id,
        "has_direct_condition": !direct_conditions.is_empty(),
        "direct_conditions": direct_conditions,
        "inherited_conditions": inherited_conditions,
        "expanded_data_gates": expanded_data_gates,
        "evidence_refs": evidence_refs,
        "paths": paths,
        "missing_evidence": if blocking_conditions.is_empty() {
            serde_json::json!(["no direct or inherited display gate found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_value_source_facts(value_source_context: &Option<serde_json::Value>) -> serde_json::Value {
    let Some(ctx) = value_source_context.as_ref() else {
        return serde_json::json!({
            "result": null,
            "confidence": "low",
            "paths": [],
            "evidence_refs": [],
            "missing_evidence": ["value_source_context not available"],
        });
    };

    let dataflow_origin = ctx.get("dataflow_field_origin");
    let dataflow_table = ctx
        .get("table_source_path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let dataflow_output_field = dataflow_origin
        .and_then(|origin| origin.get("dataflow_output_field").and_then(|v| v.as_str()))
        .unwrap_or("");

    let physical_source_fields: Vec<String> = dataflow_origin
        .and_then(|origin| {
            origin
                .get("physical_source_fields")
                .and_then(|v| v.as_array())
        })
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let via = dataflow_origin
        .and_then(|origin| origin.get("via").and_then(|v| v.as_str()))
        .unwrap_or("");
    let original_node =
        dataflow_origin.and_then(|origin| origin.get("original_node").and_then(|v| v.as_str()));
    let original_field =
        dataflow_origin.and_then(|origin| origin.get("original_field").and_then(|v| v.as_str()));

    let candidate_inputs: Vec<String> = ctx
        .get("dataflow_inputs")
        .and_then(|inputs| inputs.as_array())
        .map(|inputs| {
            inputs
                .iter()
                .filter_map(|item| {
                    item.get("node_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| item.get("name").and_then(|v| v.as_str()))
                        .or_else(|| item.get("source_file").and_then(|v| v.as_str()))
                        .map(ToString::to_string)
                })
                .collect()
        })
        .unwrap_or_default();

    let origin_path = if !physical_source_fields.is_empty() {
        Some(physical_source_fields[0].as_str())
    } else {
        None
    };
    let table_source_path = ctx.get("table_source_path").and_then(|v| v.as_str());
    let result = origin_path.or(table_source_path);
    let raw_expr = ctx.get("raw_expr").and_then(|v| v.as_str());
    let bare_symbol = ctx.get("bare_symbol").and_then(|v| v.as_str());
    let field_path = ctx.get("field_path").and_then(|v| v.as_str());
    let context_component = ctx
        .get("nearest_data_context")
        .and_then(|v| v.get("component_id"))
        .and_then(|v| v.as_str());
    let data_set = ctx
        .get("nearest_data_context")
        .and_then(|v| v.get("dataSet"))
        .and_then(|v| v.as_str());
    let graph_target = ctx
        .get("graph_edge")
        .and_then(|v| v.get("target_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let graph_edge_type = ctx
        .get("graph_edge")
        .and_then(|v| v.get("edge_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("Reads");

    let mut steps = Vec::new();
    steps.push(answer_path_step(
        1,
        ctx.get("source_component")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "Component",
        graph_edge_type,
        "outgoing",
        field_path,
        None,
        ctx.get("nearest_data_context")
            .and_then(|v| v.get("json_path"))
            .and_then(|v| v.as_str()),
        "target value expression reads a field",
    ));
    if let Some(component) = context_component {
        steps.push(answer_path_step(
            2,
            component,
            "Component",
            "inherited_container_data_context",
            "ancestor",
            field_path,
            None,
            ctx.get("nearest_data_context")
                .and_then(|v| v.get("json_path"))
                .and_then(|v| v.as_str()),
            "bare field resolved through nearest data context container",
        ));
    }
    if !graph_target.is_empty() {
        steps.push(answer_path_step(
            3,
            graph_target,
            "Field",
            graph_edge_type,
            "outgoing",
            field_path,
            table_source_path,
            None,
            "resolved dataSet field",
        ));
    }
    if let Some(origin) = origin_path {
        steps.push(answer_path_step(
            4,
            origin,
            "Model",
            "dataflow_field_origin",
            "upstream",
            field_path,
            Some(origin),
            None,
            "DataFlow field origin matched original input table",
        ));
    }

    serde_json::json!({
        "result": result,
        "confidence": if origin_path.is_some() { "high" } else if table_source_path.is_some() { "medium" } else { "low" },
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "dataflow_table": if dataflow_table.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_table)
        },
        "dataflow_output_field": if dataflow_output_field.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_output_field.to_string())
        },
        "physical_source_fields": physical_source_fields,
        "candidate_inputs": candidate_inputs,
        "via": via,
        "original_node": original_node,
        "original_field": original_field,
        "nearest_data_context": context_component,
        "data_set": data_set,
        "field_path": field_path,
        "table_source_path": table_source_path,
        "proven_physical_input": origin_path,
        "paths": [{
            "intent": "value-source",
            "result": result,
            "confidence": if origin_path.is_some() { "high" } else if table_source_path.is_some() { "medium" } else { "low" },
            "why_complete": if origin_path.is_some() {
                "field-level DataFlow origin found"
            } else {
                "stopped at page source table path"
            },
            "stop_condition_hit": if origin_path.is_some() {
                "proven_physical_input_found"
            } else {
                "table_source_path_found"
            },
            "evidence_refs": [{
                "node_id": graph_target,
                "field_path": field_path,
                "raw_expr": raw_expr,
            }],
            "steps": steps,
        }],
        "evidence_refs": [{
            "node_id": graph_target,
            "field_path": field_path,
            "raw_expr": raw_expr,
        }],
        "missing_evidence": if result.is_none() {
            serde_json::json!(["no table source path or DataFlow field origin found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_writer_facts(primary_path: &[serde_json::Value]) -> serde_json::Value {
    let writer_paths: Vec<_> = primary_path
        .iter()
        .filter(|p| {
            let path_text = p.to_string();
            path_text.contains("ActionWrites")
                || path_text.contains("FieldWrite")
                || path_text.contains("Writes")
        })
        .take(5)
        .cloned()
        .collect();

    let paths: Vec<_> = writer_paths
        .iter()
        .enumerate()
        .map(|(idx, path)| {
            let path_id = path.get("path_id").and_then(|v| v.as_str()).unwrap_or("");
            let steps = path
                .get("segments")
                .and_then(|v| v.as_array())
                .map(|segments| {
                    segments
                        .iter()
                        .enumerate()
                        .map(|(step_idx, segment)| {
                            let to_node = segment.get("to").unwrap_or(&serde_json::Value::Null);
                            let edge = segment.get("edge").unwrap_or(&serde_json::Value::Null);
                            answer_path_step(
                                step_idx + 1,
                                to_node
                                    .get("node_id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(path_id),
                                to_node
                                    .get("node_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Node"),
                                edge.get("edge_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Unknown"),
                                "causal",
                                edge.get("field_path").and_then(|v| v.as_str()),
                                to_node.get("path").and_then(|v| v.as_str()),
                                edge.get("json_path").and_then(|v| v.as_str()),
                                "selected primary path segment contributes to writer lineage",
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .filter(|steps| !steps.is_empty())
                .unwrap_or_else(|| {
                    vec![answer_path_step(
                        idx + 1,
                        path_id,
                        "Path",
                        "Writes/ActionWrites/FieldWrite",
                        "incoming",
                        None,
                        None,
                        None,
                        "primary path contains a writer edge",
                    )]
                });
            serde_json::json!({
                "intent": "writer",
                "result": path_id,
                "confidence": "high",
                "why_complete": "writer edge found in selected primary path",
                "stop_condition_hit": "writer_path_found",
                "evidence_refs": [{
                    "path_id": path_id,
                }],
                "steps": steps,
            })
        })
        .collect();

    serde_json::json!({
        "result": if writer_paths.is_empty() { serde_json::Value::Null } else { serde_json::json!("writer_paths_found") },
        "confidence": if writer_paths.is_empty() { "low" } else { "high" },
        "paths": paths,
        "evidence_refs": writer_paths.iter().map(|p| serde_json::json!({
            "path_id": p.get("path_id").and_then(|v| v.as_str()),
        })).collect::<Vec<_>>(),
        "missing_evidence": if writer_paths.is_empty() {
            serde_json::json!(["no writer path selected"])
        } else {
            serde_json::json!([])
        },
    })
}

fn build_availability_facts(
    data_empty_gates: &[serde_json::Value],
    dataflow_table: &str,
    dataflow_input_paths: &[String],
    dataflow_meta: Option<&crate::query::DataFlowMeta>,
) -> serde_json::Value {
    let gates: Vec<_> = data_empty_gates
        .iter()
        .map(compact_condition_fact)
        .collect();

    let source_filters: Vec<serde_json::Value> = data_empty_gates
        .iter()
        .filter(|g| g.get("role").and_then(|v| v.as_str()) == Some("source_filter"))
        .map(|g| serde_json::json!({
            "node_alias": g.get("node_alias").and_then(|v| v.as_str()),
            "raw_expr": g.get("raw_expr").and_then(|v| v.as_str()),
            "left": g.get("left").and_then(|v| v.as_str()),
            "operator": g.get("operator").and_then(|v| v.as_str()),
            "right": g.get("right").and_then(|v| v.as_str()),
            "referenced_fields": g.get("referenced_fields").cloned().unwrap_or(serde_json::Value::Null),
            "referenced_vars": g.get("referenced_vars").cloned().unwrap_or(serde_json::Value::Null),
        }))
        .collect();

    let output_filters: Vec<serde_json::Value> = data_empty_gates
        .iter()
        .filter(|g| g.get("role").and_then(|v| v.as_str()) == Some("output_filter"))
        .map(|g| serde_json::json!({
            "node_alias": g.get("node_alias").and_then(|v| v.as_str()),
            "raw_expr": g.get("raw_expr").and_then(|v| v.as_str()),
            "left": g.get("left").and_then(|v| v.as_str()),
            "operator": g.get("operator").and_then(|v| v.as_str()),
            "right": g.get("right").and_then(|v| v.as_str()),
            "referenced_fields": g.get("referenced_fields").cloned().unwrap_or(serde_json::Value::Null),
            "referenced_vars": g.get("referenced_vars").cloned().unwrap_or(serde_json::Value::Null),
        }))
        .collect();

    let mut referenced_vars: Vec<String> = Vec::new();
    let mut seen_vars: std::collections::HashSet<String> = std::collections::HashSet::new();
    for g in data_empty_gates
        .iter()
        .filter(|g| g.get("condition_type").and_then(|v| v.as_str()) == Some("DataFlowFilter"))
    {
        if let Some(arr) = g.get("referenced_vars").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    if seen_vars.insert(s.to_string()) {
                        referenced_vars.push(s.to_string());
                    }
                }
            }
        }
    }

    let has_dataflow_filters = data_empty_gates
        .iter()
        .any(|g| g.get("condition_type").and_then(|v| v.as_str()) == Some("DataFlowFilter"));

    let mut physical_inputs: Vec<String> = if !dataflow_input_paths.is_empty() {
        dataflow_input_paths.to_vec()
    } else {
        dataflow_meta
            .map(|dfm| dfm.get_model_table_paths())
            .unwrap_or_default()
    };
    physical_inputs.sort();
    physical_inputs.dedup();

    let mut join_rules: Vec<serde_json::Value> = Vec::new();
    let mut union_rules: Vec<serde_json::Value> = Vec::new();
    if let Some(dfm) = dataflow_meta {
        for (node_id, conditions) in &dfm.node_join_conditions {
            let alias = dfm
                .get_alias(node_id)
                .map(|s| s.as_str())
                .unwrap_or(node_id.as_str());
            for jc in conditions {
                let row_semantic = match jc.join_type.as_str() {
                    "LeftJoin" | "Left Outer Join" => "left_rows_preserved_right_fields_nullable",
                    "RightJoin" | "Right Outer Join" => "right_rows_preserved_left_fields_nullable",
                    "InnerJoin" | "Inner Join" => "both_tables_must_match",
                    "FullJoin" | "Full Outer Join" => "all_rows_preserved_nullable",
                    _ => "join_rows_filtered_by_condition",
                };
                let clauses_json: Vec<serde_json::Value> = jc
                    .clauses
                    .iter()
                    .map(|c| {
                        serde_json::json!({
                            "left": c.left_exp,
                            "operator": c.operator,
                            "right": c.right_exp,
                        })
                    })
                    .collect();
                join_rules.push(serde_json::json!({
                    "node_alias": alias,
                    "join_type": &jc.join_type,
                    "left_table": &jc.left_table,
                    "right_table": &jc.right_table,
                    "row_semantic": row_semantic,
                    "clauses": clauses_json,
                }));
            }
        }
        for (node_id, entries) in &dfm.node_union_maps {
            let alias = dfm
                .get_alias(node_id)
                .map(|s| s.as_str())
                .unwrap_or(node_id.as_str());
            let field_mappings: Vec<serde_json::Value> = entries
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "fields": e.values,
                        "visible": e.visible,
                    })
                })
                .collect();
            union_rules.push(serde_json::json!({
                "node_alias": alias,
                "row_semantic": "any_branch_can_output",
                "field_mappings": field_mappings,
            }));
        }
    }

    serde_json::json!({
        "result": if gates.is_empty() { "no_data_gate_found" } else { "data_gates_found" },
        "confidence": if gates.is_empty() { "medium" } else { "high" },
        "gates": gates,
        "dataflow_table": if dataflow_table.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(dataflow_table.to_string())
        },
        "physical_inputs": physical_inputs,
        "dataflow_availability": if has_dataflow_filters { "dataflow_filters_present" } else { "no_dataflow_filters" },
        "source_filters": source_filters,
        "output_filters": output_filters,
        "join_rules": join_rules,
        "union_rules": union_rules,
        "referenced_vars": referenced_vars,
        "paths": data_empty_gates.iter().take(3).map(|gate| serde_json::json!({
            "intent": "availability",
            "result": gate.get("raw_expr").and_then(|v| v.as_str()),
            "confidence": "high",
            "why_complete": "data availability gate found",
            "stop_condition_hit": "filter_or_total_row_count_gate_found",
            "evidence_refs": [evidence_ref_from_condition(gate)],
            "steps": [
                answer_path_step(
                    1,
                    gate.get("condition_id").and_then(|v| v.as_str()).unwrap_or(""),
                    "Condition",
                    "DependsOn",
                    "incoming",
                    None,
                    gate.get("source_file").and_then(|v| v.as_str()),
                    gate.get("json_path").and_then(|v| v.as_str()),
                    "filter or totalRowCount condition controls data availability",
                )
            ]
        })).collect::<Vec<_>>(),
        "evidence_refs": data_empty_gates.iter().map(evidence_ref_from_condition).collect::<Vec<_>>(),
        "missing_evidence": if data_empty_gates.is_empty() {
            serde_json::json!(["no data availability gate found"])
        } else {
            serde_json::json!([])
        },
    })
}

fn graph_edge_ref(node: &crate::graph::Node, edge: &crate::graph::Edge) -> serde_json::Value {
    serde_json::json!({
        "node_id": node.id,
        "node_type": format!("{:?}", node.node_type),
        "name": node.name,
        "source_file": edge.meta.as_ref().and_then(|m| m.get("source_file")).and_then(|v| v.as_str()).or(Some(node.path.as_str())),
        "edge_type": format!("{:?}", edge.edge_type),
        "field_path": edge.field_path.clone(),
        "json_path": edge.meta.as_ref().and_then(|m| m.get("json_path")).and_then(|v| v.as_str()),
    })
}

fn build_model_io_facts(
    graph: &dyn GraphReadStore,
    target_node: &crate::graph::Node,
) -> serde_json::Value {
    if target_node.node_type != crate::graph::NodeType::Model {
        return serde_json::json!({
            "result": null,
            "confidence": "none",
            "reads": [],
            "writes": [],
            "paths": [],
            "evidence_refs": [],
            "missing_evidence": ["model_io_facts only applies to model targets"],
        });
    }

    let mut reads = Vec::new();
    let mut writes = Vec::new();
    if let Some(neighbors) = graph.get_node_edges(&target_node.id).ok().flatten() {
        for edge_view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
            let node = &edge_view.node;
            let edge = &edge_view.edge;
            match edge.edge_type {
                crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                    reads.push(graph_edge_ref(node, edge));
                }
                crate::graph::EdgeType::Writes
                | crate::graph::EdgeType::ActionWrites
                | crate::graph::EdgeType::FieldWrite => {
                    writes.push(graph_edge_ref(node, edge));
                }
                _ => {}
            }
        }
    }
    reads.truncate(5);
    writes.truncate(5);
    let has_io = !reads.is_empty() || !writes.is_empty();

    serde_json::json!({
        "result": if has_io {
            "model_io_summary_found"
        } else {
            "no_model_io_edges_found"
        },
        "confidence": if has_io { "medium" } else { "low" },
        "reads": reads,
        "writes": writes,
        "paths": [],
        "evidence_refs": [],
        "missing_evidence": if has_io {
            serde_json::json!([])
        } else {
            serde_json::json!(["no read/write graph edges found for model target"])
        },
    })
}

pub(crate) fn build_primary_reason_for_intent(
    intent: TraversalIntent,
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
    primary_path: &[serde_json::Value],
    value_source_context: &Option<serde_json::Value>,
    answer_facts: &serde_json::Value,
) -> String {
    match intent {
        TraversalIntent::Writer => {
            let writer_paths = answer_facts
                .get("writer_facts")
                .and_then(|v| v.get("paths"))
                .and_then(|v| v.as_array())
                .map(|items| items.len())
                .unwrap_or(0);
            if writer_paths > 0 {
                return format!(
                    "目标 {} 的写入/生成链路已找到，共 {} 条 writer 路径",
                    target_node.id, writer_paths
                );
            }
            return format!("目标 {} 未发现明确 writer 链路", target_node.id);
        }
        TraversalIntent::ValueSource => {
            if let Some(result) = answer_facts
                .get("value_source_facts")
                .and_then(|v| v.get("result"))
                .and_then(|v| v.as_str())
            {
                return format!("目标 {} 的值来源已解析到 {}", target_node.id, result);
            }
            return format!("目标 {} 未发现明确值来源", target_node.id);
        }
        TraversalIntent::Display => {
            if !blocking_conditions.is_empty() {
                return format!(
                    "目标 {} 受 {} 个显示/启用条件影响",
                    target_node.id,
                    blocking_conditions.len()
                );
            }
            if !data_empty_gates.is_empty() {
                return format!(
                    "目标 {} 的显示受 {} 个数据门控展开条件影响",
                    target_node.id,
                    data_empty_gates.len()
                );
            }
            return format!("目标 {} 未发现明确显示条件", target_node.id);
        }
        TraversalIntent::Availability => {
            if !data_empty_gates.is_empty() {
                return format!(
                    "目标 {} 受 {} 个数据门控影响（filter/totalRowCount__）",
                    target_node.id,
                    data_empty_gates.len()
                );
            }
            return format!("目标 {} 未发现明确数据可用性门控", target_node.id);
        }
        TraversalIntent::Context | TraversalIntent::Auto => {}
    }

    if target_node.node_type == crate::graph::NodeType::Page {
        if !blocking_conditions.is_empty() {
            format!(
                "页面 {} 有 {} 个阻塞条件（visible/disable/action condition）",
                target_node.name,
                blocking_conditions.len()
            )
        } else if !data_empty_gates.is_empty() {
            format!(
                "页面 {} 有 {} 个数据门控（filter/totalRowCount__）",
                target_node.name,
                data_empty_gates.len()
            )
        } else {
            format!("页面 {} 未发现明确的阻塞条件或数据门控", target_node.name)
        }
    } else if !blocking_conditions.is_empty() {
        format!(
            "目标 {} 受 {} 个阻塞条件影响（visible/disable/action condition）",
            target_node.id,
            blocking_conditions.len()
        )
    } else if !data_empty_gates.is_empty() {
        format!(
            "目标 {} 受 {} 个数据门控影响（filter/totalRowCount__）",
            target_node.id,
            data_empty_gates.len()
        )
    } else if !primary_path.is_empty() {
        format!(
            "目标 {} 的数据链路已找到，共 {} 条主路径",
            target_node.id,
            primary_path.len()
        )
    } else if value_source_context.is_some() {
        format!("目标 {} 的裸字段值来源已解析到最近数据容器", target_node.id)
    } else {
        format!("目标 {} 未发现明确的阻塞条件或数据链路", target_node.id)
    }
}

pub(crate) fn build_traversal_policy(intent: TraversalIntent, budget: &str) -> serde_json::Value {
    let (max_paths, max_steps_per_path) = match budget {
        "compact" => (3, 6),
        "full" => (10, 12),
        _ => (5, 8),
    };
    serde_json::json!({
        "intent": intent.as_str(),
        "max_paths": max_paths,
        "max_steps_per_path": max_steps_per_path,
        "allowed_edge_types": match intent {
            TraversalIntent::Display => serde_json::json!(["DependsOn", "Contains"]),
            TraversalIntent::ValueSource => serde_json::json!(["Reads", "FieldAlias", "DataflowOutput", "DataflowInput"]),
            TraversalIntent::Writer => serde_json::json!(["Reads(target bridge)", "Writes", "ActionWrites", "FieldWrite", "FieldAlias"]),
            TraversalIntent::Availability => serde_json::json!(["DependsOn", "DataflowInput"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["Reads", "Writes", "ActionWrites", "FieldWrite", "DependsOn", "Contains"]),
        },
        "directions": match intent {
            TraversalIntent::Writer => serde_json::json!(["incoming", "reverse_alias"]),
            TraversalIntent::Display => serde_json::json!(["incoming", "ancestor"]),
            TraversalIntent::ValueSource => serde_json::json!(["outgoing", "upstream"]),
            TraversalIntent::Availability => serde_json::json!(["incoming", "filter_refs"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["incoming", "outgoing"]),
        },
        "stop_conditions": match intent {
            TraversalIntent::Display => serde_json::json!(["display_condition_found", "expanded_model_filter_found"]),
            TraversalIntent::ValueSource => serde_json::json!(["proven_physical_input_found", "table_source_path_found", "field_origin_unprovable"]),
            TraversalIntent::Writer => serde_json::json!(["writer_path_found"]),
            TraversalIntent::Availability => serde_json::json!(["filter_or_total_row_count_gate_found"]),
            TraversalIntent::Context | TraversalIntent::Auto => serde_json::json!(["path_budget_exhausted"]),
        },
        "rank_rules": ["field-level paths before model-level paths", "proven paths before candidates", "paths with evidence before inferred paths"],
    })
}

pub(crate) fn build_answer_facts(
    graph: &dyn GraphReadStore,
    intent: TraversalIntent,
    target_node: &crate::graph::Node,
    blocking_conditions: &[serde_json::Value],
    data_empty_gates: &[serde_json::Value],
    value_source_context: &Option<serde_json::Value>,
    primary_path: &[serde_json::Value],
    budget: &str,
    dataflow_meta: Option<&crate::query::DataFlowMeta>,
    dataflow_model_id: Option<&str>,
) -> serde_json::Value {
    let traversal_policy = build_traversal_policy(intent, budget);
    let mut facts = serde_json::Map::new();
    facts.insert("intent".to_string(), serde_json::json!(intent.as_str()));

    if answer_fact_enabled(intent, target_node, AnswerFactKind::Display) {
        facts.insert(
            "display_facts".to_string(),
            build_display_facts(target_node, blocking_conditions, data_empty_gates, budget),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::ValueSource) {
        facts.insert(
            "value_source_facts".to_string(),
            build_value_source_facts(value_source_context),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Writer) {
        facts.insert("writer_facts".to_string(), build_writer_facts(primary_path));
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Availability) {
        facts.insert(
            "availability_facts".to_string(),
            build_availability_facts(
                data_empty_gates,
                &target_node.path,
                &collect_dataflow_input_paths(graph, dataflow_model_id),
                dataflow_meta,
            ),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::Context) {
        facts.insert(
            "context_facts".to_string(),
            serde_json::json!({
                "result": "use_context_command_for_broad_neighbors",
                "confidence": "medium",
                "paths": [],
                "evidence_refs": [],
                "missing_evidence": ["use --context for broad surrounding context"],
            }),
        );
    }
    if answer_fact_enabled(intent, target_node, AnswerFactKind::ModelIo) {
        facts.insert(
            "model_io_facts".to_string(),
            build_model_io_facts(graph, target_node),
        );
    }

    facts.insert("traversal_policy".to_string(), traversal_policy);
    serde_json::Value::Object(facts)
}

fn collect_dataflow_input_paths(
    graph: &dyn GraphReadStore,
    target_model_id: Option<&str>,
) -> Vec<String> {
    let target_model_id = match target_model_id {
        Some(id) => id,
        None => return Vec::new(),
    };

    let mut paths = Vec::new();
    if let Some(neighbors) = graph.get_node_edges(target_model_id).ok().flatten() {
        for edge_view in neighbors.outgoing {
            let edge = edge_view.edge;
            if !matches!(edge.edge_type, crate::graph::EdgeType::DataflowInput) {
                continue;
            }
            if let Some(path) = edge.field_path.as_deref() {
                if !path.is_empty() && !paths.contains(&path.to_string()) {
                    paths.push(path.to_string());
                }
                continue;
            }
            if let Some(path) = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_file"))
                .and_then(|v| v.as_str())
            {
                if !path.is_empty() && !paths.contains(&path.to_string()) {
                    paths.push(path.to_string());
                }
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// 构建上下文数组摘要，用于 compact 模式隐藏大体量旁路明细
pub(crate) fn build_context_summary(
    items: &[serde_json::Value],
    emitted_count: usize,
    hidden: bool,
    note: &str,
) -> serde_json::Value {
    let mut by_scope: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut by_source_file: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    for item in items {
        let scope = item
            .get("condition_scope")
            .or_else(|| item.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        *by_scope.entry(scope.to_string()).or_insert(0) += 1;

        if let Some(source_file) = item.get("source_file").and_then(|v| v.as_str()) {
            *by_source_file.entry(source_file.to_string()).or_insert(0) += 1;
        }
    }

    let source_file_distinct_count = by_source_file.len();
    let by_source_file = if hidden {
        std::collections::BTreeMap::new()
    } else {
        by_source_file
    };

    serde_json::json!({
        "total_count": items.len(),
        "emitted_count": emitted_count,
        "hidden": hidden,
        "note": note,
        "by_scope": by_scope,
        "by_source_file": by_source_file,
        "source_file_distinct_count": source_file_distinct_count,
    })
}
