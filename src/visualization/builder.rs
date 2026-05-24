use crate::output::schema::{AiOutput, Evidence, OutputKind};
use crate::visualization::graph_model::{
    SourceSummary, VisualEdge, VisualGraph, VisualGroup, VisualNode,
};
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind, VisualGraphOptions};
use crate::visualization::sanitizer::{
    sanitize_metadata_value, sanitize_text as sanitize_sensitive_text,
};
use std::collections::{HashMap, HashSet};

/// 可视化图构建器
///
/// 从 AiOutput 构建 VisualGraph，支持截断、分组和敏感信息过滤。
pub struct VisualGraphBuilder;

impl VisualGraphBuilder {
    /// 从 AiOutput 构建可视化图
    pub fn from_ai_output(output: &AiOutput, options: &VisualGraphOptions) -> VisualGraph {
        let mut graph = VisualGraph::empty();
        graph.diagnostics = output.diagnostics.clone();

        // 如果只有 diagnostics 没有实质内容，生成 diagnostic-only 图
        let has_substantive_content = !output.summary.is_null()
            && output
                .summary
                .as_object()
                .map(|o| !o.is_empty())
                .unwrap_or(true)
            && output
                .summary
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(true);

        if !has_substantive_content && !output.diagnostics.is_empty() {
            return VisualGraph::from_diagnostics(output.diagnostics.clone());
        }

        // 根据 OutputKind 选择解析策略
        match output.kind {
            OutputKind::PageQuery
            | OutputKind::ComponentQuery
            | OutputKind::SuperPage
            | OutputKind::Explain => {
                Self::parse_page_like_output(output, &mut graph, options);
            }
            OutputKind::ModelQuery | OutputKind::DataFlowQuery | OutputKind::DataFlow => {
                Self::parse_model_like_output(output, &mut graph, options);
            }
            OutputKind::CrossPageQuery => {
                Self::parse_cross_page_output(output, &mut graph, options);
            }
            _ => {
                // 通用解析：从 evidence 中提取
                Self::parse_from_evidence(output, &mut graph, options);
            }
        }

        // 应用截断
        Self::apply_truncation(&mut graph, options);

        // 按 source_path 分组
        if options.group_by_source_path {
            Self::apply_grouping(&mut graph);
        }

        // 设置焦点节点
        if let Some(ref target) = options.focus_target {
            graph.focus_node = Some(target.clone());
        }

        // 计算来源摘要
        graph.source_summary = Self::compute_summary(&graph);

        graph
    }

    /// 解析页面类输出
    fn parse_page_like_output(
        output: &AiOutput,
        graph: &mut VisualGraph,
        _options: &VisualGraphOptions,
    ) {
        // 从 summary 提取目标信息
        if let Some(target_id) = output.summary.get("target_id").and_then(|v| v.as_str()) {
            let target_name = output
                .summary
                .get("target_name")
                .and_then(|v| v.as_str())
                .unwrap_or(target_id);
            let source_path = output
                .summary
                .get("source_file")
                .or_else(|| output.summary.get("source_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            graph.nodes.push(VisualNode {
                id: target_id.to_string(),
                label: Self::sanitize_label(target_name),
                kind: NodeKind::Component,
                source_path: source_path.to_string(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("type".to_string(), serde_json::json!("target"));
                    m
                },
            });
        }

        // 从 evidence 提取节点和边
        Self::parse_from_evidence(output, graph, _options);

        // 从 details 提取关联信息
        if let Some(ref details) = output.details {
            Self::parse_details(details, graph);
        }
    }

    /// 解析模型类输出
    fn parse_model_like_output(
        output: &AiOutput,
        graph: &mut VisualGraph,
        _options: &VisualGraphOptions,
    ) {
        // 从 summary 提取模型信息
        if let Some(model_id) = output.summary.get("model_id").and_then(|v| v.as_str()) {
            let model_name = output
                .summary
                .get("model_name")
                .and_then(|v| v.as_str())
                .unwrap_or(model_id);
            let source_path = output
                .summary
                .get("source_file")
                .or_else(|| output.summary.get("source_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            graph.nodes.push(VisualNode {
                id: model_id.to_string(),
                label: Self::sanitize_label(model_name),
                kind: NodeKind::Model,
                source_path: source_path.to_string(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("type".to_string(), serde_json::json!("target"));
                    m
                },
            });
        }

        Self::parse_from_evidence(output, graph, _options);

        if let Some(ref details) = output.details {
            Self::parse_details(details, graph);
        }
    }

    /// 解析跨页面输出
    fn parse_cross_page_output(
        output: &AiOutput,
        graph: &mut VisualGraph,
        _options: &VisualGraphOptions,
    ) {
        // 从 summary 提取两个页面
        if let (Some(page_a), Some(page_b)) = (
            output.summary.get("page_a").and_then(|v| v.as_str()),
            output.summary.get("page_b").and_then(|v| v.as_str()),
        ) {
            graph.nodes.push(VisualNode {
                id: page_a.to_string(),
                label: Self::sanitize_label(page_a),
                kind: NodeKind::Page,
                source_path: String::new(),
                metadata: HashMap::new(),
            });
            graph.nodes.push(VisualNode {
                id: page_b.to_string(),
                label: Self::sanitize_label(page_b),
                kind: NodeKind::Page,
                source_path: String::new(),
                metadata: HashMap::new(),
            });
        }

        Self::parse_from_evidence(output, graph, _options);

        if let Some(ref details) = output.details {
            Self::parse_details(details, graph);
        }
    }

    /// 从 evidence 列表提取节点和边
    fn parse_from_evidence(
        output: &AiOutput,
        graph: &mut VisualGraph,
        _options: &VisualGraphOptions,
    ) {
        let mut seen_nodes: HashSet<String> = HashSet::new();
        let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();

        for evidence in &output.evidence {
            // 提取节点
            if let Some(ref node_id) = evidence.node_id {
                if !seen_nodes.contains(node_id) {
                    seen_nodes.insert(node_id.clone());
                    let label = &evidence.claim;
                    let kind = Self::infer_node_kind(node_id, &evidence.edge_type);
                    let source_path = evidence.source_file.clone().unwrap_or_default();

                    graph.nodes.push(VisualNode {
                        id: node_id.clone(),
                        label: Self::sanitize_label(label),
                        kind,
                        source_path,
                        metadata: Self::build_node_metadata(evidence),
                    });
                }
            }

            // 从 claim 和 reason 中尝试提取边关系
            if let Some(ref edge_type) = evidence.edge_type {
                if let Some(ref node_id) = evidence.node_id {
                    if let Some(ref raw_expr) = evidence.raw_expr {
                        // 尝试解析表达式中的目标引用
                        if let Some(target) = Self::extract_target_from_expr(raw_expr) {
                            let edge_key = (node_id.clone(), target.clone(), edge_type.clone());
                            if !seen_edges.contains(&edge_key) {
                                seen_edges.insert(edge_key);
                                let kind = Self::parse_edge_kind(edge_type);
                                graph.edges.push(VisualEdge {
                                    from: node_id.clone(),
                                    to: target,
                                    kind,
                                    label: Some(sanitize_sensitive_text(edge_type)),
                                    direction: EdgeDirection::Forward,
                                    evidence: Some(sanitize_sensitive_text(&evidence.reason)),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// 解析 details JSON 中的关联信息
    fn parse_details(details: &serde_json::Value, graph: &mut VisualGraph) {
        // 解析 readers / writers / inputs / outputs 等数组
        let arrays = vec![
            ("readers", EdgeKind::Reads),
            ("writers", EdgeKind::Writes),
            ("inputs", EdgeKind::DependsOn),
            ("outputs", EdgeKind::Triggers),
            ("dependencies", EdgeKind::DependsOn),
            ("actions", EdgeKind::Triggers),
        ];

        for (key, default_kind) in arrays {
            if let Some(arr) = details.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    if let (Some(id), Some(name)) = (
                        item.get("id").and_then(|v| v.as_str()),
                        item.get("name")
                            .or_else(|| item.get("label"))
                            .and_then(|v| v.as_str()),
                    ) {
                        // 添加节点（如果不存在）
                        if !graph.nodes.iter().any(|n| n.id == id) {
                            let kind = item
                                .get("kind")
                                .or_else(|| item.get("node_type"))
                                .and_then(|v| v.as_str())
                                .map(|s| Self::parse_node_kind_str(s))
                                .unwrap_or(NodeKind::Component);
                            let source_path = item
                                .get("source_file")
                                .or_else(|| item.get("source_path"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            graph.nodes.push(VisualNode {
                                id: id.to_string(),
                                label: Self::sanitize_label(name),
                                kind,
                                source_path,
                                metadata: Self::json_to_metadata(item),
                            });
                        }

                        // 尝试添加边（如果有 from/to 信息）
                        if let Some(from) = item.get("from").and_then(|v| v.as_str()) {
                            if let Some(to) = item.get("to").and_then(|v| v.as_str()) {
                                let kind = item
                                    .get("edge_type")
                                    .and_then(|v| v.as_str())
                                    .map(|s| Self::parse_edge_kind(s))
                                    .unwrap_or(default_kind.clone());
                                let label = item
                                    .get("edge_label")
                                    .and_then(|v| v.as_str())
                                    .map(sanitize_sensitive_text);

                                if !graph.edges.iter().any(|e| {
                                    e.from == from
                                        && e.to == to
                                        && format!("{:?}", e.kind) == format!("{:?}", kind)
                                }) {
                                    graph.edges.push(VisualEdge {
                                        from: from.to_string(),
                                        to: to.to_string(),
                                        kind,
                                        label,
                                        direction: EdgeDirection::Forward,
                                        evidence: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// 应用截断
    fn apply_truncation(graph: &mut VisualGraph, options: &VisualGraphOptions) {
        let original_node_count = graph.nodes.len();
        let original_edge_count = graph.edges.len();

        if graph.nodes.len() > options.max_nodes {
            graph.truncated = true;
            graph.nodes.truncate(options.max_nodes);
            // 截断后只保留两端节点都在保留列表中的边
            let kept_ids: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
            graph
                .edges
                .retain(|e| kept_ids.contains(&e.from) && kept_ids.contains(&e.to));
        }

        if graph.edges.len() > options.max_edges {
            graph.truncated = true;
            graph.edges.truncate(options.max_edges);
        }

        // 如果截断了，添加提示节点
        if graph.truncated {
            graph.nodes.push(VisualNode {
                id: "truncated".to_string(),
                label: format!(
                    "... 结果已截断 ({} 节点, {} 边) ...",
                    original_node_count, original_edge_count
                ),
                kind: NodeKind::Diagnostic,
                source_path: String::new(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert(
                        "original_nodes".to_string(),
                        serde_json::json!(original_node_count),
                    );
                    m.insert(
                        "original_edges".to_string(),
                        serde_json::json!(original_edge_count),
                    );
                    m
                },
            });
        }
    }

    /// 按 source_path 分组
    fn apply_grouping(graph: &mut VisualGraph) {
        let mut path_to_nodes: HashMap<String, Vec<String>> = HashMap::new();
        for node in &graph.nodes {
            if !node.source_path.is_empty() {
                path_to_nodes
                    .entry(node.source_path.clone())
                    .or_default()
                    .push(node.id.clone());
            }
        }

        for (path, node_ids) in path_to_nodes {
            if node_ids.len() >= 2 {
                let group_id = format!("group_{}", Self::sanitize_id(&path));
                graph.groups.push(VisualGroup {
                    id: group_id,
                    label: path.clone(),
                    node_ids,
                });
            }
        }
    }

    /// 计算来源摘要
    fn compute_summary(graph: &VisualGraph) -> SourceSummary {
        let mut node_kinds: HashMap<String, usize> = HashMap::new();
        let mut edge_kinds: HashMap<String, usize> = HashMap::new();

        for node in &graph.nodes {
            *node_kinds.entry(format!("{}", node.kind)).or_insert(0) += 1;
        }

        for edge in &graph.edges {
            *edge_kinds.entry(format!("{}", edge.kind)).or_insert(0) += 1;
        }

        SourceSummary {
            total_nodes: graph.nodes.len(),
            total_edges: graph.edges.len(),
            node_kinds,
            edge_kinds,
        }
    }

    /// 推断节点类型
    fn infer_node_kind(node_id: &str, edge_type: &Option<String>) -> NodeKind {
        if node_id.starts_with("page:") || node_id.starts_with("Page") {
            NodeKind::Page
        } else if node_id.starts_with("model:") || node_id.starts_with("Model") {
            NodeKind::Model
        } else if node_id.starts_with("field:") || node_id.starts_with("Field") {
            NodeKind::Field
        } else if node_id.starts_with("action:") || node_id.starts_with("Action") {
            NodeKind::Action
        } else if edge_type
            .as_ref()
            .map(|s| s.contains("Condition"))
            .unwrap_or(false)
        {
            NodeKind::Condition
        } else {
            NodeKind::Component
        }
    }

    /// 从字符串解析节点类型
    fn parse_node_kind_str(s: &str) -> NodeKind {
        match s.to_lowercase().as_str() {
            "page" => NodeKind::Page,
            "component" => NodeKind::Component,
            "model" => NodeKind::Model,
            "field" => NodeKind::Field,
            "action" => NodeKind::Action,
            "condition" => NodeKind::Condition,
            _ => NodeKind::Component,
        }
    }

    /// 从字符串解析边类型
    fn parse_edge_kind(s: &str) -> EdgeKind {
        match s.to_lowercase().as_str() {
            "reads" => EdgeKind::Reads,
            "writes" => EdgeKind::Writes,
            "triggers" => EdgeKind::Triggers,
            "contains" => EdgeKind::Contains,
            "dependson" | "depends_on" | "depends on" => EdgeKind::DependsOn,
            _ => EdgeKind::Other(s.to_string()),
        }
    }

    /// 从表达式中提取目标引用
    fn extract_target_from_expr(expr: &str) -> Option<String> {
        // 简单启发式：提取 ${...} 或 model:xxx 或 page:xxx 格式的引用
        if let Some(start) = expr.find("${") {
            if let Some(end) = expr[start..].find('}') {
                let inner = &expr[start + 2..start + end];
                return Some(inner.trim().to_string());
            }
        }

        for prefix in &["model:", "page:", "comp:", "action:", "field:"] {
            if let Some(pos) = expr.find(prefix) {
                let rest = &expr[pos..];
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == ',' || c == ')')
                    .unwrap_or(rest.len());
                return Some(rest[..end].to_string());
            }
        }

        None
    }

    /// 构建节点元数据
    fn build_node_metadata(evidence: &Evidence) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        if let Some(ref raw_expr) = evidence.raw_expr {
            m.insert(
                "raw_expr".to_string(),
                sanitize_metadata_value(&serde_json::json!(raw_expr)),
            );
        }
        if let Some(ref json_path) = evidence.json_path {
            m.insert(
                "json_path".to_string(),
                sanitize_metadata_value(&serde_json::json!(json_path)),
            );
        }
        m.insert(
            "confidence".to_string(),
            serde_json::json!(format!("{:?}", evidence.confidence)),
        );
        m
    }

    /// 将 JSON 值转换为元数据映射
    fn json_to_metadata(value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .map(|(k, v)| (k.clone(), sanitize_metadata_value(v)))
                .collect(),
            _ => HashMap::new(),
        }
    }

    /// 清理标签文本（过滤敏感信息）
    fn sanitize_label(text: &str) -> String {
        let sanitized = sanitize_sensitive_text(text);
        // 限制长度
        if sanitized.chars().count() > 80 {
            let truncated: String = sanitized.chars().take(80).collect();
            format!("{}...", truncated)
        } else {
            sanitized
        }
    }

    /// 将字符串转换为合法的 ID（用于 Mermaid）
    fn sanitize_id(text: &str) -> String {
        text.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
}
