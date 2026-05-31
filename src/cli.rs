use crate::tool_contract::{
    InvocationAdapter, ToolCommand, ToolError, ToolErrorCode, ToolInvocation, ToolRegistry,
    ToolResponse,
};
use clap::Parser;
use std::path::PathBuf;

/// 命令行参数定义
///
/// 使用 clap 派生宏定义所有 CLI 参数和子命令。

#[derive(Parser, Debug)]
#[command(
    name = "metadata-checker",
    about = "Parse page metadata from a low-code platform JSON file",
    long_about = "metadata-checker reads a JSON file containing low-code platform page metadata, validates and extracts structured information, and prints results in either human-readable or machine-friendly (non-human) format. It is intended for integration with AI/LLM toolchains.",
    version
)]
/// 根命令参数
pub struct Cli {
    #[arg(value_name = "FILE", help = "Path to the page metadata JSON file")]
    pub input: Option<PathBuf>,

    #[arg(
        long,
        group = "output_mode",
        help = "Enter interactive REPL for expert exploration"
    )]
    pub human: bool,

    #[arg(long, help = "Alias for --human, enter interactive REPL")]
    pub interactive: bool,

    #[arg(
        long,
        group = "output_mode",
        help = "Print machine-friendly single JSON output (default)"
    )]
    pub non_human: bool,

    #[arg(
        long,
        value_name = "ID",
        help = "Query detailed info for a specific component ID"
    )]
    pub query: Option<String>,

    #[arg(
        long,
        help = "Append priority analysis (merged into JSON for non-human, text for human)"
    )]
    pub priority: bool,

    #[arg(
        long,
        help = "Output full raw structure instead of compact summary (non-human mode)"
    )]
    pub detail: bool,

    #[arg(
        long,
        value_name = "DIR",
        help = "Project directory to scan for cross-file analysis"
    )]
    pub project_dir: Option<PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Custom graph database path (default: <project-dir>/.metadata-checker.graphdb)"
    )]
    pub graph_db_path: Option<PathBuf>,

    #[arg(
        long,
        value_name = "MS",
        default_value = "10000",
        help = "Graph database lock acquisition timeout in milliseconds (default: 10000)"
    )]
    pub graph_lock_timeout_ms: u64,

    #[arg(long, help = "Check graph database status and output JSON report")]
    pub check_graph: bool,

    #[arg(long, help = "Show runtime status (loaded graphdb info)")]
    pub status: bool,

    #[arg(long, help = "Reload graph database from disk")]
    pub reload_graph: bool,

    #[arg(long, help = "Check if graph database changed and reload if needed")]
    pub check_reload: bool,

    #[arg(long, help = "Build/update graph database from project directory")]
    pub build_graph: bool,

    #[arg(
        long,
        value_name = "MODEL",
        help = "Query relationships for a specific model (requires --project-dir)"
    )]
    pub query_model: Option<String>,

    #[arg(
        long,
        value_name = "PAGE",
        help = "Query relationships for a specific page (requires --project-dir)"
    )]
    pub query_page: Option<String>,

    #[arg(
        long,
        value_name = "PAGES",
        num_args = 2,
        help = "Query cross-file relations between two pages (requires --project-dir)"
    )]
    pub query_cross: Option<Vec<String>>,

    #[arg(
        long,
        value_name = "MODEL",
        help = "Expand and query a DataFlow model's internal subgraph (requires --project-dir)"
    )]
    pub query_dataflow: Option<String>,

    #[arg(
        long,
        value_name = "ID",
        help = "Explain a component, action, model, field, page, or dataflow by ID"
    )]
    pub explain: Option<String>,

    #[arg(
        long,
        value_name = "ID",
        help = "Output minimal closure context around a target node (requires --project-dir)"
    )]
    pub context: Option<String>,

    #[arg(
        long,
        value_name = "N",
        default_value = "1",
        help = "Context depth for --context (default: 1)"
    )]
    pub depth: usize,

    #[arg(
        long,
        value_name = "BUDGET",
        default_value = "normal",
        help = "Output budget: compact | normal | full (default: normal)"
    )]
    pub budget: String,

    #[arg(
        long,
        value_name = "PAGE",
        help = "Query page-level logic summary (requires --project-dir)"
    )]
    pub query_page_logic: Option<String>,

    #[arg(
        long,
        value_name = "KEYWORD",
        help = "Find pages matching keyword (requires --project-dir)"
    )]
    pub find_page: Option<String>,

    #[arg(
        long,
        value_name = "KEYWORD",
        help = "Find models matching keyword (requires --project-dir)"
    )]
    pub find_model: Option<String>,

    #[arg(
        long,
        value_name = "KEYWORD",
        help = "Find components matching keyword (requires --project-dir)"
    )]
    pub find_component: Option<String>,

    #[arg(
        long,
        value_name = "PAGE_ID",
        help = "Page ID for --resolve-model scope"
    )]
    pub resolve_model_page: Option<String>,

    #[arg(
        long,
        num_args = 1..=2,
        value_names = ["PAGE_ID", "MODEL"],
        help = "Resolve local model ID within page scope. Accepts either 'PAGE_ID MODEL' or just 'MODEL' (with --resolve-model-page)"
    )]
    pub resolve_model: Option<Vec<String>>,

    #[arg(
        long,
        value_name = "TARGET",
        help = "Explain why a component/model/field behaves as it does (visible/disabled/data-empty). Supports comp:PAGE|ID, model:ID, field:MODEL.FIELD (requires --project-dir)"
    )]
    pub explain_condition: Option<String>,

    #[arg(
        long,
        value_name = "INTENT",
        default_value = "auto",
        help = "Traversal intent for --explain-condition: auto | display | value-source | writer | availability | context"
    )]
    pub intent: String,

    #[arg(
        long,
        value_name = "TARGET",
        help = "M35: Generate structured query advice for a target. Combines with --advise-query-page and --question-kind."
    )]
    pub advise_query: Option<String>,

    #[arg(long, value_name = "PAGE", help = "Page ID for --advise-query scope")]
    pub advise_query_page: Option<String>,

    #[arg(
        long,
        value_name = "KIND",
        help = "Question kind for --advise-query: display | value-source | availability | writer | page-logic | model-relationships"
    )]
    pub question_kind: Option<String>,

    #[arg(
        long,
        value_name = "DIR",
        help = "Session root directory (default: ~/.metadata-checker/sessions)"
    )]
    pub session_dir: Option<PathBuf>,

    #[arg(long, help = "List all sessions")]
    pub session_list: bool,

    #[arg(long, value_name = "ID", help = "Show session details")]
    pub session_show: Option<String>,

    #[arg(
        long,
        value_name = "ID",
        alias = "remote-index",
        help = "Refresh session from remote"
    )]
    pub session_refresh: Option<String>,

    #[arg(
        long,
        value_name = "URL",
        alias = "base-url",
        help = "Remote BI server URL (e.g. https://autocrm-test.xiaoshouyi.com)"
    )]
    pub remote_server: Option<String>,

    #[arg(
        long,
        value_name = "PROJECT",
        alias = "project",
        help = "Remote project reference"
    )]
    pub remote_project: Option<String>,

    #[arg(long, value_name = "USERNAME", help = "Remote BI username")]
    pub remote_username: Option<String>,

    #[arg(long, value_name = "PASSWORD", help = "Remote BI password")]
    pub remote_password: Option<String>,

    #[arg(
        long,
        value_name = "SOURCE",
        help = "Remote source filter placeholder (reserved, currently no-op)"
    )]
    pub remote_source: Option<String>,

    #[arg(
        long,
        value_name = "MODULE",
        help = "Remote module filter placeholder (reserved, currently no-op)"
    )]
    pub remote_module: Option<String>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Remote file filter placeholder (reserved, currently no-op)"
    )]
    pub remote_file: Option<String>,

    #[arg(long, value_name = "MODE", help = "Session sync mode: full | partial")]
    pub session_sync_mode: Option<String>,

    #[arg(long, value_name = "ID", help = "Delete session")]
    pub session_delete: Option<String>,

    #[arg(long, value_name = "ID", help = "Show session status")]
    pub session_status: Option<String>,
    #[arg(
        long,
        help = "启动 JSONL stdin/stdout 长驻查询服务（机器协议，非人类 REPL）"
    )]
    pub serve_stdio: bool,
}

impl Cli {
    pub fn is_human(&self) -> bool {
        self.human || self.interactive
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// 解析图数据库路径
    pub fn resolve_graph_db_path(&self) -> Option<PathBuf> {
        if let Some(ref explicit) = self.graph_db_path {
            return Some(explicit.clone());
        }
        self.project_dir
            .as_ref()
            .map(|dir| dir.join(".metadata-checker.graphdb"))
    }
}

/// CLI runtime 工具调用的入口原始输入。
///
/// Clap 负责把命令行解析成 `Cli`，该结构只承载已判定为 runtime 工具的参数，
/// 由 `CliAdapter` 统一转换为 `ToolInvocation`。
#[derive(Debug, Clone)]
pub struct CliToolInput {
    pub command: ToolCommand,
    pub target: Option<String>,
    pub budget: String,
    pub human: bool,
    pub intent: Option<String>,
    pub page_scope: Option<String>,
    pub depth: Option<usize>,
    pub check_reload: bool,
}

/// CLI 调用 adapter，负责命令行 runtime 工具参数到标准调用对象的转换。
pub struct CliAdapter;

impl InvocationAdapter for CliAdapter {
    type RawInput = CliToolInput;
    type RawOutput = serde_json::Value;

    fn parse_input(&self, raw: Self::RawInput) -> Result<ToolInvocation, ToolError> {
        let spec = ToolRegistry::find_by_command(raw.command).ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::UnknownCommand,
                format!("Tool spec not found for CLI command: {:?}", raw.command),
            )
        })?;

        crate::tool_contract::validate_budget(&raw.budget)?;

        if spec.requires_target {
            let target = raw.target.as_deref().unwrap_or("");
            if target.trim().is_empty() {
                return Err(ToolError::new(
                    ToolErrorCode::MissingTarget,
                    format!("Missing target for {}", spec.name),
                ));
            }
            crate::tool_contract::validate_target_prefix(raw.command, target)?;
        }

        if let Some(intent) = raw.intent.as_deref() {
            if !spec.supported_intents.is_empty() {
                crate::tool_contract::validate_intent(intent)?;
            }
        }

        Ok(ToolInvocation {
            command: raw.command,
            target: raw.target,
            budget: Some(raw.budget),
            intent: raw.intent,
            depth: raw.depth,
            page_scope: raw.page_scope,
            human: raw.human,
            check_reload: raw.check_reload,
        })
    }

    fn render_output(&self, response: ToolResponse) -> Result<Self::RawOutput, ToolError> {
        if response.ok {
            Ok(response.result.unwrap_or(serde_json::Value::Null))
        } else {
            let err = response.error.unwrap_or_else(|| {
                ToolError::new(ToolErrorCode::InternalError, "Missing tool error")
            });
            Ok(serde_json::json!({
                "ok": false,
                "error": {
                    "code": err.code_str(),
                    "message": err.message,
                },
                "diagnostics": response.diagnostics,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_contract::{InvocationAdapter, ToolErrorCode};

    #[test]
    fn test_cli_adapter_builds_standard_invocation() {
        let adapter = CliAdapter;
        let invocation = adapter
            .parse_input(CliToolInput {
                command: ToolCommand::QueryPageLogic,
                target: Some("page:app/actions_test.spg".to_string()),
                budget: "compact".to_string(),
                human: false,
                intent: None,
                page_scope: None,
                depth: None,
                check_reload: false,
            })
            .expect("valid CLI input must parse");

        assert_eq!(invocation.command, ToolCommand::QueryPageLogic);
        assert_eq!(
            invocation.target.as_deref(),
            Some("page:app/actions_test.spg")
        );
        assert_eq!(invocation.budget.as_deref(), Some("compact"));
    }

    #[test]
    fn test_cli_adapter_rejects_missing_target() {
        let adapter = CliAdapter;
        let err = adapter
            .parse_input(CliToolInput {
                command: ToolCommand::QueryModel,
                target: None,
                budget: "compact".to_string(),
                human: false,
                intent: None,
                page_scope: None,
                depth: None,
                check_reload: false,
            })
            .expect_err("missing target must fail");

        assert_eq!(err.code, ToolErrorCode::MissingTarget);
    }
}
