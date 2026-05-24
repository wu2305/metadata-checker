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
                "subtext": if graph.truncated {
                    format!(
                        "Nodes: {} (truncated), Edges: {} (truncated)",
                        graph.source_summary.total_nodes,
                        graph.source_summary.total_edges,
                    )
                } else {
                    format!(
                        "Nodes: {}, Edges: {}",
                        graph.source_summary.total_nodes,
                        graph.source_summary.total_edges,
                    )
                },
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

        // 稳定排序
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
                        c.get("name")
                            .and_then(|v| v.as_str())
                            == Some(&format!("{}", node.kind))
                    })
                    .unwrap_or(0);

                let symbol_size = match node.kind {
                    NodeKind::Page | NodeKind::Model => 30,
                    NodeKind::Component => 20,
                    NodeKind::Action => 18,
                    NodeKind::Field => 15,
                    NodeKind::Condition => 16,
                    NodeKind::Diagnostic => 12,
                };

                // 构建 tooltip 内容（已过滤敏感信息）
                let tooltip_content = Self::build_node_tooltip(node);

                serde_json::json!({
                    "id": sanitize_text(&node.id),
                    "name": sanitize_text(&node.label),
                    "category": category_index,
                    "symbolSize": symbol_size,
                    "value": node.metadata.len(),
                    "tooltip": tooltip_content,
                    "itemStyle": {
                        "opacity": if graph.focus_node.as_ref().map(|f| f == &node.id).unwrap_or(false) {
                            1.0
                        } else {
                            0.85
                        },
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

                if let Some(ref label) = edge.label {
                    let sanitized_label = sanitize_text(label);
                    link["label"] = serde_json::json!({
                        "show": true,
                        "formatter": sanitized_label,
                    });
                    link["value"] = serde_json::json!(sanitized_label);
                }

                if let Some(ref evidence) = edge.evidence {
                    link["tooltip"] = serde_json::json!(sanitize_text(evidence));
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
        // ECharts 支持函数字符串，但这里使用简单的字符串模板
        // 实际 tooltip 内容已在 data/links 中预计算
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
}
