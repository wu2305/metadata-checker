use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use crate::response_processor::ResponseProcessor;
use crate::runtime::{GraphRuntime, RuntimeQueryRequest};
use crate::tool_contract::ToolRegistry;

/// Stdio 支持的命令枚举
///
/// 将字符串命令解析为类型安全枚举，避免大 match 中散落字符串字面量。
#[derive(Debug, Clone, PartialEq, Eq)]
enum StdioCommand {
    ExplainCondition,
    Explain,
    QueryModel,
    QueryPageLogic,
    AdviseQuery,
    Context,
    Status,
    Reload,
    Unknown(String),
}

impl StdioCommand {
    fn parse(s: &str) -> Self {
        match s {
            "explain_condition" => StdioCommand::ExplainCondition,
            "explain" => StdioCommand::Explain,
            "query_model" => StdioCommand::QueryModel,
            "query_page_logic" => StdioCommand::QueryPageLogic,
            "advise_query" => StdioCommand::AdviseQuery,
            "context" => StdioCommand::Context,
            "status" => StdioCommand::Status,
            "reload" => StdioCommand::Reload,
            other => StdioCommand::Unknown(other.to_string()),
        }
    }

    /// 映射到 M38 统一 ToolCommand。
    fn to_tool_command(&self) -> Option<crate::tool_contract::ToolCommand> {
        use crate::tool_contract::ToolCommand;
        match self {
            StdioCommand::ExplainCondition => Some(ToolCommand::ExplainCondition),
            StdioCommand::Explain => Some(ToolCommand::Explain),
            StdioCommand::QueryModel => Some(ToolCommand::QueryModel),
            StdioCommand::QueryPageLogic => Some(ToolCommand::QueryPageLogic),
            StdioCommand::AdviseQuery => Some(ToolCommand::AdviseQuery),
            StdioCommand::Context => Some(ToolCommand::Context),
            StdioCommand::Status => Some(ToolCommand::Status),
            StdioCommand::Reload => Some(ToolCommand::ReloadGraph),
            StdioCommand::Unknown(_) => None,
        }
    }
}

/// Stdio JSONL 请求
///
/// 每行一个 JSON 对象，stdin 逐行读取
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioRequest {
    pub request_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_reload: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_scope: Option<String>,
}

/// Stdio 结构化错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioError {
    pub code: String,
    pub message: String,
}

/// Stdio JSONL 响应
///
/// 每行一个 JSON 对象，stdout 逐行输出
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StdioResponse {
    pub request_id: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<StdioError>,
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::runtime::RuntimeTiming>,
}

fn zero_timing() -> crate::runtime::RuntimeTiming {
    ResponseProcessor::zero_timing()
}

fn error_response(
    request_id: String,
    code: &str,
    message: impl Into<String>,
    diagnostics: Vec<String>,
) -> StdioResponse {
    StdioResponse {
        request_id,
        ok: false,
        result: None,
        error: Some(StdioError {
            code: code.to_string(),
            message: message.into(),
        }),
        diagnostics,
        timing: Some(zero_timing()),
    }
}

/// 启动 JSONL stdio 服务
///
/// 加载 graphdb 一次，进入 stdin/stdout 循环处理请求。
/// stderr 输出运行日志，stdout 只输出 JSONL 响应。
pub fn run_stdio_server(
    graph_db_path: &std::path::Path,
    project_dir: Option<&std::path::Path>,
) -> Result<()> {
    let mut runtime = GraphRuntime::load_with_project_dir(graph_db_path, project_dir)
        .map_err(|e| anyhow::anyhow!("Failed to load graphdb: {}", e))?;
    eprintln!(
        "[stdio-server] Graph loaded, {} nodes, ready",
        runtime.graph.graph.node_count()
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let resp = error_response(
                    String::new(),
                    "QUERY_FAILED",
                    format!("stdin read error: {}", e),
                    vec![],
                );
                write_response(&mut stdout_lock, resp)?;
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: StdioRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = error_response(
                    String::new(),
                    "INVALID_JSON",
                    format!("JSON parse error: {}", e),
                    vec![],
                );
                write_response(&mut stdout_lock, resp)?;
                continue;
            }
        };

        let resp = handle_request(&mut runtime, &request);
        write_response(&mut stdout_lock, resp)?;
    }

    eprintln!("[stdio-server] stdin closed, shutting down");
    Ok(())
}

/// 处理单个请求
fn handle_request(runtime: &mut GraphRuntime, request: &StdioRequest) -> StdioResponse {
    let mut diagnostics = Vec::new();

    let command = StdioCommand::parse(&request.command);

    // 未知命令
    let tool_command = match command.to_tool_command() {
        Some(tc) => tc,
        None => {
            return error_response(
                request.request_id.clone(),
                "UNKNOWN_COMMAND",
                format!("Unknown command: {}", request.command),
                diagnostics,
            );
        }
    };

    // M38 统一参数校验
    let budget = request
        .budget
        .clone()
        .unwrap_or_else(|| "normal".to_string());
    if let Err(err) = crate::tool_contract::validate_budget(&budget) {
        return error_response(
            request.request_id.clone(),
            err.code_str(),
            err.message,
            diagnostics,
        );
    }

    let human = request.human.unwrap_or(false);
    let intent = request.intent.clone().unwrap_or_else(|| "auto".to_string());
    if command == StdioCommand::ExplainCondition {
        if let Err(err) = crate::tool_contract::validate_intent(&intent) {
            return error_response(
                request.request_id.clone(),
                err.code_str(),
                err.message,
                diagnostics,
            );
        }
    }

    let spec = match ToolRegistry::find_by_command(tool_command) {
        Some(s) => s,
        None => {
            return error_response(
                request.request_id.clone(),
                "INTERNAL_ERROR",
                format!("Tool spec not found for command: {}", request.command),
                diagnostics,
            );
        }
    };

    // 处理 status 命令
    if tool_command == crate::tool_contract::ToolCommand::Status {
        let status = runtime.status();
        return StdioResponse {
            request_id: request.request_id.clone(),
            ok: true,
            result: Some(serde_json::to_value(status).unwrap_or(serde_json::Value::Null)),
            error: None,
            diagnostics,
            timing: Some(zero_timing()),
        };
    }

    // 处理 reload 命令
    if tool_command == crate::tool_contract::ToolCommand::ReloadGraph {
        match runtime.reload() {
            Ok(()) => {
                diagnostics.push("GRAPH_RELOADED".to_string());
                let status = runtime.status();
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: true,
                    result: Some(serde_json::to_value(status).unwrap_or(serde_json::Value::Null)),
                    error: None,
                    diagnostics,
                    timing: Some(zero_timing()),
                };
            }
            Err(e) => {
                diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                return error_response(
                    request.request_id.clone(),
                    "GRAPH_RELOAD_FAILED",
                    format!("Reload failed: {}", e),
                    diagnostics,
                );
            }
        }
    }

    // 可选：在查询前检查 graphdb 是否变更
    if request.check_reload == Some(true) {
        match runtime.reload_if_changed() {
            Ok(crate::runtime::ReloadResult::Reloaded) => {
                diagnostics.push("GRAPH_RELOADED".to_string())
            }
            Ok(crate::runtime::ReloadResult::Unchanged) => {}
            Ok(crate::runtime::ReloadResult::ReloadFailed { .. }) => {
                diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
            }
            Err(e) => {
                diagnostics.push(format!("GRAPH_RELOAD_FAILED: {}", e));
            }
        }
    }

    // target 校验
    if spec.requires_target {
        let target = request.target.as_deref().unwrap_or("");
        if target.trim().is_empty() {
            return error_response(
                request.request_id.clone(),
                "MISSING_TARGET",
                format!("Missing target for {}", spec.name),
                diagnostics,
            );
        }
        if let Err(err) = crate::tool_contract::validate_target_prefix(tool_command, target) {
            return error_response(
                request.request_id.clone(),
                err.code_str(),
                err.message,
                diagnostics,
            );
        }
    }

    // human 模式校验
    if human && !spec.supports_human {
        diagnostics.push("HUMAN_MODE_NOT_SUPPORTED".to_string());
    }

    // 构造 RuntimeQueryRequest 并统一执行
    let depth = match crate::tool_contract::validate_depth(request.depth.as_ref()) {
        Ok(d) => Some(d),
        Err(err) => {
            return error_response(
                request.request_id.clone(),
                err.code_str(),
                err.message,
                diagnostics,
            );
        }
    };

    let req = RuntimeQueryRequest {
        command: tool_command,
        target: request.target.clone().unwrap_or_default(),
        budget: budget.clone(),
        human,
        intent: Some(intent),
        page_scope: request.page_scope.clone(),
        depth,
        check_reload: false,
    };

    match runtime.query(req) {
        Ok(response) => {
            diagnostics.extend(response.diagnostics);
            StdioResponse {
                request_id: request.request_id.clone(),
                ok: true,
                result: Some(response.result),
                error: None,
                diagnostics,
                timing: Some(response.timing),
            }
        }
        Err(e) => {
            diagnostics.push(format!("Query error: {}", e));
            error_response(
                request.request_id.clone(),
                "QUERY_FAILED",
                format!("Query failed: {}", e),
                diagnostics,
            )
        }
    }
}

/// 向 stdout 写入单行 JSON 响应并 flush
fn write_response(stdout: &mut io::StdoutLock, mut resp: StdioResponse) -> Result<()> {
    let json = serialize_response_with_timing(&mut resp)
        .map_err(|e| anyhow::anyhow!("serialize response failed: {}", e))?;
    writeln!(stdout, "{}", json)?;
    stdout.flush()?;
    Ok(())
}

fn serialize_response_with_timing(resp: &mut StdioResponse) -> serde_json::Result<String> {
    let serialize_start = std::time::Instant::now();
    let mut json = serde_json::to_string(resp)?;

    for _ in 0..8 {
        let output_size_bytes = json.len() as u64;
        let serialize_ms = serialize_start.elapsed().as_millis();
        let mut changed = false;

        if let Some(timing) = resp.timing.as_mut() {
            changed =
                crate::response_processor::ResponseProcessor::update_timing_after_serialization(
                    timing,
                    output_size_bytes,
                    serialize_ms,
                );
        }

        if !changed {
            return Ok(json);
        }

        json = serde_json::to_string(resp)?;
    }

    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_failed_error_response_keeps_fixed_envelope() {
        let mut resp = error_response(
            "req-query-failed".to_string(),
            "QUERY_FAILED",
            "Query failed: synthetic failure",
            vec![],
        );

        let json =
            serialize_response_with_timing(&mut resp).expect("stdio response must serialize");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("stdio response must be valid JSON");
        assert_eq!(value["request_id"].as_str(), Some("req-query-failed"));
        assert_eq!(value["ok"].as_bool(), Some(false));
        assert_eq!(value["error"]["code"].as_str(), Some("QUERY_FAILED"));
        assert_eq!(
            value["error"]["message"].as_str(),
            Some("Query failed: synthetic failure")
        );
        assert!(
            value
                .as_object()
                .expect("response must be object")
                .contains_key("diagnostics"),
            "diagnostics must be present even when empty"
        );
        assert!(
            value["diagnostics"]
                .as_array()
                .expect("diagnostics must be array")
                .is_empty()
        );
        assert!(
            value["timing"].is_object(),
            "timing must be present on error response"
        );
        assert_eq!(
            value["timing"]["output_size_bytes"].as_u64(),
            Some(json.len() as u64),
            "output_size_bytes must match serialized response size"
        );
    }
}
