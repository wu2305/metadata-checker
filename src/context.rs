use crate::graph::GraphDB;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::io::{self, Write};

/// 输出目标节点周围的最小闭包上下文
///
/// 支持 --depth <N> 和 --budget compact|normal|full。
pub fn context_node_graph(
    graph: &GraphDB,
    node_id: &str,
    depth: usize,
    budget: &str,
    human: bool,
) -> Result<()> {
    let budget_str = budget.to_lowercase();
    if budget_str != "compact" && budget_str != "normal" && budget_str != "full" {
        anyhow::bail!(
            "Invalid budget '{}'. Expected: compact | normal | full",
            budget
        );
    }
    let node = graph
        .get_node(node_id)
        .ok_or_else(|| anyhow::anyhow!("Node '{}' not found in graph", node_id))?;

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut upstream = Vec::new();
    let mut downstream = Vec::new();
    let mut related_actions = Vec::new();
    let mut related_models = Vec::new();
    let mut related_pages = Vec::new();

    visited.insert(node_id.to_string());
    queue.push_back((node_id.to_string(), 0usize));

    while let Some((current_id, current_depth)) = queue.pop_front() {
        if current_depth >= depth {
            continue;
        }

        if let Some((outgoing, incoming)) = graph.get_node_edges(&current_id) {
            for (target, edge) in outgoing {
                if !visited.contains(&target.id) {
                    visited.insert(target.id.clone());
                    queue.push_back((target.id.clone(), current_depth + 1));
                }

                let entry = json!({
                    "from": current_id.clone(),
                    "to": target.id,
                    "name": target.name,
                    "type": format!("{:?}", target.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                });

                match edge.edge_type {
                    crate::graph::EdgeType::Reads => {
                        downstream.push(entry.clone());
                        if matches!(
                            target.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry);
                        }
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        downstream.push(entry.clone());
                        if matches!(
                            target.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry);
                        }
                    }
                    crate::graph::EdgeType::Triggers => {
                        downstream.push(entry.clone());
                        related_actions.push(entry);
                    }
                    crate::graph::EdgeType::Contains => {
                        if matches!(target.node_type, crate::graph::NodeType::Page) {
                            related_pages.push(entry);
                        }
                    }
                    _ => {
                        downstream.push(entry);
                    }
                }
            }

            for (source, edge) in incoming {
                if !visited.contains(&source.id) {
                    visited.insert(source.id.clone());
                    queue.push_back((source.id.clone(), current_depth + 1));
                }

                let entry = json!({
                    "from": source.id,
                    "to": current_id.clone(),
                    "name": source.name,
                    "type": format!("{:?}", source.node_type),
                    "edge_type": format!("{:?}", edge.edge_type),
                    "field_path": edge.field_path,
                });

                match edge.edge_type {
                    crate::graph::EdgeType::Reads => {
                        upstream.push(entry.clone());
                        if matches!(
                            source.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry);
                        }
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        upstream.push(entry.clone());
                        if matches!(
                            source.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry);
                        }
                    }
                    crate::graph::EdgeType::Triggers => {
                        upstream.push(entry.clone());
                        related_actions.push(entry);
                    }
                    crate::graph::EdgeType::Contains => {
                        if matches!(source.node_type, crate::graph::NodeType::Page) {
                            related_pages.push(entry);
                        }
                    }
                    _ => {
                        upstream.push(entry);
                    }
                }
            }
        }
    }

    // Budget-based truncation
    let (upstream_out, downstream_out, actions_out, models_out) = match budget {
        "compact" => {
            let limit = 5usize;
            (
                upstream.iter().take(limit).cloned().collect::<Vec<Value>>(),
                downstream
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
                related_actions
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
                related_models
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
            )
        }
        "full" => (
            upstream.clone(),
            downstream.clone(),
            related_actions.clone(),
            related_models.clone(),
        ),
        _ => {
            let limit = 20usize;
            (
                upstream.iter().take(limit).cloned().collect::<Vec<Value>>(),
                downstream
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
                related_actions
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
                related_models
                    .iter()
                    .take(limit)
                    .cloned()
                    .collect::<Vec<Value>>(),
            )
        }
    };

    let truncated = upstream.len() > upstream_out.len()
        || downstream.len() > downstream_out.len()
        || related_actions.len() > actions_out.len()
        || related_models.len() > models_out.len();

    let mut diagnostics = Vec::new();
    if truncated {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "OUTPUT_TRUNCATED".to_string(),
            message: "Output truncated due to budget limit".to_string(),
            location: crate::output::Location::new(),
            suggestion: Some(
                "Use --budget full or increase --depth to see more relations".to_string(),
            ),
        });
    }

    let summary = json!({
        "center_node": node_id,
        "depth": depth,
        "budget": budget,
        "related_nodes_count": visited.len() - 1,
        "truncated": truncated,
    });

    let details = json!({
        "upstream": upstream_out,
        "downstream": downstream_out,
        "related_actions": actions_out,
        "related_models": models_out,
        "related_pages": related_pages,
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Context, summary);
    output.query_target = Some(node_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Context closure around {} with depth {}", node_id, depth),
            "BFS traversal of graph edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(node_id)
        .with_source_file(&node.path),
    );
    output.next_queries = vec![
        format!("--explain {} for semantic summary", node_id),
        format!(
            "--context {} --depth {} --budget full for full closure",
            node_id,
            depth + 1
        ),
    ];

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Context: {} ===", node_id)?;
        writeln!(
            out,
            "Depth: {} | Budget: {} | Related Nodes: {}",
            depth,
            budget,
            visited.len() - 1
        )?;
        if truncated {
            writeln!(out, "⚠️  Output truncated due to budget limit")?;
        }
        writeln!(out, "\n--- Upstream ({}) ---", upstream_out.len())?;
        for u in &upstream_out {
            writeln!(out, "  {:?}", u)?;
        }
        writeln!(out, "\n--- Downstream ({}) ---", downstream_out.len())?;
        for d in &downstream_out {
            writeln!(out, "  {:?}", d)?;
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
