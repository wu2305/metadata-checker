use crate::graph::GraphDB;
use anyhow::Result;
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

pub fn query_dataflow(graph: &GraphDB, dataflow_id: &str, human: bool) -> Result<()> {
    let node = graph.get_node(dataflow_id);
    if node.is_none() {
        eprintln!("DataFlow {} not found in graph", dataflow_id);
        return Ok(());
    }
    let node = node.unwrap();

    let internal_deps = node.meta.as_ref()
        .and_then(|m| m.get("internalDeps"))
        .and_then(|v| v.as_object())
        .map(|obj| {
            let mut deps: Vec<(String, Vec<String>)> = Vec::new();
            for (k, v) in obj {
                if let Some(arr) = v.as_array() {
                    let dep_ids: Vec<String> = arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.to_string()))
                        .collect();
                    if !dep_ids.is_empty() {
                        deps.push((k.clone(), dep_ids));
                    }
                }
            }
            deps
        })
        .unwrap_or_default();

    let outgoing = graph.get_node_edges(dataflow_id)
        .map(|(out, _)| out)
        .unwrap_or_default();

    let inputs: Vec<_> = outgoing.iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::DataflowInput))
        .map(|(n, e)| (n.clone(), e.clone()))
        .collect();

    let outputs: Vec<_> = outgoing.iter()
        .filter(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::OutputsTo))
        .map(|(n, e)| (n.clone(), e.clone()))
        .collect();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== DataFlow: {} ===", dataflow_id)?;
        writeln!(out, "Name: {} | Path: {}", node.name, node.path)?;
        writeln!(out, "Model Type: {}", node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()).unwrap_or("Unknown"))?;

        writeln!(out, "\n--- Input Sources ({}): ---", inputs.len())?;
        for (n, e) in &inputs {
            writeln!(out, "  <- {} [{}] via {}", n.name, n.path, e.field_path.as_deref().unwrap_or("-"))?;
        }

        writeln!(out, "\n--- Output Targets ({}): ---", outputs.len())?;
        for (n, e) in &outputs {
            writeln!(out, "  -> {} [{}] via {}", n.name, n.path, e.field_path.as_deref().unwrap_or("-"))?;
        }

        writeln!(out, "\n--- Internal Node Dependencies ({}): ---", internal_deps.len())?;
        if internal_deps.is_empty() {
            writeln!(out, "  (No internal dependency information recorded)")?;
        } else {
            for (node_id, deps) in &internal_deps {
                writeln!(out, "  {} -> {}", node_id, deps.join(", "))?;
            }
        }
    } else {
        let result = serde_json::json!({
            "dataflow": dataflow_id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
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
            "internal_deps": internal_deps.iter().map(|(id, deps)| serde_json::json!({
                "node_id": id,
                "depends_on": deps,
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
