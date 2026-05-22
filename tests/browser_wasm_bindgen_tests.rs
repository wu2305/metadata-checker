#![cfg(feature = "browser-wasm")]

//! Browser WASM JSON wrapper 测试
//! 仅在 browser-wasm feature 下编译，验证 JS callable 包装层 JSON 解析与透传行为。

use metadata_checker::browser_wasm_bindgen::{
    js_analyze_superpage_selection, js_build_or_update_superpage_graph, js_init_runtime,
    js_load_superpage_document, js_runtime_status,
};

#[test]
fn test_js_init_runtime_invalid_options_json() {
    let output = js_init_runtime("not json");
    let envelope: serde_json::Value =
        serde_json::from_str(&output).expect("invalid options should return JSON");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["diagnostics"][0]["code"], "INVALID_OPTIONS");
}

#[test]
fn test_js_analyze_superpage_selection_invalid_selection_json() {
    let _ = js_init_runtime("{}");
    let output = js_analyze_superpage_selection("not json", "{}");
    let envelope: serde_json::Value =
        serde_json::from_str(&output).expect("invalid selection should return JSON");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["diagnostics"][0]["code"], "INVALID_SELECTION");
}

#[test]
fn test_js_analyze_superpage_selection_invalid_options_json() {
    let _ = js_init_runtime("{}");
    let selection = r#"{"source_path":"app/test.spg","file_id":"test","selected_component_ids":["btn1"],"active_component_id":"btn1"}"#;
    let output = js_analyze_superpage_selection(selection, "not json");
    let envelope: serde_json::Value =
        serde_json::from_str(&output).expect("invalid options should return JSON");
    assert_eq!(envelope["status"], "error");
    assert_eq!(envelope["diagnostics"][0]["code"], "INVALID_OPTIONS");
}

#[test]
fn test_js_exports_return_parseable_envelopes() {
    let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#;

    let init = js_init_runtime("{}");
    let init_envelope: serde_json::Value =
        serde_json::from_str(&init).expect("init output should be valid JSON");
    assert_eq!(init_envelope["status"], "ready");

    let status = js_runtime_status();
    let status_envelope: serde_json::Value =
        serde_json::from_str(&status).expect("status output should be valid JSON");
    assert_eq!(status_envelope["status"], "ready");

    let load = js_load_superpage_document("app/test.spg", raw);
    let load_envelope: serde_json::Value =
        serde_json::from_str(&load).expect("load output should be valid JSON");
    assert_eq!(load_envelope["status"], "ready");

    let build = js_build_or_update_superpage_graph("app/test.spg");
    let build_envelope: serde_json::Value =
        serde_json::from_str(&build).expect("build output should be valid JSON");
    assert_eq!(build_envelope["status"], "ready");

    let selection = r#"{"source_path":"app/test.spg","file_id":"test","selected_component_ids":["btn1"],"active_component_id":"btn1"}"#;
    let options =
        r#"{"include_priority":false,"include_conditions":false,"include_dataflow":false}"#;
    let analyze = js_analyze_superpage_selection(selection, options);
    let analyze_envelope: serde_json::Value =
        serde_json::from_str(&analyze).expect("analyze output should be valid JSON");
    assert_eq!(analyze_envelope["status"], "ready");
}
