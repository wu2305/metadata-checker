use crate::graph::{GraphDB, Node};
use crate::path::{PathFinder, PathSelector};

/// 页面逻辑输出中的路径选择结果
pub(super) struct PageLogicPaths {
    pub(super) primary_paths: Vec<serde_json::Value>,
    pub(super) related_context: Vec<serde_json::Value>,
    pub(super) candidate_paths: Vec<serde_json::Value>,
    pub(super) supporting_paths: Vec<serde_json::Value>,
    pub(super) rejected_paths: Vec<serde_json::Value>,
    pub(super) path_selection_diagnostics: Vec<String>,
}

/// 计算页面主链路、候选链路和旁路上下文
pub(super) fn build_page_logic_paths(
    graph: &GraphDB,
    page_id: &str,
    page_node: &Node,
    child_components: &Vec<&Node>,
    child_actions: &Vec<&Node>,
    data_sources: &Vec<serde_json::Value>,
    write_targets: &Vec<serde_json::Value>,
    entrypoints: &Vec<serde_json::Value>,
) -> PageLogicPaths {
    let path_query = crate::path::AnchorExtractor::extract(
        graph,
        page_id,
        page_node,
        child_components,
        child_actions,
        data_sources,
        write_targets,
        entrypoints,
    );

    let finder = crate::path::BoundedCausalPathFinder::default();
    let mut candidates = finder.find_candidates(graph, &path_query);

    for ds in data_sources {
        let field_candidates =
            crate::path::build_field_causal_paths_for_data_source(graph, page_node, ds);
        candidates.extend(field_candidates);
    }

    {
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.path_id.clone()));
    }

    let selector = crate::path::RuleBasedPathSelector;
    let selection = selector.select(&path_query, candidates);

    let mut primary_paths = Vec::new();
    let mut related_context = Vec::new();
    let mut candidate_paths = Vec::new();
    let mut supporting_paths = Vec::new();
    let mut rejected_paths = Vec::new();
    let mut path_selection_diagnostics = Vec::new();

    for p in selection.primary_paths {
        primary_paths.push(p.to_json());
    }
    for p in selection.candidate_paths {
        candidate_paths.push(p.to_json());
    }
    for p in selection.supporting_paths {
        supporting_paths.push(p.to_json());
    }
    for p in selection.related_context {
        related_context.push(p.to_json());
    }
    for p in selection.rejected_paths {
        rejected_paths.push(p.to_json());
    }
    for d in selection.selection_diagnostics {
        path_selection_diagnostics.push(d);
    }

    collect_cross_page_side_context(graph, page_id, child_components, &mut related_context);
    sort_primary_paths_by_confidence(&mut primary_paths);

    PageLogicPaths {
        primary_paths,
        related_context,
        candidate_paths,
        supporting_paths,
        rejected_paths,
        path_selection_diagnostics,
    }
}

fn collect_cross_page_side_context(
    graph: &GraphDB,
    page_id: &str,
    child_components: &[&Node],
    related_context: &mut Vec<serde_json::Value>,
) {
    let mut seen_ctx: std::collections::HashSet<String> = std::collections::HashSet::new();
    for comp in child_components {
        if let Some((comp_out, _comp_in)) = graph.get_node_edges(&comp.id) {
            for (target, edge) in comp_out {
                if target.id.starts_with("page:") && target.id != page_id {
                    let ctx_key = format!("{}->{}", comp.id, target.id);
                    if !seen_ctx.contains(&ctx_key) {
                        seen_ctx.insert(ctx_key.clone());
                        related_context.push(serde_json::json!({
                            "from": comp.id,
                            "to": target.id,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": comp.path.clone(),
                            "confidence": "low",
                            "reason": "跨页面旁路关系",
                        }));
                    }
                }
            }
        }
    }
}

fn sort_primary_paths_by_confidence(primary_paths: &mut [serde_json::Value]) {
    primary_paths.sort_by(|a, b| {
        let a_conf = a
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let b_conf = b
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        match (a_conf, b_conf) {
            ("high", "medium") | ("high", "low") | ("medium", "low") => std::cmp::Ordering::Less,
            ("medium", "high") | ("low", "high") | ("low", "medium") => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
}
