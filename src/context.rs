use crate::graph::GraphDB;
use anyhow::Result;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
use std::io::{self, Write};

/// 输出目标节点周围的最小闭包上下文
///
/// 支持 --depth <N> 和 --budget compact|normal|full。
/// 根据目标类型生成后续查询建议
fn generate_next_queries(
    node_id: &str,
    node_type: &crate::graph::NodeType,
    depth: usize,
) -> Vec<String> {
    let mut queries = vec![format!("--explain {} for semantic summary", node_id)];
    match node_type {
        crate::graph::NodeType::Component | crate::graph::NodeType::Action => {
            // 提取页面 ID（comp:page|id 或 action:page|comp|id 格式）
            if let Some(page_part) = node_id.split('|').next() {
                if let Some(page_id) = page_part.strip_prefix("comp:") {
                    queries.push(format!(
                        "--query-page-logic page:{} for page-level logic",
                        page_id
                    ));
                } else if let Some(page_id) = page_part.strip_prefix("action:") {
                    queries.push(format!(
                        "--query-page-logic page:{} for page-level logic",
                        page_id
                    ));
                }
            }
        }
        crate::graph::NodeType::Model | crate::graph::NodeType::Field => {
            if node_id.starts_with("field:") {
                if let Some(model_part) = node_id
                    .strip_prefix("field:")
                    .and_then(|s| s.split('.').next())
                {
                    queries.push(format!("--query-model {} for model details", model_part));
                }
            } else if let Some(model_name) = node_id.strip_prefix("model:") {
                queries.push(format!("--query-model {} for model details", model_name));
                queries.push(format!(
                    "--query-dataflow {} for dataflow lineage",
                    model_name
                ));
            }
        }
        crate::graph::NodeType::Page => {
            queries.push(format!(
                "--query-page-logic {} for page-level logic",
                node_id
            ));
        }
    }
    queries.push(format!(
        "--context {} --depth {} --budget full for full closure",
        node_id,
        depth + 1
    ));
    queries
}

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
    let mut related_components = Vec::new();

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
                            related_models.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        downstream.push(entry.clone());
                        if matches!(
                            target.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Triggers => {
                        downstream.push(entry.clone());
                        related_actions.push(entry.clone());
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Contains => {
                        if matches!(target.node_type, crate::graph::NodeType::Page) {
                            related_pages.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::SetsParam => {
                        downstream.push(entry.clone());
                        related_components.push(entry);
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
                            related_models.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        upstream.push(entry.clone());
                        if matches!(
                            source.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Triggers => {
                        upstream.push(entry.clone());
                        related_actions.push(entry.clone());
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::Contains => {
                        if matches!(source.node_type, crate::graph::NodeType::Page) {
                            related_pages.push(entry.clone());
                        } else if matches!(
                            source.node_type,
                            crate::graph::NodeType::Model | crate::graph::NodeType::Field
                        ) {
                            related_models.push(entry.clone());
                        }
                        related_components.push(entry);
                    }
                    crate::graph::EdgeType::OpensPage | crate::graph::EdgeType::SetsParam => {
                        upstream.push(entry.clone());
                        related_components.push(entry);
                    }
                    _ => {
                        upstream.push(entry);
                    }
                }
            }
        }
    }

    // Budget-based truncation
    let (upstream_out, downstream_out, actions_out, models_out, pages_out, components_out) =
        match budget {
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
                    related_pages
                        .iter()
                        .take(limit)
                        .cloned()
                        .collect::<Vec<Value>>(),
                    related_components
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
                related_pages.clone(),
                related_components.clone(),
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
                    related_pages
                        .iter()
                        .take(limit)
                        .cloned()
                        .collect::<Vec<Value>>(),
                    related_components
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
        || related_models.len() > models_out.len()
        || related_pages.len() > pages_out.len()
        || related_components.len() > components_out.len();

    let mut diagnostics = Vec::new();
    if truncated {
        let mut truncated_cats = Vec::new();
        if upstream.len() > upstream_out.len() {
            truncated_cats.push(format!(
                "upstream ({}/{})",
                upstream_out.len(),
                upstream.len()
            ));
        }
        if downstream.len() > downstream_out.len() {
            truncated_cats.push(format!(
                "downstream ({}/{})",
                downstream_out.len(),
                downstream.len()
            ));
        }
        if related_actions.len() > actions_out.len() {
            truncated_cats.push(format!(
                "related_actions ({}/{})",
                actions_out.len(),
                related_actions.len()
            ));
        }
        if related_models.len() > models_out.len() {
            truncated_cats.push(format!(
                "related_models ({}/{})",
                models_out.len(),
                related_models.len()
            ));
        }
        if related_pages.len() > pages_out.len() {
            truncated_cats.push(format!(
                "related_pages ({}/{})",
                pages_out.len(),
                related_pages.len()
            ));
        }
        if related_components.len() > components_out.len() {
            truncated_cats.push(format!(
                "related_components ({}/{})",
                components_out.len(),
                related_components.len()
            ));
        }
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "OUTPUT_TRUNCATED".to_string(),
            message: format!(
                "Output truncated due to budget '{}': {}",
                budget,
                truncated_cats.join(", ")
            ),
            location: crate::output::Location::new(),
            suggestion: Some(
                "Use --budget full or increase --depth to see more relations".to_string(),
            ),
        });
    }

    let high_value_relations = {
        let mut counts = serde_json::Map::new();
        counts.insert("upstream".to_string(), json!(upstream_out.len()));
        counts.insert("downstream".to_string(), json!(downstream_out.len()));
        counts.insert("related_actions".to_string(), json!(actions_out.len()));
        counts.insert("related_models".to_string(), json!(models_out.len()));
        counts.insert("related_pages".to_string(), json!(pages_out.len()));
        counts.insert(
            "related_components".to_string(),
            json!(components_out.len()),
        );
        Value::Object(counts)
    };

    let summary = json!({
        "center_node": node_id,
        "center_type": format!("{:?}", node.node_type).to_lowercase(),
        "depth": depth,
        "budget": budget,
        "related_nodes_count": visited.len().saturating_sub(1),
        "truncated": truncated,
        "high_value_relations": high_value_relations,
    });

    // Build lineage for field targets
    let mut lineage: Vec<serde_json::Value> = Vec::new();
    if matches!(node.node_type, crate::graph::NodeType::Field) {
        let field_meta = node.meta.as_ref();
        if let Some(input_field) = field_meta
            .and_then(|m| m.get("source_input_field"))
            .and_then(|v| v.as_str())
        {
            let model_name = node
                .id
                .strip_prefix("field:")
                .and_then(|s| s.split('.').next())
                .unwrap_or("");
            lineage.push(json!({
                "target_field": node.id,
                "source_fields": [format!("field:{}.{}", model_name, input_field)],
                "source_expr": null,
                "transform": "inputField mapping",
                "via_node": null,
                "confidence": "high",
            }));
        }
        if let Some(expr) = field_meta
            .and_then(|m| m.get("source_expr"))
            .and_then(|v| v.as_str())
        {
            let refs = crate::superpage::parse_expression_refs(expr);
            let source_fields: Vec<String> = refs
                .iter()
                .filter_map(|r| match r {
                    crate::superpage::RefType::ModelField(m, f) => {
                        Some(format!("field:{}.{}", m, f))
                    }
                    _ => None,
                })
                .collect();
            lineage.push(json!({
                "target_field": node.id,
                "source_fields": source_fields,
                "source_expr": expr,
                "transform": "expression calculation",
                "via_node": null,
                "confidence": if source_fields.is_empty() { "low" } else { "medium" },
            }));
        }
        // Collect page action writes from incoming edges
        if let Some((_outgoing, incoming)) = graph.get_node_edges(node_id) {
            for (source, edge) in &incoming {
                if matches!(
                    edge.edge_type,
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites
                ) {
                    let source_expr = edge
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("source_expr"))
                        .and_then(|v| v.as_str());
                    let mut source_fields: Vec<String> = Vec::new();
                    let mut resolved_refs: Vec<serde_json::Value> = Vec::new();
                    let mut has_unresolved = false;
                    if let Some(expr) = source_expr {
                        let refs = crate::superpage::parse_expression_refs(expr);
                        if refs.is_empty() {
                            has_unresolved = true;
                        }
                        for r in refs {
                            let (ref_type_str, ref_id, confidence) = match &r {
                                crate::superpage::RefType::ModelField(m, f) => {
                                    let fid = format!("field:{}.{}", m, f);
                                    source_fields.push(fid.clone());
                                    ("ModelField", fid, "high")
                                }
                                crate::superpage::RefType::ComponentValue(c) => {
                                    ("ComponentValue", format!("comp:{}", c), "high")
                                }
                                crate::superpage::RefType::ComponentProperty(c, p) => {
                                    ("ComponentProperty", format!("comp:{}.{}", c, p), "high")
                                }
                                crate::superpage::RefType::Param(p) => {
                                    ("Param", format!("param:{}", p), "high")
                                }
                                crate::superpage::RefType::UserProperty(p) => {
                                    ("UserProperty", format!("user:{}", p), "medium")
                                }
                                crate::superpage::RefType::SystemVar(v) => {
                                    ("SystemVar", v.clone(), "high")
                                }
                                crate::superpage::RefType::Other(o) => {
                                    has_unresolved = true;
                                    ("Other", o.clone(), "low")
                                }
                            };
                            resolved_refs.push(json!({
                                "ref_type": ref_type_str,
                                "ref_id": ref_id,
                                "confidence": confidence,
                            }));
                        }
                    }
                    let confidence = if source_expr.is_some() && has_unresolved {
                        "medium"
                    } else if source_expr.is_some() && resolved_refs.is_empty() {
                        "low"
                    } else {
                        "high"
                    };
                    lineage.push(json!({
                        "target_field": node.id,
                        "source_fields": source_fields,
                        "source_expr": source_expr,
                        "resolved_refs": resolved_refs,
                        "transform": "page action write",
                        "via_node": source.id.clone(),
                        "confidence": confidence,
                    }));
                }
            }
        }
    }

    let details = if lineage.is_empty() {
        json!({
            "upstream": upstream_out,
            "downstream": downstream_out,
            "related_actions": actions_out,
            "related_models": models_out,
            "related_pages": pages_out,
            "related_components": components_out,
        })
    } else {
        json!({
            "upstream": upstream_out,
            "downstream": downstream_out,
            "related_actions": actions_out,
            "related_models": models_out,
            "related_pages": pages_out,
            "related_components": components_out,
            "lineage": lineage,
        })
    };

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Context, summary);
    output.query_target = Some(node_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics;
    output.evidence.push(
        crate::output::Evidence::new(
            format!("Context closure around {} with depth {}", node_id, depth),
            "BFS traversal of graph edges from center node",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(node_id)
        .with_source_file(&node.path),
    );
    if !upstream_out.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Found {} upstream dependencies", upstream_out.len()),
                "BFS traversal: incoming edges",
            )
            .with_confidence(crate::output::Confidence::High),
        );
    }
    if !downstream_out.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Found {} downstream dependencies", downstream_out.len()),
                "BFS traversal: outgoing edges",
            )
            .with_confidence(crate::output::Confidence::High),
        );
    }
    output.next_queries = generate_next_queries(node_id, &node.node_type, depth);

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
