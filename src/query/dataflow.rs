use crate::graph::GraphDB;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, Write};

// ============================================================
// DataFlow 字段级来源追溯
// ============================================================

/// DataFlow 节点字段记录（支持 serde 反序列化）
#[derive(Debug, Clone, Deserialize)]
struct FieldRecord {
    name: String,
    dbfield: String,
    #[serde(rename = "originalField")]
    original_field: Option<String>,
    #[serde(rename = "originalNode")]
    original_node: Option<String>,
    exp: Option<String>,
}

/// DataFlow 预解析元数据（一次性反序列化 + 预建索引）
#[derive(Debug, Clone, Default)]
struct DataFlowMeta {
    /// alias -> node_id
    alias_map: HashMap<String, String>,
    /// node_id -> alias
    id_to_alias: HashMap<String, String>,
    /// node_id -> node_type
    node_types: HashMap<String, String>,
    /// node_id -> [dep_node_id]
    internal_deps: HashMap<String, Vec<String>>,
    /// 预建索引：node_id -> field_name -> FieldRecord
    field_index: HashMap<String, HashMap<String, FieldRecord>>,
}

impl DataFlowMeta {
    /// 从 node.meta 的 serde_json::Value 一次性构建
    fn from_meta(meta: &serde_json::Value) -> Self {
        let alias_map: HashMap<String, String> = meta
            .get("aliasMap")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_fields_raw: HashMap<String, Vec<serde_json::Value>> = meta
            .get("nodeFields")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let internal_deps: HashMap<String, Vec<String>> = meta
            .get("internalDeps")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let node_types: HashMap<String, String> = meta
            .get("nodeTypes")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        // Parse dimensions for fallback
        let dimensions: Vec<FieldRecord> = meta
            .get("dimensions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|dim| {
                        let name = dim.get("name")?.as_str()?.to_string();
                        let dbfield = dim.get("dbfield")?.as_str()?.to_string();
                        Some(FieldRecord {
                            name,
                            dbfield,
                            original_field: None,
                            original_node: None,
                            exp: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 预解析 node_fields：serde_json::Value -> Vec<FieldRecord>，并建立 field_name 索引
        let mut field_index: HashMap<String, HashMap<String, FieldRecord>> = HashMap::new();

        for (node_id, raw_fields) in &node_fields_raw {
            let records: Vec<FieldRecord> = raw_fields
                .iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect();
            let mut idx = HashMap::new();
            for rec in &records {
                idx.insert(rec.name.clone(), rec.clone());
            }
            field_index.insert(node_id.clone(), idx);
        }

        // Fallback: if nodeFields is empty but dimensions exist, use dimensions as default output
        if field_index.is_empty() && !dimensions.is_empty() {
            let mut idx = HashMap::new();
            for dim in &dimensions {
                idx.insert(dim.name.clone(), dim.clone());
            }
            field_index.insert("default".to_string(), idx);
        }

        // Fallback: if Output nodes have empty nodeFields, populate with dimensions
        let output_nodes: Vec<String> = node_types
            .iter()
            .filter(|(_, t)| *t == "Output")
            .map(|(id, _)| id.clone())
            .collect();
        for out_id in output_nodes {
            if !field_index.contains_key(&out_id) && !dimensions.is_empty() {
                let mut idx = HashMap::new();
                for dim in &dimensions {
                    idx.insert(dim.name.clone(), dim.clone());
                }
                field_index.insert(out_id.clone(), idx);
            }
        }

        let id_to_alias: HashMap<String, String> = alias_map
            .iter()
            .map(|(a, id)| (id.clone(), a.clone()))
            .collect();

        DataFlowMeta {
            alias_map,
            id_to_alias,
            node_types,
            internal_deps,
            field_index,
        }
    }

    /// 根据 node_id 获取字段索引（预建，O(1)）
    fn get_fields(&self, node_id: &str) -> Option<&HashMap<String, FieldRecord>> {
        self.field_index.get(node_id)
    }

    /// 根据 node_id 获取节点类型
    fn get_node_type(&self, node_id: &str) -> &str {
        self.node_types
            .get(node_id)
            .map(|s| s.as_str())
            .unwrap_or("Unknown")
    }

    /// 根据 alias 获取 node_id
    fn get_node_id(&self, alias: &str) -> Option<&String> {
        self.alias_map.get(alias)
    }

    /// 根据 node_id 获取 alias
    fn get_alias(&self, node_id: &str) -> Option<&String> {
        self.id_to_alias.get(node_id)
    }

    /// 获取 Output 类型节点的字段索引；如果没有 Output 节点，fallback 到 "default"
    fn get_output_fields(&self) -> Vec<(&str, &HashMap<String, FieldRecord>)> {
        let output_nodes: Vec<&str> = self
            .node_types
            .iter()
            .filter(|(_, t)| *t == "Output")
            .map(|(id, _)| id.as_str())
            .collect();

        if !output_nodes.is_empty() {
            output_nodes
                .into_iter()
                .filter_map(|id| self.field_index.get(id).map(|fields| (id, fields)))
                .collect()
        } else {
            self.field_index
                .get("default")
                .map(|fields| vec![("default", fields)])
                .unwrap_or_default()
        }
    }
}

#[derive(Debug, Clone)]
/// 字段追溯链中的单步信息
struct TraceStep {
    node_alias: String,
    node_type: String,
    field_name: String,
    dbfield: String,
    exp: Option<String>,
}

/// 递归追溯字段的数据来源链
fn trace_field_source(
    output_field_name: &str,
    output_field: &FieldRecord,
    meta: &DataFlowMeta,
    visited: &mut Vec<(String, String)>,
) -> Vec<TraceStep> {
    let mut steps = Vec::new();

    let mut current_field_name = output_field_name.to_string();
    let mut current_node_alias: Option<String> = output_field.original_node.clone();
    let mut current_field_dbfield = output_field.dbfield.clone();
    let mut current_exp = output_field.exp.clone();

    steps.push(TraceStep {
        node_alias: "模型输出".to_string(),
        node_type: "Output".to_string(),
        field_name: current_field_name.clone(),
        dbfield: current_field_dbfield.clone(),
        exp: current_exp.clone(),
    });

    while let Some(node_alias) = &current_node_alias {
        let node_alias = node_alias.clone();

        let node_id = match meta.get_node_id(&node_alias) {
            Some(id) => id,
            None => break,
        };

        let visit_key = (node_id.to_string(), current_field_name.clone());
        if visited.contains(&visit_key) {
            break;
        }
        visited.push(visit_key);

        let node_type = meta.get_node_type(node_id).to_string();
        let field_records = match meta.get_fields(node_id) {
            Some(recs) => recs,
            None => {
                steps.push(TraceStep {
                    node_alias: node_alias.clone(),
                    node_type: node_type.clone(),
                    field_name: format!("{} (字段未在当前节点声明)", current_field_name),
                    dbfield: "-".to_string(),
                    exp: None,
                });
                break;
            }
        };

        let field_rec = field_records.get(&current_field_name).or_else(|| {
            field_records.values().find(|f| {
                f.original_field.as_ref() == Some(&current_field_name)
                    || f.name == current_field_name
            })
        });

        if let Some(rec) = field_rec {
            current_field_name = rec.name.clone();
            current_field_dbfield = rec.dbfield.clone();
            current_exp = rec.exp.clone();
            current_node_alias = rec.original_node.clone();

            steps.push(TraceStep {
                node_alias: node_alias.clone(),
                node_type: node_type.clone(),
                field_name: current_field_name.clone(),
                dbfield: current_field_dbfield.clone(),
                exp: current_exp.clone(),
            });
        } else {
            steps.push(TraceStep {
                node_alias: node_alias.clone(),
                node_type: node_type.clone(),
                field_name: format!("{} (字段未在当前节点声明)", current_field_name),
                dbfield: "-".to_string(),
                exp: None,
            });
            break;
        }
    }

    steps
}
/// 展开 DataFlow 子图，追溯字段来源
pub fn query_dataflow(graph: &GraphDB, dataflow_id: &str, human: bool) -> Result<()> {
    let node = graph.get_node(dataflow_id);
    if node.is_none() {
        anyhow::bail!("DataFlow {} not found in graph", dataflow_id);
    }
    let node = node.unwrap();

    let raw_meta = node.meta.as_ref();
    let dfm = raw_meta.map(DataFlowMeta::from_meta).unwrap_or_default();

    let outgoing = graph
        .get_node_edges(dataflow_id)
        .map(|(out, _)| out)
        .unwrap_or_default();

    let inputs: Vec<_> = outgoing
        .iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::DataflowInput))
        .map(|&(n, e)| (n, e))
        .collect();

    let outputs: Vec<_> = outgoing
        .iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::OutputsTo))
        .map(|&(n, e)| (n, e))
        .collect();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== DataFlow: {} ===", dataflow_id)?;
        writeln!(out, "Name: {} | Path: {}", node.name, node.path)?;
        writeln!(
            out,
            "Model Type: {}",
            raw_meta
                .and_then(|m| m.get("modelType"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
        )?;

        writeln!(out, "\n--- 输入数据源 ({}个): ---", inputs.len())?;
        for (n, e) in &inputs {
            writeln!(
                out,
                "  [ModelTable] {} -> {}",
                n.name,
                e.field_path.as_deref().unwrap_or("-")
            )?;
        }

        writeln!(out, "\n--- 输出物理表 ({}个): ---", outputs.len())?;
        for (n, e) in &outputs {
            writeln!(
                out,
                "  [物理表] {}.tbl (dbTableName: {})",
                n.name,
                e.field_path.as_deref().unwrap_or("-")
            )?;
        }

        writeln!(out, "\n=== 字段级来源追溯 ===")?;

        let output_nodes = dfm.get_output_fields();
        let is_dimensions_fallback = output_nodes.iter().any(|(id, _)| *id == "default");

        if output_nodes.is_empty() {
            writeln!(out, "\n(未找到输出节点字段映射信息)")?;
        } else {
            for (node_id, fields) in output_nodes {
                let alias = dfm
                    .get_alias(node_id)
                    .map_or(node_id.to_string(), |v| v.clone());
                if is_dimensions_fallback {
                    writeln!(out, "\n--- 输出字段 (来自 dimensions) ---")?;
                } else {
                    writeln!(out, "\n--- 输出节点: {} ({}) ---", alias, node_id)?;
                }
                for (field_name, field_rec) in fields {
                    let mut visited: Vec<(String, String)> = Vec::new();
                    let trace = trace_field_source(field_name, field_rec, &dfm, &mut visited);

                    writeln!(out, "\n[字段] {}", field_name)?;

                    if trace.len() <= 1 {
                        writeln!(out, "  来源: (未记录来源关系)")?;
                        continue;
                    }

                    for (i, step) in trace.iter().enumerate() {
                        let is_output = i == 0;
                        let is_root = i == trace.len() - 1;
                        let prefix = if is_output {
                            "  输出"
                        } else if is_root {
                            "  根来源"
                        } else {
                            "  中间"
                        };
                        let exp_info = step
                            .exp
                            .as_ref()
                            .map(|e| format!(" [exp: {}]", e))
                            .unwrap_or_default();
                        let type_label = match step.node_type.as_str() {
                            "ModelTable" => "[数据表]",
                            "Select" => "[列加工]",
                            "Join" => "[关联]",
                            "Union" => "[联合]",
                            "Distinct" => "[去重]",
                            "Output" => "[输出]",
                            _ => "[节点]",
                        };
                        writeln!(
                            out,
                            "{} {} {}.{} (db={}){}",
                            prefix,
                            type_label,
                            step.node_alias,
                            step.field_name,
                            step.dbfield,
                            exp_info
                        )?;
                    }
                }
            }
        }

        writeln!(out, "\n=== 内部节点拓扑 ===")?;
        if dfm.internal_deps.is_empty() {
            writeln!(out, "(未记录内部节点依赖)")?;
        } else {
            for (node_id, deps) in &dfm.internal_deps {
                let alias = dfm
                    .get_alias(node_id)
                    .map(|a| a.as_str())
                    .unwrap_or(node_id);
                let dep_names: Vec<String> = deps
                    .iter()
                    .map(|d| {
                        dfm.get_alias(d)
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| d.clone())
                    })
                    .collect();
                writeln!(out, "  [{}] -> {}", alias, dep_names.join(", "))?;
            }
        }
    } else {
        let mut field_traces: Vec<serde_json::Value> = Vec::new();

        let output_nodes = dfm.get_output_fields();
        let is_dimensions_fallback = output_nodes.iter().any(|(id, _)| *id == "default");

        if output_nodes.is_empty() {
            field_traces.push(serde_json::json!({
                "trace_source": "missing",
                "diagnostics": "No output node fields or dimensions found",
            }));
        } else {
            for (node_id, fields) in output_nodes {
                let trace_source = if is_dimensions_fallback {
                    "dimensions_fallback"
                } else {
                    "output_node"
                };
                let alias = dfm
                    .get_alias(node_id)
                    .map_or(node_id.to_string(), |v| v.clone());

                for (field_name, field_rec) in fields {
                    let mut visited: Vec<(String, String)> = Vec::new();
                    let trace = trace_field_source(field_name, field_rec, &dfm, &mut visited);

                    field_traces.push(serde_json::json!({
                        "field": field_name,
                        "dbfield": field_rec.dbfield,
                        "output_node_id": node_id,
                        "output_node_alias": alias,
                        "trace_source": trace_source,
                        "trace": trace.iter().map(|s| serde_json::json!({
                            "node_alias": s.node_alias,
                            "node_type": s.node_type,
                            "field_name": s.field_name,
                            "dbfield": s.dbfield,
                            "exp": s.exp,
                        })).collect::<Vec<_>>(),
                    }));
                }
            }
        }

        let summary = serde_json::json!({
            "dataflow_id": dataflow_id,
            "name": node.name,
            "model_type": raw_meta.and_then(|m| m.get("modelType")).and_then(|v| v.as_str()).unwrap_or("DataFlow"),
            "input_count": inputs.len(),
            "output_count": outputs.len(),
        });

        let details = serde_json::json!({
            "inputs": inputs.iter().map(|(n, e)| serde_json::json!({
                "id": n.id,
                "name": n.name,
                "path": n.path,
                "field_path": e.field_path,
            })).collect::<Vec<_>>(),
            "outputs": outputs.iter().map(|(n, e)| serde_json::json!({
                "id": n.id,
                "name": n.name,
                "path": n.path,
                "field_path": e.field_path,
            })).collect::<Vec<_>>(),
            "field_traces": field_traces,
            "internal_deps": dfm.internal_deps.iter().map(|(id, deps)| serde_json::json!({
                "node_id": id,
                "depends_on": deps,
            })).collect::<Vec<_>>(),
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::DataFlowQuery, summary);
        output.query_target = Some(dataflow_id.to_string());
        output.details = Some(details);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "DataFlow {} has {} inputs and {} outputs",
                    dataflow_id,
                    inputs.len(),
                    outputs.len()
                ),
                "Parsed from DataFlow metadata",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(dataflow_id),
        );
        output.next_queries = vec![
            format!("--explain {} for semantic summary", dataflow_id),
            format!("--context {} --depth 2", dataflow_id),
        ];

        let output = output.validate();
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
