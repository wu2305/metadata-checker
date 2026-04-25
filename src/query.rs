use crate::graph::{GraphDB, EdgeType};
use anyhow::Result;
use std::io::{self, Write};

pub fn query_model(graph: &GraphDB, model_id: &str, human: bool) -> Result<()> {
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Model: {} ===", model_id)?;

        let readers = graph.find_readers(model_id);
        writeln!(out, "\n--- Read By ({} pages) ---", readers.len())?;
        for (node, edge) in readers {
            writeln!(out, "  {} [{}] via {:?}", node.name, node.path, edge.field_path)?;
        }

        let writers = graph.find_writers(model_id);
        writeln!(out, "\n--- Written By ({} pages) ---", writers.len())?;
        for (node, edge) in writers {
            writeln!(out, "  {} [{}] via {:?}", node.name, node.path, edge.field_path)?;
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
                writeln!(out, "  -> {} ({:?})", node.name, edge.edge_type)?;
            }
            writeln!(out, "\n--- Incoming Edges ({}): ---", incoming.len())?;
            for (node, edge) in incoming {
                writeln!(out, "  <- {} ({:?})", node.name, edge.edge_type)?;
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