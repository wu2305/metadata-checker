/// 可视化图选项
///
/// 控制 VisualGraph 的生成行为，包括截断阈值、分组策略和焦点节点。
#[derive(Debug, Clone)]
pub struct VisualGraphOptions {
    /// 最大节点数，超出则截断
    pub max_nodes: usize,
    /// 最大边数，超出则截断
    pub max_edges: usize,
    /// 是否在输出中包含详细证据
    pub include_evidence: bool,
    /// 是否按源路径分组
    pub group_by_source_path: bool,
    /// 焦点目标节点 ID
    pub focus_target: Option<String>,
}

impl Default for VisualGraphOptions {
    fn default() -> Self {
        Self {
            max_nodes: 200,
            max_edges: 500,
            include_evidence: false,
            group_by_source_path: true,
            focus_target: None,
        }
    }
}

/// 可视化节点类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum NodeKind {
    /// 页面
    Page,
    /// 组件
    Component,
    /// 模型
    Model,
    /// 字段
    Field,
    /// 动作
    Action,
    /// 条件/表达式
    Condition,
    /// 诊断/提示节点
    Diagnostic,
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeKind::Page => write!(f, "Page"),
            NodeKind::Component => write!(f, "Component"),
            NodeKind::Model => write!(f, "Model"),
            NodeKind::Field => write!(f, "Field"),
            NodeKind::Action => write!(f, "Action"),
            NodeKind::Condition => write!(f, "Condition"),
            NodeKind::Diagnostic => write!(f, "Diagnostic"),
        }
    }
}

/// 可视化边类型
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum EdgeKind {
    /// 读取关系
    Reads,
    /// 写入关系
    Writes,
    /// 触发关系
    Triggers,
    /// 包含关系
    Contains,
    /// 依赖关系
    DependsOn,
    /// 其他关系（带标签）
    Other(String),
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EdgeKind::Reads => write!(f, "Reads"),
            EdgeKind::Writes => write!(f, "Writes"),
            EdgeKind::Triggers => write!(f, "Triggers"),
            EdgeKind::Contains => write!(f, "Contains"),
            EdgeKind::DependsOn => write!(f, "DependsOn"),
            EdgeKind::Other(s) => write!(f, "{}", s),
        }
    }
}

/// 边方向
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EdgeDirection {
    /// 单向
    Forward,
    /// 双向
    Bidirectional,
}
