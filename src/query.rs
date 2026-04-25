use crate::graph::GraphDB;
use anyhow::Result;
use std::collections::HashMap;
use std::io::{self, Write};

pub fn query_model(graph: &GraphDB, model_id: &str, human: bool) -> Result<()> {
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Model: {} ===", model_id)?;

        let readers = graph.find_readers(model_id);
        writeln!(out, "\n--- Read By ({} pages) ---", readers.len())?;
        for (node, edge) in readers {
            writeln!(out, "  {} [{}] via {}", node.name, node.path, edge.field_path.as_deref().unwrap_or("-"))?;
        }

        let writers = graph.find_writers(model_id);
        writeln!(out, "\n--- Written By ({} pages) ---", writers.len())?;
        for (node, edge) in writers {
            writeln!(out, "  {} [{}] via {}", node.name, node.path, edge.field_path.as_deref().unwrap_or("-"))?;
        }
    } else {
        let readers: Vec<_> = graph.find_readers(model_id)
            .into_iter()
            .map(|(n, e)| serde_json::json!({"node": n.name, "path": n.path, "field": e.field_path}))
            .collect();
        let writers: Vec<_> = graph.find_writers(model_id)
            .into_iter()
            .map(|(n, e)| serde_json::json!({"node": n.name, "path": n.path, "field": e.field_path}))
            .collect();
        let result = serde_json::json!({
            "model": model_id,
            "readers": readers,
            "writers": writers,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

pub fn query_page(graph: &GraphDB, page_id: &str, human: bool) -> Result<()> {
    if let Some((outgoing, incoming)) = graph.get_node_edges(page_id) {
        if human {
            let mut out = io::stdout();
            writeln!(out, "=== Page: {} ===", page_id)?;
            writeln!(out, "\n--- Outgoing Edges ({}): ---", outgoing.len())?;
            for (node, edge) in outgoing {
                writeln!(out, "  -> {} ({})", node.name, format!("{:?}", edge.edge_type))?;
            }
            writeln!(out, "\n--- Incoming Edges ({}): ---", incoming.len())?;
            for (node, edge) in incoming {
                writeln!(out, "  <- {} ({})", node.name, format!("{:?}", edge.edge_type))?;
            }
        } else {
            let result = serde_json::json!({
                "page": page_id,
                "outgoing": outgoing.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
                "incoming": incoming.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    } else {
        eprintln!("Page {} not found in graph", page_id);
    }
    Ok(())
}

pub fn query_cross(graph: &GraphDB, page_a: &str, page_b: &str, human: bool) -> Result<()> {
    let paths = graph.find_cross_relations(page_a, page_b);
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Cross-File Relations: {} <-> {} ===", page_a, page_b)?;
        writeln!(out, "\nDirect Connections: {}", paths.len())?;
        for (i, path) in paths.iter().enumerate() {
            writeln!(out, "\n[Path {}]", i + 1)?;
            for (node, edge) in path {
                writeln!(out, "  {} ({:?}) -> {:?}", node.name, node.node_type, edge.edge_type)?;
            }
        }
    } else {
        let json_paths: Vec<_> = paths.iter().map(|path| {
            path.iter().map(|(n, e)| serde_json::json!({
                "name": n.name,
                "type": format!("{:?}", n.node_type),
                "edge": format!("{:?}", e.edge_type),
            })).collect::<Vec<_>>()
        }).collect();
        let result = serde_json::json!({
            "page_a": page_a,
            "page_b": page_b,
            "paths": json_paths,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

// ============================================================
// DataFlow 字段级来源追溯
// ============================================================

#[derive(Debug, Clone)]
struct FieldRecord {
    name: String,
    dbfield: String,
    original_field: Option<String>,
    original_node: Option<String>,
    exp: Option<String>,
}

#[derive(Debug, Clone)]
struct TraceStep {
    node_alias: String,
    node_type: String,
    field_name: String,
    dbfield: String,
    exp: Option<String>,
}

fn load_field_records(node_fields: &HashMap<String, Vec<serde_json::Value>>, node_id: &str) -> HashMap<String, FieldRecord> {
    let mut result = HashMap::new();
    if let Some(fields) = node_fields.get(node_id) {
        for field in fields {
            let name = field.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let dbfield = field.get("dbfield").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let original_field = field.get("originalField").and_then(|v| v.as_str()).map(|s| s.to_string());
            let original_node = field.get("originalNode").and_then(|v| v.as_str()).map(|s| s.to_string());
            let exp = field.get("exp").and_then(|v| v.as_str()).map(|s| s.to_string());
            result.insert(name.clone(), FieldRecord {
                name,
                dbfield,
                original_field,
                original_node,
                exp,
            });
        }
    }
    result
}

fn trace_field_source(
    output_field_name: &str,
    output_field: &FieldRecord,
    alias_map: &HashMap<String, String>,
    node_fields: &HashMap<String, Vec<serde_json::Value>>,
    nodes_json: &HashMap<String, serde_json::Value>,
    visited: &mut Vec<String>,
) -> Vec<TraceStep> {
    let mut steps = Vec::new();
    
    // Current step: output field
    let mut current_field_name = output_field_name.to_string();
    let mut current_node_alias: Option<String> = output_field.original_node.clone();
    let mut current_field_dbfield = output_field.dbfield.clone();
    let mut current_exp = output_field.exp.clone();
    
    // Add output step
    steps.push(TraceStep {
        node_alias: "模型输出".to_string(),
        node_type: "Output".to_string(),
        field_name: current_field_name.clone(),
        dbfield: current_field_dbfield.clone(),
        exp: current_exp.clone(),
    });
    
    // Trace backwards through originalNode
    loop {
        let node_alias = match &current_node_alias {
            Some(a) => a.clone(),
            None => break,
        };
        
        if visited.contains(&current_field_name) {
            break;
        }
        visited.push(current_field_name.clone());
        
        let node_id = match alias_map.get(&node_alias) {
            Some(id) => id,
            None => break,
        };
        
        let node_type = nodes_json.get(node_id)
            .and_then(|n| n.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        
        let field_records = load_field_records(node_fields, node_id);
        
        // Try to find the field by originalField name first, then by name
        let lookup_name = current_field_name.clone();
        let field_rec = field_records.get(&lookup_name)
            .or_else(|| {
                // Try matching by originalField if the field name differs
                field_records.values().find(|f| {
                    f.original_field.as_ref() == Some(&current_field_name) ||
                    f.name == current_field_name
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
            // Field not found in this node, record what we know and stop
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

pub fn query_dataflow(graph: &GraphDB, dataflow_id: &str, human: bool) -> Result<()> {
    let node = graph.get_node(dataflow_id);
    if node.is_none() {
        eprintln!("DataFlow {} not found in graph", dataflow_id);
        return Ok(());
    }
    let node = node.unwrap();

    let meta = node.meta.as_ref();
    
    // Extract metadata components
    let alias_map: HashMap<String, String> = meta
        .and_then(|m| m.get("aliasMap"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    
    let node_fields: HashMap<String, Vec<serde_json::Value>> = meta
        .and_then(|m| m.get("nodeFields"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    
    let internal_deps: HashMap<String, Vec<String>> = meta
        .and_then(|m| m.get("internalDeps"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    
    // We need the raw nodes JSON for node types - reconstruct from nodeFields keys + aliasMap
    let node_types: HashMap<String, String> = meta
        .and_then(|m| m.get("nodeTypes"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    
    let mut nodes_json: HashMap<String, serde_json::Value> = HashMap::new();
    for (alias, node_id) in &alias_map {
        let node_type = node_types.get(node_id).map(|s| s.as_str()).unwrap_or("Unknown");
        nodes_json.insert(node_id.clone(), serde_json::json!({"type": node_type, "alias": alias}));
    }
    // Also include output node (default)
    let default_type = node_types.get("default").map(|s| s.as_str()).unwrap_or("Output");
    nodes_json.insert("default".to_string(), serde_json::json!({"type": default_type, "alias": "模型输出"}));
    if !node_fields.contains_key("default") {
        // Try to get output fields from dimensions as fallback
    }

    let outgoing = graph.get_node_edges(dataflow_id)
        .map(|(out, _)| out)
        .unwrap_or_default();

    let inputs: Vec<_> = outgoing.iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::DataflowInput))
        .map(|&(n, e)| (n, e))
        .collect();

    let outputs: Vec<_> = outgoing.iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::OutputsTo))
        .map(|&(n, e)| (n, e))
        .collect();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== DataFlow: {} ===", dataflow_id)?;
        writeln!(out, "Name: {} | Path: {}", node.name, node.path)?;
        writeln!(out, "Model Type: {}", meta.and_then(|m| m.get("modelType")).and_then(|v| v.as_str()).unwrap_or("Unknown"))?;

        // --- Input Sources ---
        writeln!(out, "\n--- 输入数据源 ({}个): ---", inputs.len())?;
        for (n, e) in &inputs {
            writeln!(out, "  [ModelTable] {} -> {}", n.name, e.field_path.as_deref().unwrap_or("-"))?;
        }

        // --- Output Targets ---
        writeln!(out, "\n--- 输出物理表 ({}个): ---", outputs.len())?;
        for (n, e) in &outputs {
            writeln!(out, "  [物理表] {}.tbl (dbTableName: {})", n.name, e.field_path.as_deref().unwrap_or("-"))?;
        }

        // --- Field-level Source Tracing ---
        writeln!(out, "\n=== 字段级来源追溯 ===")?;
        
        // Get output node fields (default node)
        let output_fields = load_field_records(&node_fields, "default");
        
        if output_fields.is_empty() {
            writeln!(out, "\n(未找到输出节点字段映射信息)")?;
        } else {
            for (field_name, field_rec) in &output_fields {
                let mut visited = Vec::new();
                let trace = trace_field_source(
                    field_name, field_rec, &alias_map, &node_fields, &nodes_json, &mut visited
                );
                
                writeln!(out, "\n[字段] {}", field_name)?;
                
                if trace.len() <= 1 {
                    writeln!(out, "  来源: (未记录来源关系)")?;
                    continue;
                }
                
                // Print trace chain: Output -> ... -> Root Source
                for (i, step) in trace.iter().enumerate() {
                    let is_output = i == 0;
                    let is_root = i == trace.len() - 1;
                    let prefix = if is_output { "  输出" } else if is_root { "  根来源" } else { "  中间" };
                    let exp_info = step.exp.as_ref().map(|e| format!(" [exp: {}]", e)).unwrap_or_default();
                    let type_label = match step.node_type.as_str() {
                        "ModelTable" => "[数据表]",
                        "Select" => "[列加工]",
                        "Join" => "[关联]",
                        "Union" => "[联合]",
                        "Distinct" => "[去重]",
                        "Output" => "[输出]",
                        _ => "[节点]",
                    };
                    writeln!(out, "{} {} {}.{} (db={}){}", 
                        prefix, type_label, step.node_alias, step.field_name, step.dbfield, exp_info)?;
                }
            }
        }

        // --- Internal Node Topology ---
        writeln!(out, "\n=== 内部节点拓扑 ===")?;
        if internal_deps.is_empty() {
            writeln!(out, "(未记录内部节点依赖)")?;
        } else {
            // Build reverse alias map for display
            let id_to_alias: HashMap<&String, &String> = alias_map.iter()
                .map(|(a, id)| (id, a))
                .collect();
            
            for (node_id, deps) in &internal_deps {
                let alias = id_to_alias.get(node_id).map(|a| a.as_str()).unwrap_or(node_id);
                let dep_names: Vec<String> = deps.iter()
                    .map(|d| id_to_alias.get(d).map(|a| a.to_string()).unwrap_or_else(|| d.clone()))
                    .collect();
                writeln!(out, "  [{}] -> {}", alias, dep_names.join(", "))?;
            }
        }
    } else {
        // JSON mode: keep existing structure but add field trace info
        let mut field_traces: Vec<serde_json::Value> = Vec::new();
        
        let output_fields = load_field_records(&node_fields, "default");
        for (field_name, field_rec) in &output_fields {
            let mut visited = Vec::new();
            let trace = trace_field_source(
                field_name, field_rec, &alias_map, &node_fields, &nodes_json, &mut visited
            );
            
            field_traces.push(serde_json::json!({
                "field": field_name,
                "dbfield": field_rec.dbfield,
                "trace": trace.iter().map(|s| serde_json::json!({
                    "node_alias": s.node_alias,
                    "node_type": s.node_type,
                    "field_name": s.field_name,
                    "dbfield": s.dbfield,
                    "exp": s.exp,
                })).collect::<Vec<_>>(),
            }));
        }
        
        let result = serde_json::json!({
            "dataflow": dataflow_id,
            "name": node.name,
            "path": node.path,
            "model_type": meta.and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
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
            "internal_deps": internal_deps.iter().map(|(id, deps)| serde_json::json!({
                "node_id": id,
                "depends_on": deps,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
