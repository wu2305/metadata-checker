//! 单文件 .tbl 输出格式化
//!
//! M37 从 tbl_single 迁移至此，承担 human/JSON 输出职责。
//! 解析模块（tbl_single）只返回结构化对象，不负责输出。

use crate::output::schema::{
    AiOutput, Confidence, Diagnostic, DiagnosticSeverity, Evidence, Location, OutputKind,
    format_next_query,
};
use crate::superpage;
use crate::tbl_single::TblMetadata;
use anyhow::Result;
use serde_json::json;

/// 构建单文件 .tbl 的 AiOutput
pub fn build_tbl_output(meta: &TblMetadata, budget: &str) -> AiOutput {
    let is_compact = budget == "compact";
    let kind = if meta.is_dataflow {
        OutputKind::DataFlow
    } else {
        OutputKind::Table
    };

    let table_type = if meta.is_dataflow {
        "DataFlow"
    } else if meta.db_table_name.is_some() {
        "PhysicalTable"
    } else {
        "AppTable"
    };

    let field_count = meta.fields.len();
    let input_count = meta.dataflow_inputs.len();
    let output_count = meta.dataflow_outputs.len();

    let what_is_it = if meta.is_dataflow {
        format!(
            "DataFlow 加工表 {}，{} 个输入源，{} 个输出目标，{} 个字段",
            meta.table_name.as_deref().unwrap_or("unknown"),
            input_count,
            output_count,
            field_count
        )
    } else {
        format!(
            "{} {}，{} 个字段",
            table_type,
            meta.table_name.as_deref().unwrap_or("unknown"),
            field_count
        )
    };

    // 保守推断表角色
    let table_role = if meta.is_dataflow {
        "dataflow_pipeline"
    } else if meta.db_table_name.is_some() {
        "physical_storage"
    } else {
        "app_reference"
    };

    // 提取关键字段（id/key/code/name）
    let key_fields: Vec<String> = meta
        .fields
        .iter()
        .filter(|f| {
            let n = f.name.to_lowercase();
            n.contains("id") || n.contains("key") || n.contains("code") || n.contains("name")
        })
        .map(|f| f.name.clone())
        .collect();

    // 字段类型分布
    let mut field_type_summary = std::collections::HashMap::<String, usize>::new();
    for f in &meta.fields {
        let dt = f.data_type.as_deref().unwrap_or("unknown").to_string();
        *field_type_summary.entry(dt).or_insert(0) += 1;
    }

    let summary = json!({
        "table_id": meta.table_id,
        "table_name": meta.table_name,
        "table_type": table_type,
        "table_role": table_role,
        "field_count": field_count,
        "key_fields": key_fields,
        "field_type_summary": field_type_summary,
        "input_count": input_count,
        "output_count": output_count,
        "what_is_it": what_is_it,
    });

    let mut output = AiOutput::new(kind, summary);
    output.query_target = meta.input_path.clone();

    // Build details
    let fields_json: Vec<serde_json::Value> = meta
        .fields
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "dbfield": f.dbfield,
                "data_type": f.data_type,
                "length": f.length,
                "is_dimension": f.is_dimension,
                "input_field": f.input_field,
                "exp": f.exp,
                "original_field": f.original_field,
                "original_node": f.original_node,
            })
        })
        .collect();

    let dataflow_inputs_json: Vec<serde_json::Value> = meta
        .dataflow_inputs
        .iter()
        .map(|i| {
            json!({
                "node_id": i.node_id,
                "node_type": i.node_type,
                "alias": i.alias,
                "module_table_path": i.module_table_path,
            })
        })
        .collect();

    let dataflow_outputs_json: Vec<serde_json::Value> = meta
        .dataflow_outputs
        .iter()
        .map(|o| {
            json!({
                "db_table_name": o.db_table_name,
                "field_count": o.field_count,
            })
        })
        .collect();

    let internal_nodes_json: Vec<serde_json::Value> = meta
        .internal_nodes
        .iter()
        .map(|n| {
            json!({
                "node_id": n.node_id,
                "node_type": n.node_type,
                "alias": n.alias,
                "input_nodes": n.input_nodes,
                "field_count": n.fields.len(),
            })
        })
        .collect();

    // Build field lineage from dimensions
    let field_lineage: Vec<serde_json::Value> = meta
        .fields
        .iter()
        .enumerate()
        .filter_map(|(idx, f)| {
            let mut source_fields = Vec::new();
            let mut transforms = Vec::new();
            let mut confidence = Confidence::High;

            if let Some(input) = &f.input_field {
                source_fields.push(input.clone());
                transforms.push(format!("inputField: {}", input));
            }
            if let Some(exp) = &f.exp {
                transforms.push(format!("exp: {}", exp));
                // Parse expression for model refs
                let refs = superpage::parse_expression_refs(exp);
                for r in &refs {
                    if let superpage::RefType::ModelField(m, fld) = r {
                        let s = format!("{}.{}", m.clone(), fld.clone());
                        if !source_fields.contains(&s) {
                            source_fields.push(s);
                        }
                    }
                }
                if !refs.is_empty() && source_fields.is_empty() {
                    confidence = Confidence::Medium;
                }
            }
            if let Some(orig_field) = &f.original_field {
                if !source_fields.contains(orig_field) {
                    source_fields.push(orig_field.clone());
                }
                transforms.push(format!(
                    "originalField: {} (node: {})",
                    orig_field,
                    f.original_node.as_deref().unwrap_or("?")
                ));
            }

            if source_fields.is_empty() && f.exp.is_none() && f.input_field.is_none() {
                return None;
            }

            let transform = if transforms.is_empty() {
                None
            } else {
                Some(transforms.join("; "))
            };

            Some(json!({
                "target_field": format!("{}.{}", meta.table_id.as_deref().unwrap_or(""), f.name),
                "source_fields": source_fields,
                "source_expr": f.exp.as_deref().unwrap_or(""),
                "transform": transform,
                "confidence": confidence,
                "json_path": format!("dimensions[{}]", idx),
            }))
        })
        .collect();

    let details = if is_compact {
        json!({
            "fields": super::brief::truncated_array(&fields_json, 10),
            "dataflow_inputs": super::brief::truncated_array(&dataflow_inputs_json, 5),
            "dataflow_outputs": super::brief::truncated_array(&dataflow_outputs_json, 5),
            "internal_nodes": super::brief::truncated_array(&internal_nodes_json, 5),
            "field_lineage": super::brief::truncated_array(&field_lineage, 5),
        })
    } else {
        json!({
            "fields": fields_json,
            "dataflow_inputs": dataflow_inputs_json,
            "dataflow_outputs": dataflow_outputs_json,
            "internal_nodes": internal_nodes_json,
            "field_lineage": field_lineage,
        })
    };
    output.details = Some(details);

    // Evidence
    let source_file = meta.input_path.as_deref().unwrap_or("unknown");
    output.evidence.push(
        Evidence::new(
            format!(
                "Table {} parsed with {} fields",
                meta.table_name.as_deref().unwrap_or("unknown"),
                field_count
            ),
            "Direct single-file .tbl parsing",
        )
        .with_source_file(source_file)
        .with_json_path("dimensions[]")
        .with_confidence(Confidence::High),
    );

    for (idx, f) in meta.fields.iter().enumerate() {
        if f.input_field.is_some() || f.exp.is_some() || f.original_field.is_some() {
            let claim = format!(
                "Field {} has source: inputField={:?}, exp={:?}, originalField={:?}",
                f.name, f.input_field, f.exp, f.original_field
            );
            output.evidence.push(
                Evidence::new(claim, "Parsed from dimension metadata")
                    .with_source_file(source_file)
                    .with_json_path(format!("dimensions[{}]", idx))
                    .with_raw_expr(f.exp.clone().unwrap_or_default())
                    .with_confidence(Confidence::High),
            );
        }
    }

    for inp in &meta.dataflow_inputs {
        output.evidence.push(
            Evidence::new(
                format!(
                    "DataFlow input node {} (type={})",
                    inp.node_id, inp.node_type
                ),
                "Parsed from dataFlow.nodes",
            )
            .with_source_file(source_file)
            .with_node_id(&inp.node_id)
            .with_json_path(format!("dataFlow.nodes.{}", inp.node_id))
            .with_confidence(Confidence::High),
        );
    }

    // Diagnostics
    if meta.is_dataflow && meta.dataflow_outputs.is_empty() {
        output.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "DATAFLOW_NO_OUTPUT".to_string(),
            message: "DataFlow has no output physical table (dbTableName missing)".to_string(),
            location: Location {
                source_file: Some(source_file.to_string()),
                node_id: meta.table_id.clone(),
                json_path: Some("properties.dbTableName".to_string()),
            },
            suggestion: Some(
                "Check if DataFlow is intended to produce a physical table".to_string(),
            ),
            count: None,
            answer_impact: None,
            first_seen_phase: None,
        });
    }

    if meta.is_dataflow && meta.dataflow_inputs.is_empty() {
        output.diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "DATAFLOW_NO_INPUTS".to_string(),
            message: "DataFlow has no input ModelTable nodes".to_string(),
            location: Location {
                source_file: Some(source_file.to_string()),
                node_id: meta.table_id.clone(),
                json_path: Some("dataFlow.nodes".to_string()),
            },
            suggestion: Some("Verify dataFlow.nodes contains ModelTable sources".to_string()),
            count: None,
            answer_impact: None,
            first_seen_phase: None,
        });
    }

    for inp in &meta.dataflow_inputs {
        let unresolved = if let Some(ref path) = inp.module_table_path {
            path.starts_with("$DATA:/") && !path.ends_with(".tbl")
        } else {
            false
        };
        if unresolved {
            output.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Info,
                code: "DATAFLOW_INPUT_PATH_UNRESOLVED".to_string(),
                message: format!(
                    "DataFlow input node {} moduleTablePath does not end with .tbl: {}",
                    inp.node_id,
                    inp.module_table_path.as_deref().unwrap_or("")
                ),
                location: Location {
                    source_file: Some(source_file.to_string()),
                    node_id: Some(inp.node_id.clone()),
                    json_path: Some(format!("dataFlow.nodes.{}.moduleTablePath", inp.node_id)),
                },
                suggestion: Some("Verify moduleTablePath points to a valid .tbl file".to_string()),
                count: None,
                answer_impact: None,
                first_seen_phase: None,
            });
        }
    }

    // Check for unparseable expressions
    for (idx, f) in meta.fields.iter().enumerate() {
        if let Some(exp) = &f.exp {
            if exp.trim().is_empty() {
                continue;
            }
            let refs = superpage::parse_expression_refs(exp);
            if refs.is_empty() && !exp.trim().is_empty() && !exp.trim().starts_with('"') {
                // Heuristic: non-empty expression but no refs parsed — may be complex
                if exp.len() > 200 || exp.contains('\n') {
                    output.diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Info,
                        code: "EXPR_UNPARSED".to_string(),
                        message: format!(
                            "Field {} expression too complex or multiline to fully parse",
                            f.name
                        ),
                        location: Location {
                            source_file: Some(source_file.to_string()),
                            node_id: Some(format!(
                                "field:{}.{}",
                                meta.table_id.as_deref().unwrap_or(""),
                                f.name
                            )),
                            json_path: Some(format!("dimensions[{}].exp", idx)),
                        },
                        suggestion: Some(
                            "Use --project-dir --explain for full lineage".to_string(),
                        ),
                        count: None,
                        answer_impact: None,
                        first_seen_phase: None,
                    });
                }
            }
        }
    }

    // Next queries
    let table_id = meta.table_id.as_deref().unwrap_or("unknown");
    if meta.is_dataflow {
        output.next_queries = vec![
            format_next_query("--explain model:{} for model semantics", table_id),
            format_next_query(
                "--context model:{} --depth 1 for surrounding context",
                table_id,
            ),
            "--project-dir <DIR> --query-dataflow <MODEL> for full DataFlow lineage".to_string(),
        ];
    } else {
        output.next_queries = vec![
            format_next_query("--explain model:{} for model semantics", table_id),
            format_next_query(
                "--context model:{} --depth 1 for surrounding context",
                table_id,
            ),
            "--project-dir <DIR> --query-model <MODEL> for cross-file usage".to_string(),
        ];
    }

    // Compact mode: add OUTPUT_TRUNCATED and evidence_summary
    if is_compact {
        let truncated_arrays = [
            ("fields", fields_json.len(), 10),
            ("dataflow_inputs", dataflow_inputs_json.len(), 5),
            ("dataflow_outputs", dataflow_outputs_json.len(), 5),
            ("internal_nodes", internal_nodes_json.len(), 5),
            ("field_lineage", field_lineage.len(), 5),
        ];
        let truncated_parts: Vec<String> = truncated_arrays
            .iter()
            .filter(|(_, size, limit)| *size > *limit)
            .map(|(name, size, limit)| format!("{} {}>{}", name, size, limit))
            .collect();
        if !truncated_parts.is_empty() {
            output.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Info,
                code: "OUTPUT_TRUNCATED".to_string(),
                message: format!(
                    "Compact budget: arrays truncated for: {}",
                    truncated_parts.join(", ")
                ),
                location: Location {
                    source_file: meta.input_path.clone(),
                    node_id: meta.table_id.clone(),
                    json_path: None,
                },
                suggestion: Some(
                    "Use --budget normal or --budget full to see complete arrays".to_string(),
                ),
                count: None,
                answer_impact: None,
                first_seen_phase: None,
            });
        }

        let evidence_summary = super::brief::evidence_summary(&output.evidence, 5);
        let key_findings = super::brief::build_key_findings(&output.summary, &output.diagnostics);
        if let Some(obj) = output.summary.as_object_mut() {
            obj.insert("evidence_summary".to_string(), evidence_summary);
            obj.insert("key_findings".to_string(), serde_json::json!(key_findings));
        }
        output.evidence.truncate(5);
    }

    output.validate()
}

/// 打印 human 可读的单文件 .tbl 摘要
pub fn print_tbl_human(meta: &TblMetadata) -> Result<()> {
    println!(
        "=== {} 摘要 ===",
        if meta.is_dataflow { "DataFlow" } else { "表" }
    );
    println!(
        "表名: {} | 版本: {}",
        meta.table_name.as_deref().unwrap_or("N/A"),
        meta.version.as_deref().unwrap_or("N/A")
    );
    println!(
        "类型: {}",
        if meta.is_dataflow {
            "DataFlow 加工表"
        } else if meta.db_table_name.is_some() {
            "物理表"
        } else {
            "应用表"
        }
    );
    println!("字段数: {}", meta.fields.len());

    if meta.is_dataflow {
        println!("输入节点: {}", meta.dataflow_inputs.len());
        for inp in &meta.dataflow_inputs {
            println!(
                "  - {} ({}): {}",
                inp.node_id,
                inp.node_type,
                inp.alias.as_deref().unwrap_or("-")
            );
        }
        println!("输出目标: {}", meta.dataflow_outputs.len());
        for out in &meta.dataflow_outputs {
            println!("  - {}", out.db_table_name.as_deref().unwrap_or("未指定"));
        }
        println!("内部节点: {}", meta.internal_nodes.len());
    }

    println!("\n字段列表 (前 10):");
    for f in meta.fields.iter().take(10) {
        let mut tags = Vec::new();
        if f.is_dimension {
            tags.push("维度");
        }
        if f.input_field.is_some() {
            tags.push("inputField");
        }
        if f.exp.is_some() {
            tags.push("exp");
        }
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!("[{}]", tags.join(", "))
        };
        println!(
            "  {} ({}) {}",
            f.name,
            f.dbfield.as_deref().unwrap_or("-"),
            tag_str
        );
    }

    // Field lineage summary
    let lineage_count = meta
        .fields
        .iter()
        .filter(|f| f.input_field.is_some() || f.exp.is_some() || f.original_field.is_some())
        .count();
    if lineage_count > 0 {
        println!("\n有来源追溯的字段: {}", lineage_count);
    }

    println!("\n可用命令:");
    let table_id = meta.table_id.as_deref().unwrap_or("unknown");
    if meta.is_dataflow {
        println!("  --explain model:{}", table_id);
        println!("  --project-dir <DIR> --query-dataflow {}", table_id);
    } else {
        println!("  --explain model:{}", table_id);
        println!("  --project-dir <DIR> --query-model {}", table_id);
    }

    Ok(())
}
