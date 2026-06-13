//! Browser runtime API 测试
//!
//! M40.2：验证 init_runtime / runtime_status / load_superpage_document /
//! build_or_update_superpage_graph / analyze_superpage_selection。

use metadata_checker::browser::{
    AnalysisOptions, AnalysisStatus, RuntimeOptions, SuperPageSelection,
    analyze_superpage_selection, build_or_update_superpage_graph,
    enqueue_orchestrator_background_tasks, enqueue_orchestrator_foreground_selection, init_runtime,
    load_superpage_document, orchestrator_status, runtime_status, tick_orchestrator,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

static TEST_RUNTIME_LOCK: Mutex<()> = Mutex::new(());
static SOURCE_PATH_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn next_source_path(prefix: &str) -> String {
    let id = SOURCE_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("app/{prefix}-{id}.spg")
}

fn with_runtime<T>(f: impl FnOnce(&str) -> T) -> T {
    let _guard = TEST_RUNTIME_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    init_runtime(RuntimeOptions::default());
    let source_path = next_source_path("test");
    f(&source_path)
}

fn get_runtime_counts_from_status(
    status: &metadata_checker::browser::BrowserAnalysisEnvelope,
) -> (usize, usize) {
    let item = status
        .items
        .iter()
        .find(|item| item.kind == "runtime_status")
        .expect("runtime status item should exist");
    let detail = item
        .detail
        .as_object()
        .expect("runtime status detail should be object");
    let document_count = detail
        .get("document_count")
        .and_then(|value| value.as_u64())
        .expect("document_count should be number") as usize;
    let graph_count = detail
        .get("graph_count")
        .and_then(|value| value.as_u64())
        .expect("graph_count should be number") as usize;
    (document_count, graph_count)
}

fn get_orchestrator_progress(status: &metadata_checker::browser::BrowserAnalysisEnvelope) -> usize {
    let item = status
        .items
        .iter()
        .find(|item| item.kind == "orchestrator_progress")
        .expect("orchestrator progress item should exist");
    let progress = if item.detail.get("queued").is_some() {
        item.detail.clone()
    } else {
        item.detail
            .get("progress")
            .cloned()
            .expect("orchestrator progress detail should include progress")
    };
    progress
        .get("queued")
        .and_then(|value| value.as_u64())
        .expect("queued should be number") as usize;
    progress
        .get("total")
        .and_then(|value| value.as_u64())
        .expect("total should be number") as usize
}

fn find_first_item_detail<'a>(
    status: &'a metadata_checker::browser::BrowserAnalysisEnvelope,
    kind: &str,
) -> Option<&'a serde_json::Value> {
    status
        .items
        .iter()
        .find(|item| item.kind == kind)
        .map(|item| &item.detail)
}

fn visual_graph_item(
    status: &metadata_checker::browser::BrowserAnalysisEnvelope,
) -> serde_json::Value {
    find_first_item_detail(status, "visual_graph")
        .cloned()
        .expect("should include visual_graph item")
}

fn visual_graph_contains_diagnostic(status: &serde_json::Value, code: &str) -> bool {
    status
        .get("diagnostics")
        .and_then(|value| value.as_array())
        .is_some_and(|diagnostics| {
            diagnostics
                .iter()
                .any(|diag| diag.get("code").and_then(|c| c.as_str()) == Some(code))
        })
}

fn assert_task_descriptor_has_source(detail: &serde_json::Value, expected_source: &str) {
    let source_path = detail
        .get("source_path")
        .and_then(|value| value.as_str())
        .expect("task descriptor should contain source_path");
    assert_eq!(source_path, expected_source);
}

#[test]
fn test_init_runtime_returns_ready() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    let result = init_runtime(RuntimeOptions::default());
    assert_eq!(result.status, AnalysisStatus::Ready);
}

#[test]
fn test_runtime_status_after_init() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());
    let status = runtime_status();
    assert_eq!(status.status, AnalysisStatus::Ready);
}

#[test]
fn test_orchestrator_status_after_init_is_ready() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());
    let status = orchestrator_status(10);
    assert_eq!(status.status, AnalysisStatus::Ready);
    assert!(
        status
            .items
            .iter()
            .any(|item| item.kind == "orchestrator_progress"),
        "orchestrator_status should include orchestrator_progress"
    );
}

#[test]
fn test_orchestrator_foreground_enqueue_and_tick_completes_request() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());

    let source_path = next_source_path("orchestrator-foreground");
    load_superpage_document(
        &source_path,
        r#"{"version":"1.0","canvas":{"components":[]}}"#,
    );

    let selection_json = serde_json::json!({
        "source_path": source_path,
        "file_id": "test-file",
        "selected_component_ids": ["btn1"],
        "active_component_id": "btn1"
    })
    .to_string();
    let options_json = serde_json::json!({
        "generation": 1,
        "processing_ticks": 1,
    })
    .to_string();

    let enqueue_result = enqueue_orchestrator_foreground_selection(&selection_json, &options_json);
    assert_eq!(enqueue_result.status, AnalysisStatus::Ready);

    let request_id = enqueue_result
        .items
        .iter()
        .find(|item| item.kind == "foreground_request")
        .and_then(|item| item.detail.get("request_id"))
        .and_then(|v| v.as_u64())
        .expect("request_id should be present");

    let first_tick = tick_orchestrator(1, 4);
    assert_eq!(first_tick.status, AnalysisStatus::Ready);
    let tick_result = tick_orchestrator(3, 4);
    assert_eq!(tick_result.status, AnalysisStatus::Ready);
    assert!(
        tick_result
            .items
            .iter()
            .any(|item| item.kind == "orchestrator_tick"),
        "tick should include orchestrator_tick"
    );
    let has_completed_request = tick_result
        .items
        .iter()
        .filter(|item| item.kind == "foreground_request")
        .any(|item| item.detail.get("request_id") == Some(&serde_json::json!(request_id)));
    assert!(has_completed_request, "request should complete after tick");
}

#[test]
fn test_orchestrator_foreground_selection_rejects_raw_text_field() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());
    let selection_json =
        r#"{"source_path":"app/test.spg","raw_text":"{\"a\":1}","selected_component_ids":[]}"#;
    let options_json = "{}";
    let result = enqueue_orchestrator_foreground_selection(selection_json, options_json);
    assert_eq!(result.status, AnalysisStatus::Error);
    assert_eq!(result.diagnostics[0].code, "INVALID_ORCHESTRATOR_SELECTION");
}

#[test]
fn test_orchestrator_background_tasks_status_tracks_progress() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());
    let tasks_json = serde_json::json!([
        {"source_path":"app/task-a.spg","file_id":"f1","processing_ticks":1},
        {"source_path":"app/task-b.spg","file_id":"f2","processing_ticks":1}
    ])
    .to_string();
    let enqueue_result = enqueue_orchestrator_background_tasks(&tasks_json);
    assert_eq!(enqueue_result.status, AnalysisStatus::Ready);
    let progress_total = get_orchestrator_progress(&enqueue_result);
    assert!(
        progress_total >= 2,
        "total should reflect queued background tasks"
    );
}

#[test]
fn test_enqueue_background_and_tick_exposes_next_task_source_path() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());

    let tasks_json = serde_json::json!([
        {"source_path":"app/js-bg-a.spg","file_id":"f1","processing_ticks":1},
        {"source_path":"app/js-bg-b.spg","file_id":"f2","processing_ticks":1}
    ])
    .to_string();

    let enqueue_result = enqueue_orchestrator_background_tasks(&tasks_json);
    assert_eq!(enqueue_result.status, AnalysisStatus::Ready);
    let enqueue_detail = find_first_item_detail(&enqueue_result, "orchestrator_progress")
        .expect("background enqueue should include orchestrator_progress");
    assert!(
        enqueue_detail.get("queued_task_descriptors").is_some(),
        "background enqueue should include queued_task_descriptors"
    );
    let queued = enqueue_detail
        .get("queued_task_descriptors")
        .and_then(|value| value.as_array())
        .expect("queued_task_descriptors should be an array");
    assert_eq!(queued.len(), 2);

    let tick_result = tick_orchestrator(1, 4);
    assert_eq!(tick_result.status, AnalysisStatus::Ready);
    let started_task = tick_result
        .items
        .iter()
        .find(|item| item.kind == "orchestrator_started_task")
        .expect("tick should emit orchestrator_started_task");
    let source = started_task
        .detail
        .get("source_path")
        .and_then(|value| value.as_str())
        .expect("started task descriptor should include source_path");
    assert!(
        source == "app/js-bg-a.spg" || source == "app/js-bg-b.spg",
        "started task source_path should match queued background task"
    );
}

#[test]
fn test_tick_for_foreground_enqueued_request_includes_selected_components() {
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
    init_runtime(RuntimeOptions::default());

    let source_path = next_source_path("orchestrator-fg-components");
    load_superpage_document(
        &source_path,
        r#"{"version":"1.0","canvas":{"components":[{"id":"btn1","type":"button","title":"Submit"}]}}"#,
    );

    let selection_json = serde_json::json!({
        "source_path": source_path,
        "file_id": "fg-file",
        "selected_component_ids": ["btn1", "btn2"],
        "active_component_id": "btn1"
    })
    .to_string();
    let options_json = r#"{"generation":1,"processing_ticks":1}"#.to_string();
    let enqueue_result = enqueue_orchestrator_foreground_selection(&selection_json, &options_json);
    assert_eq!(enqueue_result.status, AnalysisStatus::Ready);

    let tick_result = tick_orchestrator(1, 4);
    assert_eq!(tick_result.status, AnalysisStatus::Ready);
    let started_task = tick_result
        .items
        .iter()
        .find(|item| item.kind == "orchestrator_started_task")
        .expect("tick should emit orchestrator_started_task");
    assert_task_descriptor_has_source(&started_task.detail, &source_path);

    let selected_component_ids = started_task
        .detail
        .get("selected_component_ids")
        .and_then(|value| value.as_array())
        .expect("started foreground task should include selected_component_ids");
    let selected_ids: Vec<String> = selected_component_ids
        .iter()
        .map(|value| {
            value
                .as_str()
                .expect("component id should be string")
                .to_string()
        })
        .collect();
    assert_eq!(selected_ids, vec!["btn1".to_string(), "btn2".to_string()]);

    let active_component_id = started_task
        .detail
        .get("active_component_id")
        .and_then(|value| value.as_str())
        .expect("started foreground task should include active_component_id");
    assert_eq!(active_component_id, "btn1");
}

#[test]
fn test_init_runtime_reinitialize_clears_previous_documents_and_graphs() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;

        assert_eq!(
            load_superpage_document(source_path, raw).status,
            AnalysisStatus::Ready
        );
        assert_eq!(
            build_or_update_superpage_graph(source_path).status,
            AnalysisStatus::Ready
        );

        let status_before = runtime_status();
        assert_eq!(get_runtime_counts_from_status(&status_before), (1, 1));

        let reinit = init_runtime(RuntimeOptions::default());
        assert_eq!(reinit.status, AnalysisStatus::Ready);

        let status_after = runtime_status();
        assert_eq!(status_after.status, AnalysisStatus::Ready);
        assert_eq!(get_runtime_counts_from_status(&status_after), (0, 0));
    });
}

#[test]
fn test_runtime_status_reflects_loaded_documents_and_graphs() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;

        assert_eq!(
            load_superpage_document(source_path, raw).status,
            AnalysisStatus::Ready
        );
        assert_eq!(
            build_or_update_superpage_graph(source_path).status,
            AnalysisStatus::Ready
        );

        let status = runtime_status();
        assert_eq!(get_runtime_counts_from_status(&status), (1, 1));
    });
}

#[test]
fn test_load_superpage_document_invalid_json() {
    with_runtime(|source_path| {
        let result = load_superpage_document(source_path, "not json");
        assert_eq!(result.status, AnalysisStatus::Error);
        assert_eq!(result.diagnostics[0].code, "PARSE_ERROR");
    });
}

#[test]
fn test_load_superpage_document_valid_spg() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "theme": "default",
            "canvas": {
                "components": [
                    {
                        "id": "btn1",
                        "type": "button",
                        "title": "Submit"
                    }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;
        let result = load_superpage_document(source_path, raw);
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert_eq!(result.diagnostics.len(), 0);
    });
}

#[test]
fn test_build_graph_after_load() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;
        let load_result = load_superpage_document(source_path, raw);
        assert_eq!(load_result.status, AnalysisStatus::Ready);

        let build_result = build_or_update_superpage_graph(source_path);
        assert_eq!(build_result.status, AnalysisStatus::Ready);
        assert_eq!(build_result.diagnostics.len(), 0);
    });
}

#[test]
fn test_analyze_empty_selection() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec![],
            active_component_id: None,
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Partial);
        assert_eq!(result.diagnostics[0].code, "EMPTY_SELECTION");
    });
}

#[test]
fn test_analyze_component_with_dependencies() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "input1",
                        "type": "input",
                        "title": "Name"
                    },
                    {
                        "id": "label1",
                        "type": "label",
                        "title": "Display",
                        "value": "Hello ${input1.value}"
                    }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert!(
            result.items.iter().any(|i| i.kind == "component"),
            "should have component item"
        );
        let visual_graph = result
            .items
            .iter()
            .find(|item| item.kind == "visual_graph")
            .expect("browser analyze output must include visual_graph item");
        assert_eq!(visual_graph.detail["focus_node"].is_string(), true);
        assert_eq!(
            visual_graph.detail["nodes"].as_array().unwrap().len() >= 2,
            true
        );
        assert_eq!(
            visual_graph.detail["edges"].as_array().unwrap().len() >= 1,
            true
        );
    });
}

#[test]
fn test_analyze_selection_uses_active_component_id_first() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "input1", "type": "input", "title": "Name" },
                    { "id": "label1", "type": "label", "title": "Display" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["input1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert_eq!(result.target, Some("label1".to_string()));

        let component_item = result
            .items
            .iter()
            .find(|item| item.kind == "component")
            .expect("should include component item");
        assert_eq!(component_item.detail["id"], "label1");
    });
}

#[test]
fn test_analyze_selection_falls_back_to_first_selected_when_active_is_empty() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "input1", "type": "input", "title": "Name" },
                    { "id": "label1", "type": "label", "title": "Display" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: None,
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert_eq!(result.target, Some("label1".to_string()));
    });
}

#[test]
fn test_analyze_selection_include_conditions_outputs_condition_items() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "input1", "type": "input", "title": "Name" },
                    {
                        "id": "label1",
                        "type": "label",
                        "title": "Display",
                        "visibleCondition": "${input1.value}",
                        "calcCondition": "${input1.value > 0}"
                    }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let result = analyze_superpage_selection(
            selection,
            AnalysisOptions {
                include_conditions: true,
                ..Default::default()
            },
        );
        assert_eq!(result.status, AnalysisStatus::Ready);

        let conditions_item = result
            .items
            .iter()
            .find(|item| item.kind == "conditions")
            .expect("should include conditions item");
        let fields = conditions_item.detail["fields"]
            .as_array()
            .expect("conditions fields should be an array");
        assert!(fields.iter().any(|field| field == "visibleCondition"));
        assert!(fields.iter().any(|field| field == "calcCondition"));
    });
}

#[test]
fn test_analyze_component_reads_from_graph() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "btn1",
                        "type": "button",
                        "title": "Submit",
                        "value": "${m1.name}"
                    }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["btn1".to_string()],
            active_component_id: Some("btn1".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert!(
            result.items.iter().any(|item| item.kind == "reads"),
            "should have reads item because btn1 reads model m1"
        );

        let reads_item = result
            .items
            .iter()
            .find(|item| item.kind == "reads")
            .unwrap();
        let targets = reads_item
            .detail
            .get("targets")
            .unwrap()
            .as_array()
            .unwrap();
        assert!(targets.iter().any(|t| t.as_str().unwrap().contains("m1")));
    });
}

#[test]
fn test_analyze_unknown_component_returns_not_found() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["nonexistent".to_string()],
            active_component_id: Some("nonexistent".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Error);
        assert_eq!(result.diagnostics[0].code, "COMPONENT_NOT_FOUND");
    });
}

#[test]
fn test_analyze_document_not_found() {
    with_runtime(|_source_path| {
        let selection = SuperPageSelection {
            source_path: "app/nonexistent.spg".to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["btn1".to_string()],
            active_component_id: Some("btn1".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Error);
        assert_eq!(result.diagnostics[0].code, "DOCUMENT_NOT_FOUND");
    });
}

#[test]
fn test_analyze_include_priority_warning_kept_with_component_not_found_error() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["nonexistent".to_string()],
            active_component_id: Some("nonexistent".to_string()),
        };
        let result = analyze_superpage_selection(
            selection,
            AnalysisOptions {
                include_priority: true,
                ..Default::default()
            },
        );
        assert_eq!(result.status, AnalysisStatus::Error);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "UNSUPPORTED_OPTION")
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "COMPONENT_NOT_FOUND")
        );
    });
}

#[test]
fn test_analyze_include_dataflow_warning_kept_with_document_not_found_error() {
    with_runtime(|_source_path| {
        let selection = SuperPageSelection {
            source_path: "app/nonexistent-for-runtime.spg".to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["btn1".to_string()],
            active_component_id: Some("btn1".to_string()),
        };
        let result = analyze_superpage_selection(
            selection,
            AnalysisOptions {
                include_dataflow: true,
                ..Default::default()
            },
        );
        assert_eq!(result.status, AnalysisStatus::Error);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "UNSUPPORTED_OPTION")
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "DOCUMENT_NOT_FOUND")
        );
    });
}

#[test]
fn test_analyze_include_priority_true_reports_unsupported_option() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["btn1".to_string()],
            active_component_id: Some("btn1".to_string()),
        };
        let options = AnalysisOptions {
            include_priority: true,
            ..Default::default()
        };
        let result = analyze_superpage_selection(selection, options);
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "UNSUPPORTED_OPTION"),
            "include_priority=true must report UNSUPPORTED_OPTION"
        );
    });
}

#[test]
fn test_analyze_include_dataflow_true_reports_unsupported_option() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" }
            ]
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["btn1".to_string()],
            active_component_id: Some("btn1".to_string()),
        };
        let options = AnalysisOptions {
            include_dataflow: true,
            ..Default::default()
        };
        let result = analyze_superpage_selection(selection, options);
        assert_eq!(result.status, AnalysisStatus::Ready);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "UNSUPPORTED_OPTION"),
            "include_dataflow=true must report UNSUPPORTED_OPTION"
        );
    });
}

#[test]
fn test_analyze_returns_2hop_local_graph_with_fixed_depth_and_visible_hop() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "input1",
                        "type": "input",
                        "title": "Name"
                    },
                    {
                        "id": "label1",
                        "type": "label",
                        "title": "Display",
                        "visibleCondition": "${input1.value}"
                    },
                    {
                        "id": "label2",
                        "type": "label",
                        "title": "Echo",
                        "value": "${label1.value}"
                    }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);

        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let options = AnalysisOptions {
            depth: Some(1),
            visible_hop: Some(3),
            max_nodes: Some(10),
            max_edges: Some(20),
            ..Default::default()
        };
        let result = analyze_superpage_selection(selection, options);
        assert_eq!(result.status, AnalysisStatus::Ready);
        let graph = visual_graph_item(&result);
        assert_eq!(graph["status"], "ready");
        assert_eq!(graph["depth"], 2);
        assert_eq!(graph["visible_hop"], 1);
        assert_eq!(graph["target"], "label1");
        assert!(graph["nodes"].as_array().unwrap().len() >= 3);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "UNSUPPORTED_OPTION")
        );
    });
}

#[test]
fn test_analyze_with_no_relations_returns_empty_local_graph_status() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "solo",
                        "type": "button",
                        "title": "Only Me"
                    }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["solo".to_string()],
            active_component_id: Some("solo".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        let graph = visual_graph_item(&result);
        assert_eq!(graph["status"], "empty");
        assert_eq!(graph["edges"].as_array().unwrap().len(), 0);
        assert_eq!(graph["focus_node"].as_str().is_some(), true);
    });
}

#[test]
fn test_analyze_empty_selection_returns_idle_visual_graph_and_empty_selection_diagnostic() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "btn1", "type": "button", "title": "Submit" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec![],
            active_component_id: None,
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Partial);
        assert_eq!(result.diagnostics[0].code, "EMPTY_SELECTION");
        let graph = visual_graph_item(&result);
        assert_eq!(graph["status"], "idle");
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 0);
    });
}

#[test]
fn test_analyze_local_graph_truncation_removes_dangling_edges_and_keeps_diagnostics() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "input1",
                        "type": "input"
                    },
                    {
                        "id": "label1",
                        "type": "label",
                        "value": "${input1.value}"
                    }
                ],
                "sources": [
                    { "id": "m_orders", "modelType": "App", "path": "app/models.tbl" }
                ]
            }
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let options = AnalysisOptions {
            max_nodes: Some(2),
            max_edges: Some(2),
            ..Default::default()
        };
        let result = analyze_superpage_selection(selection, options);
        assert_eq!(result.status, AnalysisStatus::Ready);
        let graph = visual_graph_item(&result);
        assert_eq!(graph["status"], "warning");
        assert_eq!(graph["truncated"], true);
        let edges = graph["edges"].as_array().unwrap();
        let valid_node_ids: std::collections::HashSet<&str> = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|node| node.get("id").and_then(|value| value.as_str()))
            .collect();
        assert!(edges.iter().all(|edge| {
            valid_node_ids.contains(edge["from"].as_str().unwrap())
                && valid_node_ids.contains(edge["to"].as_str().unwrap())
        }));
        assert!(graph["truncated_reason"].is_string());
        assert_eq!(
            graph["source_summary"]["total_nodes"],
            serde_json::json!(graph["nodes"].as_array().unwrap().len())
        );
        assert_eq!(
            graph["source_summary"]["total_edges"],
            serde_json::json!(graph["edges"].as_array().unwrap().len())
        );
    });
}

#[test]
fn test_analyze_local_graph_omits_model_edges_without_expression_refs() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    { "id": "text49", "type": "text", "title": "Static Label" }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" },
                { "id": "m2", "modelType": "App", "path": "app/m2.tbl" }
            ]
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["text49".to_string()],
            active_component_id: Some("text49".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        let graph = visual_graph_item(&result);
        assert_eq!(graph["status"], "empty");
        let edges = graph["edges"].as_array().unwrap();
        assert!(
            edges.is_empty(),
            "static components should not inherit page-wide model reads"
        );
        assert!(
            !result.items.iter().any(|item| item.kind == "reads"),
            "no reads item when component has no model expression refs"
        );
    });
}

#[test]
fn test_analyze_local_graph_links_model_only_when_expression_refs_model_field() {
    with_runtime(|source_path| {
        let raw = r#"{
            "version": "1.0",
            "canvas": {
                "components": [
                    {
                        "id": "label1",
                        "type": "label",
                        "title": "Display",
                        "value": "${m1.name}"
                    }
                ]
            },
            "sources": [
                { "id": "m1", "modelType": "App", "path": "app/m1.tbl" },
                { "id": "m2", "modelType": "App", "path": "app/m2.tbl" }
            ]
        }"#;
        load_superpage_document(source_path, raw);
        build_or_update_superpage_graph(source_path);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "test".to_string(),
            selected_component_ids: vec!["label1".to_string()],
            active_component_id: Some("label1".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        assert_eq!(result.status, AnalysisStatus::Ready);
        let graph = visual_graph_item(&result);
        let edges = graph["edges"].as_array().unwrap();
        let model_edges: Vec<_> = edges
            .iter()
            .filter(|edge| edge["label"] == "reads")
            .collect();
        assert_eq!(model_edges.len(), 1);
        assert!(model_edges[0]["to"].as_str().unwrap().contains("m1"));
        assert!(
            !model_edges[0]["to"].as_str().unwrap().contains("m2"),
            "should not fan out to unrelated page sources"
        );
        assert_eq!(model_edges[0]["evidence_status"], "available");
    });
}

#[test]
#[ignore = "local real-project diagnostic"]
fn diagnose_real_page_text49_model_fanout() {
    let path = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi/app/价审.app/demo/销售订单价格审批.spg";
    let raw = std::fs::read_to_string(path).expect("read spg");
    with_runtime(|_| {
        let source_path = "app/价审.app/demo/销售订单价格审批.spg";
        load_superpage_document(source_path, &raw);
        build_or_update_superpage_graph(source_path);
        let selection = SuperPageSelection {
            source_path: source_path.to_string(),
            file_id: "demo".to_string(),
            selected_component_ids: vec!["text49".to_string()],
            active_component_id: Some("text49".to_string()),
        };
        let result = analyze_superpage_selection(selection, AnalysisOptions::default());
        let graph = visual_graph_item(&result);
        let edges = graph["edges"].as_array().unwrap();
        let model_edges: Vec<_> = edges
            .iter()
            .filter(|edge| edge["label"] == "reads")
            .collect();
        eprintln!("status={}", graph["status"]);
        eprintln!("nodes={}", graph["nodes"].as_array().unwrap().len());
        eprintln!("edges={}", edges.len());
        eprintln!("model_reads={}", model_edges.len());
        for edge in model_edges.iter().take(5) {
            eprintln!("  {} -> {}", edge["from"], edge["to"]);
        }
        assert!(
            model_edges.is_empty(),
            "text49 should not fan out to page models"
        );
    });
}
