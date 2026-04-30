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
    let result =
        metadata_checker::query::query_page_logic(&graph, "page:app/page_relations.spg", false);
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
