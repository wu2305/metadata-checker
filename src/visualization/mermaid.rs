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
        }

        lines.push(String::new());

        // 按 (from, to, kind) 稳定排序边
        let mut sorted_edges = graph.edges.clone();
        sorted_edges.sort_by(|a, b| {
            (&a.from, &a.to, format!("{:?}", a.kind)).cmp(&(
                &b.from,
                &b.to,
                format!("{:?}", b.kind),
            ))
        });

        // 渲染边
        for edge in &sorted_edges {
            lines.push(Self::render_edge(edge));
        }

        // 渲染分组
        if !graph.groups.is_empty() {
            lines.push(String::new());
            let mut sorted_groups = graph.groups.clone();
            sorted_groups.sort_by(|a, b| a.id.cmp(&b.id));

            for group in &sorted_groups {
                lines.push(format!(
                    "subgraph {} [{}]",
                    group.id,
                    Self::escape_label(&group.label)
                ));
                for node_id in &group.node_ids {
                    lines.push(format!("    {}", Self::sanitize_mermaid_id(node_id)));
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
                    Self::escape_comment(&diag.code),
                    Self::escape_comment(&diag.message)
                ));
            }
        }

        lines.join("\n")
    }

    /// 渲染单个节点
    fn render_node(node: &VisualNode) -> String {
        let id = Self::sanitize_mermaid_id(&node.id);
        let label = Self::escape_label(&sanitize_text(&node.label));

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
        let from = Self::sanitize_mermaid_id(&edge.from);
        let to = Self::sanitize_mermaid_id(&edge.to);

        let arrow = match edge.direction {
            EdgeDirection::Forward => "-->",
            EdgeDirection::Bidirectional => "<-->",
        };

        if let Some(ref label) = edge.label {
            let escaped_label = Self::escape_label(&sanitize_text(label));
            format!("{} {}|{}| {}", from, arrow, escaped_label, to)
        } else {
            format!("{} {} {}", from, arrow, to)
        }
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

        // 后续字符
        for c in chars {
            if c.is_ascii_alphanumeric() || c == '_' {
                result.push(c);
            } else {
                result.push('_');
            }
        }

        // 确保不是 Mermaid 关键字
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
}
