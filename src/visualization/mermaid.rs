use crate::visualization::graph_model::{VisualEdge, VisualGraph, VisualNode};
use crate::visualization::options::{EdgeDirection, NodeKind};
use crate::visualization::sanitizer::sanitize_text;

/// Mermaid 渲染器
///
/// 将 VisualGraph 输出为 Mermaid flowchart 文本。
pub struct MermaidRenderer;

impl MermaidRenderer {
    /// 渲染为 Mermaid flowchart 文本
    pub fn render(graph: &VisualGraph) -> String {
        let mut lines = Vec::new();
        lines.push("graph TD".to_string());
        lines.push(String::new());

        // 按 ID 稳定排序节点
        let mut sorted_nodes = graph.nodes.clone();
        sorted_nodes.sort_by(|a, b| a.id.cmp(&b.id));

        // 渲染节点
        for node in &sorted_nodes {
            lines.push(Self::render_node(node));

            if node.collapsed {
                let node_id = Self::sanitize_mermaid_id(&sanitize_text(&node.id));
                lines.push(format!(
                    "style {} fill:#f5f5f5,stroke:#8e8e8e,stroke-width:1px",
                    node_id
                ));
                if let Some(ref token) = node.expand_token {
                    lines.push(format!(
                        "%% {} collapsed, expandable via: {}",
                        node_id,
                        Self::escape_label(&sanitize_text(token))
                    ));
                }
            }

            if let Some(depth) = node.depth {
                if depth >= 2 {
                    let node_id = Self::sanitize_mermaid_id(&sanitize_text(&node.id));
                    lines.push(format!("style {} fill:#ffffff,opacity:0.75", node_id));
                    if depth >= 3 {
                        lines.push(format!("style {} fill:#efefef,opacity:0.6", node_id));
                    }
                }
            }

            if let Some(ref importance) = node.importance {
                if importance == "static_display" {
                    let node_id = Self::sanitize_mermaid_id(&sanitize_text(&node.id));
                    lines.push(format!("style {} fill:#f0fff0,stroke:#66cdaa", node_id));
                }
            }
        }

        // 元数据说明
        if let Some(ref focus) = graph.focus_node {
            lines.push(String::new());
            lines.push(format!(
                "%% focus={} direction={} depth={} collapsed={}",
                Self::escape_label(&sanitize_text(focus)),
                Self::escape_label(&graph.direction),
                graph.depth,
                graph.collapsed
            ));
            if let Some(ref reason) = graph.truncated_reason {
                lines.push(format!(
                    "%% truncated_reason: {}",
                    Self::escape_label(&sanitize_text(reason))
                ));
            }
        }

        lines.push(String::new());

        let mut sorted_edges = graph.edges.clone();
        sorted_edges.sort_by(|a, b| {
            (&a.from, &a.to, format!("{:?}", a.kind)).cmp(&(
                &b.from,
                &b.to,
                format!("{:?}", b.kind),
            ))
        });

        for (idx, edge) in sorted_edges.iter().enumerate() {
            lines.push(Self::render_edge(edge));
            if Self::is_collapsed_edge(edge, graph) {
                lines.push(format!(
                    "linkStyle {} stroke: #c0c0c0, stroke-width: 1px, stroke-dasharray: 5 5",
                    idx
                ));
            }
        }

        // 渲染分组
        if !graph.groups.is_empty() {
            lines.push(String::new());
            let mut sorted_groups = graph.groups.clone();
            sorted_groups.sort_by(|a, b| a.id.cmp(&b.id));

            for group in &sorted_groups {
                lines.push(format!(
                    "subgraph {} [{}]",
                    Self::sanitize_mermaid_id(&sanitize_text(&group.id)),
                    Self::escape_label(&sanitize_text(&group.label))
                ));
                for node_id in &group.node_ids {
                    lines.push(format!(
                        "    {}",
                        Self::sanitize_mermaid_id(&sanitize_text(node_id))
                    ));
                }
                lines.push("end".to_string());
                lines.push(String::new());
            }
        }

        // 添加诊断注释
        if !graph.diagnostics.is_empty() {
            lines.push(String::new());
            lines.push("%% Diagnostics".to_string());
            for diag in &graph.diagnostics {
                lines.push(format!(
                    "%% [{}] {}: {}",
                    Self::escape_comment(&format!("{:?}", diag.severity)),
                    Self::escape_comment(&sanitize_text(&diag.code)),
                    Self::escape_comment(&sanitize_text(&diag.message))
                ));
            }
        }

        lines.join("\n")
    }

    /// 渲染单个节点
    fn render_node(node: &VisualNode) -> String {
        let id = Self::sanitize_mermaid_id(&sanitize_text(&node.id));
        let base_label = Self::escape_label(&sanitize_text(&node.label));
        let label = if node.depth.is_some() {
            let depth = node.depth.unwrap_or(0);
            if node.collapsed {
                format!("{} (d={} ⟲)", base_label, depth)
            } else {
                format!("{} (d={})", base_label, depth)
            }
        } else {
            base_label
        };

        match node.kind {
            NodeKind::Page => format!("{}((\"{}\"))", id, label),
            NodeKind::Model => format!("{}[(\"{}\")]", id, label),
            NodeKind::Field => format!("{}[[\"{}\"]]", id, label),
            NodeKind::Action => format!("{}>{{\"{}\"}}]", id, label),
            NodeKind::Condition => format!("{}{{\"{}\"}}", id, label),
            NodeKind::Diagnostic => format!("{}[\"{}\"]", id, label),
            NodeKind::Component => format!("{}[\"{}\"]", id, label),
        }
    }

    /// 渲染单条边
    fn render_edge(edge: &VisualEdge) -> String {
        let from = Self::sanitize_mermaid_id(&sanitize_text(&edge.from));
        let to = Self::sanitize_mermaid_id(&sanitize_text(&edge.to));

        let label = edge
            .label
            .as_deref()
            .or(edge.edge_type.as_deref())
            .unwrap_or("relation");
        let arrow = match edge.direction {
            EdgeDirection::Forward => "-->",
            EdgeDirection::Bidirectional => "<-->",
        };

        let escaped_label = Self::escape_label(&sanitize_text(label));
        format!("{} {}|{}| {}", from, arrow, escaped_label, to)
    }

    /// 将 ID 转换为 Mermaid 安全的标识符
    ///
    /// Mermaid ID 必须以字母开头，只能包含字母、数字和下划线。
    fn sanitize_mermaid_id(id: &str) -> String {
        if id.is_empty() {
            return "_empty".to_string();
        }

        let mut result = String::new();
        let mut chars = id.chars();

        // 首字符必须是字母
        if let Some(first) = chars.next() {
            if first.is_ascii_alphabetic() {
                result.push(first);
            } else {
                result.push('n');
                if first.is_ascii_alphanumeric() || first == '_' {
                    result.push(first);
                } else {
                    result.push('_');
                }
            }
        }

        for c in chars {
            if c.is_ascii_alphanumeric() || c == '_' {
                result.push(c);
            } else {
                result.push('_');
            }
        }

        match result.as_str() {
            "graph" | "subgraph" | "end" | "direction" | "style" | "class" | "click" | "link" => {
                result.insert(0, 'n')
            }
            _ => {}
        }

        result
    }

    /// 转义 Mermaid label 中的特殊字符
    ///
    /// 中文路径保留，但括号、冒号、双引号、换行需要转义。
    fn escape_label(label: &str) -> String {
        let mut result = String::new();
        for c in label.chars() {
            match c {
                '"' => result.push_str("#quot;"),
                ':' => result.push_str("#58;"),
                '\n' => result.push_str("<br/>"),
                '\r' => {}
                '(' => result.push_str("#40;"),
                ')' => result.push_str("#41;"),
                '[' => result.push_str("#91;"),
                ']' => result.push_str("#93;"),
                '{' => result.push_str("#123;"),
                '}' => result.push_str("#125;"),
                '|' => result.push_str("#124;"),
                c => result.push(c),
            }
        }
        result
    }

    /// 转义 Mermaid 注释中的特殊字符
    fn escape_comment(text: &str) -> String {
        text.replace('\n', " ").replace('\r', "")
    }

    /// 当前边是否与折叠节点相邻。
    fn is_collapsed_edge(edge: &VisualEdge, graph: &VisualGraph) -> bool {
        graph
            .nodes
            .iter()
            .any(|node| node.collapsed && (node.id == edge.from || node.id == edge.to))
    }
}
