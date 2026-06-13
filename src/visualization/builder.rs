use crate::output::schema::{AiOutput, Evidence, OutputKind};
use crate::visualization::graph_model::{
    SourceSummary, VisualEdge, VisualGraph, VisualGroup, VisualNode, classify_edge_priority,
    edge_evidence_status, summarize_edge,
};
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind, VisualGraphOptions};
use crate::visualization::sanitizer::{
    sanitize_diagnostic, sanitize_identity_id, sanitize_metadata_entry,
    sanitize_text as sanitize_sensitive_text,
};
use std::collections::{HashMap, HashSet, VecDeque};

/// 可视化图构建器
///
/// 从 AiOutput 构建 VisualGraph，支持截断、分组和敏感信息过滤。
pub struct VisualGraphBuilder;

impl VisualGraphBuilder {
    /// 从 AiOutput 构建可视化图
    pub fn from_ai_output(output: &AiOutput, options: &VisualGraphOptions) -> VisualGraph {
        let mut graph = VisualGraph::empty();
        graph.depth = Self::parse_depth_from_summary(&output.summary);
        graph.direction = Self::parse_direction_from_summary(&output.summary);
        graph.diagnostics = output.diagnostics.iter().map(sanitize_diagnostic).collect();
        graph.truncated_reason = Self::extract_truncated_reason(&graph.diagnostics);
        graph.focus_node = Self::resolve_focus_target(output, options);

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

        // 计算节点深度与折叠状态（用于浏览器渐进渲染）
        Self::enrich_node_depth_and_collapsing(&mut graph);

        // 应用截断
        Self::apply_truncation(&mut graph, options);

        // 按 source_path 分组
        if options.group_by_source_path {
            Self::apply_grouping(&mut graph);
        }

        // 以 options 中配置的焦点优先级最高
        if let Some(ref target) = options.focus_target {
            graph.focus_node = Some(sanitize_identity_id(target));
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
                id: sanitize_identity_id(target_id),
                label: Self::sanitize_label(target_name),
                kind: NodeKind::Component,
                source_path: sanitize_sensitive_text(source_path),
                depth: Some(0),
                collapsed: false,
                importance: None,
                expand_token: None,
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
            Self::parse_details(details, graph, _options);
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
                id: sanitize_identity_id(model_id),
                label: Self::sanitize_label(model_name),
                kind: NodeKind::Model,
                source_path: sanitize_sensitive_text(source_path),
                depth: Some(0),
                collapsed: false,
                importance: None,
                expand_token: None,
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("type".to_string(), serde_json::json!("target"));
                    m
                },
            });
        }

        Self::parse_from_evidence(output, graph, _options);

        if let Some(ref details) = output.details {
            Self::parse_details(details, graph, _options);
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
                id: sanitize_identity_id(page_a),
                label: Self::sanitize_label(page_a),
                kind: NodeKind::Page,
                source_path: String::new(),
                depth: None,
                collapsed: false,
                importance: None,
                expand_token: None,
                metadata: HashMap::new(),
            });
            graph.nodes.push(VisualNode {
                id: sanitize_identity_id(page_b),
                label: Self::sanitize_label(page_b),
                kind: NodeKind::Page,
                source_path: String::new(),
                depth: None,
                collapsed: false,
                importance: None,
                expand_token: None,
                metadata: HashMap::new(),
            });
        }

        Self::parse_from_evidence(output, graph, _options);

        if let Some(ref details) = output.details {
            Self::parse_details(details, graph, _options);
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
            if let Some(ref raw_node_id) = evidence.node_id {
                let node_id = sanitize_identity_id(raw_node_id);
                if !seen_nodes.contains(&node_id) {
                    seen_nodes.insert(node_id.clone());
                    let label = &evidence.claim;
                    let kind = Self::infer_node_kind(raw_node_id, &evidence.edge_type);
                    let source_path = evidence
                        .source_file
                        .as_ref()
                        .map(|source_file| sanitize_sensitive_text(source_file))
                        .unwrap_or_default();

                    graph.nodes.push(VisualNode {
                        id: node_id.clone(),
                        label: Self::sanitize_label(label),
                        kind,
                        source_path,
                        depth: None,
                        collapsed: false,
                        importance: None,
                        expand_token: None,
                        metadata: Self::build_node_metadata(evidence, _options.include_evidence),
                    });
                }
            }

            // 从 claim 和 reason 中尝试提取边关系
            if let Some(ref edge_type) = evidence.edge_type {
                if let Some(ref raw_node_id) = evidence.node_id {
                    let node_id = sanitize_identity_id(raw_node_id);
                    if let Some(ref raw_expr) = evidence.raw_expr {
                        // 尝试解析表达式中的目标引用
                        if let Some(target) = Self::extract_target_from_expr(raw_expr) {
                            let target = sanitize_identity_id(&target);
                            let edge_key = (node_id.clone(), target.clone(), edge_type.clone());
                            if !seen_edges.contains(&edge_key) {
                                seen_edges.insert(edge_key);
                                let kind = Self::parse_edge_kind(edge_type);
                                let label = sanitize_sensitive_text(edge_type);
                                let evidence = if _options.include_evidence {
                                    Some(sanitize_sensitive_text(&evidence.reason))
                                } else {
                                    None
                                };
                                graph.edges.push(VisualEdge {
                                    from: node_id.clone(),
                                    to: target,
                                    edge_type: Some(label.clone()),
                                    label: Some(label.clone()),
                                    direction: EdgeDirection::Forward,
                                    priority: classify_edge_priority(&kind, Some(label.as_str())),
                                    summary: summarize_edge(&kind, Some(label.as_str())),
                                    evidence_status: edge_evidence_status(&evidence),
                                    evidence,
                                    kind,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// 解析 details JSON 中的关联信息
    fn parse_details(
        details: &serde_json::Value,
        graph: &mut VisualGraph,
        options: &VisualGraphOptions,
    ) {
        // 解析 readers / writers / inputs / outputs 等数组
        let arrays = vec![
            ("readers", EdgeKind::Reads),
            ("writers", EdgeKind::Writes),
            ("inputs", EdgeKind::DependsOn),
            ("outputs", EdgeKind::Triggers),
            ("dependencies", EdgeKind::DependsOn),
            ("actions", EdgeKind::Triggers),
            ("upstream", EdgeKind::DependsOn),
            ("downstream", EdgeKind::Reads),
            ("related_nodes", EdgeKind::DependsOn),
            ("related_actions", EdgeKind::Triggers),
            ("related_models", EdgeKind::DependsOn),
            ("related_pages", EdgeKind::Contains),
            ("related_components", EdgeKind::DependsOn),
        ];

        for (key, default_kind) in arrays {
            if let Some(arr) = details.get(key).and_then(|v| v.as_array()) {
                for item in arr {
                    Self::parse_detail_item(item, default_kind.clone(), options, graph);
                }
            }
        }
    }

    /// 解析单条 detail 项（兼容 id 单点 + from/to 关系）
    fn parse_detail_item(
        item: &serde_json::Value,
        default_kind: EdgeKind,
        options: &VisualGraphOptions,
        graph: &mut VisualGraph,
    ) {
        if let (Some(raw_id), Some(name)) = (
            item.get("id").and_then(|v| v.as_str()),
            item.get("name")
                .or_else(|| item.get("label"))
                .and_then(|v| v.as_str()),
        ) {
            Self::ensure_node(graph, raw_id, name, item, options);

            // 尝试添加边（如果有 from/to 信息）
            if let Some(raw_from) = item.get("from").and_then(|v| v.as_str()) {
                if let Some(raw_to) = item.get("to").and_then(|v| v.as_str()) {
                    Self::ensure_relation(
                        graph,
                        raw_from,
                        raw_to,
                        item,
                        default_kind,
                        EdgeDirection::Forward,
                    );
                }
            }
            return;
        }

        if let (Some(raw_from), Some(raw_to)) = (
            item.get("from").and_then(|v| v.as_str()),
            item.get("to").and_then(|v| v.as_str()),
        ) {
            Self::ensure_node_by_endpoint(graph, item, raw_from);
            Self::ensure_node_by_endpoint(graph, item, raw_to);
            Self::ensure_relation(
                graph,
                raw_from,
                raw_to,
                item,
                default_kind,
                EdgeDirection::Forward,
            );
        }
    }

    /// 设置/补齐节点
    fn ensure_node(
        graph: &mut VisualGraph,
        raw_id: &str,
        raw_name: &str,
        item: &serde_json::Value,
        options: &VisualGraphOptions,
    ) {
        let id = sanitize_identity_id(raw_id);
        if graph.nodes.iter().any(|n| n.id == id) {
            return;
        }

        let kind = item
            .get("kind")
            .or_else(|| item.get("node_type"))
            .or_else(|| item.get("type"))
            .and_then(|v| v.as_str())
            .map(|s| Self::parse_node_kind_str(s))
            .unwrap_or_else(|| Self::infer_node_kind(raw_id, &None));

        let source_path = item
            .get("source_file")
            .or_else(|| item.get("source_path"))
            .and_then(|v| v.as_str())
            .map(sanitize_sensitive_text)
            .unwrap_or_default();

        graph.nodes.push(VisualNode {
            id: id.clone(),
            label: Self::sanitize_label(raw_name),
            kind,
            source_path,
            depth: None,
            collapsed: false,
            importance: Self::extract_string_field(item, "importance"),
            expand_token: Self::extract_string_field(item, "expand_token")
                .or_else(|| Self::extract_string_field(item, "target")),
            metadata: Self::json_to_metadata(item, options.include_evidence),
        });
    }

    /// 从关系端点创建节点（优先复用）
    fn ensure_node_by_endpoint(graph: &mut VisualGraph, item: &serde_json::Value, raw_id: &str) {
        let id = sanitize_identity_id(raw_id);
        if graph.nodes.iter().any(|n| n.id == id) {
            return;
        }

        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("label").and_then(|v| v.as_str()))
            .unwrap_or(raw_id);

        graph.nodes.push(VisualNode {
            id: id.clone(),
            label: Self::sanitize_label(name),
            kind: Self::infer_node_kind(
                &id,
                &item
                    .get("edge_type")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string),
            ),
            source_path: String::new(),
            depth: None,
            collapsed: false,
            importance: Self::extract_string_field(item, "importance"),
            expand_token: Self::extract_string_field(item, "expand_token")
                .or_else(|| Self::extract_string_field(item, "target")),
            metadata: Self::json_to_metadata(item, false),
        });
    }

    /// 确保关系边落库
    fn ensure_relation(
        graph: &mut VisualGraph,
        raw_from: &str,
        raw_to: &str,
        item: &serde_json::Value,
        default_kind: EdgeKind,
        direction: EdgeDirection,
    ) {
        let from = sanitize_identity_id(raw_from);
        let to = sanitize_identity_id(raw_to);
        let kind = item
            .get("edge_type")
            .and_then(|v| v.as_str())
            .map(|s| Self::parse_edge_kind(s))
            .unwrap_or(default_kind.clone());

        let edge_type = item
            .get("edge_type")
            .and_then(|v| v.as_str())
            .map(sanitize_sensitive_text);

        let label = item
            .get("edge_label")
            .and_then(|v| v.as_str())
            .or_else(|| edge_type.as_deref())
            .map(sanitize_sensitive_text);

        if !graph.edges.iter().any(|e| {
            e.from == from && e.to == to && format!("{:?}", e.kind) == format!("{:?}", kind)
        }) {
            graph.edges.push(VisualEdge {
                from,
                to,
                edge_type: edge_type.clone(),
                label: label.clone(),
                direction,
                priority: classify_edge_priority(&kind, label.as_deref()),
                summary: summarize_edge(&kind, label.as_deref()),
                evidence_status: "unavailable".to_string(),
                evidence: None,
                kind,
            });
        }
    }

    /// 应用截断
    fn apply_truncation(graph: &mut VisualGraph, options: &VisualGraphOptions) {
        let original_node_count = graph.nodes.len();
        let original_edge_count = graph.edges.len();
        let mut truncation_reasons: Vec<String> = Vec::new();

        if graph.nodes.len() > options.max_nodes {
            graph.truncated = true;
            truncation_reasons.push(format!(
                "nodes ({} > {})",
                original_node_count, options.max_nodes
            ));
            graph.nodes.truncate(options.max_nodes);
            // 截断后只保留两端节点都在保留列表中的边
            let kept_ids: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
            graph
                .edges
                .retain(|e| kept_ids.contains(&e.from) && kept_ids.contains(&e.to));
        }

        if graph.edges.len() > options.max_edges {
            graph.truncated = true;
            truncation_reasons.push(format!(
                "edges ({} > {})",
                original_edge_count, options.max_edges
            ));
            graph.edges.truncate(options.max_edges);
        }

        if graph.truncated && !truncation_reasons.is_empty() {
            let reason = truncation_reasons.join(", ");
            graph.truncated_reason = match graph.truncated_reason.take() {
                Some(existing) => Some(format!("{}; {}", existing, reason)),
                None => Some(reason),
            };
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
                depth: None,
                collapsed: false,
                importance: None,
                expand_token: None,
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

    /// 识别焦点节点与探索上下文
    fn resolve_focus_target(output: &AiOutput, options: &VisualGraphOptions) -> Option<String> {
        if let Some(ref target) = options.focus_target {
            return Some(sanitize_identity_id(target));
        }

        let candidates = [
            output.summary.get("center_node"),
            output.summary.get("target_id"),
            output.summary.get("model_id"),
        ];

        for candidate in candidates.iter().flatten() {
            if let Some(target) = candidate.as_str().filter(|target| !target.is_empty()) {
                return Some(sanitize_identity_id(target));
            }
        }

        output
            .query_target
            .as_ref()
            .filter(|target| !target.is_empty())
            .map(|target| sanitize_identity_id(target))
    }

    /// 从 summary 提取深度参数，保持缺省 1
    fn parse_depth_from_summary(summary: &serde_json::Value) -> usize {
        summary
            .get("depth")
            .and_then(|v| v.as_u64())
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(1)
    }

    /// 从 summary 提取方向参数
    fn parse_direction_from_summary(summary: &serde_json::Value) -> String {
        let direction = summary
            .get("direction")
            .and_then(|v| v.as_str())
            .unwrap_or("both")
            .trim()
            .to_lowercase();

        match direction.as_str() {
            "upstream" | "incoming" => "upstream".to_string(),
            "downstream" | "outgoing" => "downstream".to_string(),
            "both" | "all" | "bidirectional" => "both".to_string(),
            _ => "both".to_string(),
        }
    }

    /// 根据 diagnostics 推导截断原因
    fn extract_truncated_reason(
        diagnostics: &[crate::output::schema::Diagnostic],
    ) -> Option<String> {
        diagnostics
            .iter()
            .find(|diag| diag.code == "OUTPUT_TRUNCATED")
            .map(|diag| sanitize_sensitive_text(&diag.message))
    }

    /// 补齐节点深度、折叠和展开 token
    fn enrich_node_depth_and_collapsing(graph: &mut VisualGraph) {
        let focus = match graph.focus_node.as_deref() {
            Some(focus) => focus.to_string(),
            None => return,
        };

        let mut depth_by_id: HashMap<String, usize> = HashMap::new();
        depth_by_id.insert(focus.clone(), 0);
        let mut queue = VecDeque::from([(focus.clone(), 0usize)]);
        let mut collapsed_exists = false;

        while let Some((current_id, current_depth)) = queue.pop_front() {
            if current_depth >= graph.depth {
                continue;
            }

            let mut next_neighbors = Vec::new();
            for edge in &graph.edges {
                match graph.direction.as_str() {
                    "upstream" => {
                        if edge.to == current_id {
                            next_neighbors.push(edge.from.clone());
                        }
                    }
                    "downstream" => {
                        if edge.from == current_id {
                            next_neighbors.push(edge.to.clone());
                        }
                    }
                    _ => {
                        if edge.from == current_id {
                            next_neighbors.push(edge.to.clone());
                        }

                        if edge.to == current_id {
                            next_neighbors.push(edge.from.clone());
                        }
                    }
                }
            }

            for next_id in next_neighbors {
                if let std::collections::hash_map::Entry::Vacant(v) =
                    depth_by_id.entry(next_id.clone())
                {
                    v.insert(current_depth + 1);
                    queue.push_back((next_id, current_depth + 1));
                }
            }
        }

        for node in graph.nodes.iter_mut() {
            if let Some(depth) = depth_by_id.get(&node.id).copied() {
                node.depth = Some(depth);
                node.collapsed = depth >= 3;
                if depth >= 3 {
                    if node.expand_token.is_none() {
                        node.expand_token = Some(Self::build_expand_token(&node.id, depth + 1));
                    }
                    collapsed_exists = true;
                }
            }

            if node.importance.is_none() && matches!(node.kind, NodeKind::Component) {
                node.importance = Some("static_display".to_string());
            }
        }

        graph.collapsed = collapsed_exists;

        if graph.focus_node.as_ref().is_none() {
            for node in graph.nodes.iter_mut() {
                if node.depth == Some(0) {
                    graph.focus_node = Some(node.id.clone());
                    break;
                }
            }
        }
    }

    /// 构建展开 token
    fn build_expand_token(node_id: &str, next_depth: usize) -> String {
        let safe_id = sanitize_sensitive_text(node_id);
        format!("--context '{}' --depth {}", safe_id, next_depth)
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
                let sanitized_path = sanitize_sensitive_text(&path);
                let group_id = format!("group_{}", Self::sanitize_id(&sanitized_path));
                graph.groups.push(VisualGroup {
                    id: group_id,
                    label: sanitized_path,
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
            _ => EdgeKind::Other(sanitize_sensitive_text(s)),
        }
    }

    /// 从 JSON 字段提取字符串（未命中返回 None）
    fn extract_string_field(item: &serde_json::Value, field: &str) -> Option<String> {
        item.get(field)
            .and_then(|value| value.as_str())
            .map(sanitize_sensitive_text)
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
    fn build_node_metadata(
        evidence: &Evidence,
        include_evidence: bool,
    ) -> HashMap<String, serde_json::Value> {
        let mut m = HashMap::new();
        if include_evidence {
            if let Some(ref raw_expr) = evidence.raw_expr {
                m.insert(
                    "raw_expr".to_string(),
                    sanitize_metadata_entry("raw_expr", &serde_json::json!(raw_expr)),
                );
            }
            if let Some(ref json_path) = evidence.json_path {
                m.insert(
                    "json_path".to_string(),
                    sanitize_metadata_entry("json_path", &serde_json::json!(json_path)),
                );
            }
        }
        m.insert(
            "confidence".to_string(),
            serde_json::json!(format!("{:?}", evidence.confidence)),
        );
        m
    }

    /// 将 JSON 值转换为元数据映射
    fn json_to_metadata(
        value: &serde_json::Value,
        include_evidence: bool,
    ) -> HashMap<String, serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .filter(|(key, _)| include_evidence || !Self::is_evidence_metadata_key(key))
                .map(|(k, v)| (k.clone(), sanitize_metadata_entry(k, v)))
                .collect(),
            _ => HashMap::new(),
        }
    }

    /// 判断 metadata key 是否属于证据细节
    fn is_evidence_metadata_key(key: &str) -> bool {
        matches!(
            key,
            "raw_expr" | "rawExpression" | "evidence" | "reason" | "json_path" | "jsonPath"
        )
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
