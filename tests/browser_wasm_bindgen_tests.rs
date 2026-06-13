#![cfg(feature = "browser-wasm")]

//! Browser WASM JSON wrapper 测试
//! 仅在 browser-wasm feature 下编译，验证 JS callable 包装层 JSON 解析与透传行为。

use metadata_checker::browser_wasm_bindgen::{
    js_analysis_orchestrator_status, js_analyze_local_graph, js_analyze_superpage_selection,
    js_build_or_update_superpage_graph, js_diff_visible_manifest, js_enqueue_background_tasks,
    js_enqueue_foreground_selection, js_init_runtime, js_load_superpage_document,
    js_runtime_status, js_tick_analysis_orchestrator,
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
struct JsEnvelopeItem {
    kind: String,
    detail: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct JsEnvelope {
    status: String,
    #[serde(default)]
    items: Vec<JsEnvelopeItem>,
    diagnostics: Vec<JsDiagnostic>,
}

#[derive(Debug, Deserialize)]
struct JsVisibleManifestDiff {
    added: Vec<serde_json::Value>,
    modified: Vec<serde_json::Value>,
    deleted: Vec<serde_json::Value>,
    unchanged: Vec<serde_json::Value>,
    changed_files: Vec<serde_json::Value>,
    content_queue: Vec<serde_json::Value>,
    timing: serde_json::Value,
    diagnostics: Vec<serde_json::Value>,
}

static WASM_TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());
static WASM_TEST_SOURCE_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn parse_js_envelope(output: &str) -> JsEnvelope {
    serde_json::from_str(output).expect("wrapper output should be valid JSON")
}

fn has_diagnostic_code<'a>(envelope: &'a JsEnvelope, code: &str) -> bool {
    envelope.diagnostics.iter().any(|diag| diag.code == code)
}

fn has_item_kind(envelope: &JsEnvelope, kind: &str) -> bool {
    envelope
        .items
        .iter()
        .any(|item| item.kind == kind && item.detail.is_object())
}

fn find_item_detail<'a>(envelope: &'a JsEnvelope, kind: &str) -> Option<&'a serde_json::Value> {
    envelope
        .items
        .iter()
        .find(|item| item.kind == kind)
        .map(|item| &item.detail)
}

fn visual_graph_from_envelope(envelope: &JsEnvelope) -> serde_json::Value {
    find_item_detail(envelope, "visual_graph")
        .cloned()
        .expect("should include visual_graph item")
}

fn visual_graph_has_diagnostic(envelope: &JsEnvelope, code: &str) -> bool {
    envelope
        .items
        .iter()
        .find(|item| item.kind == "visual_graph")
        .and_then(|visual_graph| visual_graph.detail.get("diagnostics"))
        .and_then(|value| value.as_array())
        .is_some_and(|diagnostics| {
            diagnostics
                .iter()
                .any(|diag| diag.get("code").and_then(|v| v.as_str()) == Some(code))
        })
}

fn find_visible_manifest_diff(envelope: &JsEnvelope) -> JsVisibleManifestDiff {
    let diff_detail = find_item_detail(envelope, "visible_manifest_diff")
        .cloned()
        .expect("should include visible_manifest_diff item");
    serde_json::from_value(diff_detail).expect("visible_manifest_diff detail should be valid")
}

fn manifest_source_paths(entries: &[serde_json::Value]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|entry| {
            entry
                .get("source_path")
                .and_then(|value| value.as_str())
                .map(|source_path| source_path.to_string())
        })
        .collect()
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
fn test_js_enqueue_foreground_selection_invalid_selection_json() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");
    let output = js_enqueue_foreground_selection("not json", "{}");
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(
        envelope.diagnostics[0].code,
        "INVALID_ORCHESTRATOR_SELECTION"
    );
}

#[test]
fn test_js_enqueue_foreground_selection_rejects_raw_text_field() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");
    let output = js_enqueue_foreground_selection(
        r#"{"source_path":"app/test.spg","rawText":"<x>","selected_component_ids":[]}"#,
        "{}",
    );
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "error");
    assert_eq!(
        envelope.diagnostics[0].code,
        "INVALID_ORCHESTRATOR_SELECTION"
    );
}

#[test]
fn test_js_enqueue_background_tasks_and_tick_status_cycle() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");

    let tasks = serde_json::json!([
        {
            "source_path": "app/js-task-a.spg",
            "file_id": "a",
            "processing_ticks": 1
        },
        {
            "source_path": "app/js-task-b.spg",
            "file_id": "b",
            "processing_ticks": 1
        }
    ])
    .to_string();
    let enqueue_output = js_enqueue_background_tasks(&tasks);
    let enqueue_envelope = parse_js_envelope(&enqueue_output);
    assert_eq!(enqueue_envelope.status, "ready");
    assert!(has_item_kind(&enqueue_envelope, "orchestrator_progress"));

    let tick_output = js_tick_analysis_orchestrator("2", "4");
    let tick_envelope = parse_js_envelope(&tick_output);
    assert_eq!(tick_envelope.status, "ready");
    assert!(has_item_kind(&tick_envelope, "orchestrator_tick"));

    let status_output = js_analysis_orchestrator_status("4");
    let status_envelope = parse_js_envelope(&status_output);
    assert_eq!(status_envelope.status, "ready");
    assert!(has_item_kind(&status_envelope, "orchestrator_progress"));
}

#[test]
fn test_js_tick_emits_started_task_descriptor_for_background_queue() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");

    let tasks = serde_json::json!([
        {
            "source_path": "app/js-wasm-bg-a.spg",
            "file_id": "a",
            "processing_ticks": 1
        },
        {
            "source_path": "app/js-wasm-bg-b.spg",
            "file_id": "b",
            "processing_ticks": 1
        }
    ])
    .to_string();

    let enqueue_output = js_enqueue_background_tasks(&tasks);
    let enqueue_envelope = parse_js_envelope(&enqueue_output);
    assert!(has_item_kind(&enqueue_envelope, "orchestrator_progress"));

    let tick_output = js_tick_analysis_orchestrator("1", "4");
    let tick_envelope = parse_js_envelope(&tick_output);
    let started = find_item_detail(&tick_envelope, "orchestrator_started_task")
        .expect("tick should emit orchestrator_started_task");
    let source_path = started
        .get("source_path")
        .and_then(|value| value.as_str())
        .expect("started task should include source_path");
    assert!(source_path == "app/js-wasm-bg-a.spg" || source_path == "app/js-wasm-bg-b.spg");
}

#[test]
fn test_js_tick_emits_started_foreground_task_selection_fields() {
    let _guard = with_runtime_lock();
    let _ = js_init_runtime("{}");

    let selection = r#"{"source_path":"app/wasm-fg.spg","file_id":"f","selected_component_ids":["btn1","btn2"],"active_component_id":"btn1"}"#;
    let options = r#"{"generation":1,"processing_ticks":1}"#;

    let enqueue_output = js_enqueue_foreground_selection(selection, options);
    let enqueue_envelope = parse_js_envelope(&enqueue_output);
    assert_eq!(enqueue_envelope.status, "ready");
    assert!(has_item_kind(&enqueue_envelope, "foreground_request"));

    let tick_output = js_tick_analysis_orchestrator("1", "4");
    let tick_envelope = parse_js_envelope(&tick_output);
    let started = find_item_detail(&tick_envelope, "orchestrator_started_task")
        .expect("tick should emit orchestrator_started_task");

    let selected_component_ids = started
        .get("selected_component_ids")
        .and_then(|value| value.as_array())
        .expect("started foreground task should include selected_component_ids");
    let selected_ids: Vec<String> = selected_component_ids
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("selected component id should be string")
                .to_string()
        })
        .collect();
    assert_eq!(selected_ids, vec!["btn1".to_string(), "btn2".to_string()]);

    let active_component_id = started
        .get("active_component_id")
        .and_then(|value| value.as_str())
        .expect("started foreground task should include active_component_id");
    assert_eq!(active_component_id, "btn1");
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

#[test]
fn test_js_analyze_superpage_selection_empty_options_is_accepted() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"input1","type":"input","title":"Input"}]}}"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["input1"],"active_component_id":"input1"}}"#,
            source_path
        );
        let output = js_analyze_superpage_selection(&selection, "{}");
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "ready");
        let graph = visual_graph_from_envelope(&envelope);
        assert_eq!(graph["status"], "empty");
    });
}

#[test]
fn test_js_analyze_local_graph_alias_exports_fixed_local_graph_contract() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {"id":"input1","type":"input","title":"Input"},
                    {"id":"label1","type":"label","title":"Label","value":"${input1.value}"}
                ]
            }
        }"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["label1"],"active_component_id":"label1"}}"#,
            source_path
        );
        let output = js_analyze_local_graph(&selection, r#"{"depth":2,"visible_hop":1}"#);
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "ready");
        let graph = visual_graph_from_envelope(&envelope);
        assert_eq!(graph["target"], "label1");
        assert_eq!(graph["depth"], 2);
        assert_eq!(graph["visible_hop"], 1);
        assert_eq!(graph["nodes"].as_array().unwrap().is_empty(), false);
    });
}

#[test]
fn test_js_analyze_superpage_selection_applies_fixed_depth_and_visible_hop() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {"id":"input1","type":"input","title":"Input"},
                    {"id":"label1","type":"label","title":"Label","value":"${input1.value}"},
                    {"id":"btn1","type":"button","title":"Submit","value":"${label1.value}"}
                ]
            }
        }"#;
        let load = js_load_superpage_document(source_path, raw);
        let load_envelope = parse_js_envelope(&load);
        assert_eq!(load_envelope.status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["label1"],"active_component_id":"label1"}}"#,
            source_path
        );
        let options = r#"{"depth":3,"visible_hop":4}"#;
        let output = js_analyze_superpage_selection(&selection, options);
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "ready");
        assert!(has_diagnostic_code(&envelope, "UNSUPPORTED_OPTION"));
        let graph = visual_graph_from_envelope(&envelope);
        assert_eq!(graph["depth"], 2);
        assert_eq!(graph["visible_hop"], 1);
        assert_eq!(graph["target"], "label1");
    });
}

#[test]
fn test_js_analyze_superpage_selection_empty_selection_returns_idle_visual_graph() {
    with_runtime(|source_path| {
        let raw = r#"{"version":"1.0","canvas":{"components":[{"id":"label1","type":"label","title":"Label"}]}}"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":[],"active_component_id":null}}"#,
            source_path
        );
        let output = js_analyze_superpage_selection(&selection, "{}");
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "partial");
        let graph = visual_graph_from_envelope(&envelope);
        assert_eq!(graph["status"], "idle");
    });
}

#[test]
fn test_js_analyze_superpage_selection_edge_evidence_unavailable_is_reported_in_visual_graph() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {"id":"label1","type":"label","title":"Label"}
                ]
            },
            "sources":[{"id":"m1","modelType":"App","path":"app/m1.tbl"}]
        }"#;
        let load = js_load_superpage_document(source_path, raw);
        assert_eq!(parse_js_envelope(&load).status, "ready");
        assert_eq!(
            parse_js_envelope(&js_build_or_update_superpage_graph(source_path)).status,
            "ready"
        );

        let selection = format!(
            r#"{{"source_path":"{}","file_id":"test","selected_component_ids":["label1"],"active_component_id":"label1"}}"#,
            source_path
        );
        let output = js_analyze_superpage_selection(&selection, "{}");
        let envelope = parse_js_envelope(&output);
        assert_eq!(envelope.status, "ready");
        let graph = find_item_detail(&envelope, "visual_graph").expect("visual graph detail");
        let edges = graph["edges"].as_array().expect("visual graph edges");
        assert!(
            edges
                .iter()
                .any(|edge| edge["evidence_status"] == "unavailable"),
            "visual graph edges should expose unavailable evidence status"
        );
        assert!(edges.iter().all(|edge| edge["priority"].is_string()));
        assert!(edges.iter().all(|edge| edge["summary"].is_string()));
        assert!(
            visual_graph_has_diagnostic(&envelope, "EDGE_EVIDENCE_UNAVAILABLE"),
            "visual graph should report missing edge evidence"
        );
    });
}

#[test]
fn test_js_diff_visible_manifest_add_modify_delete_unchanged_and_queue() {
    let previous = serde_json::json!([
        {
            "path": "app/a.spg",
            "revision": "1",
            "modifyTime": 1000,
            "modifier": "u1"
        },
        {
            "path": "app/changed.spg",
            "revision": "1",
            "modifyTime": 1000,
            "modifier": "u2"
        },
        {
            "path": "app/unchanged.tbl",
            "revision": "1",
            "modifyTime": 1000
        },
        {
            "path": "app/deleted.spg",
            "revision": "1",
            "modifyTime": 1000
        },
        {
            "path": "app/old-folder.app",
            "isFolder": true
        }
    ])
    .to_string();
    let visible = serde_json::json!({
        "children": [
            {
                "path": "app/a.spg",
                "revision": "1",
                "modifyTime": 1000
            },
            {
                "path": "app/changed.spg",
                "revision": "2",
                "modifyTime": 1200
            },
            {
                "path": "app/unchanged.tbl",
                "revision": "1",
                "modifyTime": 1000
            },
            {
                "path": "app/added.json",
                "revision": "1",
                "modifyTime": 1100
            },
            {
                "path": "app/new-folder.app",
                "isFolder": true
            }
        ]
    })
    .to_string();

    let output = js_diff_visible_manifest("appProj", &previous, &visible);
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "ready");
    let diff = find_visible_manifest_diff(&envelope);
    assert_eq!(
        manifest_source_paths(&diff.unchanged),
        vec!["app/a.spg", "app/unchanged.tbl"]
    );
    assert_eq!(
        manifest_source_paths(&diff.modified),
        vec!["app/changed.spg"]
    );
    assert_eq!(manifest_source_paths(&diff.added), vec!["app/added.json"]);
    assert_eq!(
        manifest_source_paths(&diff.deleted),
        vec!["app/deleted.spg"]
    );
    assert!(!manifest_source_paths(&diff.changed_files).is_empty());
    assert_eq!(
        manifest_source_paths(&diff.changed_files),
        vec!["app/added.json", "app/changed.spg"]
    );
    assert_eq!(
        manifest_source_paths(&diff.content_queue),
        vec!["app/added.json", "app/changed.spg"]
    );
    assert!(
        diff.timing
            .get("total_ms")
            .and_then(|value| value.as_u64())
            .is_some()
    );
}

#[test]
fn test_js_diff_visible_manifest_skip_directory_entries() {
    let previous = serde_json::json!([
        {
            "path": "app/folder",
            "isFolder": true,
            "modifyTime": 1000
        },
        {
            "path": "app/deleted-folder",
            "isFolder": true,
            "modifyTime": 1000
        }
    ])
    .to_string();
    let visible = serde_json::json!({
        "data": {
            "children": [
                {
                    "path": "app/folder",
                    "isFolder": true,
                    "modifyTime": 2000
                },
                {
                    "path": "app/new-dir",
                    "isFolder": true,
                    "modifyTime": 2000
                },
                {
                    "path": "app/keep.spg",
                    "revision": "1",
                    "modifyTime": 1000
                }
            ]
        }
    })
    .to_string();

    let output = js_diff_visible_manifest("appProj", &previous, &visible);
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "ready");
    let diff = find_visible_manifest_diff(&envelope);
    assert_eq!(manifest_source_paths(&diff.added), vec!["app/keep.spg"]);
    assert_eq!(manifest_source_paths(&diff.modified), Vec::<String>::new());
    assert_eq!(manifest_source_paths(&diff.deleted), Vec::<String>::new());
    assert!(manifest_source_paths(&diff.unchanged).is_empty());
    assert_eq!(
        manifest_source_paths(&diff.changed_files),
        vec!["app/keep.spg"]
    );
    assert_eq!(
        manifest_source_paths(&diff.content_queue),
        vec!["app/keep.spg"]
    );
    assert!(
        diff.diagnostics.iter().any(|diag| diag.get("code")
            == Some(&serde_json::Value::String(
                "VISIBLE_MANIFEST_SKIP_DIRECTORY".to_string()
            ))),
        "expected visible manifest skip directory diagnostic"
    );
}

#[test]
fn test_js_diff_visible_manifest_revise_missing_but_modify_time_changes() {
    let previous = serde_json::json!([
        {
            "path": "app/revision-missing.spg",
            "modifyTime": 1000
        }
    ])
    .to_string();
    let visible = serde_json::json!([
        {
            "path": "app/revision-missing.spg",
            "modifyTime": 2000
        }
    ])
    .to_string();

    let output = js_diff_visible_manifest("appProj", &previous, &visible);
    let envelope = parse_js_envelope(&output);
    assert_eq!(envelope.status, "ready");
    let diff = find_visible_manifest_diff(&envelope);
    assert_eq!(
        manifest_source_paths(&diff.modified),
        vec!["app/revision-missing.spg"]
    );
    assert!(
        manifest_source_paths(&diff.changed_files)
            .contains(&"app/revision-missing.spg".to_string())
    );
    assert_eq!(
        manifest_source_paths(&diff.content_queue),
        vec!["app/revision-missing.spg"]
    );
}
