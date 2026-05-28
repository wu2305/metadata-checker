//! Browser WASM JS 绑定层
//!
//! M40.2：将 browser.rs 的纯 Rust API 通过 wasm-bindgen 暴露给 JavaScript。
//! 所有函数接收/返回 JSON 字符串，保持与 browser.rs 相同的语义。

use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

use crate::browser::{
    AnalysisOptions, AnalysisStatus, BrowserAnalysisEnvelope, RuntimeOptions, SuperPageSelection,
    analyze_superpage_selection, build_or_update_superpage_graph,
    enqueue_orchestrator_background_tasks, enqueue_orchestrator_foreground_selection, init_runtime,
    load_superpage_document, orchestrator_status, runtime_status, tick_orchestrator,
};

fn parse_json_or_error<T>(
    json: &str,
    code: &'static str,
    message_prefix: &str,
) -> Result<T, BrowserAnalysisEnvelope>
where
    T: DeserializeOwned,
{
    serde_json::from_str(json).map_err(|e| BrowserAnalysisEnvelope {
        status: AnalysisStatus::Error,
        target: None,
        items: vec![],
        diagnostics: vec![crate::browser::AnalysisDiagnostic {
            severity: "error".to_string(),
            code: code.to_string(),
            message: format!("{}{}", message_prefix, e),
        }],
    })
}

fn envelope_to_string(envelope: &BrowserAnalysisEnvelope) -> String {
    serde_json::to_string(envelope).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
}

fn parse_u64_or_error(
    json: &str,
    code: &'static str,
    message_prefix: &str,
) -> Result<u64, BrowserAnalysisEnvelope> {
    serde_json::from_str(json).map_err(|e| BrowserAnalysisEnvelope {
        status: AnalysisStatus::Error,
        target: None,
        items: vec![],
        diagnostics: vec![crate::browser::AnalysisDiagnostic {
            severity: "error".to_string(),
            code: code.to_string(),
            message: format!("{}{}", message_prefix, e),
        }],
    })
}

fn parse_usize_or_error(
    json: &str,
    code: &'static str,
    message_prefix: &str,
) -> Result<usize, BrowserAnalysisEnvelope> {
    serde_json::from_str(json).map_err(|e| BrowserAnalysisEnvelope {
        status: AnalysisStatus::Error,
        target: None,
        items: vec![],
        diagnostics: vec![crate::browser::AnalysisDiagnostic {
            severity: "error".to_string(),
            code: code.to_string(),
            message: format!("{}{}", message_prefix, e),
        }],
    })
}

/// 初始化 WASM runtime，接收 JSON 字符串选项
#[wasm_bindgen(js_name = initRuntime)]
pub fn js_init_runtime(options_json: &str) -> String {
    let options = match parse_json_or_error::<RuntimeOptions>(
        options_json,
        "INVALID_OPTIONS",
        "Failed to parse options JSON: ",
    ) {
        Ok(v) => v,
        Err(err) => return envelope_to_string(&err),
    };
    let result = init_runtime(options);
    envelope_to_string(&result)
}

/// 查询 runtime 状态
#[wasm_bindgen(js_name = runtimeStatus)]
pub fn js_runtime_status() -> String {
    let result = runtime_status();
    envelope_to_string(&result)
}

/// 加载 SuperPage 文档
#[wasm_bindgen(js_name = loadSuperpageDocument)]
pub fn js_load_superpage_document(source_path: &str, raw_text: &str) -> String {
    let result = load_superpage_document(source_path, raw_text);
    envelope_to_string(&result)
}

/// 为已加载的 SuperPage 构建或更新内存图
#[wasm_bindgen(js_name = buildOrUpdateSuperpageGraph)]
pub fn js_build_or_update_superpage_graph(source_path: &str) -> String {
    let result = build_or_update_superpage_graph(source_path);
    envelope_to_string(&result)
}

/// 分析选中组件
#[wasm_bindgen(js_name = analyzeSuperpageSelection)]
pub fn js_analyze_superpage_selection(selection_json: &str, options_json: &str) -> String {
    let selection: SuperPageSelection = match serde_json::from_str(selection_json) {
        Ok(s) => s,
        Err(e) => {
            let err = BrowserAnalysisEnvelope {
                status: AnalysisStatus::Error,
                target: None,
                items: vec![],
                diagnostics: vec![crate::browser::AnalysisDiagnostic {
                    severity: "error".to_string(),
                    code: "INVALID_SELECTION".to_string(),
                    message: format!("Failed to parse selection JSON: {}", e),
                }],
            };
            return envelope_to_string(&err);
        }
    };
    let options = match parse_json_or_error::<AnalysisOptions>(
        options_json,
        "INVALID_OPTIONS",
        "Failed to parse options JSON: ",
    ) {
        Ok(v) => v,
        Err(err) => return envelope_to_string(&err),
    };
    let result = analyze_superpage_selection(selection, options);
    envelope_to_string(&result)
}

/// 将前台请求入队到 orchestrator（JSON 入参）
#[wasm_bindgen(js_name = enqueueForegroundSelection)]
pub fn js_enqueue_foreground_selection(selection_json: &str, options_json: &str) -> String {
    let result = enqueue_orchestrator_foreground_selection(selection_json, options_json);
    envelope_to_string(&result)
}

/// 将后台任务批量入队到 orchestrator（JSON 入参）
#[wasm_bindgen(js_name = enqueueBackgroundTasks)]
pub fn js_enqueue_background_tasks(tasks_json: &str) -> String {
    let result = enqueue_orchestrator_background_tasks(tasks_json);
    envelope_to_string(&result)
}

/// 推进 orchestrator 一次逻辑 tick（JSON 入参）
#[wasm_bindgen(js_name = tickAnalysisOrchestrator)]
pub fn js_tick_analysis_orchestrator(logical_tick_json: &str, limit_json: &str) -> String {
    let logical_tick = match parse_u64_or_error(
        logical_tick_json,
        "INVALID_TICK",
        "Failed to parse tick JSON: ",
    ) {
        Ok(v) => v,
        Err(err) => return envelope_to_string(&err),
    };
    let limit =
        match parse_usize_or_error(limit_json, "INVALID_LIMIT", "Failed to parse limit JSON: ") {
            Ok(v) => v,
            Err(err) => return envelope_to_string(&err),
        };

    let result = tick_orchestrator(logical_tick, limit);
    envelope_to_string(&result)
}

/// 查询 orchestrator 状态（JSON 入参）
#[wasm_bindgen(js_name = analysisOrchestratorStatus)]
pub fn js_analysis_orchestrator_status(limit_json: &str) -> String {
    let limit =
        match parse_usize_or_error(limit_json, "INVALID_LIMIT", "Failed to parse limit JSON: ") {
            Ok(v) => v,
            Err(err) => return envelope_to_string(&err),
        };
    let result = orchestrator_status(limit);
    envelope_to_string(&result)
}
