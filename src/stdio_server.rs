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
    pub target: String,
    pub budget: Option<String>,
    pub human: Option<bool>,
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
    let runtime = GraphRuntime::load(graph_db_path)
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

        let resp = handle_request(&runtime, &request);
        write_response(&mut stdout_lock, &resp)?;
    }

    eprintln!("[stdio-server] stdin closed, shutting down");
    Ok(())
}

/// 处理单个请求
fn handle_request(runtime: &GraphRuntime, request: &StdioRequest) -> StdioResponse {
    let mut diagnostics = Vec::new();

    let command = match request.command.as_str() {
        "explain_condition" => RuntimeQueryCommand::ExplainCondition,
        other => {
            return StdioResponse {
                request_id: request.request_id.clone(),
                ok: false,
                result: None,
                error: Some(format!("Unknown command: {}", other)),
                diagnostics,
                timing: None,
            };
        }
    };

    let budget = request.budget.clone().unwrap_or_else(|| "normal".to_string());
    let human = request.human.unwrap_or(false);

    let req = RuntimeQueryRequest {
        command,
        target: request.target.clone(),
        budget,
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

/// 向 stdout 写入单行 JSON 响应并 flush
fn write_response(stdout: &mut io::StdoutLock, resp: &StdioResponse) -> Result<()> {
    let json = serde_json::to_string(resp)
        .map_err(|e| anyhow::anyhow!("serialize response failed: {}", e))?;
    writeln!(stdout, "{}", json)?;
    stdout.flush()?;
    Ok(())
}
