use crate::graph::GraphDB;
use anyhow::Result;
use std::io::{self, Write};

/// 图查询模块
///
/// 提供面向用户的查询接口，支持 human 可读格式和 JSON 格式：
///   - query_model：查询某个模型被哪些页面读取/写入
///   - query_page：查询某个页面的出边/入边关系
///   - query_cross：查询两个页面之间的跨文件关系
///   - query_dataflow：展开 DataFlow 的子图，做字段级来源追溯
///
/// 追溯节点所属的页面（通过 Contains 边）
fn find_parent_page(graph: &GraphDB, node_id: &str) -> Option<crate::graph::Node> {
    if let Some((_, incoming)) = graph.get_node_edges(node_id) {
        for (parent, edge) in incoming {
            if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
                if matches!(parent.node_type, crate::graph::NodeType::Page) {
                    return Some(parent.clone());
                }
                if let Some(page) = find_parent_page(graph, &parent.id) {
                    return Some(page);
                }
            }
            if matches!(edge.edge_type, crate::graph::EdgeType::Triggers)
                && let Some(page) = find_parent_page(graph, &parent.id)
            {
                return Some(page);
            }
        }
    }
    None
}

pub fn query_model(graph: &GraphDB, model_id: &str, human: bool) -> Result<()> {
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Model: {} ===", model_id)?;

        let readers = graph.find_readers(model_id);
        writeln!(
            out,
            "
--- Read By ({} nodes) ---",
            readers.len()
        )?;
        for (node, edge) in readers {
            let page = find_parent_page(graph, &node.id);
            let page_info = page
                .as_ref()
                .map(|p| format!(" (page: {})", p.name))
                .unwrap_or_default();
            writeln!(
                out,
                "  {} [{}] {}{} via {}",
                node.name,
                node.id,
                format!("{:?}", node.node_type).to_lowercase(),
                page_info,
                edge.field_path.as_deref().unwrap_or("-")
            )?;
        }

        let writers = graph.find_writers(model_id);
        writeln!(
            out,
            "
--- Written By ({} nodes) ---",
            writers.len()
        )?;
        for (node, edge) in writers {
            let page = find_parent_page(graph, &node.id);
            let page_info = page
                .as_ref()
                .map(|p| format!(" (page: {})", p.name))
                .unwrap_or_default();
            writeln!(
                out,
                "  {} [{}] {}{} via {}",
                node.name,
                node.id,
                format!("{:?}", node.node_type).to_lowercase(),
                page_info,
                edge.field_path.as_deref().unwrap_or("-")
            )?;
        }
    } else {
        let readers: Vec<_> = graph
            .find_readers(model_id)
            .into_iter()
            .map(|(n, e)| {
                let page = find_parent_page(graph, &n.id);
                serde_json::json!({
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "component_or_action": n.name,
                    "node_id": n.id,
                    "node_type": format!("{:?}", n.node_type),
                    "edge_type": "Reads",
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "meta": e.meta,
                })
            })
            .collect();
        let writers: Vec<_> = graph
            .find_writers(model_id)
            .into_iter()
            .map(|(n, e)| {
                let page = find_parent_page(graph, &n.id);
                serde_json::json!({
                    "page": page.as_ref().map(|p| p.name.clone()),
                    "page_id": page.as_ref().map(|p| p.id.clone()),
                    "component_or_action": n.name,
                    "node_id": n.id,
                    "node_type": format!("{:?}", n.node_type),
                    "edge_type": format!("{:?}", e.edge_type),
                    "field_path": e.field_path,
                    "source_file": n.path,
                    "meta": e.meta,
                })
            })
            .collect();
        let upstream = graph
            .find_upstream_dependencies(model_id)
            .into_iter()
            .map(|(n, e)| serde_json::json!({
                "node_id": n.id,
                "name": n.name,
                "node_type": format!("{:?}", n.node_type),
                "field_path": e.field_path,
                "source_file": n.path,
            }))
            .collect::<Vec<_>>();
        let downstream = graph
            .find_downstream_outputs(model_id)
            .into_iter()
            .map(|(n, e)| serde_json::json!({
                "node_id": n.id,
                "name": n.name,
                "node_type": format!("{:?}", n.node_type),
                "field_path": e.field_path,
                "source_file": n.path,
            }))
            .collect::<Vec<_>>();
        let result = serde_json::json!({
            "schema_version": "1.0",
            "model": model_id,
            "readers": readers,
            "writers": writers,
            "upstream_dependencies": upstream,
            "downstream_outputs": downstream,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

/// 查询页面的跨文件依赖关系
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

/// 查询两个页面之间的直接或间接关系
pub fn query_cross(graph: &GraphDB, page_a: &str, page_b: &str, human: bool) -> Result<()> {
    let paths = graph.find_cross_relations(page_a, page_b);
    if human {
        let mut out = io::stdout();
        writeln!(
            out,
            "=== Cross-File Relations: {} <-> {} ===",
            page_a, page_b
        )?;
        writeln!(out, "\nDirect Connections: {}", paths.len())?;
        for (i, path) in paths.iter().enumerate() {
            writeln!(out, "\n[Path {}]", i + 1)?;
            for (node, edge) in path {
                writeln!(
                    out,
                    "  {} ({:?}) -> {:?}",
                    node.name, node.node_type, edge.edge_type
                )?;
            }
        }
    } else {
        let json_paths: Vec<_> = paths
            .iter()
            .map(|path| {
                path.iter()
                    .map(|(n, e)| {
                        serde_json::json!({
                            "name": n.name,
                            "type": format!("{:?}", n.node_type),
                            "edge": format!("{:?}", e.edge_type),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let result = serde_json::json!({
            "page_a": page_a,
            "page_b": page_b,
            "paths": json_paths,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

mod dataflow;
pub use dataflow::query_dataflow;
