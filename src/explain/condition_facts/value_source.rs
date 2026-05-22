use crate::graph_store::GraphReadStore;

use super::conditions::{component_json_paths, is_ancestor_json_path};

fn component_property<'a>(node: &'a crate::graph::Node, key: &str) -> Option<&'a str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get("properties"))
        .and_then(|p| p.get(key))
        .and_then(|v| v.as_str())
        .or_else(|| {
            node.meta
                .as_ref()
                .and_then(|m| m.get(key))
                .and_then(|v| v.as_str())
        })
}

fn component_meta_string<'a>(node: &'a crate::graph::Node, key: &str) -> Option<&'a str> {
    node.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
}

/// 识别 `${FIELD}` 这种依赖当前行数据上下文的裸字段引用
fn extract_single_bare_symbol(raw_expr: &str) -> Option<String> {
    let trimmed = raw_expr.trim();
    let inner = trimmed
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))?
        .trim();
    if inner.is_empty() || inner.contains('.') {
        return None;
    }
    if inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Some(inner.to_string())
    } else {
        None
    }
}

fn nearest_data_context_component(
    graph: &dyn GraphReadStore,
    page_path: &str,
    target_json_path: &str,
) -> Option<crate::graph::Node> {
    let mut best: Option<(usize, crate::graph::Node)> = None;
    let nodes = graph.iter_nodes().ok()?;
    for node in nodes {
        if !matches!(node.node_type, crate::graph::NodeType::Component) || node.path != page_path {
            continue;
        }
        let has_data_context = component_meta_string(&node, "dataSet").is_some()
            || component_meta_string(&node, "source").is_some();
        if !has_data_context {
            continue;
        }
        let Some(ctx_path) = component_meta_string(&node, "json_path") else {
            continue;
        };
        if !is_ancestor_json_path(ctx_path, target_json_path) {
            continue;
        }
        let score = ctx_path.len();
        if best
            .as_ref()
            .map_or(true, |(best_score, _)| score > *best_score)
        {
            best = Some((score, node));
        }
    }
    best.map(|(_, node)| node)
}

fn model_from_table_path(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
}

fn dataflow_field_origin(
    dataflow_node: &crate::graph::Node,
    bare_symbol: &str,
) -> Option<serde_json::Value> {
    let meta = dataflow_node.meta.as_ref()?;
    let dfm = crate::query::DataFlowMeta::from_meta(meta);
    let projection = crate::query::project_output_field_origin(&dfm, bare_symbol);
    Some(projection.to_json())
}

fn build_value_source_context_from_derived_edge(
    graph: &dyn GraphReadStore,
    target_node: &crate::graph::Node,
) -> Option<serde_json::Value> {
    let neighbors = graph.get_node_edges(&target_node.id).ok().flatten()?;
    let (field_node, edge) = neighbors
        .outgoing
        .iter()
        .find(|edge_view| {
            let node = &edge_view.node;
            let edge = &edge_view.edge;
            let Some(meta) = edge.meta.as_ref() else {
                return false;
            };
            if !matches!(edge.edge_type, crate::graph::EdgeType::Reads)
                || !node.id.starts_with("field:")
                || meta.get("resolution").and_then(|v| v.as_str())
                    != Some("inherited_container_data_context")
            {
                return false;
            }
            let data_set = meta
                .get("target_model")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let bare_symbol = meta
                .get("bare_symbol")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            node.id == format!("field:{}.{}", data_set, bare_symbol)
        })
        .or_else(|| {
            neighbors.outgoing.iter().find(|edge_view| {
                let node = &edge_view.node;
                let edge = &edge_view.edge;
                matches!(edge.edge_type, crate::graph::EdgeType::Reads)
                    && node.id.starts_with("field:")
                    && edge
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("resolution"))
                        .and_then(|v| v.as_str())
                        == Some("inherited_container_data_context")
            })
        })
        .map(|edge_view| (&edge_view.node, &edge_view.edge))?;
    let meta = edge.meta.as_ref()?;
    let raw_expr = meta.get("source_expr").and_then(|v| v.as_str())?;
    let bare_symbol = meta.get("bare_symbol").and_then(|v| v.as_str())?;
    let data_set = meta.get("target_model").and_then(|v| v.as_str())?;
    let context_component_id = meta
        .get("data_context_component_id")
        .and_then(|v| v.as_str())?;
    let context_node_id = format!("comp:{}|{}", target_node.path, context_component_id);
    let context_node = graph.get_node(&context_node_id).ok().flatten();
    let data_set_model_id = format!("model:{}", data_set);
    let data_set_model = graph.get_node(&data_set_model_id).ok().flatten();
    let data_set_model_path = meta
        .get("target_model_path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| data_set_model.as_ref().map(|n| n.path.clone()));
    let dataflow_model = data_set_model_path
        .as_deref()
        .and_then(model_from_table_path)
        .map(|model| format!("model:{}", model))
        .and_then(|model_id| graph.get_node(&model_id).ok().flatten());
    let dataflow_field_origin = dataflow_model
        .as_ref()
        .and_then(|node| dataflow_field_origin(node, bare_symbol));
    let dataflow_inputs = dataflow_model
        .as_ref()
        .map(|node| {
            graph
                .get_node_edges(&node.id)
                .ok()
                .flatten()
                .map(|neighbors| {
                    neighbors
                        .outgoing
                        .into_iter()
                        .filter(|edge_view| {
                            matches!(
                                edge_view.edge.edge_type,
                                crate::graph::EdgeType::DataflowInput
                            )
                        })
                        .map(|edge_view| {
                            let input = edge_view.node;
                            let edge = edge_view.edge;
                            serde_json::json!({
                                "node_id": input.id,
                                "name": input.name,
                                "source_file": input.path,
                                "edge_type": format!("{:?}", edge.edge_type),
                                "field_path": edge.field_path,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    Some(serde_json::json!({
        "source_component": target_node.id,
        "source_field": meta.get("source_field").and_then(|v| v.as_str()).unwrap_or("value"),
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "resolution": "inherited_container_data_context",
        "graph_edge": {
            "edge_type": format!("{:?}", edge.edge_type),
            "field_path": edge.field_path,
            "target_id": field_node.id,
        },
        "nearest_data_context": {
            "node_id": context_node.as_ref().map(|node| node.id.clone()),
            "component_id": context_component_id,
            "component_type": context_node.as_ref().and_then(|node| component_meta_string(node, "component_type")),
            "json_path": meta.get("data_context_json_path").and_then(|v| v.as_str()),
            "source": meta.get("data_context_source").and_then(|v| v.as_str()),
            "dataSet": data_set,
        },
        "data_set_model": data_set_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": data_set_model_path.as_deref().unwrap_or(&node.path),
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "field_path": format!("{}.{}", data_set, bare_symbol),
        "table_source_path": data_set_model_path,
        "dataflow_model": dataflow_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "dataflow_field_origin": dataflow_field_origin,
        "dataflow_inputs": dataflow_inputs,
    }))
}

pub(in crate::explain) fn build_value_source_context(
    graph: &dyn GraphReadStore,
    target_node: &crate::graph::Node,
    page_path: &str,
) -> Option<serde_json::Value> {
    if !matches!(target_node.node_type, crate::graph::NodeType::Component) {
        return None;
    }
    if let Some(ctx) = build_value_source_context_from_derived_edge(graph, target_node) {
        return Some(ctx);
    }
    let raw_expr = component_property(target_node, "value")?;
    let bare_symbol = extract_single_bare_symbol(raw_expr)?;
    let target_json_path = component_meta_string(target_node, "json_path")
        .map(|path| format!("{}.value", path))
        .or_else(|| {
            component_json_paths(graph, &target_node.id)
                .into_iter()
                .next()
        })?;
    let context_node = nearest_data_context_component(graph, page_path, &target_json_path)?;
    let data_set = component_meta_string(&context_node, "dataSet")
        .or_else(|| component_property(&context_node, "dataSet"))?;
    let data_set_model_id = format!("model:{}", data_set);
    let data_set_model = graph.get_node(&data_set_model_id).ok().flatten();
    let data_set_model_path = data_set_model.as_ref().map(|n| n.path.clone());

    let dataflow_model = data_set_model_path
        .as_deref()
        .and_then(model_from_table_path)
        .map(|model| format!("model:{}", model))
        .and_then(|model_id| graph.get_node(&model_id).ok().flatten());
    let dataflow_field_origin = dataflow_model
        .as_ref()
        .and_then(|node| dataflow_field_origin(node, &bare_symbol));
    let dataflow_inputs = dataflow_model
        .as_ref()
        .map(|node| {
            graph
                .get_node_edges(&node.id)
                .ok()
                .flatten()
                .map(|neighbors| {
                    neighbors
                        .outgoing
                        .into_iter()
                        .filter(|edge_view| {
                            matches!(
                                edge_view.edge.edge_type,
                                crate::graph::EdgeType::DataflowInput
                            )
                        })
                        .map(|edge_view| {
                            let input = edge_view.node;
                            let edge = edge_view.edge;
                            serde_json::json!({
                                "node_id": input.id,
                                "name": input.name,
                                "source_file": input.path,
                                "edge_type": format!("{:?}", edge.edge_type),
                                "field_path": edge.field_path,
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();

    Some(serde_json::json!({
        "source_component": target_node.id,
        "source_field": "value",
        "raw_expr": raw_expr,
        "bare_symbol": bare_symbol,
        "resolution": "nearest_data_context_container",
        "nearest_data_context": {
            "node_id": context_node.id,
            "component_id": context_node.name,
            "component_type": component_meta_string(&context_node, "component_type"),
            "json_path": component_meta_string(&context_node, "json_path"),
            "source": component_meta_string(&context_node, "source"),
            "dataSet": data_set,
        },
        "data_set_model": data_set_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "field_path": format!("{}.{}", data_set, bare_symbol),
        "table_source_path": data_set_model_path,
        "dataflow_model": dataflow_model.as_ref().map(|node| serde_json::json!({
            "node_id": node.id,
            "name": node.name,
            "path": node.path,
            "model_type": node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str()),
        })),
        "dataflow_field_origin": dataflow_field_origin,
        "dataflow_inputs": dataflow_inputs,
    }))
}
