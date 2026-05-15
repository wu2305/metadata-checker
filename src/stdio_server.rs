use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

use crate::runtime::{GraphRuntime, RuntimeQueryCommand, RuntimeQueryRequest};

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
    pub human: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_reload: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<usize>,
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
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<crate::runtime::RuntimeTiming>,
}

/// 启动 JSONL stdio 服务
///
/// 加载 graphdb 一次，进入 stdin/stdout 循环处理请求。
/// stderr 输出运行日志，stdout 只输出 JSONL 响应。
pub fn run_stdio_server(graph_db_path: &std::path::Path) -> Result<()> {
    let mut runtime = GraphRuntime::load(graph_db_path)
        .map_err(|e| anyhow::anyhow!("Failed to load graphdb: {}", e))?;
    eprintln!("[stdio-server] Graph loaded, {} nodes, ready", runtime.graph.graph.node_count());

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let resp = StdioResponse {
                    request_id: String::new(),
                    ok: false,
                    result: None,
                    error: Some(format!("stdin read error: {}", e)),
                    diagnostics: vec![],
                    timing: None,
                };
                write_response(&mut stdout_lock, &resp)?;
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let request: StdioRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = StdioResponse {
                    request_id: String::new(),
                    ok: false,
                    result: None,
                    error: Some(format!("JSON parse error: {}", e)),
                    diagnostics: vec![],
                    timing: None,
                };
                write_response(&mut stdout_lock, &resp)?;
                continue;
            }
        };

        let resp = handle_request(&mut runtime, &request);
        write_response(&mut stdout_lock, &resp)?;
    }

    eprintln!("[stdio-server] stdin closed, shutting down");
    Ok(())
}

/// 处理单个请求
fn handle_request(runtime: &mut GraphRuntime, request: &StdioRequest) -> StdioResponse {
    let mut diagnostics = Vec::new();

    // 处理 status 命令（不需要 target）
    if request.command == "status" {
        let status = runtime.status();
        return StdioResponse {
            request_id: request.request_id.clone(),
            ok: true,
            result: Some(serde_json::to_value(status).unwrap_or(serde_json::Value::Null)),
            error: None,
            diagnostics,
            timing: None,
        };
    }

    // 处理 reload 命令（不需要 target）
    if request.command == "reload" {
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
                    timing: None,
                };
            }
            Err(e) => {
                diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some(format!("Reload failed: {}", e)),
                    diagnostics,
                    timing: None,
                };
            }
        }
    }

    // 可选：在查询前检查 graphdb 是否变更
    if request.check_reload == Some(true) {
        match runtime.reload_if_changed() {
            Ok(crate::runtime::ReloadResult::Reloaded) => diagnostics.push("GRAPH_RELOADED".to_string()),
            Ok(crate::runtime::ReloadResult::Unchanged) => {}
            Ok(crate::runtime::ReloadResult::ReloadFailed { .. }) => {
                diagnostics.push("GRAPH_RELOAD_FAILED".to_string());
            }
            Err(e) => {
                diagnostics.push(format!("GRAPH_RELOAD_FAILED: {}", e));
            }
        }
    }

    let budget = request.budget.clone().unwrap_or_else(|| "normal".to_string());
    let human = request.human.unwrap_or(false);

    match request.command.as_str() {
        "explain_condition" => {
            if request.target.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some("Missing target for explain_condition".to_string()),
                    diagnostics,
                    timing: None,
                };
            }
            let req = RuntimeQueryRequest {
                command: RuntimeQueryCommand::ExplainCondition,
                target: request.target.clone().unwrap_or_default(),
                budget: budget.clone(),
                human,
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
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: false,
                        result: None,
                        error: Some(format!("Query failed: {}", e)),
                        diagnostics,
                        timing: None,
                    }
                }
            }
        }
        "query_model" => {
            if request.target.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some("Missing target for query_model".to_string()),
                    diagnostics,
                    timing: None,
                };
            }
            let start = std::time::Instant::now();
            match crate::query::build_query_model_output(
                &runtime.graph,
                &request.target.clone().unwrap_or_default(),
                &budget,
            ) {
                Ok(result) => {
                    let total_ms = start.elapsed().as_millis();
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: true,
                        result: Some(result),
                        error: None,
                        diagnostics,
                        timing: Some(crate::runtime::RuntimeTiming {
                            graph_load_ms: 0,
                            query_compute_ms: total_ms,
                            serialize_ms: 0,
                            total_ms,
                        }),
                    }
                }
                Err(e) => {
                    diagnostics.push(format!("Query error: {}", e));
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: false,
                        result: None,
                        error: Some(format!("Query failed: {}", e)),
                        diagnostics,
                        timing: None,
                    }
                }
            }
        }
        "context" => {
            if request.target.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some("Missing target for context".to_string()),
                    diagnostics,
                    timing: None,
                };
            }
            let depth = request.depth.unwrap_or(1);
            let start = std::time::Instant::now();
            match crate::context::build_context_output(
                &runtime.graph,
                &request.target.clone().unwrap_or_default(),
                depth,
                &budget,
            ) {
                Ok(result) => {
                    let total_ms = start.elapsed().as_millis();
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: true,
                        result: Some(result),
                        error: None,
                        diagnostics,
                        timing: Some(crate::runtime::RuntimeTiming {
                            graph_load_ms: 0,
                            query_compute_ms: total_ms,
                            serialize_ms: 0,
                            total_ms,
                        }),
                    }
                }
                Err(e) => {
                    diagnostics.push(format!("Query error: {}", e));
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: false,
                        result: None,
                        error: Some(format!("Query failed: {}", e)),
                        diagnostics,
                        timing: None,
                    }
                }
            }
        }
        "explain" => {
            if request.target.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some("Missing target for explain".to_string()),
                    diagnostics,
                    timing: None,
                };
            }
            let start = std::time::Instant::now();
            match crate::explain::build_explain_output(
                &runtime.graph,
                &request.target.clone().unwrap_or_default(),
            ) {
                Ok(result) => {
                    let total_ms = start.elapsed().as_millis();
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: true,
                        result: Some(result),
                        error: None,
                        diagnostics,
                        timing: Some(crate::runtime::RuntimeTiming {
                            graph_load_ms: 0,
                            query_compute_ms: total_ms,
                            serialize_ms: 0,
                            total_ms,
                        }),
                    }
                }
                Err(e) => {
                    diagnostics.push(format!("Query error: {}", e));
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: false,
                        result: None,
                        error: Some(format!("Query failed: {}", e)),
                        diagnostics,
                        timing: None,
                    }
                }
            }
        }
        "query_page_logic" => {
            if request.target.as_ref().map(|s| s.is_empty()).unwrap_or(true) {
                return StdioResponse {
                    request_id: request.request_id.clone(),
                    ok: false,
                    result: None,
                    error: Some("Missing target for query_page_logic".to_string()),
                    diagnostics,
                    timing: None,
                };
            }
            let start = std::time::Instant::now();
            match crate::query::build_query_page_logic_output(
                &runtime.graph,
                &request.target.clone().unwrap_or_default(),
                None,
                &budget,
            ) {
                Ok(result) => {
                    let total_ms = start.elapsed().as_millis();
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: true,
                        result: Some(result),
                        error: None,
                        diagnostics,
                        timing: Some(crate::runtime::RuntimeTiming {
                            graph_load_ms: 0,
                            query_compute_ms: total_ms,
                            serialize_ms: 0,
                            total_ms,
                        }),
                    }
                }
                Err(e) => {
                    diagnostics.push(format!("Query error: {}", e));
                    StdioResponse {
                        request_id: request.request_id.clone(),
                        ok: false,
                        result: None,
                        error: Some(format!("Query failed: {}", e)),
                        diagnostics,
                        timing: None,
                    }
                }
            }
        }
        other => {
            StdioResponse {
                request_id: request.request_id.clone(),
                ok: false,
                result: None,
                error: Some(format!("Unknown command: {}", other)),
                diagnostics,
                timing: None,
            }
        }
    }
}

/// 向 stdout 写入单行 JSON 响应并 flush
fn write_response(stdout: &mut io::StdoutLock, resp: &StdioResponse) -> Result<()> {
    let json = serde_json::to_string(resp)
        .map_err(|e| anyhow::anyhow!("serialize response failed: {}", e))?;
    writeln!(stdout, "{}", json)?;
    stdout.flush()?;
    Ok(())
}
