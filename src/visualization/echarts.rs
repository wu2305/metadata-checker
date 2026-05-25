use crate::visualization::graph_model::VisualGraph;
use crate::visualization::options::NodeKind;
use crate::visualization::sanitizer::{is_sensitive_key, sanitize_metadata_value, sanitize_text};

/// ECharts 渲染器
///
/// 将 VisualGraph 输出为 ECharts graph option JSON（serde_json::Value）。
/// 不依赖 DOM，纯 JSON 输出。
pub struct EChartsRenderer;

impl EChartsRenderer {
    /// 渲染为 ECharts option JSON
    pub fn render(graph: &VisualGraph) -> serde_json::Value {
        let categories = Self::build_categories(graph);
        let data = Self::build_nodes(graph, &categories);
        let links = Self::build_links(graph);

        let truncated_note = if graph.truncated {
            let mut note = format!(
                "Nodes: {} (truncated), Edges: {} (truncated)",
                graph.source_summary.total_nodes, graph.source_summary.total_edges,
            );
            if let Some(ref reason) = graph.truncated_reason {
                note.push_str(&format!("; {}", sanitize_text(reason)));
            }
            note
        } else {
            format!(
                "Nodes: {}, Edges: {}",
                graph.source_summary.total_nodes, graph.source_summary.total_edges,
            )
        };

        serde_json::json!({
            "series": [{
                "type": "graph",
                "layout": "force",
                "animation": false,
                "roam": true,
                "label": {
                    "show": true,
                    "position": "right",
                    "formatter": "{b}",
                },
                "edgeLabel": {
                    "show": true,
                    "formatter": "{c}",
                },
                "data": data,
                "links": links,
                "categories": categories,
                "force": {
                    "repulsion": 100,
                    "gravity": 0.1,
                    "edgeLength": [50, 200],
                },
                "tooltip": {
                    "show": true,
                    "formatter": Self::build_tooltip_formatter(),
                },
                "lineStyle": {
                    "curveness": 0.2,
                },
                "emphasis": {
                    "focus": "adjacency",
                    "lineStyle": {
                        "width": 4,
                    },
                },
            }],
            "title": {
                "text": graph.focus_node.as_ref()
                    .map(|id| format!("Graph: {}", sanitize_text(id)))
                    .unwrap_or_else(|| "Metadata Graph".to_string()),
                "subtext": format!(
                    "{} | direction={}, depth={}, collapsed={}",
                    truncated_note,
                    graph.direction,
                    graph.depth,
                    graph.collapsed
                ),
            },
        })
    }

    /// 构建 categories（按 NodeKind）
    fn build_categories(graph: &VisualGraph) -> Vec<serde_json::Value> {
        let mut kinds: Vec<NodeKind> = graph
            .nodes
            .iter()
            .map(|n| n.kind.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        kinds.sort_by(|a, b| format!("{:?}", a).cmp(&format!("{:?}", b)));

        kinds
            .into_iter()
            .map(|kind| {
                serde_json::json!({
                    "name": format!("{}", kind),
                    "itemStyle": Self::kind_to_style(&kind),
                })
            })
            .collect()
    }

    /// 构建节点数据
    fn build_nodes(
        graph: &VisualGraph,
        categories: &[serde_json::Value],
    ) -> Vec<serde_json::Value> {
        graph
            .nodes
            .iter()
            .map(|node| {
                let category_index = categories
                    .iter()
                    .position(|c| {
                        c.get("name").and_then(|v| v.as_str()) == Some(&format!("{}", node.kind))
                    })
                    .unwrap_or(0);

                let base_symbol_size: i32 = match node.kind {
                    NodeKind::Page | NodeKind::Model => 30,
                    NodeKind::Component => 20,
                    NodeKind::Action => 18,
                    NodeKind::Field => 15,
                    NodeKind::Condition => 16,
                    NodeKind::Diagnostic => 12,
                };

                let symbol_size = match node.depth {
                    Some(depth) if depth >= 3 => base_symbol_size.saturating_sub(4),
                    Some(2) => base_symbol_size.saturating_sub(2),
                    _ => base_symbol_size,
                };

                // 构建 tooltip 内容（已过滤敏感信息）
                let tooltip_content = Self::build_node_tooltip(node);
                let opacity = if graph
                    .focus_node
                    .as_ref()
                    .map(|f| f == &node.id)
                    .unwrap_or(false)
                {
                    1.0
                } else if let Some(depth) = node.depth {
                    if depth >= 2 {
                        0.65
                    } else if depth >= 1 {
                        0.8
                    } else {
                        1.0
                    }
                } else {
                    0.8
                };

                let symbol = if node.collapsed { "rect" } else { "circle" };
                let color = if node.importance.as_deref() == Some("critical") {
                    "#f56c6c"
                } else if node.importance.as_deref() == Some("static_display") {
                    "#91cc75"
                } else {
                    "#5470c6"
                };

                serde_json::json!({
                    "id": sanitize_text(&node.id),
                    "name": sanitize_text(&node.label),
                    "category": category_index,
                    "symbol": symbol,
                    "symbolSize": symbol_size,
                    "value": node.metadata.len(),
                    "tooltip": tooltip_content,
                    "itemStyle": {
                        "opacity": opacity,
                        "color": color,
                    },
                })
            })
            .collect()
    }

    /// 构建边数据
    fn build_links(graph: &VisualGraph) -> Vec<serde_json::Value> {
        graph
            .edges
            .iter()
            .map(|edge| {
                let mut link = serde_json::json!({
                    "source": sanitize_text(&edge.from),
                    "target": sanitize_text(&edge.to),
                });

                let label = edge
                    .label
                    .as_deref()
                    .or(edge.edge_type.as_deref())
                    .unwrap_or("relation");

                let fallback_kind = edge.kind.to_string();
                let display_type = edge.edge_type.as_deref().unwrap_or(fallback_kind.as_str());

                let sanitized_label = sanitize_text(label);
                link["label"] = serde_json::json!({
                    "show": true,
                    "formatter": sanitized_label,
                });
                link["value"] = serde_json::json!(sanitized_label);
                link["edgeType"] = serde_json::json!(sanitize_text(display_type));

                if let Some(ref evidence) = edge.evidence {
                    link["tooltip"] = serde_json::json!(sanitize_text(evidence));
                }

                if Self::has_collapsed_endpoints(edge, graph) {
                    link["lineStyle"] = serde_json::json!({
                        "opacity": 0.4,
                        "type": "dashed",
                    });
                }

                link
            })
            .collect()
    }

    /// 构建节点 tooltip 内容
    fn build_node_tooltip(node: &crate::visualization::graph_model::VisualNode) -> String {
        let mut lines = vec![format!("<b>{}</b>", sanitize_text(&node.label))];
        lines.push(format!("Type: {}", node.kind));

        if !node.source_path.is_empty() {
            lines.push(format!("Source: {}", sanitize_text(&node.source_path)));
        }
        if let Some(depth) = node.depth {
            lines.push(format!("Depth: {}", depth));
        }
        if node.collapsed {
            lines.push("State: collapsed (expandable)".to_string());
        }
        if let Some(ref token) = node.expand_token {
            lines.push(format!("Expand: {}", sanitize_text(token)));
        }
        if let Some(ref importance) = node.importance {
            lines.push(format!("Importance: {}", sanitize_text(importance)));
        }

        // 添加元数据（过滤敏感信息）
        for (key, value) in &node.metadata {
            if Self::is_sensitive_key(key) {
                continue;
            }
            let sanitized_value = sanitize_metadata_value(value);
            let value_str = match sanitized_value {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            lines.push(format!("{}: {}", sanitize_text(key), value_str));
        }

        lines.join("<br/>")
    }

    /// 构建 tooltip formatter 函数字符串
    fn build_tooltip_formatter() -> String {
        "{b}".to_string()
    }

    /// 根据节点类型返回样式
    fn kind_to_style(kind: &NodeKind) -> serde_json::Value {
        match kind {
            NodeKind::Page => serde_json::json!({ "color": "#5470c6" }),
            NodeKind::Component => serde_json::json!({ "color": "#91cc75" }),
            NodeKind::Model => serde_json::json!({ "color": "#fac858" }),
            NodeKind::Field => serde_json::json!({ "color": "#ee6666" }),
            NodeKind::Action => serde_json::json!({ "color": "#73c0de" }),
            NodeKind::Condition => serde_json::json!({ "color": "#3ba272" }),
            NodeKind::Diagnostic => serde_json::json!({ "color": "#fc8452" }),
        }
    }

    /// 检查键名是否敏感
    fn is_sensitive_key(key: &str) -> bool {
        is_sensitive_key(key)
    }

    /// 只要边任一端是折叠节点，就使用弱化样式。
    fn has_collapsed_endpoints(
        edge: &crate::visualization::graph_model::VisualEdge,
        graph: &VisualGraph,
    ) -> bool {
        graph
            .nodes
            .iter()
            .any(|node| node.collapsed && (node.id == edge.from || node.id == edge.to))
    }
}
