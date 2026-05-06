//! 单文件 .tbl 解析与输出模块
//!
//! 负责在没有项目图数据库时，直接从单个 .tbl JSON 文件提取表/DataFlow 语义。

use crate::output::schema::{
    AiOutput, Confidence, Diagnostic, DiagnosticSeverity, Evidence, Location, OutputKind,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

/// 从 .tbl JSON 提取的字段信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TblField {
    pub name: String,
    pub dbfield: Option<String>,
    pub data_type: Option<String>,
    pub length: Option<i64>,
    pub is_dimension: bool,
    pub input_field: Option<String>,
    pub exp: Option<String>,
    pub original_field: Option<String>,
    pub original_node: Option<String>,
}

/// 单文件 .tbl 元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TblMetadata {
    pub input_path: Option<String>,
    pub table_id: Option<String>,
    pub table_name: Option<String>,
    pub version: Option<String>,
    pub db_table_name: Option<String>,
    pub is_dataflow: bool,
    pub fields: Vec<TblField>,
    pub dataflow_inputs: Vec<DataFlowInput>,
    pub dataflow_outputs: Vec<DataFlowOutput>,
    pub internal_nodes: Vec<DataFlowNode>,
    pub raw: serde_json::Value,
}

/// DataFlow 输入源
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowInput {
    pub node_id: String,
    pub node_type: String,
    pub alias: Option<String>,
    pub module_table_path: Option<String>,
}

/// DataFlow 输出目标
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowOutput {
    pub db_table_name: Option<String>,
    pub field_count: usize,
}

/// DataFlow 内部节点
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DataFlowNode {
    pub node_id: String,
    pub node_type: String,
    pub alias: Option<String>,
    pub input_nodes: Vec<String>,
    pub fields: Vec<TblField>,
}

/// 解析单个 .tbl 文件
pub fn parse_tbl(path: &Path, raw: serde_json::Value) -> Result<TblMetadata> {
    let mut meta = TblMetadata {
        input_path: Some(path.display().to_string()),
        raw: raw.clone(),
        ..Default::default()
    };

    let obj = raw.as_object();

    meta.version = obj
        .and_then(|o| o.get("version"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let props = obj
        .and_then(|o| o.get("properties"))
        .and_then(|v| v.as_object());
    meta.db_table_name = props
        .and_then(|p| p.get("dbTableName"))
        .and_then(|v| v.as_str())
        .map(String::from);

    meta.is_dataflow = raw.get("dataFlow").is_some();

    // Table name from properties.name or file stem
    meta.table_name = props
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()));

    // Table id: use dbTableName if available, else file stem
    meta.table_id = meta
        .db_table_name
        .clone()
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()));

    // Parse dimensions (fields)
    if let Some(dims) = raw.get("dimensions").and_then(|d| d.as_array()) {
        for (idx, dim) in dims.iter().enumerate() {
            if let Some(dim_obj) = dim.as_object() {
                let field = parse_dim_field(dim_obj, idx);
                meta.fields.push(field);
            }
        }
    }

    // Parse DataFlow nodes if present
    if meta.is_dataflow {
        if let Some(nodes) = raw
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
        {
            for (node_id, node_val) in nodes {
                if let Some(node_obj) = node_val.as_object() {
                    let node_type = node_obj
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let alias = node_obj
                        .get("alias")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let module_table_path = node_obj
                        .get("moduleTablePath")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    let input_nodes: Vec<String> = node_obj
                        .get("inputNodes")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    let mut node_fields = Vec::new();
                    if let Some(fields) = node_obj.get("fields").and_then(|v| v.as_array()) {
                        for (idx, f) in fields.iter().enumerate() {
                            if let Some(fobj) = f.as_object() {
                                node_fields.push(parse_dim_field(fobj, idx));
                            }
                        }
                    }

                    meta.internal_nodes.push(DataFlowNode {
                        node_id: node_id.clone(),
                        node_type: node_type.clone(),
                        alias: alias.clone(),
                        input_nodes,
                        fields: node_fields,
                    });

                    // Input nodes: ModelTable or FileInput
                    if node_type == "ModelTable"
                        || node_type == "modelTable"
                        || node_type == "FileInput"
                    {
                        meta.dataflow_inputs.push(DataFlowInput {
                            node_id: node_id.clone(),
                            node_type,
                            alias,
                            module_table_path,
                        });
                    }
                }
            }
        }

        // Output: physical table (dbTableName)
        if let Some(db_name) = &meta.db_table_name {
            meta.dataflow_outputs.push(DataFlowOutput {
                db_table_name: Some(db_name.clone()),
                field_count: meta.fields.len(),
            });
        }
    }

    Ok(meta)
}

fn parse_dim_field(dim_obj: &serde_json::Map<String, serde_json::Value>, _idx: usize) -> TblField {
    TblField {
        name: dim_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        dbfield: dim_obj
            .get("dbfield")
            .and_then(|v| v.as_str())
            .map(String::from),
        data_type: dim_obj
            .get("dataType")
            .and_then(|v| v.as_str())
            .map(String::from),
        length: dim_obj.get("length").and_then(|v| v.as_i64()),
        is_dimension: dim_obj
            .get("isDimension")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        input_field: dim_obj
            .get("inputField")
            .and_then(|v| v.as_str())
            .map(String::from),
        exp: dim_obj
            .get("exp")
            .and_then(|v| v.as_str())
            .map(String::from),
        original_field: dim_obj
            .get("originalField")
            .and_then(|v| v.as_str())
            .map(String::from),
        original_node: dim_obj
            .get("originalNode")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

/// 构建单文件 .tbl 的 AiOutput
pub fn build_tbl_output(meta: &TblMetadata) -> AiOutput {
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

    let summary = json!({
        "table_id": meta.table_id,
        "table_name": meta.table_name,
        "table_type": table_type,
        "field_count": field_count,
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
                let refs = crate::superpage::parse_expression_refs(exp);
                for r in &refs {
                    if let crate::superpage::RefType::ModelField(m, fld) = r {
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

    let details = json!({
        "fields": fields_json,
        "dataflow_inputs": dataflow_inputs_json,
        "dataflow_outputs": dataflow_outputs_json,
        "internal_nodes": internal_nodes_json,
        "field_lineage": field_lineage,
    });
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
            });
        }
    }

    // Check for unparseable expressions
    for (idx, f) in meta.fields.iter().enumerate() {
        if let Some(exp) = &f.exp {
            if exp.trim().is_empty() {
                continue;
            }
            let refs = crate::superpage::parse_expression_refs(exp);
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
                    });
                }
            }
        }
    }

    // Next queries
    let table_id = meta.table_id.as_deref().unwrap_or("unknown");
    if meta.is_dataflow {
        output.next_queries = vec![
            format!("--explain model:{} for model semantics", table_id),
            format!(
                "--context model:{} --depth 1 for surrounding context",
                table_id
            ),
            "--project-dir <DIR> --query-dataflow <MODEL> for full DataFlow lineage".to_string(),
        ];
    } else {
        output.next_queries = vec![
            format!("--explain model:{} for model semantics", table_id),
            format!(
                "--context model:{} --depth 1 for surrounding context",
                table_id
            ),
            "--project-dir <DIR> --query-model <MODEL> for cross-file usage".to_string(),
        ];
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
