use crate::graph::{Edge, Node};
use crate::graph_store::GraphReadStore;
use crate::output::schema::format_next_query;
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

/// 适配 GraphReadStore::get_node_edges 到 query 内部使用的元组格式
fn get_node_edges_compat(
    graph: &dyn GraphReadStore,
    node_id: &str,
) -> Result<Option<(Vec<(Node, Edge)>, Vec<(Node, Edge)>)>> {
    Ok(graph.get_node_edges(node_id)?.map(|neighbors| {
        let outgoing = neighbors
            .outgoing
            .into_iter()
            .map(|v| (v.node, v.edge))
            .collect();
        let incoming = neighbors
            .incoming
            .into_iter()
            .map(|v| (v.node, v.edge))
            .collect();
        (outgoing, incoming)
    }))
}

/// 搜索与目标 ID 相似的候选节点（诊断层 helper，不依赖 GraphDB 具体类型）
pub fn find_candidates(
    graph: &dyn GraphReadStore,
    target_id: &str,
    limit: usize,
) -> Result<Vec<(crate::graph::Node, String)>> {
    let mut candidates = Vec::new();
    let target_lower = target_id.to_lowercase();

    let target_prefix = if target_id.starts_with("model:") {
        Some("model")
    } else if target_id.starts_with("page:") {
        Some("page")
    } else if target_id.starts_with("comp:") {
        Some("comp")
    } else if target_id.starts_with("action:") {
        Some("action")
    } else if target_id.starts_with("field:") {
        Some("field")
    } else {
        None
    };

    let target_bare = target_id
        .strip_prefix("model:")
        .or_else(|| target_id.strip_prefix("page:"))
        .or_else(|| target_id.strip_prefix("comp:"))
        .or_else(|| target_id.strip_prefix("action:"))
        .or_else(|| target_id.strip_prefix("field:"))
        .unwrap_or(target_id);

    for node in graph.iter_nodes()? {
        if node.id == target_id || node.id.trim().is_empty() || node.name.trim().is_empty() {
            continue;
        }
        let node_bare = node
            .id
            .strip_prefix("model:")
            .or_else(|| node.id.strip_prefix("page:"))
            .or_else(|| node.id.strip_prefix("comp:"))
            .or_else(|| node.id.strip_prefix("action:"))
            .or_else(|| node.id.strip_prefix("field:"))
            .unwrap_or(&node.id);

        let mut score: f64 = 0.0;
        let mut reason = "substring match";

        if let Some(prefix) = target_prefix {
            if node.id.starts_with(prefix) {
                if node_bare.to_lowercase() == target_bare.to_lowercase() {
                    score = 100.0;
                    reason = "exact bare name match";
                } else if node_bare
                    .to_lowercase()
                    .contains(&target_bare.to_lowercase())
                    || target_bare
                        .to_lowercase()
                        .contains(&node_bare.to_lowercase())
                {
                    score = 80.0;
                    reason = "bare name substring match";
                } else if node.id.to_lowercase().contains(&target_lower)
                    || target_lower.contains(&node.id.to_lowercase())
                {
                    score = 60.0;
                    reason = "full id substring match";
                }
            }
            if node_bare.starts_with(target_bare) && node_bare != target_bare {
                if node_bare
                    .trim_start_matches(target_bare)
                    .parse::<f64>()
                    .is_ok()
                {
                    score = score.max(40.0_f64);
                    reason = "numbered suffix variant";
                }
            }
            if node_bare.starts_with(target_bare) {
                if let Some(rest) = node_bare.strip_prefix(target_bare) {
                    if rest.parse::<f64>().is_ok() {
                        score = score.max(40.0_f64);
                        reason = "numbered suffix variant";
                    }
                }
            }
            if score == 0.0 && target_id.starts_with("comp:") && node.id.starts_with("comp:") {
                score = 5.0;
                reason = "same prefix (component)";
            }
        } else {
            if node.name.to_lowercase() == target_lower || node.id.to_lowercase() == target_lower {
                score = 100.0;
                reason = "exact match";
            } else if node.name.to_lowercase().contains(&target_lower)
                || node.id.to_lowercase().contains(&target_lower)
            {
                score = 80.0;
                reason = "name/id substring match";
            } else if node.path.to_lowercase().contains(&target_lower) {
                score = 50.0;
                reason = "path substring match";
            }
        }

        if score > 0.0 {
            candidates.push((node.clone(), score, reason.to_string()));
        }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let result: Vec<_> = candidates
        .into_iter()
        .filter(|(n, _, _)| seen.insert(n.id.clone()))
        .take(limit)
        .map(|(n, _, r)| (n, r))
        .collect();
    Ok(result)
}

/// 追溯节点所属的页面（通过 Contains 边）
pub fn find_parent_page(
    graph: &dyn GraphReadStore,
    node_id: &str,
) -> Result<Option<crate::graph::Node>> {
    if let Some((_, incoming)) = get_node_edges_compat(graph, node_id)? {
        for (parent, edge) in incoming {
            if matches!(edge.edge_type, crate::graph::EdgeType::Contains) {
                if matches!(parent.node_type, crate::graph::NodeType::Page) {
                    return Ok(Some(parent.clone()));
                }
                if let Ok(Some(page)) = find_parent_page(graph, &parent.id) {
                    return Ok(Some(page));
                }
            }
            if matches!(edge.edge_type, crate::graph::EdgeType::Triggers)
                && let Ok(Some(page)) = find_parent_page(graph, &parent.id)
            {
                return Ok(Some(page));
            }
        }
    }
    Ok(None)
}

/// 构建 query_page 输出（返回 Value，不打印）
pub fn build_query_page_output(
    graph: &dyn GraphReadStore,
    page_id: &str,
) -> Result<serde_json::Value> {
    if let Some((outgoing, incoming)) = get_node_edges_compat(graph, page_id)? {
        let summary = serde_json::json!({
            "page_id": page_id,
            "outgoing_count": outgoing.len(),
            "incoming_count": incoming.len(),
            "relation_types": outgoing.iter().map(|(_, e)| format!("{:?}", e.edge_type)).collect::<std::collections::HashSet<String>>().into_iter().collect::<Vec<String>>(),
        });

        let details = serde_json::json!({
            "outgoing": outgoing.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
            "incoming": incoming.iter().map(|(n, e)| serde_json::json!({"name": n.name, "type": format!("{:?}", e.edge_type)})).collect::<Vec<_>>(),
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::PageQuery, summary);
        output.query_target = Some(page_id.to_string());
        output.details = Some(details);
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} has {} outgoing edges", page_id, outgoing.len()),
                "Graph traversal: outgoing edges from page node",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id),
        );
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} has {} incoming edges", page_id, incoming.len()),
                "Graph traversal: incoming edges to page node",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id),
        );
        output.next_queries = vec![format_next_query(
            "--query-page-logic {} for page-level logic summary",
            page_id,
        )];

        let output = output.validate();
        Ok(serde_json::to_value(output)?)
    } else {
        let candidates = find_candidates(graph, page_id, 5)?;
        let out = crate::output::schema::build_target_not_found_output(
            crate::output::schema::OutputKind::PageQuery,
            page_id,
            &candidates,
        );
        Ok(serde_json::to_value(out)?)
    }
}

/// 查询页面的跨文件依赖关系（保留旧入口，直接打印 stdout）
pub fn query_page(graph: &dyn GraphReadStore, page_id: &str, human: bool) -> Result<()> {
    if let Some((outgoing, incoming)) = get_node_edges_compat(graph, page_id)? {
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
            let val = build_query_page_output(graph, page_id)?;
            println!("{}", serde_json::to_string_pretty(&val)?);
        }
    } else {
        let val = build_query_page_output(graph, page_id)?;
        println!("{}", serde_json::to_string_pretty(&val)?);
        return Ok(());
    }
    Ok(())
}

/// 查找两个页面之间的共同依赖路径（从 GraphDB 方法下沉为独立函数）
pub fn find_cross_relations(
    graph: &dyn GraphReadStore,
    page_a: &str,
    page_b: &str,
) -> Result<Vec<Vec<(crate::graph::Node, crate::graph::Edge)>>> {
    let neighbors_a = get_node_edges_compat(graph, page_a)?;
    let neighbors_b = get_node_edges_compat(graph, page_b)?;

    let mut paths = Vec::new();
    if let (Some((out_a, _)), Some((out_b, _))) = (neighbors_a, neighbors_b) {
        let a_targets: std::collections::HashSet<String> =
            out_a.iter().map(|(n, _)| n.id.clone()).collect();
        let b_targets: std::collections::HashSet<String> =
            out_b.iter().map(|(n, _)| n.id.clone()).collect();

        for target_id in a_targets.intersection(&b_targets) {
            if let (Some(node_a), Some(_node_b)) =
                (graph.get_node(page_a)?, graph.get_node(page_b)?)
            {
                if let Some(target_node) = graph.get_node(target_id)? {
                    let edge_a = out_a
                        .iter()
                        .find(|(n, _)| &n.id == target_id)
                        .map(|(_, e)| e.clone());
                    let edge_b = out_b
                        .iter()
                        .find(|(n, _)| &n.id == target_id)
                        .map(|(_, e)| e.clone());
                    if let (Some(ea), Some(eb)) = (edge_a, edge_b) {
                        paths.push(vec![(node_a.clone(), ea), (target_node.clone(), eb)]);
                    }
                }
            }
        }
    }
    Ok(paths)
}

/// 构建 query_cross 输出（返回 Value，不打印）
pub fn build_query_cross_output(
    graph: &dyn GraphReadStore,
    page_a: &str,
    page_b: &str,
) -> Result<serde_json::Value> {
    let paths = find_cross_relations(graph, page_a, page_b)?;
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

    let summary = serde_json::json!({
        "page_a": page_a,
        "page_b": page_b,
        "path_count": paths.len(),
    });

    let details = serde_json::json!({
        "paths": json_paths,
    });

    let mut output =
        crate::output::AiOutput::new(crate::output::OutputKind::CrossPageQuery, summary);
    output.query_target = Some(format!("{} <-> {}", page_a, page_b));
    output.details = Some(details);
    output.evidence.push(
        crate::output::Evidence::new(
            format!(
                "Cross-page query found {} paths between {} and {}",
                paths.len(),
                page_a,
                page_b
            ),
            "Graph traversal: find_cross_relations",
        )
        .with_confidence(crate::output::Confidence::High),
    );
    for (i, path) in paths.iter().take(5).enumerate() {
        let nodes: Vec<String> = path.iter().map(|(n, _)| n.name.clone()).collect();
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Path {}: {}", i + 1, nodes.join(" -> ")),
                "Graph traversal: individual cross-page path",
            )
            .with_confidence(crate::output::Confidence::High),
        );
    }
    output.next_queries = vec![
        format_next_query("--query-page {} for page dependencies", page_a),
        format_next_query("--query-page {} for page dependencies", page_b),
    ];

    let output = output.validate();
    Ok(serde_json::to_value(output)?)
}

/// 查询两个页面之间的直接或间接关系（保留旧入口，直接打印 stdout）
pub fn query_cross(
    graph: &dyn GraphReadStore,
    page_a: &str,
    page_b: &str,
    human: bool,
) -> Result<()> {
    let paths = find_cross_relations(graph, page_a, page_b)?;
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

        let summary = serde_json::json!({
            "page_a": page_a,
            "page_b": page_b,
            "path_count": paths.len(),
        });

        let details = serde_json::json!({
            "paths": json_paths,
        });

        let mut output =
            crate::output::AiOutput::new(crate::output::OutputKind::CrossPageQuery, summary);
        output.query_target = Some(format!("{} <-> {}", page_a, page_b));
        output.details = Some(details);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Cross-page query found {} paths between {} and {}",
                    paths.len(),
                    page_a,
                    page_b
                ),
                "Graph traversal: find_cross_relations",
            )
            .with_confidence(crate::output::Confidence::High),
        );
        for (i, path) in paths.iter().take(5).enumerate() {
            let nodes: Vec<String> = path.iter().map(|(n, _)| n.name.clone()).collect();
            output.evidence.push(
                crate::output::Evidence::new(
                    format!("Path {}: {}", i + 1, nodes.join(" -> ")),
                    "Graph traversal: individual cross-page path",
                )
                .with_confidence(crate::output::Confidence::High),
            );
        }
        output.next_queries = vec![
            format_next_query("--query-page {} for page dependencies", page_a),
            format_next_query("--query-page {} for page dependencies", page_b),
        ];

        let output = output.validate();
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}

mod dataflow;
mod model;
pub use dataflow::DataFlowMeta;
pub use dataflow::DataflowFieldOriginProjection;
pub use dataflow::DataflowFilterClause;
pub use dataflow::DataflowFilterProjection;
pub use dataflow::DataflowJoinCondition;
pub use dataflow::DataflowUnionMapEntry;
pub use dataflow::build_query_dataflow_output;
pub use dataflow::build_via_value;
pub use dataflow::project_output_field_origin;
pub use dataflow::query_dataflow;
pub use model::build_query_model_output;
pub use model::query_model;

mod page_logic;
#[cfg(feature = "cli-local")]
pub use page_logic::collect_page_logic_nodes_for_test;
pub use page_logic::query_page_logic;
pub use page_logic::{
    MaterializedAvailabilityFactsIndex, PageDependencyIndex, PageLogicAvailabilityCache,
    build_page_logic_availability_cache, build_query_page_logic_output,
    build_query_page_logic_output_profiled,
    build_query_page_logic_output_profiled_with_availability_cache,
    build_query_page_logic_output_profiled_with_dense_snapshot,
    build_query_page_logic_output_profiled_with_materialized_availability,
    build_query_page_logic_output_with_availability_cache,
    build_query_page_logic_output_with_dense_snapshot,
};
/// 返回匹配节点列表，按名称相似度排序。
pub fn find_nodes(
    graph: &dyn GraphReadStore,
    keyword: &str,
    node_type_filter: Option<&str>,
    limit: usize,
) -> Result<crate::output::schema::AiOutput> {
    let keyword_lower = keyword.to_lowercase();
    let mut matches = Vec::new();

    for node in graph.iter_nodes()? {
        let node_type_str = format!("{:?}", node.node_type).to_lowercase();
        if let Some(filter) = node_type_filter {
            if !node_type_str.contains(filter) {
                continue;
            }
        }
        let score = if node.id.to_lowercase() == keyword_lower
            || node.name.to_lowercase() == keyword_lower
        {
            100.0
        } else if node.id.to_lowercase().contains(&keyword_lower)
            || node.name.to_lowercase().contains(&keyword_lower)
        {
            80.0
        } else if node.path.to_lowercase().contains(&keyword_lower) {
            50.0
        } else {
            0.0
        };
        if score > 0.0 {
            matches.push((node.clone(), score));
        }
    }

    matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let matches: Vec<_> = matches
        .into_iter()
        .filter(|(n, _)| seen.insert(n.id.clone()))
        .take(limit)
        .collect();

    let match_count = matches.len();
    let match_details: Vec<serde_json::Value> = matches
        .iter()
        .map(|(n, score)| {
            serde_json::json!({
                "id": n.id,
                "name": n.name,
                "node_type": format!("{:?}", n.node_type),
                "source_file": n.path,
                "match_score": score,
                "match_reason": if *score >= 100.0 {
                    "exact match"
                } else if *score >= 80.0 {
                    "name/id substring match"
                } else {
                    "path match"
                },
            })
        })
        .collect();

    let kind = match node_type_filter {
        Some("page") => crate::output::schema::OutputKind::PageQuery,
        Some("model") => crate::output::schema::OutputKind::ModelQuery,
        Some("component") => crate::output::schema::OutputKind::ComponentQuery,
        _ => crate::output::schema::OutputKind::PageQuery,
    };

    let summary = serde_json::json!({
        "keyword": keyword,
        "match_count": match_count,
        "what_is_it": format!("Found {} nodes matching '{}'", match_count, keyword),
    });

    let mut output = crate::output::schema::AiOutput::new(kind, summary);
    output.query_target = Some(keyword.to_string());
    output.details = Some(serde_json::json!({
        "matches": match_details,
    }));

    for (n, score) in &matches {
        output.evidence.push(
            crate::output::schema::Evidence::new(
                format!("Node {} ({}) matches with score {}", n.id, n.name, score),
                "Keyword search against graph node index",
            )
            .with_confidence(if *score >= 100.0 {
                crate::output::schema::Confidence::High
            } else if *score >= 80.0 {
                crate::output::schema::Confidence::Medium
            } else {
                crate::output::schema::Confidence::Low
            })
            .with_node_id(&n.id)
            .with_source_file(&n.path),
        );
    }

    if match_count > 0 {
        let top = &matches[0].0;
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--explain {} for semantic summary",
                &top.id,
            ));
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--context {} --depth 2 for surrounding context",
                &top.id,
            ));
    } else {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Info,
            code: "NO_MATCHES_FOUND".to_string(),
            message: format!("No nodes found matching keyword '{}'", keyword),
            location: crate::output::schema::Location::default(),
            suggestion: Some("Try a broader keyword or verify spelling".to_string()),
        });
    }

    Ok(output.validate())
}

/// 在页面作用域内解析局部模型 ID
///
/// 从页面的数据源、组件绑定、action reads/writes、DataFlow 输入输出中搜索与 local_model_id 匹配的模型。
pub fn resolve_model_in_page(
    graph: &dyn GraphReadStore,
    page_id: &str,
    local_model_id: &str,
) -> crate::output::schema::AiOutput {
    let page_node = match graph.get_node(page_id) {
        Ok(Some(n)) => n,
        Ok(None) | Err(_) => {
            let mut out = crate::output::schema::AiOutput::new(
                crate::output::schema::OutputKind::ModelQuery,
                serde_json::json!({
                    "page_id": page_id,
                    "local_model_id": local_model_id,
                    "resolved_count": 0,
                    "what_is_it": format!("Page {} not found", page_id),
                }),
            );
            out.diagnostics.push(crate::output::schema::Diagnostic {
                severity: crate::output::schema::DiagnosticSeverity::Error,
                code: "TARGET_NOT_FOUND".to_string(),
                message: format!("Page '{}' not found in graph", page_id),
                location: crate::output::schema::Location::default(),
                suggestion: Some("Verify page ID or use --find-page to search".to_string()),
            });
            out.next_queries
                .push(crate::output::schema::format_next_query(
                    "--find-page {} to search for similar pages",
                    page_id,
                ));
            return out.validate();
        }
    };

    let mut candidates: Vec<(crate::graph::Node, f64, String)> = Vec::new();
    let local_lower = local_model_id.to_lowercase();

    // 从页面出边收集关联模型
    if let Ok(Some(neighbors)) = graph.get_node_edges(page_id) {
        for edge_view in &neighbors.outgoing {
            let target = &edge_view.node;
            let edge = &edge_view.edge;
            let score = if target.id.to_lowercase() == local_lower {
                100.0
            } else if target.id.to_lowercase().contains(&local_lower)
                || target.name.to_lowercase().contains(&local_lower)
            {
                80.0
            } else {
                0.0
            };
            if score > 0.0 {
                candidates.push((target.clone(), score, format!("{:?}", edge.edge_type)));
            }
        }
    }

    // 全局搜索裸名匹配
    let bare = local_model_id
        .strip_prefix("model:")
        .unwrap_or(local_model_id);
    if let Ok(nodes) = graph.iter_nodes() {
        for node in nodes {
            if node.id.starts_with("model:") {
                let node_bare = node.id.strip_prefix("model:").unwrap_or(&node.id);
                if node_bare.eq_ignore_ascii_case(bare) {
                    candidates.push((node, 60.0, "global model match".to_string()));
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|(n, _, _)| seen.insert(n.id.clone()))
        .collect();

    let resolved_count = candidates.len();
    let ambiguous = resolved_count > 1;

    let candidate_details: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(n, score, reason)| {
            serde_json::json!({
                "model_id": n.id,
                "model_name": n.name,
                "confidence": score,
                "match_reason": reason,
            })
        })
        .collect();

    let summary = serde_json::json!({
        "page_id": page_id,
        "local_model_id": local_model_id,
        "resolved_count": resolved_count,
        "ambiguous": ambiguous,
        "what_is_it": if resolved_count == 0 {
            format!("No model resolved for '{}' in page {}", local_model_id, page_id)
        } else if ambiguous {
            format!("Multiple models match '{}' in page {}", local_model_id, page_id)
        } else {
            format!("Model {} resolved in page {}", candidates[0].0.id, page_id)
        },
    });

    let mut output = crate::output::schema::AiOutput::new(
        crate::output::schema::OutputKind::ModelQuery,
        summary,
    );
    output.query_target = Some(format!("{}|{}", page_id, local_model_id));
    output.details = Some(serde_json::json!({
        "candidates": candidate_details,
        "page_source_file": page_node.path,
    }));

    for (n, score, reason) in &candidates {
        output.evidence.push(
            crate::output::schema::Evidence::new(
                format!("Model {} matched via {} (score {})", n.id, reason, score),
                "Page-scoped model resolution",
            )
            .with_confidence(if *score >= 80.0 {
                crate::output::schema::Confidence::High
            } else {
                crate::output::schema::Confidence::Medium
            })
            .with_node_id(&n.id)
            .with_source_file(&page_node.path),
        );
    }

    if resolved_count == 0 {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Warning,
            code: "MODEL_UNRESOLVED".to_string(),
            message: format!(
                "No model matching '{}' found in page {}",
                local_model_id, page_id
            ),
            location: crate::output::schema::Location {
                source_file: Some(page_node.path),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Use --find-model to search globally".to_string()),
        });
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--find-model {} to search globally",
                local_model_id,
            ));
    } else if ambiguous {
        output.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Info,
            code: "AMBIGUOUS_RESOLUTION".to_string(),
            message: format!(
                "Multiple models match '{}'; candidate count: {}",
                local_model_id, resolved_count
            ),
            location: crate::output::schema::Location {
                source_file: Some(page_node.path),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Use --explain <MODEL_ID> to verify specific model".to_string()),
        });
        for (n, _, _) in &candidates {
            output
                .next_queries
                .push(crate::output::schema::format_next_query(
                    "--explain {} for semantic summary",
                    &n.id,
                ));
        }
    } else {
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--explain {} for semantic summary",
                &candidates[0].0.id,
            ));
        output
            .next_queries
            .push(crate::output::schema::format_next_query(
                "--query-model {} for model dependencies",
                &candidates[0].0.id,
            ));
    }

    output.validate()
}
