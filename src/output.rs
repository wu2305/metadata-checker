use crate::dependency::DependencyGraph;
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
    print_non_human_to(meta, None, &mut io::stdout())
}

/// 输出紧凑 summary（默认 non-human 模式）
pub fn print_summary(
    meta: &PageMetadata,
    priority_analyses: Option<&[crate::priority::PriorityAnalysis]>,
) -> Result<()> {
    print_summary_to(meta, priority_analyses, &mut io::stdout())
}

pub fn print_summary_to(
    meta: &PageMetadata,
    priority_analyses: Option<&[crate::priority::PriorityAnalysis]>,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(spg) = &meta.superpage {
        let graph = DependencyGraph::new(spg);
        let _topo = graph.topological_sort();
        let cycles = graph.detect_cycles();

        // Collect models read/written
        let mut models_read: Vec<String> = Vec::new();
        let _models_written: Vec<String> = Vec::new();
        for expr in &spg.expressions {
            for ref_type in &expr.refs {
                if let RefType::ModelField(model, _) = ref_type {
                    models_read.push(model.clone());
                }
            }
        }
        models_read.sort();
        models_read.dedup();

        // Components with expressions
        let important_components: Vec<Value> = spg
            .expressions
            .iter()
            .map(|e| {
                json!({
                    "id": e.component_id,
                    "field": e.field,
                    "raw_expr": e.raw_expr,
                })
            })
            .collect();

        let _priority_json = priority_analyses.map(|analyses| {
            analyses
                .iter()
                .map(|a| {
                    json!({
                        "component_id": a.component_id,
                        "priority_result": format!("{:?}", a.priority_result),
                    })
                })
                .collect::<Vec<Value>>()
        });

        let mut diagnostics = Vec::new();
        if !cycles.is_empty() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "CYCLE_DEPENDENCY".to_string(),
                message: "Cycle dependencies detected".to_string(),
                location: crate::output::Location::new(),
                suggestion: Some("Check component expressions for circular references".to_string()),
            });
        } else {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "OK".to_string(),
                message: "No cycles detected".to_string(),
                location: crate::output::Location::new(),
                suggestion: None,
            });
        }

        let summary = json!({
            "page_info": {
                "version": spg.version,
                "theme": spg.theme,
                "component_count": spg.components.len(),
                "expression_count": spg.expressions.len(),
                "has_cycles": !cycles.is_empty(),
            },
            "important_components": important_components,
            "data_sources": spg.sources.iter().map(|s| json!({
                "id": s.id,
                "model_type": s.model_type,
                "path": s.path,
            })).collect::<Vec<Value>>(),
            "page_params": spg.params.iter().map(|p| json!({
                "id": p.id,
                "name": p.name,
                "default_value": p.default_value,
            })).collect::<Vec<Value>>(),
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::SuperPage, summary);
        output.diagnostics = diagnostics;
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "SuperPage has {} components and {} expressions",
                    spg.components.len(),
                    spg.expressions.len()
                ),
                "Parsed from input file",
            )
            .with_confidence(crate::output::Confidence::High),
        );
        output.next_queries = vec![
            "--query <COMPONENT_ID> for component details".to_string(),
            "--explain <COMPONENT_ID> for semantic explanation".to_string(),
            "--priority for defaultValue vs exp analysis".to_string(),
            "--detail for full raw structure".to_string(),
            "--project-dir <DIR> --query-model <MODEL> for cross-file model usage".to_string(),
        ];

        let output = output.validate();
        writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
        return Ok(());
    }

    let summary = json!({
        "page_id": meta.page_id,
        "version": meta.version,
        "component_count": meta.components.len(),
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::SuperPage, summary);
    output.evidence.push(
        crate::output::Evidence::new("Legacy page metadata parsed", "Non-SuperPage format")
            .with_confidence(crate::output::Confidence::Medium),
    );
    output.next_queries = vec!["--detail for full raw structure".to_string()];

    let output = output.validate();
    writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
    out.flush()?;
    Ok(())
}

pub fn print_non_human_to(
    meta: &PageMetadata,
    priority_analyses: Option<&[crate::priority::PriorityAnalysis]>,
    out: &mut dyn Write,
) -> Result<()> {
    if let Some(spg) = &meta.superpage {
        let graph = DependencyGraph::new(spg);
        let topo = graph.topological_sort();
        let cycles = graph.detect_cycles();

        let priority_json = priority_analyses.map(|analyses| {
            analyses.iter().map(|a| json!({
                "component_id": a.component_id,
                "component_type": a.component_type,
                "priority_result": format!("{:?}", a.priority_result),
                "default_value": a.default_value_expr.as_ref().map(|e| e.raw_expr.clone()),
                "exp": a.calc_exp_expr.as_ref().map(|e| e.raw_expr.clone()),
                "calc_condition": a.calc_condition_expr.as_ref().map(|e| e.raw_expr.clone()),
            })).collect::<Vec<Value>>()
        });

        let mut diagnostics = Vec::new();
        if !cycles.is_empty() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "CYCLE_DEPENDENCY".to_string(),
                message: "Cycle dependencies detected".to_string(),
                location: crate::output::Location::new(),
                suggestion: Some("Check component expressions for circular references".to_string()),
            });
        } else {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "OK".to_string(),
                message: "No cycles detected".to_string(),
                location: crate::output::Location::new(),
                suggestion: None,
            });
        }

        let summary = json!({
            "page_info": {
                "version": spg.version,
                "theme": spg.theme,
                "component_count": spg.components.len(),
                "expression_count": spg.expressions.len(),
                "has_cycles": !cycles.is_empty(),
            },
        });

        let details = json!({
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
                "resolved_refs": e.resolved_refs.iter().map(|rr| json!({
                    "type": match &rr.ref_type {
                        RefType::ComponentValue(id) => json!({"kind": "component_value", "id": id}),
                        RefType::ComponentProperty(id, prop) => json!({"kind": "component_property", "id": id, "property": prop}),
                        RefType::ModelField(model, field) => json!({"kind": "model_field", "model": model, "field": field}),
                        RefType::Param(id) => json!({"kind": "param", "id": id}),
                        RefType::UserProperty(prop) => json!({"kind": "user_property", "property": prop}),
                        RefType::SystemVar(var) => json!({"kind": "system_var", "var": var}),
                        RefType::Other(s) => json!({"kind": "other", "value": s}),
                    },
                    "confidence": format!("{:?}", rr.confidence),
                    "reason": rr.reason,
                    "unresolved": rr.unresolved,
                })).collect::<Vec<Value>>(),
            })).collect::<Vec<Value>>(),
            "dependency_order": topo,
            "cycles": cycles,
            "priority_analysis": priority_json,
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::SuperPage, summary);
        output.details = Some(details);
        output.diagnostics = diagnostics;
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "SuperPage parsed with {} components and {} expressions",
                    spg.components.len(),
                    spg.expressions.len()
                ),
                "Direct parsing from input file",
            )
            .with_confidence(crate::output::Confidence::High),
        );
        output.next_queries = vec![
            "--query <COMPONENT_ID> for component details".to_string(),
            "--explain <COMPONENT_ID> for semantic explanation".to_string(),
            "--priority for defaultValue vs exp analysis".to_string(),
            "--project-dir <DIR> --query-model <MODEL> for cross-file model usage".to_string(),
        ];

        let output = output.validate();
        writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
        return Ok(());
    }

    let summary = json!({
        "page_id": meta.page_id,
        "version": meta.version,
        "component_count": meta.components.len(),
    });

    let details = json!({
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

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::SuperPage, summary);
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new("Legacy page metadata parsed", "Non-SuperPage format")
            .with_confidence(crate::output::Confidence::Medium),
    );
    output.next_queries = vec!["--detail for full raw structure".to_string()];

    let output = output.validate();
    writeln!(out, "{}", serde_json::to_string_pretty(&output)?)?;
    out.flush()?;
    Ok(())
}

mod component;
pub mod schema;
pub use component::*;
pub use schema::*;
