use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use crate::response_processor::ResponseProcessor;
use crate::runtime::{GraphRuntime, RuntimeQueryRequest};
use crate::tool_contract::{
    InvocationAdapter, ToolError, ToolErrorCode, ToolInvocation, ToolRegistry, ToolResponse,
};

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

/// stdio 调用 adapter，负责 JSONL 请求和标准工具调用之间的转换。
pub struct StdioAdapter;

impl InvocationAdapter for StdioAdapter {
    type RawInput = StdioRequest;
    type RawOutput = StdioResponse;

    fn parse_input(&self, raw: Self::RawInput) -> std::result::Result<ToolInvocation, ToolError> {
        let spec = ToolRegistry::find_by_name(&raw.command).ok_or_else(|| {
            ToolError::new(
                ToolErrorCode::UnknownCommand,
                format!(
                    "Unknown command: '{}'. Supported: {}",
                    raw.command,
                    ToolRegistry::command_names().join(", ")
                ),
            )
        })?;

        let budget = raw.budget.unwrap_or_else(|| "normal".to_string());
        crate::tool_contract::validate_budget(&budget)?;

        let intent = raw.intent.unwrap_or_else(|| "auto".to_string());
        if !spec.supported_intents.is_empty() {
            crate::tool_contract::validate_intent(&intent)?;
        }

        if spec.requires_target {
            let target = raw.target.as_deref().unwrap_or("");
            if target.trim().is_empty() {
                return Err(ToolError::new(
                    ToolErrorCode::MissingTarget,
                    format!("Missing target for {}", spec.name),
                ));
            }
            crate::tool_contract::validate_target_prefix(spec.command, target)?;
        }

        let depth = crate::tool_contract::validate_depth(raw.depth.as_ref())?;

        Ok(ToolInvocation {
            command: spec.command,
            target: raw.target,
            budget: Some(budget),
            intent: Some(intent),
            depth: Some(depth),
            page_scope: raw.page_scope,
            human: raw.human.unwrap_or(false),
            check_reload: raw.check_reload.unwrap_or(false),
        })
    }

    fn render_output(
        &self,
        response: ToolResponse,
    ) -> std::result::Result<StdioResponse, ToolError> {
        Ok(if response.ok {
            StdioResponse {
                request_id: String::new(),
                ok: true,
                result: response.result,
                error: None,
                diagnostics: response.diagnostics,
                timing: Some(zero_timing()),
            }
        } else {
            let err = response.error.unwrap_or_else(|| {
                ToolError::new(ToolErrorCode::InternalError, "Missing tool error")
            });
            error_response(
                String::new(),
                err.code_str(),
                err.message,
                response.diagnostics,
            )
        })
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

/// 解析并处理单行 stdio JSONL 输入，供 benchmark 与集成测试复用。
pub fn dispatch_stdio_line(runtime: &mut GraphRuntime, line: &str) -> StdioResponse {
    if line.trim().is_empty() {
        return error_response(
            String::new(),
            "INVALID_JSON",
            "empty request line".to_string(),
            vec![],
        );
    }

    let request: StdioRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                String::new(),
                "INVALID_JSON",
                format!("JSON parse error: {}", error),
                vec![],
            );
        }
    };

    handle_request(runtime, &request)
}

/// 序列化 stdio 响应并回填 `timing.output_size_bytes`。
pub fn serialize_stdio_response(resp: &mut StdioResponse) -> Result<String> {
    serialize_response_with_timing(resp)
        .map_err(|error| anyhow::anyhow!("serialize stdio response failed: {}", error))
}

/// 处理单个请求
fn handle_request(runtime: &mut GraphRuntime, request: &StdioRequest) -> StdioResponse {
    let mut diagnostics = Vec::new();
    let adapter = StdioAdapter;
    let invocation = match adapter.parse_input(request.clone()) {
        Ok(i) => i,
        Err(err) => {
            return error_response(
                request.request_id.clone(),
                err.code_str(),
                err.message,
                diagnostics,
            );
        }
    };
    let spec = ToolRegistry::find_by_command(invocation.command).expect("registered command");

    // 处理 status 命令
    if invocation.command == crate::tool_contract::ToolCommand::Status {
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
    if invocation.command == crate::tool_contract::ToolCommand::ReloadGraph {
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
    if invocation.check_reload {
        match runtime.reload_if_changed() {
            Ok(crate::runtime::ReloadResult::Reloaded) => {
                diagnostics.push("GRAPH_RELOADED".to_string())
            }
            Ok(crate::runtime::ReloadResult::Unchanged) => {
                diagnostics.push("GRAPH_UNCHANGED".to_string())
            }
            Ok(crate::runtime::ReloadResult::ReloadFailed { .. }) => {
                diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
            }
            Err(e) => {
                diagnostics.push(format!("GRAPH_RELOAD_FAILED: {}", e));
            }
        }
    }

    // human 模式校验
    let human = if invocation.human && !spec.supports_human {
        diagnostics.push("HUMAN_MODE_NOT_SUPPORTED".to_string());
        false
    } else {
        invocation.human
    };

    // 构造 RuntimeQueryRequest 并统一执行
    let req = RuntimeQueryRequest {
        command: invocation.command,
        target: invocation.target.unwrap_or_default(),
        budget: invocation.budget.unwrap_or_else(|| "normal".to_string()),
        human,
        intent: invocation.intent,
        page_scope: invocation.page_scope,
        depth: invocation.depth,
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

    // 先只收敛 output_size_bytes；serialize_ms 每轮都会增长，不能参与 changed 判定。
    for _ in 0..8 {
        let output_size_bytes = json.len() as u64;
        let Some(timing) = resp.timing.as_mut() else {
            return Ok(json);
        };
        if timing.output_size_bytes == output_size_bytes {
            break;
        }
        timing.output_size_bytes = output_size_bytes;
        json = serde_json::to_string(resp)?;
    }

    let serialize_ms = serialize_start.elapsed().as_millis();
    if let Some(timing) = resp.timing.as_mut() {
        timing.serialize_ms = serialize_ms;
        timing.total_ms = timing.graph_load_ms + timing.query_compute_ms + serialize_ms;
    }
    for _ in 0..4 {
        json = serde_json::to_string(resp)?;
        let output_size_bytes = json.len() as u64;
        let Some(timing) = resp.timing.as_mut() else {
            return Ok(json);
        };
        if timing.output_size_bytes == output_size_bytes {
            break;
        }
        timing.output_size_bytes = output_size_bytes;
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

    /// 大响应 + 非零 timing 时，output_size_bytes 仍须与最终 JSON 行长度一致。
    #[test]
    fn test_success_response_output_size_matches_serialized_json() {
        let mut resp = StdioResponse {
            request_id: "req-explain".to_string(),
            ok: true,
            result: Some(serde_json::json!({
                "kind": "Explain",
                "summary": { "what_is_it": "x".repeat(8_000) }
            })),
            error: None,
            diagnostics: Vec::new(),
            timing: Some(crate::runtime::RuntimeTiming {
                graph_load_ms: 12,
                query_compute_ms: 345,
                serialize_ms: 0,
                total_ms: 0,
                output_size_bytes: 1,
            }),
        };

        let json =
            serialize_response_with_timing(&mut resp).expect("stdio response must serialize");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("stdio response must be valid JSON");
        assert_eq!(
            value["timing"]["output_size_bytes"].as_u64(),
            Some(json.len() as u64),
            "output_size_bytes must match serialized response size"
        );
    }
}
