//! Browser WASM JS 绑定层
//!
//! M40.2：将 browser.rs 的纯 Rust API 通过 wasm-bindgen 暴露给 JavaScript。
//! 所有函数接收/返回 JSON 字符串，保持与 browser.rs 相同的语义。

use wasm_bindgen::prelude::*;

use crate::browser::{
    AnalysisOptions, AnalysisStatus, BrowserAnalysisEnvelope, RuntimeOptions, SuperPageSelection,
    init_runtime, load_superpage_document, runtime_status,
    build_or_update_superpage_graph, analyze_superpage_selection,
};

/// 初始化 WASM runtime，接收 JSON 字符串选项
#[wasm_bindgen(js_name = initRuntime)]
pub fn js_init_runtime(options_json: &str) -> String {
    let options: RuntimeOptions = serde_json::from_str(options_json).unwrap_or_default();
    let result = init_runtime(options);
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
}

/// 查询 runtime 状态
#[wasm_bindgen(js_name = runtimeStatus)]
pub fn js_runtime_status() -> String {
    let result = runtime_status();
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
}

/// 加载 SuperPage 文档
#[wasm_bindgen(js_name = loadSuperpageDocument)]
pub fn js_load_superpage_document(source_path: &str, raw_text: &str) -> String {
    let result = load_superpage_document(source_path, raw_text);
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
}

/// 为已加载的 SuperPage 构建或更新内存图
#[wasm_bindgen(js_name = buildOrUpdateSuperpageGraph)]
pub fn js_build_or_update_superpage_graph(source_path: &str) -> String {
    let result = build_or_update_superpage_graph(source_path);
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
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
            return serde_json::to_string(&err).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string());
        }
    };
    let options: AnalysisOptions = serde_json::from_str(options_json).unwrap_or_default();
    let result = analyze_superpage_selection(selection, options);
    serde_json::to_string(&result).unwrap_or_else(|_| "{\"status\":\"error\"}".to_string())
}
