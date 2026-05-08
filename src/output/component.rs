use crate::dependency::DependencyGraph;
use crate::output::schema::format_next_query;
use crate::superpage::{RefType, SuperPageMetadata};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, Write};

pub fn print_component_query_human(
    spg: &SuperPageMetadata,
    graph: &DependencyGraph,
    target_id: &str,
    show_priority: bool,
) -> Result<()> {
    print_component_query_human_to(spg, graph, target_id, show_priority, &mut io::stdout())
}

pub fn print_component_query_human_to(
    spg: &SuperPageMetadata,
    graph: &DependencyGraph,
    target_id: &str,
    show_priority: bool,
    out: &mut dyn Write,
) -> Result<()> {
    let comp = spg
        .components
        .iter()
        .find(|c| c.id == target_id)
        .ok_or_else(|| anyhow::anyhow!("Component '{}' not found", target_id))?;

    writeln!(out, "\n=== Component: {} ===", target_id)?;
    writeln!(
        out,
        "Type: {} | Parent: {}",
        comp.component_type,
        comp.parent_id.as_deref().unwrap_or("(root)")
    )?;

    let comp_exprs: Vec<_> = spg
        .expressions
        .iter()
        .filter(|e| e.component_id == target_id)
        .collect();

    if !comp_exprs.is_empty() {
        writeln!(out, "\n--- Expressions ({}) ---", comp_exprs.len())?;
        for expr in comp_exprs {
            writeln!(out, "[{}] {}", expr.field, expr.raw_expr)?;
            if !expr.refs.is_empty() {
                writeln!(out, "  References:")?;
                for ref_type in &expr.refs {
                    match ref_type {
                        RefType::ComponentValue(id) => {
                            writeln!(out, "    - ComponentValue: {}", id)?
                        }
                        RefType::ComponentProperty(id, prop) => {
                            writeln!(out, "    - ComponentProperty: {}.{}", id, prop)?
                        }
                        RefType::ModelField(model, field) => {
                            writeln!(out, "    - ModelField: {}.{}", model, field)?
                        }
                        RefType::Param(id) => writeln!(out, "    - Param: {}", id)?,
                        RefType::UserProperty(prop) => {
                            writeln!(out, "    - UserProperty: {}", prop)?
                        }
                        RefType::SystemVar(var) => writeln!(out, "    - SystemVar: {}", var)?,
                        RefType::Other(s) => writeln!(out, "    - Other: {}", s)?,
                    };
                }
            }
        }
    }

    let upstream = graph.dependencies.get(target_id);
    if let Some(refs) = upstream {
        let comp_refs: Vec<_> = refs
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    RefType::ComponentValue(_) | RefType::ComponentProperty(_, _)
                )
            })
            .collect();
        if !comp_refs.is_empty() {
            writeln!(out, "\n--- Upstream Dependencies ---")?;
            for ref_type in comp_refs {
                let dep_id = match ref_type {
                    RefType::ComponentValue(id) => id.as_str(),
                    RefType::ComponentProperty(id, _) => id.as_str(),
                    _ => continue,
                };
                if let Some(dep_comp) = spg.components.iter().find(|c| c.id == dep_id) {
                    writeln!(out, "\n> {} ({})", dep_id, dep_comp.component_type)?;
                    let dep_exprs: Vec<_> = spg
                        .expressions
                        .iter()
                        .filter(|e| e.component_id == dep_id)
                        .collect();
                    for expr in dep_exprs {
                        writeln!(out, "  [{}] {}", expr.field, expr.raw_expr)?;
                    }
                }
            }
        }
    }

    let downstream = graph.reverse_deps.get(target_id);
    if let Some(dep_ids) = downstream
        && !dep_ids.is_empty()
    {
        writeln!(out, "\n--- Downstream Dependencies ---")?;
        for dep_id in dep_ids {
            if let Some(dep_comp) = spg.components.iter().find(|c| c.id == *dep_id) {
                writeln!(out, "\n> {} ({})", dep_id, dep_comp.component_type)?;
                let dep_exprs: Vec<_> = spg
                    .expressions
                    .iter()
                    .filter(|e| e.component_id == *dep_id)
                    .collect();
                for expr in dep_exprs {
                    writeln!(out, "  [{}] {}", expr.field, expr.raw_expr)?;
                }
            }
        }
    }

    let value_fields = ["value", "exp", "defaultValue"];
    for field in &value_fields {
        if let Some(trace) = crate::dependency::trace_value_source(spg, graph, target_id, field, 10)
        {
            writeln!(
                out,
                "\n--- Value Source Trace ({}.{}) ---",
                target_id, field
            )?;
            writeln!(out, "Raw       : {}", trace.raw_expr)?;
            writeln!(out, "Expanded  : {}", trace.expanded_expr)?;
            writeln!(out, "Source    : {:?}", trace.source_type)?;
            writeln!(
                out,
                "External  : {}",
                if trace.is_external_input { "Yes" } else { "No" }
            )?;
            if !trace.source_chain.is_empty() {
                writeln!(out, "Chain:")?;
                for (i, node) in trace.source_chain.iter().enumerate() {
                    writeln!(out, "  [{}] {}: {}", i + 1, node.component_id, node.expr)?;
                }
            }
            break;
        }
    }

    if show_priority {
        writeln!(out)?;
        let analyses = crate::priority::analyze_priority(spg);
        if let Some(analysis) = analyses.iter().find(|a| a.component_id == target_id) {
            let icon = match &analysis.priority_result {
                crate::priority::ExpDefaultValuePriority::OnlyDefaultValue => "[D]",
                crate::priority::ExpDefaultValuePriority::OnlyExp => "[E]",
                crate::priority::ExpDefaultValuePriority::ExpDominatesNoCalcCondition => "[E>D]",
                crate::priority::ExpDefaultValuePriority::ExpWithCalcConditionDefaultValueFallback => "[E~D]",
                crate::priority::ExpDefaultValuePriority::AmbiguousConflict => "[!?]",
            };
            writeln!(out, "--- Priority ---")?;
            writeln!(
                out,
                "{} {} ({}) — {:?}",
                icon, analysis.component_id, comp.component_type, analysis.priority_result
            )?;
            if let Some(dv) = &analysis.default_value_expr {
                writeln!(out, "  [defaultValue] {}", dv.raw_expr)?;
            }
            if let Some(exp) = &analysis.calc_exp_expr {
                writeln!(out, "  [exp] {}", exp.raw_expr)?;
            }
            if let Some(cc) = &analysis.calc_condition_expr {
                writeln!(out, "  [calcCondition] {}", cc.raw_expr)?;
            }
        }
    }

    writeln!(out, "\n=== End of Query ===")?;
    out.flush()?;
    Ok(())
}

pub fn print_component_query_json(
    spg: &SuperPageMetadata,
    graph: &DependencyGraph,
    target_id: &str,
    show_priority: bool,
) -> Result<()> {
    print_component_query_json_to(spg, graph, target_id, show_priority, &mut io::stdout())
}

pub fn print_component_query_json_to(
    spg: &SuperPageMetadata,
    graph: &DependencyGraph,
    target_id: &str,
    show_priority: bool,
    out: &mut dyn Write,
) -> Result<()> {
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

    let upstream: Vec<Value> = graph.dependencies.get(target_id)
        .map(|refs| {
            refs.iter()
                .filter_map(|r| match r {
                    RefType::ComponentValue(id) | RefType::ComponentProperty(id, _) => {
                        spg.components.iter().find(|c| c.id == *id).map(|dep_comp| {
                            let dep_exprs: Vec<Value> = spg.expressions.iter()
                                .filter(|e| e.component_id == *id)
                                .map(|e| json!({"field": e.field, "raw_expr": e.raw_expr}))
                                .collect();
                            json!({"id": id, "type": dep_comp.component_type, "expressions": dep_exprs})
                        })
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let downstream: Vec<Value> =
        graph
            .reverse_deps
            .get(target_id)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| {
                        spg.components.iter().find(|c| c.id == *id).map(|dep_comp| {
                        let dep_exprs: Vec<Value> = spg.expressions.iter()
                            .filter(|e| e.component_id == *id)
                            .map(|e| json!({"field": e.field, "raw_expr": e.raw_expr}))
                            .collect();
                        json!({"id": id, "type": dep_comp.component_type, "expressions": dep_exprs})
                    })
                    })
                    .collect()
            })
            .unwrap_or_default();

    let value_trace = {
        let mut trace_val = None;
        for field in &["value", "exp", "defaultValue"] {
            if let Some(trace) =
                crate::dependency::trace_value_source(spg, graph, target_id, field, 10)
            {
                trace_val = Some(json!({
                    "field": trace.field,
                    "raw_expr": trace.raw_expr,
                    "expanded_expr": trace.expanded_expr,
                    "source_type": format!("{:?}", trace.source_type),
                    "is_external_input": trace.is_external_input,
                    "source_chain": trace.source_chain.iter().map(|n| json!({
                        "component_id": n.component_id,
                        "expr": n.expr,
                        "source_type": format!("{:?}", n.source_type),
                    })).collect::<Vec<Value>>(),
                }));
                break;
            }
        }
        trace_val.unwrap_or(Value::Null)
    };

    let priority =
        if show_priority {
            let analyses = crate::priority::analyze_priority(spg);
            analyses.iter().find(|a| a.component_id == target_id).map(|a| json!({
            "result": format!("{:?}", a.priority_result),
            "default_value": a.default_value_expr.as_ref().map(|e| e.raw_expr.clone()),
            "exp": a.calc_exp_expr.as_ref().map(|e| e.raw_expr.clone()),
            "calc_condition": a.calc_condition_expr.as_ref().map(|e| e.raw_expr.clone()),
        })).unwrap_or(Value::Null)
        } else {
            Value::Null
        };

    let summary = json!({
        "component_id": comp.id,
        "component_type": comp.component_type,
        "parent_id": comp.parent_id,
        "what_is_it": format!("{} component {}", comp.component_type, comp.id),
        "reads_from": comp_exprs.iter().filter_map(|e| {
            if e.refs.iter().any(|r| matches!(r, RefType::ModelField(_, _) | RefType::Param(_) | RefType::ComponentValue(_))) {
                Some(json!({ "field": e.field, "raw_expr": e.raw_expr }))
            } else { None }
        }).collect::<Vec<serde_json::Value>>(),
        "writes_to": [],
        "triggered_by": [],
        "affects": downstream.iter().map(|d| d.get("id").cloned().unwrap_or(serde_json::Value::Null)).collect::<Vec<serde_json::Value>>(),
    });

    let details = json!({
        "properties": comp.properties,
        "expressions": comp_exprs
            .iter()
            .map(|e| {
                let mut expr_struct =
                    crate::action_semantics::build_expression_struct(Some(&e.raw_expr));
                if let Some(obj) = expr_struct.as_object_mut() {
                    obj.insert("field".to_string(), json!(e.field));
                }
                expr_struct
            })
            .collect::<Vec<serde_json::Value>>(),
        "upstream": upstream,
        "downstream": downstream,
        "value_trace": value_trace,
        "priority": priority,
    });

    let mut output =
        crate::output::AiOutput::new(crate::output::OutputKind::ComponentQuery, summary);
    output.query_target = Some(comp.id.clone());
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Component {} is a {}", comp.id, comp.component_type),
            "Direct component definition in parsed metadata",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&comp.id),
    );
    output.next_queries = vec![
        format_next_query("--explain {} for semantic summary", &comp.id),
        format_next_query("--context {} --depth 2 for surrounding closure", &comp.id),
    ];

    let output = output.validate();
    writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
    out.flush()?;
    Ok(())
}
