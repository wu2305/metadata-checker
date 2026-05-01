use clap::Parser;
use metadata_checker::output::{AiOutput, OutputKind};
use std::sync::Mutex;

static CLI_LOCK: Mutex<()> = Mutex::new(());
use metadata_checker::parser::parse_file;
use std::path::PathBuf;

/// 验证 AiOutput 契约：顶层字段、kind 枚举、evidence 结构、diagnostics 结构
fn assert_ai_output_contract(output: &AiOutput) {
    assert!(
        !output.schema_version.is_empty(),
        "schema_version must not be empty"
    );
    assert!(
        matches!(
            output.kind,
            OutputKind::SuperPage
                | OutputKind::PageQuery
                | OutputKind::ModelQuery
                | OutputKind::CrossPageQuery
                | OutputKind::DataFlowQuery
                | OutputKind::ComponentQuery
                | OutputKind::PriorityQuery
                | OutputKind::Explain
                | OutputKind::Context
                | OutputKind::PageLogic
        ),
        "kind must be a known variant"
    );
    assert!(
        output.summary.is_object() || output.summary.is_array(),
        "summary must be an object or array"
    );
    // evidence 不能为空，如果 summary 有实质内容
    let has_substantive = !output.summary.is_null()
        && output
            .summary
            .as_object()
            .map(|o| !o.is_empty())
            .unwrap_or(true)
        && output
            .summary
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(true);
    if has_substantive {
        assert!(
            !output.evidence.is_empty(),
            "evidence must not be empty when summary has substantive content"
        );
    }
    // 检查 evidence 结构
    for ev in &output.evidence {
        assert!(!ev.claim.is_empty(), "evidence.claim must not be empty");
        assert!(!ev.reason.is_empty(), "evidence.reason must not be empty");
    }
    // 检查 diagnostics 结构
    for diag in &output.diagnostics {
        assert!(!diag.code.is_empty(), "diagnostic.code must not be empty");
        assert!(
            !diag.message.is_empty(),
            "diagnostic.message must not be empty"
        );
    }
    // next_queries 必须为非空数组（如果 summary 有实质内容）
    if has_substantive {
        assert!(
            !output.next_queries.is_empty(),
            "next_queries must not be empty when summary has substantive content"
        );
    }
}

#[test]
fn test_cli_explain_option_parses() {
    let cli = metadata_checker::cli::Cli::parse_from([
        "metadata-checker",
        "tests/fixtures/test_superpage.spg",
        "--explain",
        "input1",
    ]);
    assert_eq!(cli.explain, Some("input1".to_string()));
}

#[test]
fn test_cli_context_option_parses() {
    let cli = metadata_checker::cli::Cli::parse_from([
        "metadata-checker",
        "--project-dir",
        "/tmp",
        "--context",
        "button1",
        "--depth",
        "2",
        "--budget",
        "compact",
    ]);
    assert_eq!(cli.context, Some("button1".to_string()));
    assert_eq!(cli.depth, 2);
    assert_eq!(cli.budget, "compact");
}

#[test]
fn test_cli_query_page_logic_parses() {
    let cli = metadata_checker::cli::Cli::parse_from([
        "metadata-checker",
        "--project-dir",
        "/tmp",
        "--query-page-logic",
        "page:test.spg",
    ]);
    assert_eq!(cli.query_page_logic, Some("page:test.spg".to_string()));
}

#[test]
fn test_explain_component_does_not_panic() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");

    let _ = metadata_checker::explain::explain_component_spg(&spg, "input1", false);
}

#[test]
fn test_json_schema_has_unified_top_level() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    metadata_checker::output::print_summary_to(&meta, None, &mut buf)
        .expect("print_summary_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let output: AiOutput =
        serde_json::from_str(&s).expect("Summary should deserialize as AiOutput");
    assert_eq!(output.schema_version, "1.0");
    assert_eq!(output.kind, OutputKind::SuperPage);
    assert!(output.summary.is_object());
    assert_ai_output_contract(&output);
}

#[test]
fn test_detail_output_contract() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    metadata_checker::output::print_non_human_to(&meta, None, &mut buf)
        .expect("print_non_human_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let output: AiOutput = serde_json::from_str(&s).expect("Detail should deserialize as AiOutput");
    assert_eq!(output.kind, OutputKind::SuperPage);
    assert!(output.details.is_some());
    assert_ai_output_contract(&output);
}

#[test]
fn test_component_query_contract() {
    use metadata_checker::dependency::DependencyGraph;

    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");
    let graph = DependencyGraph::new(&spg);

    let mut buf: Vec<u8> = Vec::new();
    metadata_checker::output::print_component_query_json_to(
        &spg, &graph, "input3", false, &mut buf,
    )
    .expect("Should print JSON query");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let output: AiOutput =
        serde_json::from_str(&s).expect("Component query should deserialize as AiOutput");
    assert_eq!(output.kind, OutputKind::ComponentQuery);
    assert_eq!(output.query_target, Some("input3".to_string()));
    assert_ai_output_contract(&output);
}

#[test]
fn test_evidence_not_empty_when_summary_has_content() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    metadata_checker::output::print_summary_to(&meta, None, &mut buf)
        .expect("print_summary_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let output: AiOutput =
        serde_json::from_str(&s).expect("Summary should deserialize as AiOutput");
    assert!(
        !output.evidence.is_empty(),
        "evidence must not be empty for a valid SuperPage summary"
    );
}

#[test]
fn test_diagnostics_uniform_structure() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    metadata_checker::output::print_summary_to(&meta, None, &mut buf)
        .expect("print_summary_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let output: AiOutput =
        serde_json::from_str(&s).expect("Summary should deserialize as AiOutput");
    for diag in &output.diagnostics {
        assert!(
            !diag.code.is_empty(),
            "diagnostic code must not be empty: {:?}",
            diag
        );
        assert!(
            !diag.message.is_empty(),
            "diagnostic message must not be empty: {:?}",
            diag
        );
    }
}

// ============================================================
// 项目级 contract 测试
// ============================================================

use metadata_checker::graph::GraphDB;
use metadata_checker::scanner::scan_project;
use std::path::Path;

fn setup_graph_db(suffix: &str) -> (std::path::PathBuf, GraphDB) {
    let db_path =
        std::env::temp_dir().join(format!("metadata-checker-contract-test-{}.db", suffix));
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");
    (db_path, graph)
}

#[test]
fn test_query_model_contract() {
    let (_db_path, graph) = setup_graph_db("model");
    let result = metadata_checker::query::query_model(&graph, "model:model1", false);
    assert!(
        result.is_ok(),
        "query_model should succeed for existing model"
    );
}

#[test]
fn test_query_page_contract() {
    let (_db_path, graph) = setup_graph_db("page");
    let result = metadata_checker::query::query_page(&graph, "page:app/page_relations.spg", false);
    assert!(
        result.is_ok(),
        "query_page should succeed for existing page"
    );
}

#[test]
fn test_query_page_not_found_returns_error() {
    let (_db_path, graph) = setup_graph_db("page_nf");
    let result = metadata_checker::query::query_page(&graph, "page:nonexistent.spg", false);
    assert!(
        result.is_err(),
        "query_page should fail for nonexistent page"
    );
}

#[test]
fn test_query_cross_contract() {
    let (_db_path, graph) = setup_graph_db("cross");
    let result = metadata_checker::query::query_cross(
        &graph,
        "page:app/page_relations.spg",
        "page:app/actions_test.spg",
        false,
    );
    assert!(result.is_ok(), "query_cross should succeed");
}

#[test]
fn test_query_dataflow_contract() {
    let (_db_path, graph) = setup_graph_db("df");
    let result = metadata_checker::query::query_dataflow(&graph, "model:dataflow_output", false);
    assert!(
        result.is_ok(),
        "query_dataflow should succeed for existing dataflow"
    );
}

#[test]
fn test_query_dataflow_not_found_returns_error() {
    let (_db_path, graph) = setup_graph_db("df_nf");
    let result = metadata_checker::query::query_dataflow(&graph, "model:nonexistent", false);
    assert!(
        result.is_err(),
        "query_dataflow should fail for nonexistent dataflow"
    );
}

#[test]
fn test_explain_graph_contract() {
    let (_db_path, graph) = setup_graph_db("explain");
    let result = metadata_checker::explain::explain_node_graph(&graph, "model:model1", false);
    assert!(
        result.is_ok(),
        "explain_node_graph should succeed for existing node"
    );
}

#[test]
fn test_context_graph_contract() {
    let (_db_path, graph) = setup_graph_db("context");
    let result =
        metadata_checker::context::context_node_graph(&graph, "model:model1", 1, "normal", false);
    assert!(
        result.is_ok(),
        "context_node_graph should succeed for existing node"
    );
}

#[test]
fn test_query_page_logic_contract() {
    let (_db_path, graph) = setup_graph_db("logic");
    let result = metadata_checker::query::query_page_logic(
        &graph,
        "page:app/page_relations.spg",
        None,
        false,
    );
    assert!(
        result.is_ok(),
        "query_page_logic should succeed for existing page"
    );
}

/// 通过编译后的二进制 CLI 捕获 JSON 输出
fn run_cli(args: &[&str]) -> String {
    let _guard = CLI_LOCK.lock().unwrap();
    let bin = std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker");
    let cmd_output = std::process::Command::new(&bin)
        .args(args)
        .output()
        .expect("Failed to run metadata-checker binary");
    String::from_utf8(cmd_output.stdout).expect("Invalid UTF-8")
}

fn run_cli_stderr(args: &[&str]) -> String {
    let _guard = CLI_LOCK.lock().unwrap();
    let bin = std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker");
    let cmd_output = std::process::Command::new(&bin)
        .args(args)
        .output()
        .expect("Failed to run metadata-checker binary");
    String::from_utf8(cmd_output.stderr).expect("Invalid UTF-8")
}

#[test]
fn test_cli_query_model_contract() {
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    // build-graph 输出 human text, 但先确保 graph 建立
    assert!(output.contains("Graph database built"));

    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("query_model output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    assert_eq!(ai.query_target, Some("model:model1".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_query_page_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page",
        "page:app/page_relations.spg",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("query_page output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageQuery);
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_query_cross_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-cross",
        "page:app/page_relations.spg",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("query_cross output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::CrossPageQuery);
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_query_dataflow_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-dataflow",
        "dataflow_output",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_dataflow output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::DataFlowQuery);
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_explain_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "model:model1",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("explain output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(ai.query_target, Some("model:model1".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    // Semantic assertions for M2
    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert!(
        summary.contains_key("importance"),
        "summary must have importance"
    );
    assert!(
        summary.get("importance").and_then(|v| v.as_str()).is_some(),
        "importance must be a string"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // Model explain should have read_by_count and written_by_count
    assert!(
        summary.contains_key("read_by_count"),
        "model explain must have read_by_count"
    );
    assert!(
        summary.contains_key("written_by_count"),
        "model explain must have written_by_count"
    );
}

#[test]
fn test_cli_explain_component_single_file_contract() {
    // 单文件模式 explain component with actions
    let output = run_cli(&["tests/fixtures/actions_test.spg", "--explain", "button1"]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("single-file explain output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(ai.query_target, Some("button1".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("entrypoint"),
        "button with actions should be entrypoint"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("component"),
        "type must be component"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // button1 has submitData action → writes should not be empty
    let writes = details_obj
        .get("writes")
        .expect("writes must exist")
        .as_array()
        .expect("writes must be array");
    assert!(
        !writes.is_empty(),
        "button1 with submitData should have writes"
    );

    // should have LINEAGE_DEFERRED_TO_M6 diagnostic
    assert!(
        ai.diagnostics
            .iter()
            .any(|d| d.code == "LINEAGE_DEFERRED_TO_M6"),
        "must have LINEAGE_DEFERRED_TO_M6 diagnostic"
    );
}

#[test]
fn test_cli_explain_component_single_file_no_actions_contract() {
    let output = run_cli(&["tests/fixtures/test_superpage.spg", "--explain", "input1"]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("single-file explain output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(ai.query_target, Some("input1".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("calculated_display"),
        "input without actions should be calculated_display"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    // input1 has expression referencing param1-model1.A → reads should not be empty
    let reads = details_obj
        .get("reads")
        .expect("reads must exist")
        .as_array()
        .expect("reads must be array");
    assert!(
        !reads.is_empty(),
        "input1 with value expression should have reads"
    );
}

#[test]
fn test_cli_explain_action_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "action:app/actions_test.spg|button1|action1",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain action output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(
        ai.query_target,
        Some("action:app/actions_test.spg|button1|action1".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("action"),
        "type must be action"
    );
    assert!(
        summary.contains_key("parent_component"),
        "action summary must have parent_component"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // submitData action should have writes
    let writes = details_obj
        .get("writes")
        .expect("writes must exist")
        .as_array()
        .expect("writes must be array");
    assert!(!writes.is_empty(), "submitData action should have writes");

    // triggered_by should contain parent component
    let triggered_by = details_obj
        .get("triggered_by")
        .expect("triggered_by must exist")
        .as_array()
        .expect("triggered_by must be array");
    assert!(
        triggered_by
            .iter()
            .any(|t| t.get("name").and_then(|v| v.as_str()) == Some("button1")),
        "action should be triggered by button1"
    );

    // M5 semantic fields
    assert_eq!(
        details_obj.get("action_category").and_then(|v| v.as_str()),
        Some("data_write"),
        "submitData should be data_write"
    );
    assert!(
        details_obj
            .get("semantic_summary")
            .and_then(|v| v.as_str())
            .is_some(),
        "semantic_summary must be present"
    );
    assert!(
        details_obj.contains_key("blocks_on"),
        "blocks_on must be present"
    );
    assert!(
        details_obj.contains_key("condition"),
        "condition must be present"
    );
    assert_eq!(
        details_obj.get("trigger_type").and_then(|v| v.as_str()),
        Some("click")
    );
}

#[test]
fn test_cli_explain_page_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("explain page output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(
        ai.query_target,
        Some("page:app/actions_test.spg".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("page"),
        "type must be page"
    );
    assert!(
        summary.contains_key("entrypoint_count"),
        "page summary must have entrypoint_count"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // Entrypoints should only include user-triggerable components (buttons with actions), not inputs with submitField
    let entrypoints = details_obj
        .get("entrypoints")
        .expect("entrypoints must exist")
        .as_array()
        .expect("entrypoints must be array");
    let ep_names: Vec<String> = entrypoints
        .iter()
        .filter_map(|e| {
            e.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        !ep_names.contains(&"input1".to_string()),
        "input1 with submitField should not be counted as entrypoint"
    );
    assert!(
        !ep_names.contains(&"input2".to_string()),
        "input2 with submitField should not be counted as entrypoint"
    );
    assert!(
        ep_names.contains(&"button1".to_string()),
        "button1 with action should be an entrypoint"
    );
}

#[test]
fn test_cli_explain_component_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "comp:app/actions_test.spg|button1",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain component output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(
        ai.query_target,
        Some("comp:app/actions_test.spg|button1".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("component"),
        "type must be component"
    );
    assert!(
        summary.contains_key("action_count"),
        "component summary must have action_count"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // button1 triggers action1 which writes model1 → component writes should aggregate action writes
    let writes = details_obj
        .get("writes")
        .expect("writes must exist")
        .as_array()
        .expect("writes must be array");
    assert!(
        !writes.is_empty(),
        "component explain should aggregate triggered action writes"
    );
}

#[test]
fn test_cli_explain_field_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "field:model1.name",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain field output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(ai.query_target, Some("field:model1.name".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("field"),
        "type must be field"
    );
    assert!(
        summary.contains_key("parent_model"),
        "field summary must have parent_model"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );

    // Field model1.name is written by input1 (submitField) and button actions
    let written_by_count = summary
        .get("written_by_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        written_by_count > 0,
        "field:model1.name should have writers from model edges with matching field_path"
    );
    let writes = details_obj
        .get("writes")
        .expect("writes must exist")
        .as_array()
        .expect("writes must be array");
    assert!(
        !writes.is_empty(),
        "field explain writes must be populated from parent model edge backfill"
    );
}

#[test]
fn test_cli_explain_dataflow_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "model:dataflow_output",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain dataflow output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_eq!(ai.query_target, Some("model:dataflow_output".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert_eq!(
        summary.get("type").and_then(|v| v.as_str()),
        Some("dataflow"),
        "type must be dataflow"
    );
    assert!(
        summary.contains_key("input_count"),
        "dataflow summary must have input_count"
    );
    assert!(
        summary.contains_key("output_count"),
        "dataflow summary must have output_count"
    );

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(details_obj.contains_key("reads"), "details must have reads");
    assert!(
        details_obj.contains_key("writes"),
        "details must have writes"
    );
    assert!(
        details_obj.contains_key("triggered_by"),
        "details must have triggered_by"
    );
    assert!(
        details_obj.contains_key("affects"),
        "details must have affects"
    );
    assert!(
        details_obj.contains_key("lineage"),
        "details must have lineage"
    );
    assert!(
        details_obj.contains_key("inputs"),
        "dataflow details must have inputs"
    );
    assert!(
        details_obj.contains_key("outputs"),
        "dataflow details must have outputs"
    );

    // DataFlow should have inputs from outgoing DataflowInput edges
    let input_count = summary
        .get("input_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        input_count > 0,
        "dataflow_output should have at least 1 input from outgoing DataflowInput edges"
    );
    let inputs = details_obj
        .get("inputs")
        .expect("inputs must exist")
        .as_array()
        .expect("inputs must be array");
    assert!(
        !inputs.is_empty(),
        "dataflow inputs array should not be empty"
    );
}

#[test]
fn test_cli_explain_field_lineage_input_field() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "field:dataflow_output.工单号",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain field output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_ai_output_contract(&ai);

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    let lineage = details_obj
        .get("lineage")
        .expect("lineage must exist")
        .as_array()
        .expect("lineage must be array");
    assert!(
        !lineage.is_empty(),
        "field:dataflow_output.工单号 should have lineage entries"
    );
    let first = lineage.first().unwrap().as_object().unwrap();
    assert_eq!(
        first.get("target_field").and_then(|v| v.as_str()),
        Some("field:dataflow_output.工单号")
    );
    assert_eq!(
        first.get("transform").and_then(|v| v.as_str()),
        Some("inputField mapping")
    );
    let source_fields = first
        .get("source_fields")
        .and_then(|v| v.as_array())
        .expect("source_fields must be array");
    assert!(
        source_fields
            .iter()
            .any(|v| v.as_str() == Some("field:dataflow_output.workNo")),
        "source_fields should contain field:dataflow_output.workNo"
    );
    assert_eq!(
        first.get("confidence").and_then(|v| v.as_str()),
        Some("high")
    );
}

#[test]
fn test_cli_explain_field_lineage_expr_field() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "field:dataflow_output.预约单号",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain field output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_ai_output_contract(&ai);

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    let lineage = details_obj
        .get("lineage")
        .expect("lineage must exist")
        .as_array()
        .expect("lineage must be array");
    assert!(
        !lineage.is_empty(),
        "field:dataflow_output.预约单号 should have lineage entries"
    );
    let first = lineage.first().unwrap().as_object().unwrap();
    assert_eq!(
        first.get("target_field").and_then(|v| v.as_str()),
        Some("field:dataflow_output.预约单号")
    );
    assert_eq!(
        first.get("transform").and_then(|v| v.as_str()),
        Some("expression calculation")
    );
    assert_eq!(
        first.get("source_expr").and_then(|v| v.as_str()),
        Some("appointmentNo")
    );
    assert_eq!(
        first.get("confidence").and_then(|v| v.as_str()),
        Some("low")
    );

    // Should have LINEAGE_EXPR_UNPARSED diagnostic because appointmentNo is not a model.field reference
    assert!(
        ai.diagnostics
            .iter()
            .any(|d| d.code == "LINEAGE_EXPR_UNPARSED"),
        "must have LINEAGE_EXPR_UNPARSED diagnostic for unresolved expression"
    );
}

#[test]
fn test_cli_explain_dataflow_internal_topology_detail() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "model:dataflow_output",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("explain dataflow output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_ai_output_contract(&ai);

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(
        details_obj.contains_key("internal_topology"),
        "dataflow explain must have internal_topology"
    );
    let topology = details_obj
        .get("internal_topology")
        .expect("internal_topology must exist")
        .as_object()
        .expect("internal_topology must be object");
    let nodes = topology
        .get("nodes")
        .and_then(|v| v.as_array())
        .expect("topology nodes must be array");
    assert!(
        !nodes.is_empty(),
        "internal_topology nodes must not be empty"
    );
    let edges = topology
        .get("edges")
        .and_then(|v| v.as_array())
        .expect("topology edges must be array");
    assert!(
        !edges.is_empty(),
        "internal_topology edges must not be empty"
    );

    // Verify specific node types from fixture
    let node_types: Vec<&str> = nodes
        .iter()
        .filter_map(|n| {
            n.as_object()
                .and_then(|o| o.get("type"))
                .and_then(|v| v.as_str())
        })
        .collect();
    assert!(
        node_types.contains(&"ModelTable"),
        "internal topology should contain ModelTable node"
    );
    assert!(
        node_types.contains(&"Select"),
        "internal topology should contain Select node"
    );
    assert!(
        node_types.contains(&"Output"),
        "internal topology should contain Output node"
    );

    let summary = ai.summary.as_object().expect("summary must be object");
    let internal_count = summary
        .get("internal_node_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        internal_count >= 3,
        "dataflow_output should have at least 3 internal nodes"
    );
}

#[test]
fn test_cli_context_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "model:model1",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("context output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_query_page_logic_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/page_relations.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

// ============================================================

// ============================================================
// M3 Context 扩展 contract 测试
// ============================================================

#[test]
fn test_cli_context_component_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "comp:app/actions_test.spg|button1",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context component output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_eq!(
        ai.query_target,
        Some("comp:app/actions_test.spg|button1".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("center_type").and_then(|v| v.as_str()),
        Some("component")
    );
    assert_eq!(
        summary.get("budget").and_then(|v| v.as_str()),
        Some("normal")
    );
    assert_eq!(summary.get("depth").and_then(|v| v.as_u64()), Some(1));

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    assert!(details.contains_key("upstream"));
    assert!(details.contains_key("downstream"));
    assert!(details.contains_key("related_actions"));
    assert!(details.contains_key("related_models"));
    assert!(details.contains_key("related_pages"));
    assert!(details.contains_key("related_components"));
}

#[test]
fn test_cli_context_action_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "action:app/actions_test.spg|button1|action1",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context action output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_eq!(
        ai.query_target,
        Some("action:app/actions_test.spg|button1|action1".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_context_field_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "field:model1.name",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context field output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_eq!(ai.query_target, Some("field:model1.name".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_context_page_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "page:app/actions_test.spg",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("context page output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_eq!(
        ai.query_target,
        Some("page:app/actions_test.spg".to_string())
    );
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_context_dataflow_graph_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "model:dataflow_output",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context dataflow output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_eq!(ai.query_target, Some("model:dataflow_output".to_string()));
    assert!(!ai.evidence.is_empty(), "evidence must not be empty");
    assert_ai_output_contract(&ai);
}

#[test]
fn test_cli_context_budget_invalid() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let stderr_output = run_cli_stderr(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "model:model1",
        "--depth",
        "1",
        "--budget",
        "invalid",
    ]);
    assert!(
        stderr_output.contains("Invalid budget") || stderr_output.contains("invalid"),
        "invalid budget should report error on stderr, got: {}",
        stderr_output
    );
}

#[test]
fn test_cli_context_budget_compact_truncates() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "page:app/actions_test.spg",
        "--depth",
        "2",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context compact output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("budget").and_then(|v| v.as_str()),
        Some("compact")
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    let total_related: usize = [
        "upstream",
        "downstream",
        "related_actions",
        "related_models",
        "related_pages",
        "related_components",
    ]
    .iter()
    .map(|k| {
        details
            .get(*k)
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    })
    .sum();
    assert!(
        total_related <= 30,
        "compact budget should keep total related items small, got {}",
        total_related
    );
}

#[test]
fn test_cli_context_budget_full_no_truncation() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output_normal = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "page:app/actions_test.spg",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let output_full = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "page:app/actions_test.spg",
        "--depth",
        "1",
        "--budget",
        "full",
    ]);

    let ai_normal: AiOutput =
        serde_json::from_str(&output_normal).expect("normal output must be AiOutput");
    let ai_full: AiOutput =
        serde_json::from_str(&output_full).expect("full output must be AiOutput");

    let details_normal_val = ai_normal.details.expect("details must exist");
    let details_normal = details_normal_val
        .as_object()
        .expect("details must be object");
    let details_full_val = ai_full.details.expect("details must exist");
    let details_full = details_full_val
        .as_object()
        .expect("details must be object");

    let total_normal: usize = [
        "upstream",
        "downstream",
        "related_actions",
        "related_models",
        "related_pages",
        "related_components",
    ]
    .iter()
    .map(|k| {
        details_normal
            .get(*k)
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    })
    .sum();
    let total_full: usize = [
        "upstream",
        "downstream",
        "related_actions",
        "related_models",
        "related_pages",
        "related_components",
    ]
    .iter()
    .map(|k| {
        details_full
            .get(*k)
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0)
    })
    .sum();

    assert!(
        total_full >= total_normal,
        "full budget should include at least as many items as normal, got full={} normal={}",
        total_full,
        total_normal
    );

    assert!(
        !ai_full
            .diagnostics
            .iter()
            .any(|d| d.code == "OUTPUT_TRUNCATED"),
        "full budget should not have OUTPUT_TRUNCATED diagnostic"
    );
}

#[test]
fn test_cli_context_component_semantics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "comp:app/actions_test.spg|button1",
        "--depth",
        "2",
        "--budget",
        "full",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context component semantics output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_ai_output_contract(&ai);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    let pages = details
        .get("related_pages")
        .and_then(|v| v.as_array())
        .expect("related_pages must be array");
    let page_ids: Vec<String> = pages
        .iter()
        .filter_map(|p| {
            p.get("from")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        page_ids.iter().any(|id| id.contains("actions_test.spg")),
        "button1 context should include its parent page, got pages: {:?}",
        page_ids
    );

    let actions = details
        .get("related_actions")
        .and_then(|v| v.as_array())
        .expect("related_actions must be array");
    let action_ids: Vec<String> = actions
        .iter()
        .filter_map(|a| a.get("to").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(
        action_ids.iter().any(|id| id.contains("action1")),
        "button1 context should include triggered action1, got actions: {:?}",
        action_ids
    );

    let models = details
        .get("related_models")
        .and_then(|v| v.as_array())
        .expect("related_models must be array");
    let model_ids: Vec<String> = models
        .iter()
        .filter_map(|m| m.get("to").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();
    assert!(
        model_ids.iter().any(|id| id.contains("model1")),
        "button1 context should include model1 (written by action), got models: {:?}",
        model_ids
    );
}

#[test]
fn test_cli_context_field_semantics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "field:model1.name",
        "--depth",
        "2",
        "--budget",
        "full",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context field semantics output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_ai_output_contract(&ai);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    let models = details
        .get("related_models")
        .and_then(|v| v.as_array())
        .expect("related_models must be array");
    let model_ids: Vec<String> = models
        .iter()
        .filter_map(|m| {
            m.get("from")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        model_ids.iter().any(|id| id == "model:model1"),
        "field:model1.name context should include parent model:model1, got models: {:?}",
        model_ids
    );
}

#[test]
fn test_cli_context_dataflow_semantics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "model:dataflow_output",
        "--depth",
        "2",
        "--budget",
        "full",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context dataflow semantics output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_ai_output_contract(&ai);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // DataflowInput edges are outgoing from dataflow_output to its input sources
    let downstream = details
        .get("downstream")
        .and_then(|v| v.as_array())
        .expect("downstream must be array");
    let has_dataflow_input = downstream
        .iter()
        .any(|d| d.get("edge_type").and_then(|v| v.as_str()) == Some("DataflowInput"));
    assert!(
        has_dataflow_input,
        "dataflow_output context should have DataflowInput edges in downstream, got: {:?}",
        downstream
    );
}

#[test]
fn test_cli_context_next_queries_no_double_prefix() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);

    for target in [
        "model:model1",
        "model:dataflow_output",
        "field:model1.name",
        "comp:app/actions_test.spg|button1",
        "action:app/actions_test.spg|button1|action1",
        "page:app/actions_test.spg",
    ] {
        let output = run_cli(&[
            "--project-dir",
            "tests/fixtures/test_project",
            "--context",
            target,
            "--depth",
            "1",
            "--budget",
            "normal",
        ]);
        let ai: AiOutput = serde_json::from_str(&output).expect("context output must be AiOutput");
        assert_eq!(ai.kind, OutputKind::Context);

        for q in &ai.next_queries {
            assert!(
                !q.contains("--query-model model:"),
                "next_queries must not contain double model: prefix for --query-model, got: {}",
                q
            );
            assert!(
                !q.contains("--query-dataflow model:"),
                "next_queries must not contain double model: prefix for --query-dataflow, got: {}",
                q
            );
        }
    }
}

// ============================================================
// M4 PageLogic 语义测试
// ============================================================

#[test]
fn test_cli_query_page_logic_actions_test_semantics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "summary must have what_is_it"
    );
    assert!(
        summary.contains_key("page_role"),
        "summary must have page_role"
    );

    // entrypoints: 5 buttons, no inputs
    let ep_count = summary
        .get("entrypoint_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(
        ep_count, 10,
        "actions_test should have 10 button entrypoints (including new actions)"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    let entrypoints = details
        .get("entrypoints")
        .and_then(|v| v.as_array())
        .expect("entrypoints must be array");
    let ep_names: Vec<String> = entrypoints
        .iter()
        .filter_map(|e| {
            e.get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        !ep_names.contains(&"input1".to_string()),
        "input1 should not be an entrypoint"
    );
    assert!(
        !ep_names.contains(&"input2".to_string()),
        "input2 should not be an entrypoint"
    );
    assert!(
        ep_names.contains(&"button1".to_string()),
        "button1 should be an entrypoint"
    );

    // action_flows: should have 5 actions
    let action_flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");
    assert_eq!(
        action_flows.len(),
        10,
        "actions_test should have 10 action flows (including new actions)"
    );

    // write_targets: should include model1 and model2
    let write_targets = details
        .get("write_targets")
        .and_then(|v| v.as_array())
        .expect("write_targets must be array");
    let target_models: Vec<String> = write_targets
        .iter()
        .filter_map(|wt| {
            wt.get("target_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .collect();
    assert!(
        target_models.iter().any(|m| m == "model:model1"),
        "write targets should include model:model1"
    );
    assert!(
        target_models.iter().any(|m| m == "model:model2"),
        "write targets should include model:model2"
    );

    // page_inputs: should have param1
    let page_inputs = details
        .get("page_inputs")
        .and_then(|v| v.as_array())
        .expect("page_inputs must be array");
    assert!(
        !page_inputs.is_empty(),
        "actions_test should have page_inputs from raw file"
    );

    // next_queries: no double model: prefix
    for q in &ai.next_queries {
        assert!(
            !q.contains("--query-model model:"),
            "next_queries must not contain double model: prefix, got: {}",
            q
        );
    }
}

#[test]
fn test_cli_query_page_logic_page_relations_navigation() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/page_relations.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    let nav_count = summary
        .get("navigation_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        nav_count >= 2,
        "page_relations should have at least 2 navigation items (OpensPage + EmbedsPage)"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // navigation should include link and embedsuperpage
    let navigation = details
        .get("navigation")
        .and_then(|v| v.as_array())
        .expect("navigation must be array");
    let has_opens_page = navigation.iter().any(|n| {
        n.get("type").and_then(|v| v.as_str()) == Some("OpensPage")
            || n.get("type").and_then(|v| v.as_str()) == Some("ActionNavigates")
    });
    let has_embeds_page = navigation
        .iter()
        .any(|n| n.get("type").and_then(|v| v.as_str()) == Some("EmbedsPage"));
    assert!(
        has_opens_page,
        "page_relations should have OpensPage navigation"
    );
    assert!(
        has_embeds_page,
        "page_relations should have EmbedsPage navigation"
    );

    // action_flows should include setParamValue with reads
    let action_flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");
    let has_set_param = action_flows
        .iter()
        .any(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("setParamValue"));
    assert!(
        has_set_param,
        "page_relations should have setParamValue action flow"
    );

    // data_sources should include model1.fieldA and model1.fieldB
    let data_sources = details
        .get("data_sources")
        .and_then(|v| v.as_array())
        .expect("data_sources must be array");
    assert!(
        !data_sources.is_empty(),
        "page_relations should have data_sources from action reads"
    );
}

#[test]
fn test_cli_query_page_logic_risk_diagnostics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/page_relations.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);

    let summary = ai.summary.as_object().expect("summary must be object");
    let risk_count = summary
        .get("risk_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        risk_count > 0,
        "page_relations should have risk diagnostics (NO_WRITE_TARGETS)"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    let risks = details
        .get("risk_diagnostics")
        .and_then(|v| v.as_array())
        .expect("risk_diagnostics must be array");
    let has_no_write_targets = risks
        .iter()
        .any(|r| r.get("code").and_then(|v| v.as_str()) == Some("NO_WRITE_TARGETS"));
    assert!(
        has_no_write_targets,
        "readonly page should have NO_WRITE_TARGETS diagnostic"
    );
}

#[test]
fn test_cli_query_page_logic_action_meta_no_collision() {
    // button1 和 button2 都有 action1，但元数据不应串线
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    let action_flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");

    // 找到 button1 和 button2 各自的 action1
    let button1_flow = action_flows.iter().find(|f| {
        f.get("component_id").and_then(|v| v.as_str()) == Some("comp:app/actions_test.spg|button1")
            && f.get("action_id").and_then(|v| v.as_str()) == Some("action1")
    });
    let button2_flow = action_flows.iter().find(|f| {
        f.get("component_id").and_then(|v| v.as_str()) == Some("comp:app/actions_test.spg|button2")
            && f.get("action_id").and_then(|v| v.as_str()) == Some("action1")
    });

    assert!(button1_flow.is_some(), "button1 should have action1 flow");
    assert!(button2_flow.is_some(), "button2 should have action1 flow");

    // button1 action1 应有 wait_prev: "input1.value"
    assert_eq!(
        button1_flow
            .unwrap()
            .get("blocks_on")
            .and_then(|v| v.get("raw"))
            .and_then(|v| v.as_str()),
        Some("input1.value"),
        "button1 action1 should have wait_prev from raw file"
    );

    // button2 action1 应有 condition: "input1.value=='test'"（来自 conditionExp）
    assert_eq!(
        button2_flow
            .unwrap()
            .get("condition")
            .and_then(|v| v.get("raw_expr"))
            .and_then(|v| v.as_str()),
        Some("input1.value=='test'"),
        "button2 action1 should have condition from conditionExp"
    );
}

#[test]
fn test_cli_query_page_logic_human_no_duplicate_model() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let stdout = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
        "--human",
    ]);

    // human 模式不应输出 model1.model1.field 这种重复模型名
    assert!(
        !stdout.contains("model1.model1"),
        "human mode should not duplicate model name in field display, got:\n{}",
        stdout
    );
    // 应正常输出 model1.age / model1.name 等
    assert!(
        stdout.contains("model1.age") || stdout.contains("model1.name"),
        "human mode should still show correct field paths"
    );
}

#[test]
fn test_cli_query_page_logic_human_shows_semantics() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let stdout = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
        "--human",
    ]);

    // human 模式应展示 action_category 和 semantic_summary
    assert!(
        stdout.contains("data_write"),
        "human mode should show action_category like data_write, got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("summary:"),
        "human mode should show semantic_summary, got:\n{}",
        stdout
    );
    // button1 有 waitPrev，应展示 waits for
    assert!(
        stdout.contains("waits for:"),
        "human mode should show blocks_on raw, got:\n{}",
        stdout
    );
    // button2 有 conditionExp，应展示 condition
    assert!(
        stdout.contains("condition:"),
        "human mode should show condition raw_expr, got:\n{}",
        stdout
    );
}

#[test]
fn test_cli_explain_and_page_logic_semantic_summary_consistency() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);

    // --explain action:app/actions_test.spg|button2|action1
    let explain_out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "action:app/actions_test.spg|button2|action1",
    ]);
    let explain_ai: AiOutput =
        serde_json::from_str(&explain_out).expect("explain output must be AiOutput");
    let explain_details = explain_ai
        .details
        .expect("details must exist")
        .as_object()
        .unwrap()
        .clone();
    let explain_summary = explain_details
        .get("semantic_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // --query-page-logic
    let page_out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
    ]);
    let page_ai: AiOutput =
        serde_json::from_str(&page_out).expect("page_logic output must be AiOutput");
    let page_details = page_ai
        .details
        .expect("details must exist")
        .as_object()
        .unwrap()
        .clone();
    let flows = page_details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");
    let button2_flow = flows.iter().find(|f| {
        f.get("component_id").and_then(|v| v.as_str()) == Some("comp:app/actions_test.spg|button2")
            && f.get("action_id").and_then(|v| v.as_str()) == Some("action1")
    });
    assert!(
        button2_flow.is_some(),
        "button2 action1 should exist in page_logic"
    );
    let page_summary = button2_flow
        .unwrap()
        .get("semantic_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // 两者都应该提到 "更新" 和 "model2"
    assert!(
        explain_summary.contains("更新") && explain_summary.contains("model2"),
        "explain semantic_summary should mention '更新' and 'model2', got: {}",
        explain_summary
    );
    assert!(
        page_summary.contains("更新") && page_summary.contains("model2"),
        "page_logic semantic_summary should mention '更新' and 'model2', got: {}",
        page_summary
    );
}

#[test]
fn test_cli_query_page_logic_unknown_action_type() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");

    // 应有 UNKNOWN_ACTION_TYPE diagnostic
    assert!(
        ai.diagnostics
            .iter()
            .any(|d| d.code == "UNKNOWN_ACTION_TYPE"),
        "should have UNKNOWN_ACTION_TYPE diagnostic for someCustomAction"
    );

    // action_flows 中未知 action 的 category 应为 unknown
    let details = ai
        .details
        .expect("details must exist")
        .as_object()
        .unwrap()
        .clone();
    let flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");
    let unknown_flow = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("someCustomAction"));
    assert!(
        unknown_flow.is_some(),
        "unknown action should appear in flows"
    );
    assert_eq!(
        unknown_flow
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("unknown")
    );
}

#[test]
fn test_cli_query_page_logic_new_action_types() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");

    let details = ai
        .details
        .expect("details must exist")
        .as_object()
        .unwrap()
        .clone();
    let flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");

    // showDialog should be navigation
    let show_dialog = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("showDialog"));
    assert!(show_dialog.is_some(), "showDialog action should exist");
    assert_eq!(
        show_dialog
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("navigation")
    );

    // closeDialog should be ui_control
    let close_dialog = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("closeDialog"));
    assert!(close_dialog.is_some(), "closeDialog action should exist");
    assert_eq!(
        close_dialog
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("ui_control")
    );

    // switchPanel should be ui_control
    let switch_panel = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("switchPanel"));
    assert!(switch_panel.is_some(), "switchPanel action should exist");
    assert_eq!(
        switch_panel
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("ui_control")
    );

    // validateData should be validation
    let validate = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("validateData"));
    assert!(validate.is_some(), "validateData action should exist");
    assert_eq!(
        validate
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("validation")
    );
}

#[test]
fn test_scanner_show_dialog_close_dialog_edges() {
    use metadata_checker::graph::{EdgeType, GraphDB};
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-test-dialog.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // showDialog action should create ActionControlsComponent edge
    let show_dialog_action = graph
        .node_indices
        .get("action:app/actions_test.spg|buttonShowDialog|actionShowDialog");
    assert!(
        show_dialog_action.is_some(),
        "showDialog action should exist in graph"
    );
    let outgoing: Vec<_> = graph
        .graph
        .edges_directed(*show_dialog_action.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::ActionControlsComponent))
        .collect();
    assert!(
        !outgoing.is_empty(),
        "showDialog should create ActionControlsComponent edge"
    );

    // closeDialog action should create ActionControlsComponent edge
    let close_dialog_action = graph
        .node_indices
        .get("action:app/actions_test.spg|buttonCloseDialog|actionCloseDialog");
    assert!(
        close_dialog_action.is_some(),
        "closeDialog action should exist in graph"
    );
    let outgoing: Vec<_> = graph
        .graph
        .edges_directed(*close_dialog_action.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::ActionControlsComponent))
        .collect();
    assert!(
        !outgoing.is_empty(),
        "closeDialog should create ActionControlsComponent edge"
    );

    // switchPanel action should create ActionControlsComponent edge
    let switch_panel_action = graph
        .node_indices
        .get("action:app/actions_test.spg|buttonSwitchPanel|actionSwitchPanel");
    assert!(
        switch_panel_action.is_some(),
        "switchPanel action should exist in graph"
    );
    let outgoing: Vec<_> = graph
        .graph
        .edges_directed(*switch_panel_action.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::ActionControlsComponent))
        .collect();
    assert!(
        !outgoing.is_empty(),
        "switchPanel should create ActionControlsComponent edge"
    );

    // validateData action should create ActionValidates edge
    let validate_action = graph
        .node_indices
        .get("action:app/actions_test.spg|buttonValidate|actionValidate");
    assert!(
        validate_action.is_some(),
        "validateData action should exist in graph"
    );
    let outgoing: Vec<_> = graph
        .graph
        .edges_directed(*validate_action.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::ActionValidates))
        .collect();
    assert!(
        !outgoing.is_empty(),
        "validateData should create ActionValidates edge"
    );

    let _ = std::fs::remove_file(&db_path);
}
