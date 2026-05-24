use crate::output::schema::Diagnostic;
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind};
use crate::visualization::sanitizer::{
    sanitize_diagnostic, sanitize_metadata_entry, sanitize_text,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 可视化图
///
/// 分析结果到可视化图的稳定中间模型，可被 Mermaid / ECharts 渲染器消费。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualGraph {
    /// 节点列表
    pub nodes: Vec<VisualNode>,
    /// 边列表
    pub edges: Vec<VisualEdge>,
    /// 分组列表
    pub groups: Vec<VisualGroup>,
    /// 焦点节点 ID
    pub focus_node: Option<String>,
    /// 诊断信息
    pub diagnostics: Vec<Diagnostic>,
    /// 是否被截断
    pub truncated: bool,
    /// 来源摘要
    pub source_summary: SourceSummary,
}

impl VisualGraph {
    /// 创建空的可视化图
    pub fn empty() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            groups: Vec::new(),
            focus_node: None,
            diagnostics: Vec::new(),
            truncated: false,
            source_summary: SourceSummary::default(),
        }
    }

    /// 创建仅包含诊断信息的图
    pub fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        let mut graph = Self::empty();
        graph.diagnostics = diagnostics.iter().map(sanitize_diagnostic).collect();
        for diag in &graph.diagnostics {
            let node_id = sanitize_text(&format!("diag_{}", diag.code));
            graph.nodes.push(VisualNode {
                id: node_id.clone(),
                label: sanitize_text(&format!("[{}] {}", diag.code, diag.message)),
                kind: NodeKind::Diagnostic,
                source_path: diag.location.source_file.clone().unwrap_or_default(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert(
                        "severity".to_string(),
                        serde_json::json!(format!("{:?}", diag.severity)),
                    );
                    m.insert(
                        "code".to_string(),
                        sanitize_metadata_entry("code", &serde_json::json!(diag.code.clone())),
                    );
                    m
                },
            });
        }
        graph
    }
}

/// 可视化节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualNode {
    /// 节点唯一标识
    pub id: String,
    /// 显示标签
    pub label: String,
    /// 节点类型
    pub kind: NodeKind,
    /// 来源文件路径
    pub source_path: String,
    /// 附加元数据
    pub metadata: HashMap<String, serde_json::Value>,
}

/// 可视化边
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualEdge {
    /// 起始节点 ID
    pub from: String,
    /// 目标节点 ID
    pub to: String,
    /// 边类型
    pub kind: EdgeKind,
    /// 显示标签
    pub label: Option<String>,
    /// 方向
    pub direction: EdgeDirection,
    /// 证据说明
    pub evidence: Option<String>,
}

/// 可视化分组
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualGroup {
    /// 分组唯一标识
    pub id: String,
    /// 显示标签
    pub label: String,
    /// 组内节点 ID 列表
    pub node_ids: Vec<String>,
}

/// 来源摘要统计
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SourceSummary {
    /// 总节点数（截断前）
    pub total_nodes: usize,
    /// 总边数（截断前）
    pub total_edges: usize,
    /// 各类型节点数量
    pub node_kinds: HashMap<String, usize>,
    /// 各类型边数量
    pub edge_kinds: HashMap<String, usize>,
}
