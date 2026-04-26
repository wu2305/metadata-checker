use crate::dependency::{DependencyGraph, ValueTrace};
use crate::parser::PageMetadata;
use crate::superpage::{RefType, SuperPageMetadata};
use anyhow::Result;
use serde_json::{Value, json};
use std::io::{self, Write};

/// 输出格式化模块
///
/// 将解析结果输出为：
/// - human 模式：面向人类的可读文本（支持交互式组件查询）
/// - JSON 模式：结构化数据供下游工具消费
///
/// 同时处理 --query 参数，支持按组件 ID 精确查询。

pub fn print_human(meta: &PageMetadata) -> Result<()> {
    print_human_to(meta, &mut io::stdout())
}

/// human 模式输出：组件树 + 表达式 + 依赖 + 优先级
/// human 模式输出：组件树 + 表达式 + 依赖 + 优先级
pub fn print_human_to(meta: &PageMetadata, out: &mut dyn Write) -> Result<()> {
    if let Some(spg) = &meta.superpage {
        print_superpage_human(spg, out)?;
        return Ok(());
    }

    writeln!(out, "=== Page Metadata Report ===")?;
    writeln!(
        out,
        "Page ID      : {}",
        meta.page_id.as_deref().unwrap_or("N/A")
    )?;
    writeln!(
        out,
        "Version      : {}",
        meta.version.as_deref().unwrap_or("N/A")
    )?;

    writeln!(
        out,
        "\n--- Components ({} total) ---",
        meta.components.len()
    )?;
    for (i, comp) in meta.components.iter().enumerate() {
        writeln!(out, "[{}]", i + 1)?;
        writeln!(
            out,
            "  ID         : {}",
            comp.id.as_deref().unwrap_or("N/A")
        )?;
        writeln!(
            out,
            "  Type       : {}",
            comp.component_type.as_deref().unwrap_or("N/A")
        )?;
        writeln!(
            out,
            "  Title      : {}",
            comp.title.as_deref().unwrap_or("N/A")
        )?;
        writeln!(
            out,
            "  DB Field   : {}",
            comp.db_field.as_deref().unwrap_or("N/A")
        )?;
    }

    writeln!(
        out,
        "\n--- Data Bindings ({} total) ---",
        meta.data_bindings.len()
    )?;
    for (i, bind) in meta.data_bindings.iter().enumerate() {
        writeln!(out, "[{}]", i + 1)?;
        writeln!(
            out,
            "  Source ID  : {}",
            bind.source_id.as_deref().unwrap_or("N/A")
        )?;
        writeln!(
            out,
            "  Path/Name  : {}",
            bind.path.as_deref().unwrap_or("N/A")
        )?;
        writeln!(
            out,
            "  Filter     : {}",
            bind.filter.as_deref().unwrap_or("N/A")
        )?;
    }

    if !meta.settings.is_empty() {
        writeln!(out, "\n--- Settings ---")?;
        for (k, v) in &meta.settings {
            writeln!(out, "  {} : {}", k, v)?;
        }
    }

    writeln!(out, "\n=== End of Report ===")?;
    out.flush()?;
    Ok(())
}

fn print_superpage_human(spg: &SuperPageMetadata, out: &mut dyn Write) -> Result<()> {
    writeln!(out, "=== SuperPage Metadata Report ===")?;
    writeln!(
        out,
        "Version      : {}",
        spg.version.as_deref().unwrap_or("N/A")
    )?;
    writeln!(
        out,
        "Theme        : {}",
        spg.theme.as_deref().unwrap_or("N/A")
    )?;
    writeln!(out, "Components   : {}", spg.components.len())?;
    writeln!(out, "Expressions  : {}", spg.expressions.len())?;

    writeln!(out, "\n--- Page Parameters ({}) ---", spg.params.len())?;
    for (i, param) in spg.params.iter().enumerate() {
        writeln!(out, "[{}] {}", i + 1, param.id)?;
        writeln!(out, "  Name  : {}", param.name)?;
        if let Some(desc) = &param.desc {
            writeln!(out, "  Desc  : {}", desc)?;
        }
        if let Some(val) = &param.default_value {
            writeln!(out, "  Default: {}", val)?;
        }
    }

    writeln!(out, "\n--- Data Sources ({}) ---", spg.sources.len())?;
    for (i, src) in spg.sources.iter().enumerate() {
        writeln!(out, "[{}] {}", i + 1, src.id)?;
        if let Some(mt) = &src.model_type {
            writeln!(out, "  Type : {}", mt)?;
        }
        if let Some(path) = &src.path {
            writeln!(out, "  Path : {}", path)?;
        }
    }

    writeln!(
        out,
        "\n--- Components with Expressions ({}) ---",
        spg.expressions.len()
    )?;
    let mut current_comp = "";
    for expr in &spg.expressions {
        if expr.component_id != current_comp {
            current_comp = &expr.component_id;
            let comp_type = spg
                .components
                .iter()
                .find(|c| c.id == current_comp)
                .map(|c| c.component_type.as_str())
                .unwrap_or("unknown");
            writeln!(out, "\n> {} ({})", current_comp, comp_type)?;
        }
        writeln!(out, "  [{}] {}", expr.field, expr.raw_expr)?;
        if !expr.refs.is_empty() {
            writeln!(out, "    References:")?;
            for ref_type in &expr.refs {
                match ref_type {
                    RefType::ComponentValue(id) => writeln!(out, "      - ComponentValue: {}", id)?,
                    RefType::ComponentProperty(id, prop) => {
                        writeln!(out, "      - ComponentProperty: {}.{}", id, prop)?
                    }
                    RefType::ModelField(model, field) => {
                        writeln!(out, "      - ModelField: {}.{}", model, field)?
                    }
                    RefType::Param(id) => writeln!(out, "      - Param: {}", id)?,
                    RefType::UserProperty(prop) => writeln!(out, "      - UserProperty: {}", prop)?,
                    RefType::SystemVar(var) => writeln!(out, "      - SystemVar: {}", var)?,
                    RefType::Other(s) => writeln!(out, "      - Other: {}", s)?,
                };
            }
        }
    }

    let graph = DependencyGraph::new(spg);
    let topo = graph.topological_sort();
    if !topo.is_empty() {
        writeln!(out, "\n--- Dependency Order ---")?;
        for (i, id) in topo.iter().enumerate() {
            writeln!(out, "  [{}] {}", i + 1, id)?;
        }
    }

    let cycles = graph.detect_cycles();
    if !cycles.is_empty() {
        writeln!(out, "\n⚠️  Cycles Detected:")?;
        for cycle in cycles {
            writeln!(out, "  {}", cycle.join(" → "))?;
        }
    }

    writeln!(out, "\n=== End of Report ===")?;
    out.flush()?;
    Ok(())
}

pub fn print_non_human(meta: &PageMetadata) -> Result<()> {
    print_non_human_to(meta, &mut io::stdout())
}

pub fn print_non_human_to(meta: &PageMetadata, out: &mut dyn Write) -> Result<()> {
    if let Some(spg) = &meta.superpage {
        let graph = DependencyGraph::new(spg);
        let topo = graph.topological_sort();
        let cycles = graph.detect_cycles();

        let summary = json!({
            "schema_version": "1.0",
            "kind": "SuperPage",
            "truncated": false,
            "diagnostics": {
                "cycle_count": cycles.len(),
                "has_cycles": !cycles.is_empty(),
                "component_count": spg.components.len(),
                "expression_count": spg.expressions.len(),
            },
            "version": spg.version,
            "theme": spg.theme,
            "params": spg.params.iter().map(|p| json!({
                "id": p.id,
                "name": p.name,
                "desc": p.desc,
                "default_value": p.default_value,
            })).collect::<Vec<Value>>(),
            "sources": spg.sources.iter().map(|s| json!({
                "id": s.id,
                "model_type": s.model_type,
                "path": s.path,
            })).collect::<Vec<Value>>(),
            "components": spg.components.iter().map(|c| json!({
                "id": c.id,
                "type": c.component_type,
                "parent_id": c.parent_id,
                "properties": c.properties,
            })).collect::<Vec<Value>>(),
            "expressions": spg.expressions.iter().map(|e| json!({
                "component_id": e.component_id,
                "field": e.field,
                "raw_expr": e.raw_expr,
                "refs": e.refs.iter().map(|r| match r {
                    RefType::ComponentValue(id) => json!({"type": "component_value", "id": id}),
                    RefType::ComponentProperty(id, prop) => json!({"type": "component_property", "id": id, "property": prop}),
                    RefType::ModelField(model, field) => json!({"type": "model_field", "model": model, "field": field}),
                    RefType::Param(id) => json!({"type": "param", "id": id}),
                    RefType::UserProperty(prop) => json!({"type": "user_property", "property": prop}),
                    RefType::SystemVar(var) => json!({"type": "system_var", "var": var}),
                    RefType::Other(s) => json!({"type": "other", "value": s}),
                }).collect::<Vec<Value>>(),
            })).collect::<Vec<Value>>(),
            "dependency_order": topo,
            "cycles": cycles,
            "next_queries": [
                "--query <COMPONENT_ID> for component details",
                "--priority for defaultValue vs exp analysis",
                "--project-dir <DIR> --query-model <MODEL> for cross-file model usage",
            ],
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&summary)?)?;
        return Ok(());
    }

    let summary = json!({
        "page_id": meta.page_id,
        "version": meta.version,
        "components": meta.components.iter().map(|c| json!({
            "id": c.id,
            "type": c.component_type,
            "title": c.title,
            "db_field": c.db_field,
        })).collect::<Vec<Value>>(),
        "data_bindings": meta.data_bindings.iter().map(|b| json!({
            "source_id": b.source_id,
            "path": b.path,
            "filter": b.filter,
        })).collect::<Vec<Value>>(),
        "settings": meta.settings,
    });
    writeln!(out, "{}", serde_json::to_string_pretty(&summary)?)?;
    out.flush()?;
    Ok(())
}

pub fn print_value_traces(traces: &[ValueTrace], human: bool) -> Result<()> {
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Value Source Traces ===")?;
        for trace in traces {
            writeln!(out, "\n--- {}.{} ---", trace.component_id, trace.field)?;
            writeln!(out, "Raw Expression: {}", trace.raw_expr)?;
            writeln!(out, "Expanded     : {}", trace.expanded_expr)?;
            writeln!(out, "Source Type  : {:?}", trace.source_type)?;
            writeln!(
                out,
                "External Input: {}",
                if trace.is_external_input { "Yes" } else { "No" }
            )?;
            if !trace.source_chain.is_empty() {
                writeln!(out, "Source Chain:")?;
                for (i, node) in trace.source_chain.iter().enumerate() {
                    writeln!(out, "  [{}] {}: {}", i + 1, node.component_id, node.expr)?;
                }
            }
        }
        writeln!(out, "\n=== End of Traces ===")?;
        out.flush()?;
    } else {
        let json_traces: Vec<Value> = traces
            .iter()
            .map(|t| {
                json!({
                    "component_id": t.component_id,
                    "field": t.field,
                    "raw_expr": t.raw_expr,
                    "expanded_expr": t.expanded_expr,
                    "source_type": format!("{:?}", t.source_type),
                    "is_external_input": t.is_external_input,
                    "source_chain": t.source_chain.iter().map(|n| json!({
                        "component_id": n.component_id,
                        "expr": n.expr,
                        "source_type": format!("{:?}", n.source_type),
                    })).collect::<Vec<Value>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_traces)?);
    }
    Ok(())
}

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
    if let Some(dep_ids) = downstream {
        if !dep_ids.is_empty() {
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

    let result = json!({
        "component_id": comp.id,
        "type": comp.component_type,
        "parent_id": comp.parent_id,
        "properties": comp.properties,
        "expressions": comp_exprs.iter().map(|e| json!({
            "field": e.field,
            "raw_expr": e.raw_expr,
            "refs": e.refs.iter().map(|r| match r {
                RefType::ComponentValue(id) => json!({"type": "component_value", "id": id}),
                RefType::ComponentProperty(id, prop) => json!({"type": "component_property", "id": id, "property": prop}),
                RefType::ModelField(model, field) => json!({"type": "model_field", "model": model, "field": field}),
                RefType::Param(id) => json!({"type": "param", "id": id}),
                RefType::UserProperty(prop) => json!({"type": "user_property", "property": prop}),
                RefType::SystemVar(var) => json!({"type": "system_var", "var": var}),
                RefType::Other(s) => json!({"type": "other", "value": s}),
            }).collect::<Vec<Value>>(),
        })).collect::<Vec<Value>>(),
        "upstream": upstream,
        "downstream": downstream,
        "value_trace": value_trace,
        "priority": priority,
    });

    writeln!(out, "{}", serde_json::to_string_pretty(&result)?)?;
    out.flush()?;
    Ok(())
}
