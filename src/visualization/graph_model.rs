use crate::output::schema::Diagnostic;
use crate::visualization::options::{EdgeDirection, EdgeKind, NodeKind};
use crate::visualization::sanitizer::{
    sanitize_diagnostic, sanitize_identity_id, sanitize_metadata_entry, sanitize_text,
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
    /// 当前分析目标（一般为组件 ID 或空）
    #[serde(default = "default_target")]
    pub target: String,
    /// 状态：ready/empty/warning/error/idle
    #[serde(default = "default_status")]
    pub status: String,
    /// 诊断信息
    pub diagnostics: Vec<Diagnostic>,
    /// 探索深度
    #[serde(default = "default_depth")]
    pub depth: usize,
    /// 可视范围 hop 数
    #[serde(default = "default_visible_hop")]
    pub visible_hop: usize,
    /// 探索方向（如 both/upstream/downstream）
    #[serde(default = "default_direction")]
    pub direction: String,
    /// 已折叠内容是否可继续展开
    #[serde(default)]
    pub collapsed: bool,
    /// 是否被截断
    #[serde(default)]
    pub truncated: bool,
    /// 截断原因
    #[serde(default)]
    pub truncated_reason: Option<String>,
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
            target: default_target(),
            status: default_status(),
            diagnostics: Vec::new(),
            depth: default_depth(),
            visible_hop: default_visible_hop(),
            direction: default_direction(),
            collapsed: false,
            truncated: false,
            truncated_reason: None,
            source_summary: SourceSummary::default(),
        }
    }

    /// 创建仅包含诊断信息的图
    pub fn from_diagnostics(diagnostics: Vec<Diagnostic>) -> Self {
        let mut graph = Self::empty();
        graph.diagnostics = diagnostics.iter().map(sanitize_diagnostic).collect();
        for (raw_diag, diag) in diagnostics.iter().zip(graph.diagnostics.iter()) {
            let node_id = sanitize_identity_id(&format!("diag_{}", raw_diag.code));
            graph.nodes.push(VisualNode {
                id: node_id.clone(),
                label: sanitize_text(&format!("[{}] {}", diag.code, diag.message)),
                kind: NodeKind::Diagnostic,
                source_path: diag.location.source_file.clone().unwrap_or_default(),
                depth: None,
                collapsed: false,
                importance: None,
                expand_token: None,
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
        graph.target = default_target();
        graph.status = "error".to_string();
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
    /// 与焦点节点距离（跳数）
    #[serde(default)]
    pub depth: Option<usize>,
    /// 该节点是否是折叠节点
    #[serde(default)]
    pub collapsed: bool,
    /// 重要性等级（供前端样式）
    #[serde(default)]
    pub importance: Option<String>,
    /// 折叠节点可继续探索的查询 token
    #[serde(default)]
    pub expand_token: Option<String>,
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
    /// 关系类型明文（与 kind 对齐）
    #[serde(default)]
    pub edge_type: Option<String>,
    /// 显示标签
    pub label: Option<String>,
    /// 方向
    pub direction: EdgeDirection,
    /// 关系优先级：filter/visibility/source/action/other
    #[serde(default = "default_edge_priority")]
    pub priority: String,
    /// 关系摘要
    #[serde(default)]
    pub summary: String,
    /// 证据状态：available/unavailable
    #[serde(default = "default_evidence_status")]
    pub evidence_status: String,
    /// 证据说明
    pub evidence: Option<String>,
}

/// 根据边类型和标签推导稳定展示优先级。
pub fn classify_edge_priority(kind: &EdgeKind, label: Option<&str>) -> String {
    let kind_text = kind.to_string().to_lowercase();
    let label_text = label.unwrap_or_default().to_lowercase();
    let merged = format!("{} {}", kind_text, label_text);
    if merged.contains("filter")
        || merged.contains("condition")
        || merged.contains("validate")
        || merged.contains("calc_condition")
        || merged.contains("条件")
    {
        "filter".to_string()
    } else if merged.contains("display")
        || merged.contains("visible")
        || merged.contains("visibility")
        || merged.contains("show")
        || merged.contains("hide")
        || merged.contains("controls_component")
        || merged.contains("显示")
    {
        "visibility".to_string()
    } else if merged.contains("read")
        || merged.contains("source")
        || merged.contains("input")
        || merged.contains("field")
        || merged.contains("dataflow")
        || merged.contains("alias")
    {
        "source".to_string()
    } else if merged.contains("action")
        || merged.contains("write")
        || merged.contains("trigger")
        || merged.contains("calc")
        || merged.contains("output")
        || merged.contains("set")
    {
        "action".to_string()
    } else {
        "other".to_string()
    }
}

/// 构造稳定边摘要。
pub fn summarize_edge(kind: &EdgeKind, label: Option<&str>) -> String {
    let label_text = label.unwrap_or_default();
    if label_text.is_empty() {
        kind.to_string()
    } else {
        format!("{}: {}", kind, label_text)
    }
}

/// 根据证据是否存在返回稳定证据状态。
pub fn edge_evidence_status(evidence: &Option<String>) -> String {
    if evidence
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        "available".to_string()
    } else {
        "unavailable".to_string()
    }
}

fn default_direction() -> String {
    "both".to_string()
}

fn default_depth() -> usize {
    1
}

fn default_visible_hop() -> usize {
    1
}

fn default_status() -> String {
    "ready".to_string()
}

fn default_edge_priority() -> String {
    "other".to_string()
}

fn default_evidence_status() -> String {
    "unavailable".to_string()
}

fn default_target() -> String {
    String::new()
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
