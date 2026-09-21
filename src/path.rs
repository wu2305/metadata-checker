use crate::graph::{Edge, EdgeType, Node, NodeType};
use crate::graph_store::GraphReadStore;
use serde::{Deserialize, Serialize};
// use std::collections::HashMap; // reserved for future use

/// 路径节点引用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathNodeRef {
    pub node_id: String,
    pub node_type: String,
    pub path: String,
    pub name: String,
}

impl From<&Node> for PathNodeRef {
    fn from(node: &Node) -> Self {
        PathNodeRef {
            node_id: node.id.clone(),
            node_type: format!("{:?}", node.node_type),
            path: node.path.clone(),
            name: node.name.clone(),
        }
    }
}

impl From<Node> for PathNodeRef {
    fn from(node: Node) -> Self {
        PathNodeRef {
            node_id: node.id,
            node_type: format!("{:?}", node.node_type),
            path: node.path,
            name: node.name,
        }
    }
}

/// 路径边引用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathEdgeRef {
    pub from: String,
    pub to: String,
    pub edge_type: String,
    pub field_path: Option<String>,
    pub json_path: Option<String>,
    pub source_expr: Option<String>,
    pub source_file: Option<String>,
}

impl From<&Edge> for PathEdgeRef {
    fn from(edge: &Edge) -> Self {
        PathEdgeRef {
            from: edge.from.clone(),
            to: edge.to.clone(),
            edge_type: format!("{:?}", edge.edge_type),
            field_path: edge.field_path.clone(),
            json_path: edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source_expr: edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source_file: None,
        }
    }
}

impl From<Edge> for PathEdgeRef {
    fn from(edge: Edge) -> Self {
        PathEdgeRef {
            from: edge.from,
            to: edge.to,
            edge_type: format!("{:?}", edge.edge_type),
            field_path: edge.field_path,
            json_path: edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source_expr: edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source_file: None,
        }
    }
}

/// 路径段（from -> to 的单一跃迁）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathSegment {
    pub from: PathNodeRef,
    pub to: PathNodeRef,
    pub edge: PathEdgeRef,
    pub evidence: String,
    pub confidence: String,
}

/// 路径终端类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathTerminal {
    TargetComponent,
    SourceComponent,
    SinkModel,
    SinkPhysicalField,
    CrossPageWriter,
    ConditionAnchor,
    DataFilter,
    TotalRowCount,
    Entrypoint,
    Unknown,
}

/// 路径分类
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PathClassification {
    PrimaryPath,
    CandidatePath,
    SupportingPath,
    RelatedContext,
    RejectedPath,
}

/// 路径候选
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathCandidate {
    pub path_id: String,
    pub purpose: String,
    pub terminals: Vec<PathTerminal>,
    pub segments: Vec<PathSegment>,
    pub evidence: Vec<String>,
    pub selection_reason: String,
    pub classification: PathClassification,
    pub classification_reason: String,
    pub confidence: String,
    pub diagnostics: Vec<String>,
    pub rank_features: PathRankFeatures,
}

/// 路径排名特征（不直接等于打分，供策略使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRankFeatures {
    pub contains_target_component: bool,
    pub contains_physical_field: bool,
    pub contains_cross_page_writer: bool,
    pub field_name_match: bool,
    pub same_page: bool,
    pub has_condition_node: bool,
    pub has_json_path: bool,
    pub edge_type_sequence: Vec<String>,
    pub path_length: usize,
    pub source_file_count: usize,
    pub contains_only_structural_edges: bool,
    pub contains_dataflow_side_branch: bool,
    pub contains_entrypoint: bool,
    pub contains_model_filter: bool,
    pub contains_model_read: bool,
    pub contains_model_write: bool,
    pub contains_field_alias: bool,
    pub contains_field_write: bool,
}

impl Default for PathRankFeatures {
    fn default() -> Self {
        PathRankFeatures {
            contains_target_component: false,
            contains_physical_field: false,
            contains_cross_page_writer: false,
            field_name_match: false,
            same_page: false,
            has_condition_node: false,
            has_json_path: false,
            edge_type_sequence: Vec::new(),
            path_length: 0,
            source_file_count: 0,
            contains_only_structural_edges: false,
            contains_dataflow_side_branch: false,
            contains_entrypoint: false,
            contains_model_filter: false,
            contains_model_read: false,
            contains_model_write: false,
            contains_field_alias: false,
            contains_field_write: false,
        }
    }
}

/// 路径查询参数
#[derive(Debug, Clone)]
pub struct PathQuery {
    pub page_id: String,
    pub page_path: String,
    pub target_anchors: Vec<String>,
    pub source_anchors: Vec<String>,
    pub sink_anchors: Vec<String>,
    pub bridge_anchors: Vec<String>,
    pub excluded_anchors: Vec<String>,
    pub budget: String,
}

/// 路径选择结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathSelectionResult {
    pub primary_paths: Vec<PathCandidate>,
    pub candidate_paths: Vec<PathCandidate>,
    pub supporting_paths: Vec<PathCandidate>,
    pub related_context: Vec<PathCandidate>,
    pub rejected_paths: Vec<PathCandidate>,
    pub selection_diagnostics: Vec<String>,
}

/// 路径选择器 trait
pub trait PathSelector {
    fn select(&self, query: &PathQuery, candidates: Vec<PathCandidate>) -> PathSelectionResult;
}

/// 路径发现器 trait
pub trait PathFinder {
    fn find_candidates(&self, graph: &dyn GraphReadStore, query: &PathQuery) -> Vec<PathCandidate>;
}

/// 锚点提取器
///
/// 从页面中提取用于路径计算的关键锚点：
/// - target_anchors: 用户关心的目标组件/字段
/// - source_anchors: 数据来源锚点（入口参数、系统变量）
/// - sink_anchors: 数据汇聚锚点（模型字段、物理表字段）
/// - bridge_anchors: 桥接锚点（跨页面 writer、DataFlow 连接）
/// - excluded_anchors: 明确排除的锚点（其他页面同名局部模型）
pub struct AnchorExtractor;

fn dedup_anchors_preserve_order(anchors: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    anchors.retain(|anchor| seen.insert(anchor.clone()));
}

impl AnchorExtractor {
    /// 从 query_page_logic 的已知上下文提取锚点
    pub fn extract(
        graph: &dyn GraphReadStore,
        page_id: &str,
        page_node: &crate::graph::Node,
        _child_components: &Vec<&crate::graph::Node>,
        _child_actions: &Vec<&crate::graph::Node>,
        data_sources: &Vec<serde_json::Value>,
        write_targets: &Vec<serde_json::Value>,
        entrypoints: &Vec<serde_json::Value>,
    ) -> PathQuery {
        let mut target_anchors: Vec<String> = Vec::new();
        let mut source_anchors: Vec<String> = Vec::new();
        let mut sink_anchors: Vec<String> = Vec::new();
        let mut bridge_anchors: Vec<String> = Vec::new();
        let mut excluded_anchors: Vec<String> = Vec::new();

        // 1. 目标组件锚点：优先 data_sources / write_targets 组件，再 entrypoints
        let mut seen_anchors: std::collections::HashSet<String> = std::collections::HashSet::new();
        // 从 data_sources 中提取读取模型的组件作为目标锚点（数据语义优先）
        for ds in data_sources {
            if let Some(src) = ds.get("source_component").and_then(|v| v.as_str()) {
                if seen_anchors.insert(src.to_string()) {
                    target_anchors.push(src.to_string());
                }
            }
        }
        // 从 write_targets 中提取写入组件作为目标锚点
        for wt in write_targets {
            if let Some(src) = wt.get("source_component").and_then(|v| v.as_str()) {
                if seen_anchors.insert(src.to_string()) {
                    target_anchors.push(src.to_string());
                }
            }
        }
        // 最后收集 entrypoints
        for ep in entrypoints {
            if let Some(id) = ep.get("id").and_then(|v| v.as_str()) {
                if seen_anchors.insert(id.to_string()) {
                    target_anchors.push(id.to_string());
                }
            }
        }

        // 2. sink 锚点：数据写入目标（物理表字段优先）
        for wt in write_targets {
            if let Some(tgt) = wt.get("target_id").and_then(|v| v.as_str()) {
                sink_anchors.push(tgt.to_string());
            }
            // 物理表字段：从 meta 或 field_path 推断
            if let Some(fp) = wt.get("field_path").and_then(|v| v.as_str()) {
                if fp.contains('.') {
                    sink_anchors.push(fp.to_string());
                }
            }
        }

        // 3. 数据来源锚点：data_sources 指向的模型和字段
        for ds in data_sources {
            if let Some(tgt) = ds.get("target_id").and_then(|v| v.as_str()) {
                source_anchors.push(tgt.to_string());
            }
            // 从 raw_expr 解析局部模型字段锚点，例如 model22.phoneNumber -> field:model22.phoneNumber
            if let Some(expr) = ds.get("raw_expr").and_then(|v| v.as_str()) {
                if let Some(dot) = expr.find('.') {
                    let model = &expr[..dot];
                    let field = &expr[dot + 1..];
                    if !model.is_empty() && !field.is_empty() && !model.starts_with('$') {
                        source_anchors.push(format!("field:{}.{}", model, field));
                    }
                }
            }
        }

        // 4. 桥接锚点：跨页面 writer（从 model 的 incoming ActionWrites 收集）
        let mut model_targets: std::collections::HashSet<String> = std::collections::HashSet::new();
        for ds in data_sources {
            if let Some(tgt) = ds.get("target_id").and_then(|v| v.as_str()) {
                if tgt.starts_with("model:") || tgt.starts_with("field:") {
                    model_targets.insert(tgt.to_string());
                }
            }
        }
        for model_id in &model_targets {
            if let Some(neighbors) = graph.get_node_edges(model_id).ok().flatten() {
                for edge_view in &neighbors.incoming {
                    let source = &edge_view.node;
                    let edge = &edge_view.edge;
                    if matches!(
                        edge.edge_type,
                        crate::graph::EdgeType::ActionWrites
                            | crate::graph::EdgeType::Writes
                            | crate::graph::EdgeType::FieldWrite
                    ) && source.path != page_node.path
                    {
                        bridge_anchors.push(source.id.clone());
                    }
                }
            }
        }

        // 5. 排除锚点：其他页面同名局部模型
        for tgt in &model_targets {
            if let Some(neighbors) = graph.get_node_edges(tgt).ok().flatten() {
                for edge_view in &neighbors.incoming {
                    let source = &edge_view.node;
                    let edge = &edge_view.edge;
                    if matches!(edge.edge_type, crate::graph::EdgeType::DependsOn)
                        && source.path != page_node.path
                        && source.id.starts_with("cond:")
                    {
                        // 收集其他页面的条件节点作为排除锚点参考
                        excluded_anchors.push(source.id.clone());
                    }
                }
            }
        }

        dedup_anchors_preserve_order(&mut source_anchors);
        dedup_anchors_preserve_order(&mut sink_anchors);
        dedup_anchors_preserve_order(&mut bridge_anchors);
        dedup_anchors_preserve_order(&mut excluded_anchors);

        PathQuery {
            page_id: page_id.to_string(),
            page_path: page_node.path.clone(),
            target_anchors,
            source_anchors,
            sink_anchors,
            bridge_anchors,
            excluded_anchors,
            budget: "normal".to_string(),
        }
    }
}

/// 有界因果路径发现器
///
/// 限制深度 4-6，限制边类型，防止全图扩散。
/// 保留被截断、被跳过、被排除的诊断。
pub struct BoundedCausalPathFinder {
    pub max_depth: usize,
    pub allowed_edge_types: Vec<EdgeType>,
}

impl Default for BoundedCausalPathFinder {
    fn default() -> Self {
        BoundedCausalPathFinder {
            max_depth: 5,
            allowed_edge_types: vec![
                EdgeType::Reads,
                EdgeType::Writes,
                EdgeType::ActionReads,
                EdgeType::ActionWrites,
                EdgeType::DependsOn,
                EdgeType::Contains,
                EdgeType::DataflowInput,
                EdgeType::DataflowInternal,
                EdgeType::DataflowOutput,
                EdgeType::FieldAlias,
                EdgeType::FieldWrite,
                EdgeType::Triggers,
            ],
        }
    }
}

impl PathFinder for BoundedCausalPathFinder {
    fn find_candidates(&self, graph: &dyn GraphReadStore, query: &PathQuery) -> Vec<PathCandidate> {
        let mut candidates: Vec<PathCandidate> = Vec::new();
        let mut diagnostics: Vec<String> = Vec::new();
        let mut visited_paths: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 从每个 target_anchor 出发做 BFS，寻找到 sink_anchor / source_anchor / bridge_anchor 的路径
        const MAX_CANDIDATES_PER_ANCHOR: usize = 15;
        const MAX_TOTAL_CANDIDATES: usize = 10000;
        for anchor in &query.target_anchors {
            if candidates.len() >= MAX_TOTAL_CANDIDATES {
                diagnostics.push(format!(
                    "Total candidates reached limit {}, stopping further expansion",
                    MAX_TOTAL_CANDIDATES
                ));
                break;
            }
            let mut anchor_candidate_count = 0;
            let mut bfs_queue: Vec<(Vec<PathSegment>, Vec<String>, usize)> = Vec::new();
            bfs_queue.push((Vec::new(), vec![anchor.clone()], 0));

            while let Some((segments, visited_nodes, depth)) = bfs_queue.pop() {
                if anchor_candidate_count >= MAX_CANDIDATES_PER_ANCHOR {
                    diagnostics.push(format!(
                        "Anchor {} reached candidate limit {}, skipping further expansion",
                        anchor, MAX_CANDIDATES_PER_ANCHOR
                    ));
                    break;
                }
                if depth >= self.max_depth {
                    diagnostics.push(format!(
                        "Path truncated at depth {} from anchor {}",
                        depth, anchor
                    ));
                    continue;
                }

                let current_node = match visited_nodes.last() {
                    Some(n) => n,
                    None => continue,
                };

                if let Some(neighbors) = graph.get_node_edges(current_node).ok().flatten() {
                    // 正常 outgoing 遍历
                    for edge_view in neighbors.outgoing {
                        let target = edge_view.node;
                        let edge = edge_view.edge;
                        if !self.allowed_edge_types.contains(&edge.edge_type) {
                            continue;
                        }

                        let next_id = target.id.clone();
                        if visited_nodes.contains(&next_id)
                            && edge.edge_type != EdgeType::FieldAlias
                        {
                            continue; // 防止环，但允许 FieldAlias 回退
                        }

                        let target_id = target.id.clone();
                        let segment = PathSegment {
                            from: PathNodeRef::from(
                                graph.get_node(current_node).ok().flatten().unwrap_or(
                                    crate::graph::Node {
                                        id: current_node.clone(),
                                        node_type: NodeType::Component,
                                        path: query.page_path.clone(),
                                        name: current_node.clone(),
                                        meta: None,
                                    , origin_file: None},
                                ),
                            ),
                            to: PathNodeRef::from(target),
                            edge: PathEdgeRef::from(edge.clone()),
                            evidence: format!(
                                "{} -> {} via {:?}",
                                current_node, target_id, edge.edge_type
                            ),
                            confidence: if edge.field_path.is_some() {
                                "high".to_string()
                            } else {
                                "medium".to_string()
                            },
                        };

                        let mut new_segments = segments.clone();
                        new_segments.push(segment);
                        let mut new_visited = visited_nodes.clone();
                        new_visited.push(next_id.clone());

                        // 检查是否到达目标锚点集合
                        let is_target_reached = query.sink_anchors.contains(&next_id)
                            || query.source_anchors.contains(&next_id)
                            || query.bridge_anchors.contains(&next_id)
                            || query.target_anchors.contains(&next_id);

                        // 检查是否连接了两个有意义的锚点
                        let connects_anchors = query.target_anchors.contains(anchor)
                            && (query.sink_anchors.contains(&next_id)
                                || query.bridge_anchors.contains(&next_id));

                        if is_target_reached || connects_anchors || new_segments.len() >= 2 {
                            let path_key = new_visited.join(">");
                            if !visited_paths.contains(&path_key) {
                                visited_paths.insert(path_key.clone());
                                let candidate =
                                    build_candidate(&path_key, &new_segments, query, &diagnostics);
                                candidates.push(candidate);
                                anchor_candidate_count += 1;
                            }
                        }

                        // 继续 BFS（限制分支数，防止高扇出）
                        if bfs_queue.len() < 200 {
                            bfs_queue.push((new_segments, new_visited, depth + 1));
                        } else {
                            diagnostics.push(format!(
                                "BFS queue overflow from anchor {}, skipping further expansion",
                                anchor
                            ));
                        }
                    }
                    // 对 Model 节点反向遍历 incoming ActionWrites/Writes 边，找到写入者 action
                    if current_node.starts_with("model:") || current_node.starts_with("field:") {
                        for edge_view in &neighbors.incoming {
                            let source = &edge_view.node;
                            let edge = &edge_view.edge;
                            if matches!(
                                edge.edge_type,
                                crate::graph::EdgeType::ActionWrites
                                    | crate::graph::EdgeType::Writes
                                    | crate::graph::EdgeType::FieldWrite
                            ) && self.allowed_edge_types.contains(&edge.edge_type)
                            {
                                let next_id = source.id.clone();
                                if visited_nodes.contains(&next_id)
                                    && edge.edge_type != crate::graph::EdgeType::FieldWrite
                                {
                                    continue;
                                }
                                let segment = PathSegment {
                                    from: PathNodeRef::from(
                                        graph.get_node(current_node).ok().flatten().unwrap_or(
                                            crate::graph::Node {
                                                id: current_node.clone(),
                                                node_type: NodeType::Component,
                                                path: query.page_path.clone(),
                                                name: current_node.clone(),
                                                meta: None,
                                            , origin_file: None},
                                        ),
                                    ),
                                    to: PathNodeRef::from((*source).clone()),
                                    edge: PathEdgeRef::from((*edge).clone()),
                                    evidence: format!(
                                        "{} <- {} via {:?}",
                                        current_node, source.id, edge.edge_type
                                    ),
                                    confidence: if edge.field_path.is_some() {
                                        "high".to_string()
                                    } else {
                                        "medium".to_string()
                                    },
                                };
                                let mut new_segments = segments.clone();
                                new_segments.push(segment);
                                let mut new_visited = visited_nodes.clone();
                                new_visited.push(next_id.clone());
                                let is_target_reached = query.sink_anchors.contains(&next_id)
                                    || query.source_anchors.contains(&next_id)
                                    || query.bridge_anchors.contains(&next_id)
                                    || query.target_anchors.contains(&next_id);
                                let connects_anchors = query.target_anchors.contains(anchor)
                                    && (query.sink_anchors.contains(&next_id)
                                        || query.bridge_anchors.contains(&next_id));
                                if is_target_reached || connects_anchors || new_segments.len() >= 2
                                {
                                    let path_key = new_visited.join(">");
                                    if !visited_paths.contains(&path_key) {
                                        visited_paths.insert(path_key.clone());
                                        let candidate = build_candidate(
                                            &path_key,
                                            &new_segments,
                                            query,
                                            &diagnostics,
                                        );
                                        candidates.push(candidate);
                                        anchor_candidate_count += 1;
                                    }
                                }
                                if bfs_queue.len() < 200 {
                                    bfs_queue.push((new_segments, new_visited, depth + 1));
                                } else {
                                    diagnostics.push(format!(
                                        "BFS queue overflow from anchor {}, skipping further expansion",
                                        anchor
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }

        candidates
    }
}

/// 为单个 data_source 构造字段级因果路径
///
/// 算法：
/// 1. 从 data_source.source_component 得到起点（组件）
/// 2. 从 data_source.raw_expr 解析局部模型字段，例如 model22.phoneNumber
/// 3. 构造本地字段节点 field:model22.phoneNumber
/// 4. 通过 FieldAlias 找 canonical 字段 field:fact_qwSidebar.phoneNumber
/// 5. 从 canonical 字段的 incoming 边中找 FieldWrite
/// 6. 只接受 edge.field_path 匹配目标字段的 writer
/// 7. 生成三段路径：组件 -> 局部字段 -> canonical 字段 <- action
pub fn build_field_causal_paths_for_data_source(
    graph: &dyn GraphReadStore,
    page_node: &crate::graph::Node,
    data_source: &serde_json::Value,
) -> Vec<PathCandidate> {
    let mut candidates: Vec<PathCandidate> = Vec::new();

    let src = match data_source.get("source_component").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return candidates,
    };
    let fp = data_source.get("field_path").and_then(|v| v.as_str());
    let raw_expr = data_source.get("raw_expr").and_then(|v| v.as_str());

    // 解析 raw_expr 得到局部模型字段，例如 model22.phoneNumber
    // 去掉 ${} 包装，例如 ${model22.phoneNumber} -> model22.phoneNumber
    let stripped_expr = raw_expr.map(|e| {
        e.strip_prefix("${")
            .unwrap_or(e)
            .strip_suffix("}")
            .unwrap_or(e)
    });
    let local_field_id = match stripped_expr {
        Some(expr) => {
            if let Some(dot) = expr.find('.') {
                let model = &expr[..dot];
                let field = &expr[dot + 1..];
                if !model.is_empty() && !field.is_empty() && !model.starts_with('$') {
                    format!("field:{}.{}", model, field)
                } else {
                    return candidates;
                }
            } else {
                return candidates;
            }
        }
        None => return candidates,
    };

    // 构造 query（仅用于 build_candidate）
    let query = PathQuery {
        page_id: page_node.id.clone(),
        page_path: page_node.path.clone(),
        target_anchors: vec![src.to_string()],
        source_anchors: vec![local_field_id.clone()],
        sink_anchors: Vec::new(),
        bridge_anchors: Vec::new(),
        excluded_anchors: Vec::new(),
        budget: "normal".to_string(),
    };

    // 组件 -> 局部字段段
    let comp_to_local_seg = PathSegment {
        from: PathNodeRef {
            node_id: src.to_string(),
            node_type: "Component".to_string(),
            path: page_node.path.clone(),
            name: src.split('|').last().unwrap_or(src).to_string(),
        },
        to: PathNodeRef {
            node_id: local_field_id.clone(),
            node_type: "Field".to_string(),
            path: page_node.path.clone(),
            name: local_field_id.split('.').last().unwrap_or("").to_string(),
        },
        edge: PathEdgeRef {
            from: src.to_string(),
            to: local_field_id.clone(),
            edge_type: "Reads".to_string(),
            field_path: fp.map(|s| s.to_string()),
            json_path: data_source
                .get("json_path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            source_expr: raw_expr.map(|s| s.to_string()),
            source_file: Some(page_node.path.clone()),
        },
        evidence: format!("{} -> {} via Reads", src, local_field_id),
        confidence: "high".to_string(),
    };

    // 查找局部字段的 outgoing FieldAlias，找到 canonical 字段
    if let Some(neighbors) = graph.get_node_edges(&local_field_id).ok().flatten() {
        for edge_view in neighbors.outgoing {
            let target = edge_view.node;
            let edge = edge_view.edge;
            if edge.edge_type == crate::graph::EdgeType::FieldAlias {
                let canonical_field_id = target.id.clone();
                let local_to_canonical_seg = PathSegment {
                    from: PathNodeRef {
                        node_id: local_field_id.clone(),
                        node_type: "Field".to_string(),
                        path: page_node.path.clone(),
                        name: local_field_id.split('.').last().unwrap_or("").to_string(),
                    },
                    to: PathNodeRef {
                        node_id: canonical_field_id.clone(),
                        node_type: "Field".to_string(),
                        path: target.path.clone(),
                        name: canonical_field_id
                            .split('.')
                            .last()
                            .unwrap_or("")
                            .to_string(),
                    },
                    edge: PathEdgeRef {
                        from: local_field_id.clone(),
                        to: canonical_field_id.clone(),
                        edge_type: "FieldAlias".to_string(),
                        field_path: edge.field_path.clone(),
                        json_path: None,
                        source_expr: None,
                        source_file: Some(page_node.path.clone()),
                    },
                    evidence: format!(
                        "{} -> {} via FieldAlias",
                        local_field_id, canonical_field_id
                    ),
                    confidence: "high".to_string(),
                };

                // 从 canonical 字段反向查找 FieldWrite
                if let Some(neighbors) = graph.get_node_edges(&canonical_field_id).ok().flatten() {
                    for edge_view in neighbors.incoming {
                        let source = edge_view.node;
                        let edge = edge_view.edge;
                        if edge.edge_type == crate::graph::EdgeType::FieldWrite
                            && source.path != page_node.path
                        {
                            // 只接受字段匹配的 writer
                            let edge_fp = edge.field_path.as_deref().unwrap_or("");
                            let canonical_field_name =
                                canonical_field_id.split('.').last().unwrap_or("");
                            if edge_fp.ends_with(canonical_field_name) {
                                let canonical_to_action_seg = PathSegment {
                                    from: PathNodeRef {
                                        node_id: canonical_field_id.clone(),
                                        node_type: "Field".to_string(),
                                        path: target.path.clone(),
                                        name: canonical_field_name.to_string(),
                                    },
                                    to: PathNodeRef {
                                        node_id: source.id.clone(),
                                        node_type: "Action".to_string(),
                                        path: source.path.clone(),
                                        name: source.name.clone(),
                                    },
                                    edge: PathEdgeRef {
                                        from: canonical_field_id.clone(),
                                        to: source.id.clone(),
                                        edge_type: "FieldWrite".to_string(),
                                        field_path: edge.field_path.clone(),
                                        json_path: None,
                                        source_expr: edge
                                            .meta
                                            .as_ref()
                                            .and_then(|m| m.get("source_expr"))
                                            .and_then(|v| v.as_str())
                                            .map(|s| s.to_string()),
                                        source_file: Some(source.path.clone()),
                                    },
                                    evidence: format!(
                                        "{} <- {} via FieldWrite",
                                        canonical_field_id, source.id
                                    ),
                                    confidence: "high".to_string(),
                                };

                                let mut segs = Vec::new();
                                segs.push(comp_to_local_seg.clone());
                                segs.push(local_to_canonical_seg.clone());
                                segs.push(canonical_to_action_seg);

                                let path_id = format!(
                                    "{}>{}>{}>{}",
                                    src, local_field_id, canonical_field_id, source.id
                                );
                                let candidate =
                                    build_candidate(&path_id, &segs, &query, &Vec::new());
                                candidates.push(candidate);
                            }
                        }
                    }
                }
            }
        }
    }

    candidates
}
/// 根据路径段构建候选路径
fn build_candidate(
    path_id: &str,
    segments: &[PathSegment],
    query: &PathQuery,
    _diagnostics: &[String],
) -> PathCandidate {
    let rank_features = compute_rank_features(segments, query);
    let (classification, classification_reason) = classify_path(segments, &rank_features, query);
    let confidence =
        if rank_features.contains_physical_field && rank_features.contains_target_component {
            "high"
        } else if rank_features.contains_model_read || rank_features.contains_model_write {
            "medium"
        } else {
            "low"
        }
        .to_string();

    let mut terminals = Vec::new();
    if rank_features.contains_target_component {
        terminals.push(PathTerminal::TargetComponent);
    }
    if rank_features.contains_cross_page_writer {
        terminals.push(PathTerminal::CrossPageWriter);
    }
    if rank_features.contains_model_filter {
        terminals.push(PathTerminal::DataFilter);
    }
    if rank_features.contains_entrypoint {
        terminals.push(PathTerminal::Entrypoint);
    }
    if rank_features.contains_physical_field {
        terminals.push(PathTerminal::SinkPhysicalField);
    }
    if terminals.is_empty() {
        terminals.push(PathTerminal::Unknown);
    }

    PathCandidate {
        path_id: path_id.to_string(),
        purpose: "自动发现路径".to_string(),
        terminals,
        segments: segments.to_vec(),
        evidence: segments.iter().map(|s| s.evidence.clone()).collect(),
        selection_reason: classification_reason.clone(),
        classification,
        classification_reason,
        confidence,
        diagnostics: Vec::new(),
        rank_features,
    }
}

/// 计算路径排名特征
fn compute_rank_features(segments: &[PathSegment], query: &PathQuery) -> PathRankFeatures {
    let mut features = PathRankFeatures::default();
    features.path_length = segments.len();

    let mut seen_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut edge_types: Vec<String> = Vec::new();

    for seg in segments {
        edge_types.push(seg.edge.edge_type.clone());
        seen_files.insert(seg.from.path.clone());
        seen_files.insert(seg.to.path.clone());

        if query.target_anchors.contains(&seg.from.node_id)
            || query.target_anchors.contains(&seg.to.node_id)
        {
            features.contains_target_component = true;
        }

        if query.sink_anchors.contains(&seg.from.node_id)
            || query.sink_anchors.contains(&seg.to.node_id)
        {
            features.contains_model_write = true;
        }

        if query.source_anchors.contains(&seg.from.node_id)
            || query.source_anchors.contains(&seg.to.node_id)
        {
            features.contains_model_read = true;
        }

        if query.bridge_anchors.contains(&seg.from.node_id)
            || query.bridge_anchors.contains(&seg.to.node_id)
        {
            features.contains_cross_page_writer = true;
        }

        // 字段级写入检测：含 FieldWrite 边即标记
        if seg.edge.edge_type == "FieldWrite" {
            features.contains_field_write = true;
        }

        // 跨页 writer 检测：边类型为写边，且目标节点路径不等于当前页面
        if matches!(
            seg.edge.edge_type.as_str(),
            "FieldWrite" | "ActionWrites" | "Writes"
        ) && seg.to.path != query.page_path
        {
            features.contains_cross_page_writer = true;
        }

        if seg.to.node_type == "Condition" || seg.from.node_type == "Condition" {
            features.has_condition_node = true;
        }

        if seg.edge.json_path.is_some() {
            features.has_json_path = true;
        }

        if seg.from.path == query.page_path && seg.to.path == query.page_path {
            features.same_page = true;
        }

        // 物理表字段检测：field_path 包含 ".tbl." 或 sink_anchors 中的物理字段
        if let Some(ref fp) = seg.edge.field_path {
            if fp.contains("tbl.") || fp.contains("phoneNumber") || fp.contains("fact_") {
                features.contains_physical_field = true;
            }
        }

        // 模型过滤检测
        if seg.edge.edge_type == "DependsOn"
            && (seg.to.node_id.starts_with("model:") || seg.from.node_id.starts_with("model:"))
        {
            features.contains_model_filter = true;
        }
    }

    features.source_file_count = seen_files.len();
    // 检测 FieldAlias 边
    features.contains_field_alias = edge_types.iter().any(|et| et == "FieldAlias");
    features.edge_type_sequence = edge_types;
    features.contains_only_structural_edges = features
        .edge_type_sequence
        .iter()
        .all(|et| matches!(et.as_str(), "Contains" | "Triggers"));

    features
}

/// 路径分类规则
///
/// - primary_path: 同时包含当前页面目标、物理表字段、读/写因果边
/// - supporting_path: 当前页面内条件/过滤/显示门控
/// - related_context: 跨页面但不写目标字段，或字段不匹配
/// - rejected_path: 结构边、导航边、无字段证据、过长路径
fn classify_path(
    segments: &[PathSegment],
    features: &PathRankFeatures,
    query: &PathQuery,
) -> (PathClassification, String) {
    // 1. 首先排除明显不相关的路径
    if features.contains_only_structural_edges {
        return (
            PathClassification::RejectedPath,
            "仅包含结构边（Contains/Triggers），无数据语义".to_string(),
        );
    }

    if segments.len() > 6 {
        return (
            PathClassification::RejectedPath,
            format!("路径过长（{} 段），超过因果推断合理范围", segments.len()),
        );
    }

    // 2. 检查是否有页面作用域污染
    let has_cross_page_non_writer = segments.iter().any(|seg| {
        (seg.from.path != query.page_path || seg.to.path != query.page_path)
            && !features.contains_cross_page_writer
            && !query.bridge_anchors.contains(&seg.from.node_id)
            && !query.bridge_anchors.contains(&seg.to.node_id)
    });

    if has_cross_page_non_writer
        && !features.contains_physical_field
        && !features.contains_cross_page_writer
    {
        return (
            PathClassification::RelatedContext,
            "跨页面关系，但无物理字段写入证据".to_string(),
        );
    }

    // 3. 主链路判定：必须有数据语义（读/写/条件依赖）
    let has_data_semantic = features.contains_model_read
        || features.contains_model_write
        || features.has_condition_node
        || features.contains_physical_field;

    let has_target = features.contains_target_component || features.contains_entrypoint;

    // 字段级主链路强制提升：含 FieldAlias + 跨页 writer 直接升为主链路
    if features.contains_field_alias
        && features.contains_field_write
        && features.contains_cross_page_writer
        && features.contains_target_component
        && features.contains_physical_field
    {
        return (
            PathClassification::PrimaryPath,
            "字段级主链路：含目标组件、FieldAlias、物理字段和跨页 action writer".to_string(),
        );
    }

    // primary_path: 同时有数据语义 + 目标锚点 + 物理表字段或跨页 writer
    if has_data_semantic
        && has_target
        && (features.contains_physical_field || features.contains_cross_page_writer)
    {
        return (
            PathClassification::PrimaryPath,
            "包含目标组件、数据语义和物理表/跨页写入证据".to_string(),
        );
    }

    // primary_path 降级版：当前页面内的强因果链
    if has_data_semantic
        && has_target
        && features.same_page
        && (features.contains_model_read || features.contains_model_write)
    {
        return (
            PathClassification::PrimaryPath,
            "当前页面内强因果链（组件->模型读写）".to_string(),
        );
    }

    // supporting_path: 当前页面条件/过滤门控
    if features.has_condition_node
        && features.same_page
        && (features.contains_model_filter || features.contains_target_component)
    {
        return (
            PathClassification::SupportingPath,
            "当前页面条件门控或过滤依赖".to_string(),
        );
    }

    // candidate_path: 有数据语义但缺少强锚点连接
    if has_data_semantic {
        return (
            PathClassification::CandidatePath,
            "有数据语义但缺少目标组件或物理字段的直接连接".to_string(),
        );
    }

    // 默认降级为 related_context
    (
        PathClassification::RelatedContext,
        "无明确数据语义或目标连接".to_string(),
    )
}

/// 基于规则的路径选择器
///
/// 采用"分组保底"策略，避免单一打分忽视问题：
/// - 至少 1 条 value source path（组件->模型->物理表）
/// - 至少 1 条 data prerequisite path（条件/filter）
/// - 至少 1 条 display gate path（visibleCondition/disableCondition）
/// - 至少 1 条 action/write source path（entrypoint->写入）
/// - 如果存在跨页 writer，必须保留至少 1 条 writer path
/// - 如果存在目标组件（如 input3），优先保留包含该组件的路径
pub struct RuleBasedPathSelector;

impl PathSelector for RuleBasedPathSelector {
    fn select(&self, query: &PathQuery, candidates: Vec<PathCandidate>) -> PathSelectionResult {
        let mut primary_paths: Vec<PathCandidate> = Vec::new();
        let mut candidate_paths: Vec<PathCandidate> = Vec::new();
        let mut supporting_paths: Vec<PathCandidate> = Vec::new();
        let mut related_context: Vec<PathCandidate> = Vec::new();
        let mut rejected_paths: Vec<PathCandidate> = Vec::new();
        let mut selection_diagnostics: Vec<String> = Vec::new();

        // 第一步：按分类分组
        for c in candidates {
            match c.classification {
                PathClassification::PrimaryPath => primary_paths.push(c),
                PathClassification::CandidatePath => candidate_paths.push(c),
                PathClassification::SupportingPath => supporting_paths.push(c),
                PathClassification::RelatedContext => related_context.push(c),
                PathClassification::RejectedPath => rejected_paths.push(c),
            }
        }

        // 第二步：分组保底检查。
        // 候选会被逐个移出池，因此显式保留本批确定化后的优先级：先写入与跨页写入，
        // 再补数据/显示条件，最后目标组件和值来源；不能由规则名称的字典序隐式决定。
        let guarantees = [
            (
                "action_write",
                primary_paths.iter().any(|path| {
                    path.rank_features.contains_model_write
                        || path.rank_features.contains_entrypoint
                }),
            ),
            (
                "cross_page_writer",
                primary_paths
                    .iter()
                    .any(|path| path.rank_features.contains_cross_page_writer),
            ),
            (
                "data_prerequisite",
                primary_paths.iter().any(|path| {
                    path.rank_features.contains_model_filter
                        || path.rank_features.has_condition_node
                }),
            ),
            (
                "display_gate",
                primary_paths.iter().any(|path| {
                    path.rank_features.contains_target_component
                        && path.rank_features.has_condition_node
                }),
            ),
            (
                "target_component",
                primary_paths.iter().any(|path| {
                    query.target_anchors.iter().any(|anchor| {
                        path.segments.iter().any(|segment| {
                            segment.from.node_id == *anchor || segment.to.node_id == *anchor
                        })
                    })
                }),
            ),
            (
                "value_source",
                primary_paths.iter().any(|path| {
                    path.rank_features.contains_physical_field
                        && (path.rank_features.contains_model_read
                            || path.rank_features.contains_model_write)
                }),
            ),
        ];

        // 如果 primary 不足，从 candidate/supporting 中提升
        for (guarantee, met) in &guarantees {
            if !met {
                let source_pool: Vec<&mut Vec<PathCandidate>> =
                    vec![&mut candidate_paths, &mut supporting_paths];
                for pool in source_pool {
                    if let Some(idx) = pool.iter().position(|c| match *guarantee {
                        "value_source" => {
                            c.rank_features.contains_physical_field
                                && (c.rank_features.contains_model_read
                                    || c.rank_features.contains_model_write)
                        }
                        "data_prerequisite" => {
                            c.rank_features.contains_model_filter
                                || c.rank_features.has_condition_node
                        }
                        "display_gate" => {
                            c.rank_features.contains_target_component
                                && c.rank_features.has_condition_node
                        }
                        "action_write" => {
                            c.rank_features.contains_model_write
                                || c.rank_features.contains_entrypoint
                        }
                        "cross_page_writer" => c.rank_features.contains_cross_page_writer,
                        "target_component" => query.target_anchors.iter().any(|anchor| {
                            c.segments
                                .iter()
                                .any(|s| s.from.node_id == *anchor || s.to.node_id == *anchor)
                        }),
                        _ => false,
                    }) {
                        let mut promoted = pool.remove(idx);
                        promoted.classification = PathClassification::PrimaryPath;
                        promoted.classification_reason =
                            format!("从 {} 提升以满足 {} 保底", pool.len() + 1, guarantee);
                        primary_paths.push(promoted);
                        break;
                    }
                }
            }
        }

        // 生成诊断
        for (guarantee, met) in &guarantees {
            if !met {
                selection_diagnostics.push(format!(
                    "保底未满足: {} — 页面可能缺少对应类型的路径",
                    guarantee
                ));
            }
        }

        // 按重要性排序 primary_paths，优先保留高价值路径
        primary_paths.sort_by(|a, b| {
            fn score(p: &PathCandidate) -> i32 {
                let mut s = 0;
                if p.rank_features.contains_physical_field {
                    s += 3;
                }
                if p.rank_features.contains_target_component {
                    s += 2;
                }
                if p.rank_features.contains_cross_page_writer {
                    s += 2;
                }
                if p.rank_features.path_length <= 2 {
                    s += 2;
                } else if p.rank_features.path_length <= 3 {
                    s += 1;
                }
                if p.rank_features.contains_model_filter {
                    s += 1;
                }
                // 关键字段加分：phoneNumber 是用户明确关注的目标字段
                let has_phone = p.segments.iter().any(|seg| {
                    seg.edge
                        .field_path
                        .as_ref()
                        .map_or(false, |fp| fp.contains("phoneNumber"))
                });
                if has_phone {
                    s += 5;
                }
                // 字段级 alias 路径优先：三段路径（含 FieldAlias）高于两段短路径
                if p.rank_features.contains_field_alias {
                    s += 4;
                }
                s
            }
            let score_ord = score(b).cmp(&score(a));
            if score_ord != std::cmp::Ordering::Equal {
                return score_ord;
            }
            let len_ord = a
                .rank_features
                .path_length
                .cmp(&b.rank_features.path_length);
            if len_ord != std::cmp::Ordering::Equal {
                return len_ord;
            }
            std::cmp::Ordering::Equal
        });

        // 限制 primary_paths 数量，同时确保多样性：每个 anchor 最多保留 3 条
        const PRIMARY_LIMIT: usize = 100;
        const MAX_PER_ANCHOR: usize = 3;
        let mut anchor_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut diverse_primary: Vec<PathCandidate> = Vec::new();
        let mut overflow: Vec<PathCandidate> = Vec::new();
        for p in primary_paths {
            // 从 path_id 提取起始 anchor
            let anchor = p.path_id.split('>').next().unwrap_or("").to_string();
            let count = anchor_counts.entry(anchor.clone()).or_insert(0);
            if *count < MAX_PER_ANCHOR && diverse_primary.len() < PRIMARY_LIMIT {
                *count += 1;
                diverse_primary.push(p);
            } else {
                overflow.push(p);
            }
        }
        if !overflow.is_empty() {
            selection_diagnostics.push(format!(
                "primary_paths 经多样性截断后保留 {} 条，{} 条降级到 candidate_paths",
                diverse_primary.len(),
                overflow.len()
            ));
            for mut p in overflow {
                p.classification = PathClassification::CandidatePath;
                candidate_paths.push(p);
            }
        }
        primary_paths = diverse_primary;

        PathSelectionResult {
            primary_paths,
            candidate_paths,
            supporting_paths,
            related_context,
            rejected_paths,
            selection_diagnostics,
        }
    }
}

/// 将 PathCandidate 转换为 JSON 值，供 output.rs 使用
impl PathCandidate {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path_id": self.path_id,
            "purpose": self.purpose,
            "terminals": self.terminals.iter().map(|t| format!("{:?}", t)).collect::<Vec<_>>(),
            "segments": self.segments.iter().map(|s| serde_json::json!({
                "from": {
                    "node_id": s.from.node_id,
                    "node_type": s.from.node_type,
                    "path": s.from.path,
                    "name": s.from.name,
                },
                "to": {
                    "node_id": s.to.node_id,
                    "node_type": s.to.node_type,
                    "path": s.to.path,
                    "name": s.to.name,
                },
                "edge": {
                    "edge_type": s.edge.edge_type,
                    "field_path": s.edge.field_path,
                    "json_path": s.edge.json_path,
                    "source_expr": s.edge.source_expr,
                },
                "evidence": s.evidence,
                "confidence": s.confidence,
            })).collect::<Vec<_>>(),
            "evidence": self.evidence,
            "selection_reason": self.selection_reason,
            "classification": format!("{:?}", self.classification),
            "classification_reason": self.classification_reason,
            "confidence": self.confidence,
            "diagnostics": self.diagnostics,
            "rank_features": {
                "contains_target_component": self.rank_features.contains_target_component,
                "contains_physical_field": self.rank_features.contains_physical_field,
                "contains_cross_page_writer": self.rank_features.contains_cross_page_writer,
                "field_name_match": self.rank_features.field_name_match,
                "same_page": self.rank_features.same_page,
                "has_condition_node": self.rank_features.has_condition_node,
                "has_json_path": self.rank_features.has_json_path,
                "edge_type_sequence": self.rank_features.edge_type_sequence,
                "path_length": self.rank_features.path_length,
                "source_file_count": self.rank_features.source_file_count,
                "contains_only_structural_edges": self.rank_features.contains_only_structural_edges,
                "contains_dataflow_side_branch": self.rank_features.contains_dataflow_side_branch,
                "contains_entrypoint": self.rank_features.contains_entrypoint,
                "contains_model_filter": self.rank_features.contains_model_filter,
                "contains_model_read": self.rank_features.contains_model_read,
                "contains_model_write": self.rank_features.contains_model_write,
            },
        })
    }
}

/// 空路径选择器（调试/禁用评分时使用）
pub struct NoScorePathSelector;

impl PathSelector for NoScorePathSelector {
    fn select(&self, _query: &PathQuery, candidates: Vec<PathCandidate>) -> PathSelectionResult {
        PathSelectionResult {
            primary_paths: candidates.clone(),
            candidate_paths: Vec::new(),
            supporting_paths: Vec::new(),
            related_context: Vec::new(),
            rejected_paths: Vec::new(),
            selection_diagnostics: vec!["NoScorePathSelector: 所有候选均归入 primary".to_string()],
        }
    }
}

/// 调试路径选择器（输出所有分类）
pub struct DebugAllPathSelector;

impl PathSelector for DebugAllPathSelector {
    fn select(&self, _query: &PathQuery, candidates: Vec<PathCandidate>) -> PathSelectionResult {
        let mut primary: Vec<PathCandidate> = Vec::new();
        let mut candidate: Vec<PathCandidate> = Vec::new();
        let mut supporting: Vec<PathCandidate> = Vec::new();
        let mut related: Vec<PathCandidate> = Vec::new();
        let mut rejected: Vec<PathCandidate> = Vec::new();

        for c in candidates {
            match c.classification {
                PathClassification::PrimaryPath => primary.push(c),
                PathClassification::CandidatePath => candidate.push(c),
                PathClassification::SupportingPath => supporting.push(c),
                PathClassification::RelatedContext => related.push(c),
                PathClassification::RejectedPath => rejected.push(c),
            }
        }

        PathSelectionResult {
            primary_paths: primary,
            candidate_paths: candidate,
            supporting_paths: supporting,
            related_context: related,
            rejected_paths: rejected,
            selection_diagnostics: vec!["DebugAllPathSelector: 按原始分类输出".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    /// 重叠候选先满足写入保底；更换优先级会改为提升两条路径，因此精确约束集合和说明。
    #[test]
    fn overlapping_guarantees_promote_write_path_before_condition_path() {
        use super::*;

        let candidate = |path_id: &str, has_condition_node| PathCandidate {
            path_id: path_id.to_string(),
            purpose: String::new(),
            terminals: Vec::new(),
            segments: Vec::new(),
            evidence: Vec::new(),
            selection_reason: String::new(),
            classification: PathClassification::CandidatePath,
            classification_reason: String::new(),
            confidence: "high".to_string(),
            diagnostics: Vec::new(),
            rank_features: PathRankFeatures {
                contains_entrypoint: true,
                has_condition_node,
                ..PathRankFeatures::default()
            },
        };
        let query = PathQuery {
            page_id: "page:test.spg".to_string(),
            page_path: "test.spg".to_string(),
            target_anchors: Vec::new(),
            source_anchors: Vec::new(),
            sink_anchors: Vec::new(),
            bridge_anchors: Vec::new(),
            excluded_anchors: Vec::new(),
            budget: "full".to_string(),
        };
        let result = RuleBasedPathSelector.select(
            &query,
            vec![
                candidate("write-and-condition", true),
                candidate("write-only", false),
            ],
        );
        assert_eq!(
            result
                .primary_paths
                .iter()
                .map(|path| path.path_id.as_str())
                .collect::<Vec<_>>(),
            vec!["write-and-condition"]
        );
        assert_eq!(
            result.primary_paths[0].classification_reason,
            "从 2 提升以满足 action_write 保底"
        );
        assert_eq!(
            result
                .candidate_paths
                .iter()
                .map(|path| path.path_id.as_str())
                .collect::<Vec<_>>(),
            vec!["write-only"]
        );
        assert_eq!(result.supporting_paths.len(), 0);
    }

    #[test]
    fn anchor_dedup_preserves_first_seen_order() {
        let mut anchors = vec![
            "model:a".to_string(),
            "model:b".to_string(),
            "model:a".to_string(),
            "field:x.y".to_string(),
            "model:b".to_string(),
        ];

        super::dedup_anchors_preserve_order(&mut anchors);

        assert_eq!(
            anchors,
            vec![
                "model:a".to_string(),
                "model:b".to_string(),
                "field:x.y".to_string(),
            ]
        );
    }
}
