//! Browser runtime API 测试
//!
//! M40.2：验证 init_runtime / runtime_status / load_superpage_document /
//! build_or_update_superpage_graph / analyze_superpage_selection。

use metadata_checker::browser::{
    AnalysisOptions, AnalysisStatus, RuntimeOptions, SuperPageSelection,
    analyze_superpage_selection, build_or_update_superpage_graph, init_runtime,
    load_superpage_document, runtime_status,
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
    let _guard = TEST_RUNTIME_LOCK.lock().unwrap();
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
    let detail = item.detail.as_object().expect("runtime status detail should be object");
    let document_count = detail
        .get("document_count")
        .and_then(|value| value.as_u64())
        .expect("document_count should be number")
        as usize;
    let graph_count = detail
        .get("graph_count")
        .and_then(|value| value.as_u64())
        .expect("graph_count should be number")
        as usize;
    (document_count, graph_count)
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

        assert_eq!(load_superpage_document(source_path, raw).status, AnalysisStatus::Ready);
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

        assert_eq!(load_superpage_document(source_path, raw).status, AnalysisStatus::Ready);
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
        assert!(result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "UNSUPPORTED_OPTION"));
        assert!(result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "COMPONENT_NOT_FOUND"));
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
        assert!(result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "UNSUPPORTED_OPTION"));
        assert!(result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "DOCUMENT_NOT_FOUND"));
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
