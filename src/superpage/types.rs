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

/// `RefType::ComponentValue` 的来源文法（spec A1b）。
///
/// `b.value`、`b.step` 和裸 `b` 三种来源文法此前都被压成同一个 id，值追溯的
/// 替换点没有信息可依，只能按后缀猜。这里保留引用产生时的文法形态，
/// 使替换 pattern 与来源文法一致：
/// - `Value`：`id.value` 显式取值，替换 pattern 为 `id.value`；
/// - `Suffix`：`id.<其它后缀>`（如 `.step`）引用的是组件其它属性，
///   依赖成立，但替换成值的展开式是错的，替换点必须跳过；
/// - `Bare`：裸 `id`（含 `${id}` 全组件引用），替换 pattern 为 `id`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentValueForm {
    Value,
    Suffix,
    Bare,
}

/// 表达式中引用的对象类型
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RefType {
    /// 组件值引用。第一项是被引用组件 id，第二项是来源文法（A1b）。
    ComponentValue(String, ComponentValueForm),
    /// 组件属性引用。不变式：property 永不为空——裸 `${id}` 全组件引用在 parse 层
    /// （`resolve_ref_type` / `resolve_ref_token`）已归一为 `ComponentValue`
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
    pub target_component: Vec<String>,
    pub panelbook: Option<String>,
    pub panel: Option<String>,
    pub dialog: Option<String>,
}

/// 组件上的表达式及其解析出的引用列表
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentExpr {
    pub component_id: String,
    pub field: String,
    pub raw_expr: String,
    pub refs: Vec<RefType>,
    pub resolved_refs: Vec<ResolvedRef>,
    pub diagnostics: Vec<super::expr_ast::ExprDiagnostic>,
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
