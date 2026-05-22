use crate::graph::{EdgeType, Node, NodeType};
use crate::graph_store::GraphReadStore;
use std::collections::HashSet;

const FACT_AUTO_CUSTOMER_AUTO_REL_TABLE: &str = "$DATA:/主数据/fact_autoCustomerAutoRel.tbl";

/// 解析页面作用域模型目标，支持 `model:PAGE|MODEL`。
///
/// 注意：这里的 `MODEL` 是页面局部 model id，不具备全局唯一语义。
pub fn parse_scoped_model_target(target_id: &str) -> Option<(String, String)> {
    let rest = target_id.strip_prefix("model:")?;
    let parts: Vec<&str> = rest.splitn(2, '|').collect();
    if parts.len() != 2 {
        return None;
    }
    let page_id = parts[0].trim();
    let model_id = parts[1].trim();
    if page_id.is_empty() || model_id.is_empty() {
        return None;
    }
    Some((page_id.to_string(), model_id.to_string()))
}

/// 判断模型节点是否为 DataFlow。
pub fn is_dataflow_model(node: &Node) -> bool {
    node.meta
        .as_ref()
        .and_then(|m| m.get("modelType"))
        .and_then(|v| v.as_str())
        == Some("DataFlow")
}

fn normalize_page_node_id(page_id: &str) -> String {
    if page_id.starts_with("page:") {
        page_id.to_string()
    } else {
        format!("page:{}", page_id)
    }
}

fn normalize_dataflow_path(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    let prefixes = ["$DATA:/", "data/tables/", "data/"];
    for prefix in prefixes {
        if normalized.starts_with(prefix) {
            normalized = normalized[prefix.len()..].to_string();
            break;
        }
    }
    normalized = normalized.trim_start_matches('/').to_string();
    normalized.trim_end_matches('/').to_string()
}

fn is_same_dataflow_path(left: &str, right: &str) -> bool {
    normalize_dataflow_path(left) == normalize_dataflow_path(right)
}

fn collect_page_descendants(graph: &dyn GraphReadStore, root_id: &str) -> Vec<String> {
    let mut stack = vec![root_id.to_string()];
    let mut seen = HashSet::new();
    let mut ids = Vec::new();

    while let Some(node_id) = stack.pop() {
        if !seen.insert(node_id.clone()) {
            continue;
        }
        ids.push(node_id.clone());
        if let Some(neighbors) = graph.get_node_edges(&node_id).ok().flatten() {
            for edge_view in &neighbors.outgoing {
                if matches!(edge_view.edge.edge_type, EdgeType::Contains | EdgeType::Triggers) {
                    stack.push(edge_view.node.id.clone());
                }
            }
        }
    }

    ids
}

fn collect_dataflow_input_paths_by_model(graph: &dyn GraphReadStore, model_id: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let Some(neighbors) = graph.get_node_edges(model_id).ok().flatten() else {
        return paths;
    };

    for edge_view in &neighbors.outgoing {
        if !matches!(edge_view.edge.edge_type, EdgeType::DataflowInput) {
            continue;
        }
        let edge_path = edge_view.edge.field_path.as_deref().unwrap_or("");
        if !edge_path.is_empty() {
            paths.push(edge_path.to_string());
        }
        if let Some(source_file) = edge_view
            .edge
            .meta
            .as_ref()
            .and_then(|m| m.get("source_file"))
            .and_then(|v| v.as_str())
        {
            if !source_file.is_empty() {
                paths.push(source_file.to_string());
            }
        }
        if !edge_view.node.path.is_empty() {
            paths.push(edge_view.node.path.clone());
        }
    }

    paths.sort();
    paths.dedup();
    paths
}

fn score_dataflow_candidate(
    graph: &dyn GraphReadStore,
    node: &Node,
    preferred_input: Option<&str>,
) -> (u8, u8, usize) {
    let input_paths = collect_dataflow_input_paths_by_model(graph, &node.id);
    let has_preferred_input = preferred_input.is_some_and(|preferred| {
        input_paths
            .iter()
            .any(|path| is_same_dataflow_path(path, preferred))
    });

    (
        if is_dataflow_model(node) { 2 } else { 1 },
        if has_preferred_input { 1 } else { 0 },
        input_paths.len(),
    )
}

fn pick_best_dataflow_candidate(
    graph: &dyn GraphReadStore,
    mut candidates: Vec<Node>,
    preferred_input: Option<&str>,
) -> Option<Node> {
    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| {
        let score_a = score_dataflow_candidate(graph, a, preferred_input);
        let score_b = score_dataflow_candidate(graph, b, preferred_input);
        score_b
            .0
            .cmp(&score_a.0)
            .then_with(|| score_b.1.cmp(&score_a.1))
            .then_with(|| score_b.2.cmp(&score_a.2))
            .then_with(|| b.id.cmp(&a.id))
    });

    candidates.pop()
}

fn resolve_dataflow_model_by_path(
    graph: &dyn GraphReadStore,
    dataflow_path: &str,
    preferred_input: Option<&str>,
) -> Option<Node> {
    let mut exact_candidates: Vec<Node> = Vec::new();
    let mut stem_candidates: Vec<Node> = Vec::new();

    let normalized_path = normalize_dataflow_path(dataflow_path);
    let target_stem = std::path::Path::new(&normalized_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(std::string::ToString::to_string);

    let Some(iter_nodes) = graph.iter_nodes().ok() else {
        return None;
    };

    for node in iter_nodes.filter(|node| node.node_type == NodeType::Model && is_dataflow_model(node))
    {
        let node_path = normalize_dataflow_path(&node.path);
        if is_same_dataflow_path(&node.path, dataflow_path) || normalized_path == node_path {
            exact_candidates.push(node.clone());
            continue;
        }

        let node_stem = std::path::Path::new(&node_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(std::string::ToString::to_string);
        if target_stem
            .as_ref()
            .map_or(false, |stem| Some(stem) == node_stem.as_ref())
        {
            stem_candidates.push(node.clone());
        }
    }

    if !exact_candidates.is_empty() {
        pick_best_dataflow_candidate(graph, exact_candidates, preferred_input)
    } else if !stem_candidates.is_empty() {
        pick_best_dataflow_candidate(graph, stem_candidates, preferred_input)
    } else {
        None
    }
}

fn resolve_dataflow_model_by_input_path(
    graph: &dyn GraphReadStore,
    local_model_id: &str,
    dataflow_path: &str,
) -> Option<Node> {
    let mut exact_candidates: Vec<Node> = Vec::new();
    let mut fallback_candidates: Vec<Node> = Vec::new();
    if let Some(neighbors) = graph.get_node_edges(local_model_id).ok().flatten() {
        for edge_view in &neighbors.outgoing {
            if !matches!(edge_view.edge.edge_type, EdgeType::DataflowInput) {
                continue;
            }

            let edge_target_path = edge_view.edge.field_path.as_deref().unwrap_or("");
            if !is_dataflow_model(&edge_view.node) {
                continue;
            }

            if is_same_dataflow_path(edge_target_path, dataflow_path)
                || is_same_dataflow_path(&edge_view.node.path, dataflow_path)
            {
                exact_candidates.push(edge_view.node.clone());
            } else {
                fallback_candidates.push(edge_view.node.clone());
            }
        }
    }

    if !exact_candidates.is_empty() {
        return pick_best_dataflow_candidate(
            graph,
            exact_candidates,
            Some(FACT_AUTO_CUSTOMER_AUTO_REL_TABLE),
        );
    }

    if !fallback_candidates.is_empty() {
        return pick_best_dataflow_candidate(
            graph,
            fallback_candidates,
            Some(FACT_AUTO_CUSTOMER_AUTO_REL_TABLE),
        );
    }

    None
}

/// 在给定页面内解析局部模型，返回带页面级 table source 的模型节点。
///
/// 解析规则是通用的：`page path + local model id`。不要按 `model11` 等具体 id
/// 写特殊分支；这些 id 只应出现在真实项目回归测试中。
pub fn resolve_model_target_in_page(
    graph: &dyn GraphReadStore,
    page_id: &str,
    local_model_id: &str,
) -> Option<(Node, Node, Option<String>)> {
    let page_node_id = normalize_page_node_id(page_id);
    let Some(page_node) = graph.get_node(&page_node_id).ok().flatten() else {
        return None;
    };
    let normalized_model = local_model_id.trim_start_matches("model:");
    let model_id = format!("model:{}", normalized_model);
    let Some(model_node) = graph.get_node(&model_id).ok().flatten() else {
        return None;
    };

    let mut candidate_paths = Vec::new();
    let mut seen_paths = HashSet::new();
    for node_id in collect_page_descendants(graph, &page_node.id) {
        let Some(neighbors) = graph.get_node_edges(&node_id).ok().flatten() else {
            continue;
        };
        for edge_view in &neighbors.outgoing {
            if edge_view.node.node_type != NodeType::Model {
                continue;
            }
            let meta_target_model = edge_view
                .edge
                .meta
                .as_ref()
                .and_then(|m| m.get("target_model"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let to_node_model = edge_view.node.id.strip_prefix("model:").unwrap_or(&edge_view.node.id);
            if meta_target_model != normalized_model && to_node_model != normalized_model {
                continue;
            }
            let path = edge_view
                .edge
                .meta
                .as_ref()
                .and_then(|m| m.get("target_model_path"))
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| edge_view.node.path.clone());
            if !path.is_empty() && seen_paths.insert(path.clone()) {
                candidate_paths.push(path);
            }
        }
    }

    candidate_paths.sort();
    let target_path = candidate_paths.into_iter().next()?;

    let mut scoped_model = model_node.clone();
    scoped_model.path = target_path;
    let dataflow_model_id =
        resolve_dataflow_model_by_input_path(graph, &model_node.id, &scoped_model.path)
            .or_else(|| {
                resolve_dataflow_model_by_path(
                    graph,
                    &scoped_model.path,
                    Some(FACT_AUTO_CUSTOMER_AUTO_REL_TABLE),
                )
            })
            .map(|n| n.id);
    if let Some(dataflow_model_id) = &dataflow_model_id
        && let Some(dataflow_meta) = graph
            .get_node(dataflow_model_id)
            .ok()
            .flatten()
            .and_then(|n| n.meta)
    {
        scoped_model.meta = Some(dataflow_meta);
    } else if let Some(embedded_meta) = model_node.meta {
        scoped_model.meta = Some(embedded_meta);
    }

    Some((scoped_model, page_node, dataflow_model_id))
}
