#![cfg(feature = "browser-wasm")]

//! Browser WASM JSON wrapper 测试
//! 仅在 browser-wasm feature 下编译，验证 JS callable 包装层 JSON 解析与透传行为。

use metadata_checker::browser_wasm_bindgen::{
    js_analyze_superpage_selection, js_build_or_update_superpage_graph, js_init_runtime,
    js_load_superpage_document, js_runtime_status,
};
use metadata_checker::remote_metadata_provider::wasm_bindings::{
    js_fetch_remote_file_content, js_fetch_remote_file_info, js_load_remote_superpage_document,
};
use serde::Deserialize;
use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

#[derive(Debug, Deserialize)]
struct JsDiagnostic {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct JsEnvelope {
    status: String,
    diagnostics: Vec<JsDiagnostic>,
}

static WASM_TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());
static WASM_TEST_SOURCE_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn parse_js_envelope(output: &str) -> JsEnvelope {
    serde_json::from_str(output).expect("wrapper output should be valid JSON")
}

fn has_diagnostic_code<'a>(envelope: &'a JsEnvelope, code: &str) -> bool {
    envelope.diagnostics.iter().any(|diag| diag.code == code)
}

fn with_runtime<T>(f: impl FnOnce(&str) -> T) -> T {
    let _guard = WASM_TEST_RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = js_init_runtime("{}");
    let source_path = format!(
        "app/wasm-wasm_bindgen-test-{}.spg",
        WASM_TEST_SOURCE_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    f(&source_path)
}

fn with_runtime_lock() -> std::sync::MutexGuard<'static, ()> {
    WASM_TEST_RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run_remote_wrapper<F>(future: F) -> String
where
    F: Future<Output = String>,
{
    let mut future = pin!(future);
    struct TestWaker {
        thread: std::thread::Thread,
    }
    impl Wake for TestWaker {
        fn wake(self: std::sync::Arc<Self>) {
            self.thread.unpark();
        }
        fn wake_by_ref(self: &std::sync::Arc<Self>) {
            self.thread.unpark();
        }
    }

    let waker = Waker::from(std::sync::Arc::new(TestWaker {
        thread: std::thread::current(),
    }));
    let mut cx = Context::from_waker(&waker);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);

    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => {
                let now = std::time::Instant::now();
                if now >= deadline {
                    panic!("remote wrapper did not resolve in 1s");
                }
                std::thread::park_timeout(deadline - now);
            }
        }
    }
}

fn valid_file_ref_json() -> &'static str {
    r#"{"project_ref":"analyzer","source_path":"app/demo.spg","file_id":"test","remote_ref":"remote"}"#
}

#[test]
fn test_js_init_runtime_invalid_options_json() {
    let output = js_init_runtime("not json");
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_OPTIONS");
}

#[test]
fn test_js_fetch_remote_file_info_invalid_file_ref_json() {
    let output = run_remote_wrapper(js_fetch_remote_file_info(
        "".to_string(),
        "not json".to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_FILE_REF");
}

#[test]
fn test_js_fetch_remote_file_content_invalid_file_ref_json() {
    let output = run_remote_wrapper(js_fetch_remote_file_content(
        "".to_string(),
        "not json".to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_FILE_REF");
}

#[test]
fn test_js_load_remote_superpage_document_invalid_file_ref_json() {
    let output = run_remote_wrapper(js_load_remote_superpage_document(
        "".to_string(),
        "not json".to_string(),
        r#"{}"#.to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_FILE_REF");
}

#[test]
fn test_js_load_remote_superpage_document_invalid_options_json() {
    let output = run_remote_wrapper(js_load_remote_superpage_document(
        "http://example.invalid".to_string(),
        valid_file_ref_json().to_string(),
        "not json".to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_OPTIONS");
}

#[test]
fn test_js_fetch_remote_file_info_empty_base_url_returns_request_error_envelope() {
    let output = run_remote_wrapper(js_fetch_remote_file_info(
        "".to_string(),
        valid_file_ref_json().to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "REMOTE_FETCH_FAILED");
}

#[test]
fn test_js_fetch_remote_file_content_empty_base_url_returns_request_error_envelope() {
    let output = run_remote_wrapper(js_fetch_remote_file_content(
        "".to_string(),
        valid_file_ref_json().to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "REMOTE_FETCH_FAILED");
}

#[test]
fn test_js_load_remote_superpage_document_empty_base_url_returns_request_error_envelope() {
    let output = run_remote_wrapper(js_load_remote_superpage_document(
        "".to_string(),
        valid_file_ref_json().to_string(),
        r#"{}"#.to_string(),
    ));
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "REMOTE_FETCH_FAILED");
}

#[test]
fn test_js_analyze_superpage_selection_invalid_selection_json() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");
    // Keep 与 runtime 解析入口一致：options 需要 include_* 全量字段。
    let output = js_analyze_superpage_selection(
        "not json",
        r#"{"include_priority":false,"include_dataflow":false,"include_conditions":false}"#,
    );
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_SELECTION");
}

#[test]
fn test_js_analyze_superpage_selection_invalid_options_json() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");
    let selection = r#"{"source_path":"app/test.spg","file_id":"test","selected_component_ids":["btn1"],"active_component_id":"btn1"}"#;
    let output = js_analyze_superpage_selection(selection, "not json");
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(envelope.diagnostics[0].code, "INVALID_OPTIONS");
}

#[test]
fn test_js_exports_return_parseable_envelopes() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#;

        let init = js_init_runtime("{}");
        let init_envelope = parse_js_envelope(&init);
        assert_eq!(init_envelope.status, "ready");

        let status = js_runtime_status();
        let status_envelope = parse_js_envelope(&status);
        assert_eq!(status_envelope.status, "ready");

        let load = js_load_superpage_document(source_path, raw);
        let load_envelope = parse_js_envelope(&load);
        assert_eq!(load_envelope.status, "ready");

        let build = js_build_or_update_superpage_graph(source_path);
        let build_envelope = parse_js_envelope(&build);
        assert_eq!(build_envelope.status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["btn1"],"active_component_id":"btn1"}}"#,
            source_path
        );
        let options =
            r#"{"include_priority":false,"include_conditions":false,"include_dataflow":false}"#;
        let analyze = js_analyze_superpage_selection(&selection, options);
        let analyze_envelope = parse_js_envelope(&analyze);
        assert_eq!(analyze_envelope.status, "ready");
    });
}

#[test]
fn test_js_load_superpage_document_invalid_json_returns_parse_error() {
    with_runtime(|source_path| {
        let output = js_load_superpage_document(source_path, "not json");
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "error");
        assert_eq!(envelope.diagnostics[0].code, "PARSE_ERROR");
    });
}

#[test]
fn test_js_build_or_update_superpage_graph_nonexistent_source_path() {
    with_runtime(|_source_path| {
        let output = js_build_or_update_superpage_graph("app/missing.spg");
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "error");
        assert_eq!(envelope.diagnostics[0].code, "DOCUMENT_NOT_FOUND");
    });
}

#[test]
fn test_js_analyze_superpage_selection_empty_selection() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#;
        let load = js_load_superpage_document(source_path, raw);
        let load_envelope = parse_js_envelope(&load);
        assert_eq!(load_envelope.status, "ready", "{load_envelope:#?}");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":[],"active_component_id":null}}"#,
            source_path
        );
        let output = js_analyze_superpage_selection(
            &selection,
            r#"{"include_priority":false,"include_dataflow":false,"include_conditions":false}"#,
        );
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "partial", "{envelope:#?}");
        assert_eq!(envelope.diagnostics[0].code, "EMPTY_SELECTION");
    });
}

#[test]
fn test_js_analyze_superpage_selection_unsupported_options_are_parseable_and_distinguishable() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["btn1"],"active_component_id":"btn1"}}"#,
            source_path
        );
        let options =
            r#"{"include_priority":true,"include_dataflow":true,"include_conditions":false}"#;
        let output = js_analyze_superpage_selection(&selection, options);
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "ready");

        assert!(has_diagnostic_code(&envelope, "UNSUPPORTED_OPTION"));
        let unsupported_messages: Vec<&str> = envelope
            .diagnostics
            .iter()
            .filter(|d| d.code == "UNSUPPORTED_OPTION")
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            unsupported_messages
                .iter()
                .any(|m| m.contains("include_priority")),
            "should distinguish include_priority option"
        );
        assert!(
            unsupported_messages
                .iter()
                .any(|m| m.contains("include_dataflow")),
            "should distinguish include_dataflow option"
        );
    });
}

#[test]
fn test_js_analyze_superpage_selection_unknown_component_with_priority_option_reports_both_codes() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["missing"],"active_component_id":"missing"}}"#,
            source_path
        );
        let options =
            r#"{"include_priority":true,"include_dataflow":false,"include_conditions":false}"#;
        let output = js_analyze_superpage_selection(&selection, options);
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "error");
        assert!(has_diagnostic_code(&envelope, "UNSUPPORTED_OPTION"));
        assert!(has_diagnostic_code(&envelope, "COMPONENT_NOT_FOUND"));
    });
}
