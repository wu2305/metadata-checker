//! 运行期工具契约统一模块
//!
//! M38：消除 CLI / stdio / 未来 MCP 在"可调用工具、参数校验、错误码、输出 envelope"上的分叉。
//! 只统一已加载 graphdb 后的运行期工具契约。

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// 运行期工具枚举，表示所有已加载 graphdb 后可以调用的能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCommand {
    ExplainCondition,
    Explain,
    Context,
    QueryModel,
    QueryPage,
    QueryCross,
    QueryDataflow,
    QueryPageLogic,
    FindPage,
    FindModel,
    FindComponent,
    AdviseQuery,
    Status,
    ReloadGraph,
    CheckReload,
    DiffRefresh,
}

/// 工具声明，供 CLI / stdio / MCP 共享 help、校验和 tool list。
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub command: ToolCommand,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub requires_target: bool,
    pub requires_graph: bool,
    pub mutates_runtime: bool,
    pub supports_human: bool,
    pub supported_budgets: &'static [&'static str],
    pub supported_intents: &'static [&'static str],
}

/// 标准工具响应，adapter 用它渲染各自的外层 envelope。
#[derive(Debug, Clone)]
pub struct ToolResponse {
    pub ok: bool,
    pub result: Option<serde_json::Value>,
    pub error: Option<ToolError>,
    pub diagnostics: Vec<String>,
}

impl ToolResponse {
    /// 构造成功响应。
    pub fn ok(result: serde_json::Value, diagnostics: Vec<String>) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
            diagnostics,
        }
    }

    /// 构造失败响应。
    pub fn error(error: ToolError, diagnostics: Vec<String>) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(error),
            diagnostics,
        }
    }
}

/// 调用方式 adapter：各入口只负责输入解析和输出渲染。
pub trait InvocationAdapter {
    type RawInput;
    type RawOutput;

    /// 将入口原始输入解析为标准调用对象。
    fn parse_input(&self, raw: Self::RawInput) -> Result<ToolInvocation, ToolError>;

    /// 将标准响应渲染为入口自己的输出 envelope。
    fn render_output(&self, response: ToolResponse) -> Result<Self::RawOutput, ToolError>;
}

/// 统一错误码。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ToolErrorCode {
    UnknownCommand,
    MissingTarget,
    InvalidTarget,
    InvalidBudget,
    InvalidIntent,
    InvalidDepth,
    InvalidArgument,
    TargetNotFound,
    GraphDbNotFound,
    GraphReloadFailed,
    HumanModeNotSupported,
    QueryFailed,
    InternalError,
}

impl ToolErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolErrorCode::UnknownCommand => "UNKNOWN_COMMAND",
            ToolErrorCode::MissingTarget => "MISSING_TARGET",
            ToolErrorCode::InvalidTarget => "INVALID_TARGET",
            ToolErrorCode::InvalidBudget => "INVALID_BUDGET",
            ToolErrorCode::InvalidIntent => "INVALID_INTENT",
            ToolErrorCode::InvalidDepth => "INVALID_DEPTH",
            ToolErrorCode::InvalidArgument => "INVALID_ARGUMENT",
            ToolErrorCode::TargetNotFound => "TARGET_NOT_FOUND",
            ToolErrorCode::GraphDbNotFound => "GRAPH_DB_NOT_FOUND",
            ToolErrorCode::GraphReloadFailed => "GRAPH_RELOAD_FAILED",
            ToolErrorCode::HumanModeNotSupported => "HUMAN_MODE_NOT_SUPPORTED",
            ToolErrorCode::QueryFailed => "QUERY_FAILED",
            ToolErrorCode::InternalError => "INTERNAL_ERROR",
        }
    }
}

/// 统一工具错误。
#[derive(Debug, Clone)]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code_str(&self) -> &'static str {
        self.code.as_str()
    }
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code_str(), self.message)
    }
}

impl std::error::Error for ToolError {}

/// adapter 解析后的标准调用对象。
#[derive(Debug, Clone)]
pub struct ToolInvocation {
    pub command: ToolCommand,
    pub target: Option<String>,
    pub budget: Option<String>,
    pub intent: Option<String>,
    pub depth: Option<usize>,
    pub page_scope: Option<String>,
    pub human: bool,
    pub check_reload: bool,
}

impl ToolInvocation {
    /// 构造最小调用。
    pub fn new(command: ToolCommand) -> Self {
        Self {
            command,
            target: None,
            budget: None,
            intent: None,
            depth: None,
            page_scope: None,
            human: false,
            check_reload: false,
        }
    }
}

/// 统一工具注册表。
pub struct ToolRegistry;

impl ToolRegistry {
    /// 返回所有运行期工具声明。
    pub fn all_specs() -> Vec<ToolSpec> {
        vec![
            ToolSpec {
                command: ToolCommand::ExplainCondition,
                name: "explain_condition",
                aliases: &["explain-condition"],
                description: "解释组件/模型/字段的可见性、禁用、数据空等行为",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: true,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[
                    "auto",
                    "display",
                    "value-source",
                    "value_source",
                    "writer",
                    "availability",
                    "context",
                    "action",
                ],
            },
            ToolSpec {
                command: ToolCommand::Explain,
                name: "explain",
                aliases: &[],
                description: "解释节点语义",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::Context,
                name: "context",
                aliases: &[],
                description: "输出目标节点周围的上下文",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::QueryModel,
                name: "query_model",
                aliases: &["query-model"],
                description: "查询模型关系",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::QueryPage,
                name: "query_page",
                aliases: &["query-page"],
                description: "查询页面关系",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::QueryCross,
                name: "query_cross",
                aliases: &["query-cross"],
                description: "查询跨文件关系",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::QueryDataflow,
                name: "query_dataflow",
                aliases: &["query-dataflow"],
                description: "展开并查询 DataFlow 模型内部子图",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::QueryPageLogic,
                name: "query_page_logic",
                aliases: &["query-page-logic"],
                description: "查询页面级逻辑摘要",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::FindPage,
                name: "find_page",
                aliases: &["find-page"],
                description: "查找匹配关键词的页面",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::FindModel,
                name: "find_model",
                aliases: &["find-model"],
                description: "查找匹配关键词的模型",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::FindComponent,
                name: "find_component",
                aliases: &["find-component"],
                description: "查找匹配关键词的组件",
                requires_target: true,
                requires_graph: true,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::AdviseQuery,
                name: "advise_query",
                aliases: &["advise-query"],
                description: "为目标生成结构化查询建议",
                requires_target: true,
                requires_graph: false,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &["compact", "normal", "full"],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::Status,
                name: "status",
                aliases: &[],
                description: "返回当前 runtime 状态快照",
                requires_target: false,
                requires_graph: false,
                mutates_runtime: false,
                supports_human: false,
                supported_budgets: &[],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::ReloadGraph,
                name: "reload_graph",
                aliases: &["reload"],
                description: "强制重新读取当前 graphdb 文件",
                requires_target: false,
                requires_graph: false,
                mutates_runtime: true,
                supports_human: false,
                supported_budgets: &[],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::CheckReload,
                name: "check_reload",
                aliases: &["check-reload"],
                description: "仅在 graphdb 文件变化时重载",
                requires_target: false,
                requires_graph: false,
                mutates_runtime: true,
                supports_human: false,
                supported_budgets: &[],
                supported_intents: &[],
            },
            ToolSpec {
                command: ToolCommand::DiffRefresh,
                name: "diff_refresh",
                aliases: &[],
                description: "执行一轮远程元数据差量刷新（需启动时绑定 session context）",
                requires_target: false,
                requires_graph: false,
                mutates_runtime: true,
                supports_human: false,
                supported_budgets: &[],
                supported_intents: &[],
            },
        ]
    }

    /// 按名称或别名查找工具。
    pub fn find_by_name(name: &str) -> Option<ToolSpec> {
        Self::all_specs()
            .into_iter()
            .find(|spec| spec.name == name || spec.aliases.contains(&name))
    }

    /// 查找工具对应的 spec。
    pub fn find_by_command(command: ToolCommand) -> Option<ToolSpec> {
        Self::all_specs()
            .into_iter()
            .find(|spec| spec.command == command)
    }

    /// 返回所有工具名称列表。
    pub fn command_names() -> Vec<&'static str> {
        Self::all_specs().iter().map(|s| s.name).collect()
    }
}

/// 参数校验辅助函数。
pub fn validate_budget(budget: &str) -> Result<(), ToolError> {
    match budget {
        "compact" | "normal" | "full" => Ok(()),
        other => Err(ToolError::new(
            ToolErrorCode::InvalidBudget,
            format!(
                "Invalid budget '{}'. Expected: compact | normal | full",
                other
            ),
        )),
    }
}

pub fn validate_intent(intent: &str) -> Result<(), ToolError> {
    match intent {
        "auto" | "display" | "value-source" | "value_source" | "writer" | "availability"
        | "context" | "action" => Ok(()),
        other => Err(ToolError::new(
            ToolErrorCode::InvalidIntent,
            format!(
                "Invalid intent '{}'. Expected: auto | display | value-source | writer | availability | context | action",
                other
            ),
        )),
    }
}

pub fn validate_depth(depth: Option<&serde_json::Value>) -> Result<usize, ToolError> {
    match depth {
        None => Ok(1),
        Some(v) => v.as_u64().map(|n| n as usize).ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::InvalidDepth,
                "Invalid depth. Expected non-negative integer",
            )
        }),
    }
}

/// 校验 target 前缀是否合法。
pub fn validate_target_prefix(command: ToolCommand, target: &str) -> Result<(), ToolError> {
    if command == ToolCommand::QueryCross {
        parse_query_cross_target(target)?;
        return Ok(());
    }

    let allowed: &[&str] = match command {
        ToolCommand::QueryModel => &["model:"],
        ToolCommand::QueryPageLogic | ToolCommand::QueryPage => &["page:"],
        ToolCommand::ExplainCondition | ToolCommand::Explain | ToolCommand::Context => {
            &["comp:", "action:", "model:", "field:", "page:", "dataflow:"]
        }
        ToolCommand::AdviseQuery => &[],
        ToolCommand::QueryDataflow => &["model:", "dataflow:"],
        ToolCommand::QueryCross => unreachable!("QueryCross handled above"),
        ToolCommand::FindPage | ToolCommand::FindModel | ToolCommand::FindComponent => &[],
        ToolCommand::Status | ToolCommand::ReloadGraph | ToolCommand::CheckReload => &[],
        ToolCommand::DiffRefresh => &[],
    };

    if allowed.is_empty() {
        return Ok(());
    }

    if target != target.trim()
        || target.contains('\0')
        || !allowed.iter().any(|prefix| target.starts_with(prefix))
    {
        return Err(ToolError::new(
            ToolErrorCode::InvalidTarget,
            format!(
                "Invalid target '{}' for command '{}'",
                target,
                ToolRegistry::find_by_command(command)
                    .map(|s| s.name)
                    .unwrap_or("unknown")
            ),
        ));
    }

    Ok(())
}

/// 解析 `query_cross` 的双页面 target。
///
/// 运行期协议用一个字符串传递两个页面节点，格式固定为
/// `page:PAGE_A,page:PAGE_B`。这里统一做结构校验，避免 adapter 放过坏参数后
/// runtime 以业务 JSON 伪装成功响应。
pub fn parse_query_cross_target(target: &str) -> Result<(String, String), ToolError> {
    if target != target.trim() || target.contains('\0') {
        return Err(ToolError::new(
            ToolErrorCode::InvalidTarget,
            format!("Invalid query_cross target '{}'", target),
        ));
    }

    let parts: Vec<&str> = target.split(',').map(str::trim).collect();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidArgument,
            "query_cross target must be two comma-separated page targets",
        ));
    }

    if !parts.iter().all(|part| part.starts_with("page:")) {
        return Err(ToolError::new(
            ToolErrorCode::InvalidTarget,
            format!("query_cross target must use page: prefix: '{}'", target),
        ));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_contains_all_tools() {
        let specs = ToolRegistry::all_specs();
        let commands: Vec<_> = specs.iter().map(|s| s.command).collect();

        assert!(commands.contains(&ToolCommand::ExplainCondition));
        assert!(commands.contains(&ToolCommand::Explain));
        assert!(commands.contains(&ToolCommand::Context));
        assert!(commands.contains(&ToolCommand::QueryModel));
        assert!(commands.contains(&ToolCommand::QueryPage));
        assert!(commands.contains(&ToolCommand::QueryCross));
        assert!(commands.contains(&ToolCommand::QueryDataflow));
        assert!(commands.contains(&ToolCommand::QueryPageLogic));
        assert!(commands.contains(&ToolCommand::FindPage));
        assert!(commands.contains(&ToolCommand::FindModel));
        assert!(commands.contains(&ToolCommand::FindComponent));
        assert!(commands.contains(&ToolCommand::AdviseQuery));
        assert!(commands.contains(&ToolCommand::Status));
        assert!(commands.contains(&ToolCommand::ReloadGraph));
        assert!(commands.contains(&ToolCommand::CheckReload));
        assert!(commands.contains(&ToolCommand::DiffRefresh));
    }

    #[test]
    fn test_registry_excludes_build_graph() {
        let specs = ToolRegistry::all_specs();
        let names: Vec<_> = specs.iter().map(|s| s.name).collect();
        assert!(!names.contains(&"build_graph"));
        assert!(!names.contains(&"refresh_index"));
        assert!(!names.contains(&"rebuild_graph"));
    }

    #[test]
    fn test_find_by_name_and_alias() {
        let spec = ToolRegistry::find_by_name("explain_condition").unwrap();
        assert_eq!(spec.command, ToolCommand::ExplainCondition);

        let alias = ToolRegistry::find_by_name("explain-condition").unwrap();
        assert_eq!(alias.command, ToolCommand::ExplainCondition);

        assert!(ToolRegistry::find_by_name("unknown").is_none());
    }

    #[test]
    fn test_find_by_command() {
        let spec = ToolRegistry::find_by_command(ToolCommand::Status).unwrap();
        assert!(!spec.requires_target);
        assert!(!spec.requires_graph);
        assert!(!spec.mutates_runtime);

        let reload = ToolRegistry::find_by_command(ToolCommand::ReloadGraph).unwrap();
        assert!(reload.mutates_runtime);
        assert!(!reload.requires_target);
    }

    #[test]
    fn test_registry_human_support_only_explain_condition() {
        for spec in ToolRegistry::all_specs() {
            assert_eq!(
                spec.supports_human,
                spec.command == ToolCommand::ExplainCondition,
                "{} human support flag must match runtime behavior",
                spec.name
            );
        }
    }

    #[test]
    fn test_validate_budget() {
        assert!(validate_budget("compact").is_ok());
        assert!(validate_budget("normal").is_ok());
        assert!(validate_budget("full").is_ok());
        assert_eq!(
            validate_budget("unknown").unwrap_err().code,
            ToolErrorCode::InvalidBudget
        );
    }

    #[test]
    fn test_validate_intent() {
        assert!(validate_intent("auto").is_ok());
        assert!(validate_intent("display").is_ok());
        assert_eq!(
            validate_intent("bad").unwrap_err().code,
            ToolErrorCode::InvalidIntent
        );
    }

    #[test]
    fn test_validate_depth() {
        assert_eq!(validate_depth(None).unwrap(), 1);
        assert_eq!(validate_depth(Some(&serde_json::json!(3))).unwrap(), 3);
        assert_eq!(
            validate_depth(Some(&serde_json::json!(-1)))
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidDepth
        );
    }

    #[test]
    fn test_validate_target_prefix() {
        assert!(validate_target_prefix(ToolCommand::QueryModel, "model:x").is_ok());
        assert_eq!(
            validate_target_prefix(ToolCommand::QueryModel, "page:x")
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidTarget
        );

        assert!(validate_target_prefix(ToolCommand::Status, "anything").is_ok());
    }

    #[test]
    fn test_validate_query_cross_target() {
        assert!(
            validate_target_prefix(ToolCommand::QueryCross, "page:app/a.spg,page:app/b.spg")
                .is_ok()
        );
        assert_eq!(
            validate_target_prefix(ToolCommand::QueryCross, "page:app/a.spg")
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidArgument
        );
        assert_eq!(
            validate_target_prefix(ToolCommand::QueryCross, "model:a,model:b")
                .unwrap_err()
                .code,
            ToolErrorCode::InvalidTarget
        );
    }
}
