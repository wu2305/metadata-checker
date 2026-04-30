use std::collections::HashMap;

/// 组件定义
#[derive(Debug, Clone, PartialEq)]
pub struct SpgComponent {
    pub id: String,
    pub component_type: String,
    pub parent_id: Option<String>,
    pub properties: HashMap<String, String>,
    pub actions: Vec<SpgAction>,
    pub submit_data: Option<bool>,
    pub res_path: Option<String>,
}

/// 表达式中引用的对象类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RefType {
    ComponentValue(String),
    ComponentProperty(String, String),
    ModelField(String, String),
    Param(String),
    UserProperty(String),
    SystemVar(String),
    Other(String),
}

/// 引用解析置信度
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confidence {
    /// 精确匹配已知 component / source / param ID
    High,
    /// 基于命名模式推断（如 input*.value）
    Medium,
    /// 启发式猜测或无法确认
    Low,
    /// 完全无法解析
    Unresolved,
}

/// 带解析元数据的引用
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRef {
    pub ref_type: RefType,
    pub confidence: Confidence,
    pub reason: String,
    pub unresolved: bool,
}

/// 组件动作（交互行为）
#[derive(Debug, Clone, PartialEq)]
pub struct SpgAction {
    pub id: String,
    pub action_type: String,
    pub trigger_type: String,
    pub submit_range: Option<String>,
    pub data_set: Option<String>,
    pub data_range: Option<String>,
    pub field_values: Vec<(String, String, String)>,
    pub submit_component: Vec<String>,
    pub path: Option<String>,
    pub short_url: Option<String>,
    pub data: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub target_type: String,
    pub wait_prev: Option<String>,
    pub condition: Option<String>,
    pub condition_exp: Option<String>,
}

/// 组件上的表达式及其解析出的引用列表
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentExpr {
    pub component_id: String,
    pub field: String,
    pub raw_expr: String,
    pub refs: Vec<RefType>,
    pub resolved_refs: Vec<ResolvedRef>,
}

/// 解析后的 SuperPage 完整元数据
#[derive(Debug, Default, Clone)]
pub struct SuperPageMetadata {
    pub version: Option<String>,
    pub theme: Option<String>,
    pub params: Vec<SpgParam>,
    pub sources: Vec<SpgSource>,
    pub components: Vec<SpgComponent>,
    pub expressions: Vec<ComponentExpr>,
    pub reference_resources: Vec<String>,
    pub raw: serde_json::Value,
}

/// 页面级参数
#[derive(Debug, Clone)]
pub struct SpgParam {
    pub id: String,
    pub name: String,
    pub desc: Option<String>,
    pub default_value: Option<String>,
}

/// 页面数据源（模型引用）
#[derive(Debug, Clone)]
pub struct SpgSource {
    pub id: String,
    pub model_type: Option<String>,
    pub path: Option<String>,
    pub content: Option<serde_json::Value>,
}
