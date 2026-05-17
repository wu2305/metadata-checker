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
    let result = metadata_checker::query::query_model(&graph, "model:model1", false, "compact");
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
        result.is_ok(),
        "query_page should return Ok with TARGET_NOT_FOUND output for nonexistent page"
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
        result.is_ok(),
        "query_dataflow should return Ok with TARGET_NOT_FOUND output for nonexistent dataflow"
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
        "compact",
    );
    assert!(
        result.is_ok(),
        "query_page_logic should succeed for existing page"
    );
}

#[test]
fn test_m31_explain_condition_inherits_parent_total_row_count_gate() {
    let (_db_path, graph) = setup_graph_db("m31_parent_gate");
    let result = metadata_checker::explain::build_explain_condition_output(
        &graph,
        "comp:app/actions_test.spg|text_total_child",
        "normal",
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");

    let blocking = details
        .get("blocking_conditions")
        .and_then(|v| v.as_array())
        .expect("blocking_conditions must be array");
    assert!(
        blocking.iter().any(|c| {
            c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited")
                && c.get("inherited_from").and_then(|v| v.as_str())
                    == Some("comp:app/actions_test.spg|panel_total_gate")
                && c.get("raw_expr")
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains("model1.totalRowCount__ > 0"))
        }),
        "子组件必须继承父容器的 totalRowCount__ 显示门控"
    );

    let gates = details
        .get("data_empty_gates")
        .and_then(|v| v.as_array())
        .expect("data_empty_gates must be array");
    assert!(
        gates.iter().any(|g| {
            g.get("condition_scope").and_then(|v| v.as_str())
                == Some("expanded_from_total_row_count")
                && g.get("raw_expr")
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains("param1"))
        }),
        "totalRowCount__ 门控必须展开当前页面 model1 的 param1 filter"
    );
    assert!(
        gates.iter().any(|g| {
            g.get("condition_scope").and_then(|v| v.as_str())
                == Some("expanded_from_total_row_count")
                && g.get("raw_expr")
                    .and_then(|v| v.as_str())
                    .map_or(false, |s| s.contains("WXWORK_USER_ID"))
        }),
        "totalRowCount__ 门控必须展开当前页面 model1 的用户 filter"
    );
}

#[test]
fn test_m31_explain_condition_dedupes_same_child_and_parent_gate() {
    let (_db_path, graph) = setup_graph_db("m31_dedupe_gate");
    let result = metadata_checker::explain::build_explain_condition_output(
        &graph,
        "comp:app/actions_test.spg|text_total_child_duplicate",
        "normal",
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");

    let blocking = details
        .get("blocking_conditions")
        .and_then(|v| v.as_array())
        .expect("blocking_conditions must be array");
    let same_gate_count = blocking
        .iter()
        .filter(|c| {
            c.get("raw_expr")
                .and_then(|v| v.as_str())
                .map_or(false, |s| s.contains("model1.totalRowCount__ > 0"))
        })
        .count();
    assert_eq!(
        same_gate_count, 1,
        "子组件和父组件相同 visibleCondition 必须去重为一条"
    );
    let gate = blocking
        .iter()
        .find(|c| {
            c.get("raw_expr")
                .and_then(|v| v.as_str())
                .map_or(false, |s| s.contains("model1.totalRowCount__ > 0"))
        })
        .expect("deduped gate must exist");
    assert!(
        gate.get("deduped_condition_ids")
            .and_then(|v| v.as_array())
            .map_or(false, |arr| !arr.is_empty()),
        "去重后必须保留重复条件来源证据"
    );
}

#[test]
fn test_m32_explain_condition_resolves_bare_value_from_nearest_data_context() {
    let (_db_path, graph) = setup_graph_db("m32_bare_value_context");
    let (outgoing, _) = graph
        .get_node_edges("comp:app/actions_test.spg|text_bare_field_child")
        .expect("text_bare_field_child must have graph edges");
    assert!(
        outgoing.iter().any(|(node, edge)| {
            node.id == "field:model1.name"
                && edge.edge_type == metadata_checker::graph::EdgeType::Reads
                && edge
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("resolution"))
                    .and_then(|v| v.as_str())
                    == Some("inherited_container_data_context")
        }),
        "扫描阶段必须把裸字段 ${{name}} 直接打平成 field:model1.name Reads 边"
    );

    let result = metadata_checker::explain::build_explain_condition_output(
        &graph,
        "comp:app/actions_test.spg|text_bare_field_child",
        "normal",
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");
    let value_context = details
        .get("value_source_context")
        .and_then(|v| v.as_object())
        .expect("value_source_context must exist for bare field value");

    assert_eq!(
        value_context.get("raw_expr").and_then(|v| v.as_str()),
        Some("${name}"),
        "裸字段值来源必须保留原始 value 表达式"
    );
    assert_eq!(
        value_context.get("bare_symbol").and_then(|v| v.as_str()),
        Some("name"),
        "必须识别裸字段名"
    );
    assert_eq!(
        value_context
            .get("nearest_data_context")
            .and_then(|v| v.get("component_id"))
            .and_then(|v| v.as_str()),
        Some("slider_data_context"),
        "必须解析到最近带 dataSet 的祖先容器"
    );
    assert_eq!(
        value_context
            .get("nearest_data_context")
            .and_then(|v| v.get("dataSet"))
            .and_then(|v| v.as_str()),
        Some("model1"),
        "裸字段必须继承容器 dataSet"
    );
    assert_eq!(
        value_context.get("field_path").and_then(|v| v.as_str()),
        Some("model1.name"),
        "裸字段最终应被解释为 dataSet.field"
    );
    assert_eq!(
        value_context
            .get("data_set_model")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str()),
        Some("data/table1.tbl"),
        "dataSet model 必须指向页面 source 声明的表路径"
    );
    assert_eq!(
        value_context
            .get("graph_edge")
            .and_then(|v| v.get("target_id"))
            .and_then(|v| v.as_str()),
        Some("field:model1.name"),
        "value_source_context 必须来自扫描阶段派生出的字段 Reads 边"
    );

    let answer_facts = details
        .get("answer_facts")
        .and_then(|v| v.as_object())
        .expect("M33 answer_facts must exist");
    let value_facts = answer_facts
        .get("value_source_facts")
        .and_then(|v| v.as_object())
        .expect("value_source_facts must exist");
    assert_eq!(
        value_facts.get("result").and_then(|v| v.as_str()),
        Some("data/table1.tbl"),
        "M33 value_source_facts.result 应直接给出表路径"
    );
    assert_eq!(
        value_facts
            .get("nearest_data_context")
            .and_then(|v| v.as_str()),
        Some("slider_data_context"),
        "M33 value_source_facts 必须保留最近数据容器"
    );
    assert!(
        value_facts
            .get("paths")
            .and_then(|v| v.as_array())
            .map_or(false, |paths| paths.iter().any(|p| {
                p.get("steps")
                    .and_then(|v| v.as_array())
                    .map_or(false, |steps| {
                        steps.iter().any(|s| {
                            s.get("why_included")
                                .and_then(|v| v.as_str())
                                .map_or(false, |why| why.contains("bare field"))
                        })
                    })
            })),
        "M33 value_source_facts.paths.steps 必须说明 why_included"
    );
}

#[test]
fn test_m33_compact_explain_condition_hides_large_context_arrays() {
    let (_db_path, graph) = setup_graph_db("m33_compact_context_budget");
    let result = metadata_checker::explain::build_explain_condition_output(
        &graph,
        "comp:app/actions_test.spg|text_total_child",
        "compact",
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");

    assert!(
        details
            .get("answer_facts")
            .and_then(|v| v.as_object())
            .is_some(),
        "compact 输出必须保留 answer_facts 作为 AI 默认入口"
    );
    assert_eq!(
        details
            .get("supporting_context")
            .and_then(|v| v.as_array())
            .map(|items| items.len()),
        Some(0),
        "compact 输出不应展开 supporting_context 大数组"
    );
    assert!(
        details
            .get("supporting_context_summary")
            .and_then(|v| v.as_object())
            .is_some(),
        "compact 输出必须用 supporting_context_summary 保留计数"
    );
    assert_eq!(
        details
            .get("related_context")
            .and_then(|v| v.as_array())
            .map(|items| items.len()),
        Some(0),
        "compact 输出不应展开 related_context 大数组"
    );
    assert!(
        details
            .get("related_context_summary")
            .and_then(|v| v.as_object())
            .is_some(),
        "compact 输出必须用 related_context_summary 保留计数"
    );
}

#[test]
fn test_m33_normal_explain_condition_keeps_context_arrays_for_audit() {
    let (_db_path, graph) = setup_graph_db("m33_normal_context_budget");
    let result = metadata_checker::explain::build_explain_condition_output(
        &graph,
        "comp:app/actions_test.spg|text_total_child",
        "normal",
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");

    let supporting_len = details
        .get("supporting_context")
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .expect("supporting_context must be array");
    let emitted_count = details
        .get("supporting_context_summary")
        .and_then(|v| v.get("emitted_count"))
        .and_then(|v| v.as_u64())
        .expect("supporting_context_summary.emitted_count must exist");
    assert_eq!(
        emitted_count as usize, supporting_len,
        "normal 输出的 supporting_context_summary 必须与展开数组一致"
    );
}

#[test]
fn test_m33_display_intent_rejects_value_source_paths() {
    let (_db_path, graph) = setup_graph_db("m33_display_intent_rejects_value_paths");
    let result = metadata_checker::explain::build_explain_condition_output_with_intent(
        &graph,
        "comp:app/actions_test.spg|text_bare_field_child",
        "normal",
        metadata_checker::explain::TraversalIntent::Display,
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");
    let answer_facts = details
        .get("answer_facts")
        .and_then(|v| v.as_object())
        .expect("answer_facts must exist");

    assert!(
        answer_facts.get("display_facts").is_some(),
        "display intent 必须输出 display_facts"
    );
    assert!(
        answer_facts.get("value_source_facts").is_none(),
        "display intent 不应输出 active value_source_facts"
    );
    assert_eq!(
        details
            .get("primary_path")
            .and_then(|v| v.as_array())
            .map(|items| items.len()),
        Some(0),
        "display intent 不应把 Reads/DataflowInput 路径放入 primary_path"
    );
    assert!(
        details
            .get("rejected_paths")
            .and_then(|v| v.as_array())
            .map_or(false, |items| {
                items.iter().any(|p| {
                    p.get("reject_reason").and_then(|v| v.as_str())
                        == Some("edge_type_not_allowed_for_display_intent")
                })
            }),
        "display intent 必须把值来源路径放入 rejected_paths 并说明 reject_reason"
    );
}

#[test]
fn test_m33_compact_hides_primary_and_value_context_details() {
    let (_db_path, graph) = setup_graph_db("m33_compact_hides_path_details");
    let result = metadata_checker::explain::build_explain_condition_output_with_intent(
        &graph,
        "comp:app/actions_test.spg|text_bare_field_child",
        "compact",
        metadata_checker::explain::TraversalIntent::ValueSource,
    )
    .expect("explain-condition must succeed");
    let details = result
        .get("details")
        .and_then(|v| v.as_object())
        .expect("details must be object");

    assert_eq!(
        details
            .get("primary_path")
            .and_then(|v| v.as_array())
            .map(|items| items.len()),
        Some(0),
        "compact 输出不应展开 primary_path 明细"
    );
    assert!(
        details
            .get("primary_path_summary")
            .and_then(|v| v.as_object())
            .is_some(),
        "compact 输出必须保留 primary_path_summary"
    );
    assert!(
        details
            .get("value_source_context")
            .map_or(false, |v| v.is_null()),
        "compact 输出不应展开 value_source_context 大对象"
    );
    assert_eq!(
        details["answer_facts"]["value_source_facts"]["result"].as_str(),
        Some("data/table1.tbl"),
        "compact 输出必须通过 answer_facts 直接回答值来源"
    );
}

#[test]
fn test_m31_m32_docs_define_condition_and_value_source_contract() {
    let schema = std::fs::read_to_string("docs/schema.md").expect("schema doc must be readable");
    for term in [
        "condition_scope",
        "direct",
        "inherited",
        "expanded_from_total_row_count",
        "referenced_by_model_filter",
        "deduped_condition_ids",
        "value_source_context",
        "answer_facts",
        "why_included",
        "nearest_data_context",
        "bare_symbol",
    ] {
        assert!(
            schema.contains(term),
            "docs/schema.md 缺少 explain-condition 契约: {}",
            term
        );
    }

    for skill_path in skill_md_paths_for_protocol_check() {
        let skill_md = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("{} must be readable: {}", skill_path.display(), err));
        assert!(
            skill_md.contains("condition_scope")
                && skill_md.contains("expanded_from_total_row_count")
                && skill_md.contains("referenced_by_model_filter")
                && skill_md.contains("value_source_context")
                && skill_md.contains("answer_facts")
                && skill_md.contains("why_included")
                && skill_md.contains("nearest_data_context"),
            "{} 必须说明 M31/M32 explain-condition 读取规则",
            skill_path.display()
        );
    }
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
fn test_cli_context_related_nodes_and_components_contract_m8() {
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
        "full",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("context output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    let summary = ai.summary.as_object().expect("summary must be object");
    let high_value_relations = summary
        .get("high_value_relations")
        .and_then(|v| v.as_object())
        .expect("high_value_relations must be object");
    assert!(
        high_value_relations.contains_key("related_nodes"),
        "summary.high_value_relations should contain related_nodes"
    );

    let details = ai
        .details
        .expect("details must exist")
        .as_object()
        .expect("details must be object")
        .clone();
    let related_nodes = details
        .get("related_nodes")
        .and_then(|v| v.as_array())
        .expect("related_nodes must be array");
    let related_components = details
        .get("related_components")
        .and_then(|v| v.as_array())
        .expect("related_components must be array");
    assert!(
        related_nodes
            .iter()
            .any(|n| n.get("type").and_then(|v| v.as_str()) != Some("Component")),
        "related_nodes should include non-component node types"
    );
    assert!(
        related_components
            .iter()
            .all(|n| n.get("type").and_then(|v| v.as_str()) == Some("Component")),
        "related_components should only contain component nodes"
    );
}

#[test]
fn test_cli_context_edge_level_evidence_m8() {
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
    let ai: AiOutput = serde_json::from_str(&output).expect("context output must be AiOutput");
    let has_triggers_edge_evidence = ai.evidence.iter().any(|ev| {
        ev.edge_type.as_deref() == Some("Triggers")
            && (ev.claim.contains("Downstream edge") || ev.claim.contains("Upstream edge"))
            && ev
                .node_id
                .as_deref()
                .map(|id| id.contains("action:app/actions_test.spg|button1|action1"))
                .unwrap_or(false)
    });
    assert!(
        has_triggers_edge_evidence,
        "context should include edge-level sampled evidence for Triggers edge"
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
        ep_count, 14,
        "actions_test should have 14 button entrypoints (including new actions)"
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
        14,
        "actions_test should have 14 action flows (including new actions)"
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
fn test_cli_query_page_logic_visibility_rules_expression_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/visibility_contract.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    let visibility_rules = details
        .get("visibility_rules")
        .and_then(|v| v.as_array())
        .expect("visibility_rules must be array");
    assert!(
        visibility_rules.len() >= 3,
        "visibility_contract should expose at least 3 visibility rules"
    );

    for rule in visibility_rules {
        for key in [
            "expression",
            "raw_expr",
            "refs",
            "resolved_refs",
            "unresolved_refs",
            "ambiguous_refs",
            "diagnostics",
            "confidence",
        ] {
            assert!(
                rule.get(key).is_some(),
                "visibility rule should contain unified field '{}'",
                key
            );
        }
    }

    let has_ambiguous_diag = visibility_rules.iter().any(|r| {
        r.get("diagnostics")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .any(|d| d.get("code").and_then(|v| v.as_str()) == Some("EXPR_AMBIGUOUS_REF"))
            })
            .unwrap_or(false)
    });
    assert!(
        has_ambiguous_diag,
        "visibility rule should include EXPR_AMBIGUOUS_REF diagnostics"
    );

    let has_unsupported_diag = visibility_rules.iter().any(|r| {
        r.get("diagnostics")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|d| {
                    d.get("code").and_then(|v| v.as_str()) == Some("EXPR_UNSUPPORTED_FUNCTION")
                })
            })
            .unwrap_or(false)
    });
    assert!(
        has_unsupported_diag,
        "visibility rule should include EXPR_UNSUPPORTED_FUNCTION diagnostics"
    );

    let risks = details
        .get("risk_diagnostics")
        .and_then(|v| v.as_array())
        .expect("risk_diagnostics must be array");
    let has_visibility_unresolved = risks
        .iter()
        .any(|r| r.get("code").and_then(|v| v.as_str()) == Some("VISIBILITY_RULE_UNRESOLVED"));
    assert!(
        has_visibility_unresolved,
        "visibility rule unresolved diagnostics should be propagated to risk diagnostics"
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
    let unknown_diag = ai
        .diagnostics
        .iter()
        .find(|d| d.code == "UNKNOWN_ACTION_TYPE")
        .expect("UNKNOWN_ACTION_TYPE diagnostic should exist");
    assert_eq!(
        unknown_diag.severity,
        metadata_checker::output::DiagnosticSeverity::Warning
    );
    assert!(
        unknown_diag.location.json_path.is_some(),
        "UNKNOWN_ACTION_TYPE diagnostic should include json_path location"
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

    // script/webAPI/showMessage/showFilesGallary 不应产生 UNKNOWN_ACTION_TYPE
    let unknown_types: Vec<&str> = ai
        .diagnostics
        .iter()
        .filter(|d| d.code == "UNKNOWN_ACTION_TYPE")
        .filter_map(|d| d.message.split("'").nth(1))
        .collect();
    assert!(
        !unknown_types.iter().any(|t| *t == "script"),
        "script should not produce UNKNOWN_ACTION_TYPE"
    );
    assert!(
        !unknown_types.iter().any(|t| *t == "webAPI"),
        "webAPI should not produce UNKNOWN_ACTION_TYPE"
    );
    assert!(
        !unknown_types.iter().any(|t| *t == "showMessage"),
        "showMessage should not produce UNKNOWN_ACTION_TYPE"
    );
    assert!(
        !unknown_types.iter().any(|t| *t == "showFilesGallary"),
        "showFilesGallary should not produce UNKNOWN_ACTION_TYPE"
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

    // script should be script_execution
    let script = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("script"));
    assert!(script.is_some(), "script action should exist");
    assert_eq!(
        script
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("script_execution")
    );

    // webAPI should be api_call
    let web_api = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("webAPI"));
    assert!(web_api.is_some(), "webAPI action should exist");
    assert_eq!(
        web_api
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("api_call")
    );

    // showMessage should be message_prompt
    let show_msg = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("showMessage"));
    assert!(show_msg.is_some(), "showMessage action should exist");
    assert_eq!(
        show_msg
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("message_prompt")
    );

    // showFilesGallary should be file_gallery
    let file_gallery = flows
        .iter()
        .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some("showFilesGallary"));
    assert!(
        file_gallery.is_some(),
        "showFilesGallary action should exist"
    );
    assert_eq!(
        file_gallery
            .unwrap()
            .get("action_category")
            .and_then(|v| v.as_str()),
        Some("file_gallery")
    );
}

#[test]
fn test_cli_query_page_logic_action_category_contract_m8() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/action_category_contract.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");
    let details = ai
        .details
        .expect("details must exist")
        .as_object()
        .expect("details must be object")
        .clone();
    let flows = details
        .get("action_flows")
        .and_then(|v| v.as_array())
        .expect("action_flows must be array");

    let find_cat = |action_type: &str| {
        flows
            .iter()
            .find(|f| f.get("action_type").and_then(|v| v.as_str()) == Some(action_type))
            .and_then(|f| f.get("action_category"))
            .and_then(|v| v.as_str())
    };
    assert_eq!(find_cat("loadData"), Some("data_read"));
    assert_eq!(find_cat("newData"), Some("data_initialization"));
    assert_eq!(find_cat("resetData"), Some("data_refresh"));
    assert_eq!(find_cat("refreshModels"), Some("data_refresh"));
    assert_eq!(find_cat("refreshData"), Some("data_refresh"));
    assert!(
        !ai.diagnostics
            .iter()
            .any(|d| d.code == "ACTION_FLOW_INCOMPLETE"),
        "load/refresh/init/validate actions should not be flagged as ACTION_FLOW_INCOMPLETE"
    );
}

#[test]
fn test_cli_query_page_logic_component_write_evidence_source_node_m8() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/action_category_contract.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");

    let evidence = ai
        .evidence
        .iter()
        .find(|ev| {
            ev.claim
                .contains("Write model1.name via comp:app/action_category_contract.spg|input1")
        })
        .expect("component submit write evidence should include concrete component id");
    assert_eq!(
        evidence.node_id.as_deref(),
        Some("comp:app/action_category_contract.spg|input1"),
        "component write evidence node_id should be component id"
    );
}

#[test]
fn test_cli_query_page_logic_evidence_traceability_m8() {
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
    assert!(
        !ai.evidence.is_empty(),
        "page logic evidence must not be empty"
    );

    let has_action_flow_evidence = ai.evidence.iter().any(|ev| {
        ev.claim.contains("Action flow")
            && ev.source_file.as_deref().is_some()
            && ev.edge_type.as_deref().is_some()
            && ev.json_path.as_deref().is_some()
    });
    assert!(
        has_action_flow_evidence,
        "action_flows should expose traceable evidence with source_file/edge_type/json_path"
    );

    let has_navigation_evidence = ai.evidence.iter().any(|ev| {
        ev.claim.contains("Navigation")
            && ev.edge_type.as_deref().is_some()
            && ev.json_path.as_deref().is_some()
    });
    assert!(
        has_navigation_evidence,
        "navigation should expose traceable evidence"
    );
    assert!(
        ai.diagnostics.iter().any(|d| d.code == "EVIDENCE_SAMPLED"),
        "when details exceed evidence sample cap, should emit EVIDENCE_SAMPLED diagnostic"
    );
}

#[test]
fn test_cli_query_page_logic_visibility_evidence_traceability_m8() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/visibility_contract.spg",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("query_page_logic output must be AiOutput");

    let has_visibility_evidence = ai.evidence.iter().any(|ev| {
        ev.claim.contains("Visibility rule")
            && ev.raw_expr.as_deref().is_some()
            && ev.json_path.as_deref().is_some()
            && ev.source_file.as_deref().is_some()
    });
    assert!(
        has_visibility_evidence,
        "visibility rules should expose raw_expr/json_path/source_file evidence"
    );

    let unresolved_diag = ai
        .diagnostics
        .iter()
        .find(|d| d.code == "VISIBILITY_RULE_UNRESOLVED")
        .expect("should contain VISIBILITY_RULE_UNRESOLVED diagnostic");
    assert_eq!(
        unresolved_diag.severity,
        metadata_checker::output::DiagnosticSeverity::Warning
    );
    assert!(
        unresolved_diag.location.json_path.is_some(),
        "visibility unresolved diagnostic should contain json_path location"
    );
}

#[test]
fn test_cli_explain_action_evidence_traceability_m8() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "action:app/actions_test.spg|button2|action1",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("explain output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    assert_ai_output_contract(&ai);

    let has_relation_evidence = ai.evidence.iter().any(|ev| {
        (ev.claim.contains("reads relation") || ev.claim.contains("writes relation"))
            && ev.edge_type.as_deref().is_some()
            && ev.raw_expr.as_deref().is_some()
            && ev.json_path.as_deref().is_some()
    });
    assert!(
        has_relation_evidence,
        "explain action should expose relation evidence with edge/raw_expr/json_path"
    );

    let write_relation = ai
        .evidence
        .iter()
        .find(|ev| ev.claim.contains("writes relation"))
        .expect("explain action should contain writes relation evidence");
    assert_eq!(
        write_relation.source_file.as_deref(),
        Some("app/actions_test.spg"),
        "relation evidence should use action source file, not target model file"
    );
    if write_relation.json_path.as_deref() == Some("<graph-edge-derived>") {
        assert_eq!(
            write_relation.confidence,
            metadata_checker::output::Confidence::Medium,
            "graph-derived relation evidence should be conservative confidence"
        );
        assert!(
            write_relation
                .reason
                .contains("raw json_path is not available"),
            "graph-derived relation evidence should explain precision limitation"
        );
    }
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

#[test]
fn test_cli_explain_field_lineage_action_writes() {
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
        "field:model1.name should have lineage entries from action writes"
    );
    // At least one entry should have source_expr and source_fields populated
    let has_action_write = lineage.iter().any(|item| {
        let obj = item.as_object().unwrap();
        obj.get("transform").and_then(|v| v.as_str()) == Some("page action write")
            && obj.get("source_expr").and_then(|v| v.as_str()).is_some()
            && obj
                .get("source_fields")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
    });
    assert!(
        has_action_write,
        "lineage must contain action write with source_expr and source_fields"
    );
}

#[test]
fn test_cli_explain_field_lineage_chain_dataflow() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--explain",
        "field:df_b.id",
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
        "field:df_b.id should have chain lineage through physical_x/df_a"
    );
    let has_chain = lineage.iter().any(|item| {
        let obj = item.as_object().unwrap();
        obj.get("transform")
            .and_then(|v| v.as_str())
            .map(|s| s.starts_with("DataFlow chain"))
            .unwrap_or(false)
    });
    assert!(has_chain, "lineage must contain DataFlow chain transform");

    // df_b has two inputs: physical_x and df_a; both should appear in source_fields
    let all_source_fields: Vec<String> = lineage
        .iter()
        .filter_map(|item| item.as_object().unwrap().get("source_fields"))
        .filter_map(|v| v.as_array())
        .flat_map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())))
        .collect();
    assert!(
        all_source_fields.iter().any(|s| s == "field:physical_x.id"),
        "df_b.id lineage must trace back to field:physical_x.id"
    );
    assert!(
        all_source_fields.iter().any(|s| s == "field:df_a.id"),
        "df_b.id lineage must trace back to field:df_a.id"
    );
}

#[test]
fn test_cli_context_field_lineage() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--context",
        "field:dataflow_output.工单号",
        "--depth",
        "1",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput =
        serde_json::from_str(&output).expect("context field output must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Context);
    assert_ai_output_contract(&ai);

    let details = ai.details.expect("details must exist");
    let details_obj = details.as_object().expect("details must be object");
    assert!(
        details_obj.contains_key("lineage"),
        "context field must have lineage"
    );
    let lineage = details_obj
        .get("lineage")
        .expect("lineage must exist")
        .as_array()
        .expect("lineage must be array");
    assert!(
        !lineage.is_empty(),
        "context field:dataflow_output.工单号 should have lineage"
    );
    let first = lineage.first().unwrap().as_object().unwrap();
    assert_eq!(
        first.get("target_field").and_then(|v| v.as_str()),
        Some("field:dataflow_output.工单号")
    );
    let source_fields = first
        .get("source_fields")
        .and_then(|v| v.as_array())
        .expect("source_fields must be array");
    assert!(
        source_fields
            .iter()
            .any(|v| v.as_str() == Some("field:dataflow_output.workNo")),
        "context lineage source_fields should contain field:dataflow_output.workNo"
    );
}

#[test]
fn test_cli_explain_dataflow_lineage_schema() {
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
    let lineage = details_obj
        .get("lineage")
        .expect("lineage must exist")
        .as_array()
        .expect("lineage must be array");
    assert!(
        !lineage.is_empty(),
        "dataflow_output should have lineage entries"
    );
    let first = lineage.first().unwrap().as_object().unwrap();
    // M6 schema: target_field must use field:<model>.<field> format
    let target_field = first
        .get("target_field")
        .and_then(|v| v.as_str())
        .expect("target_field must be string");
    assert!(
        target_field.starts_with("field:"),
        "target_field must use field:<model>.<field> format, got: {}",
        target_field
    );
    // via_node must exist
    assert!(first.contains_key("via_node"), "lineage must have via_node");
    // evidence must exist
    assert!(first.contains_key("evidence"), "lineage must have evidence");
    let evidence = first.get("evidence").unwrap().as_object().unwrap();
    assert!(
        evidence.contains_key("source_file"),
        "evidence must have source_file"
    );
    assert!(
        evidence.contains_key("node_id"),
        "evidence must have node_id"
    );
    assert!(
        evidence.contains_key("json_path"),
        "evidence must have json_path"
    );

    let has_lineage_top_evidence = ai.evidence.iter().any(|ev| {
        ev.claim.contains("Lineage for")
            && ev.raw_expr.as_deref().is_some()
            && ev.json_path.as_deref().is_some()
            && ev.edge_type.as_deref().is_some()
    });
    assert!(
        has_lineage_top_evidence,
        "top-level evidence should include lineage trace items with raw_expr/json_path/edge_type"
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_explain_component_m12() {
    let graph_db_path = "/tmp/m12_test_xiaoshouyi.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    // button1: 真实类型、entrypoint、semantic_summary 包含 showDialog
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain",
        "comp:app/售后.app/首页.spg|button1",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("explain button1 must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("type_detail").and_then(|v| v.as_str()),
        Some("button"),
        "button1 type_detail must be real component type 'button', not 'button1'"
    );
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("entrypoint"),
        "button1 with action must be entrypoint"
    );
    let semantic = summary
        .get("semantic_summary")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        semantic.contains("打开对话框") || semantic.contains("showDialog"),
        "button1 semantic_summary must mention dialog opening, got: {}",
        semantic
    );

    // text11: 真实类型 text、data_source_display、无无意义 LINEAGE_SOURCE_MISSING
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain",
        "comp:app/售后.app/首页.spg|text11",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("explain text11 must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("type_detail").and_then(|v| v.as_str()),
        Some("text"),
        "text11 type_detail must be real component type 'text'"
    );
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("data_source_display"),
        "text11 reading model data must be data_source_display"
    );
    assert!(
        !ai.diagnostics
            .iter()
            .any(|d| d.code == "LINEAGE_SOURCE_MISSING"),
        "text11 must not emit LINEAGE_SOURCE_MISSING"
    );

    // button2: 真实类型 button、entrypoint
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain",
        "comp:app/售后.app/首页.spg|button2",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("explain button2 must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("type_detail").and_then(|v| v.as_str()),
        Some("button"),
        "button2 type_detail must be real component type 'button'"
    );
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("entrypoint"),
        "button2 with action must be entrypoint"
    );

    // 表单输入组件: 真实类型 input、form_input
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain",
        "comp:app/价审.app/测试.spg|input1",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("explain input1 must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert_eq!(
        summary.get("type_detail").and_then(|v| v.as_str()),
        Some("input"),
        "input1 type_detail must be real component type 'input'"
    );
    assert_eq!(
        summary.get("importance").and_then(|v| v.as_str()),
        Some("form_input"),
        "input1 must be classified as form_input"
    );

    // 全局检查：evidence 中不出现 node_id="?"
    for ev in &ai.evidence {
        assert_ne!(
            ev.node_id.as_deref(),
            Some("?"),
            "evidence must not contain node_id='?'"
        );
        assert_ne!(
            ev.raw_expr.as_deref(),
            Some("n/a"),
            "evidence must not contain raw_expr='n/a'"
        );
    }

    // 弱证据 confidence 检查：graph-edge-derived 且缺 source_file 的证据应为 Low
    let has_low_graph_derived = ai.evidence.iter().any(|ev| {
        ev.json_path.as_deref() == Some("<graph-edge-derived>")
            && ev.source_file.is_none()
            && ev.confidence == metadata_checker::output::Confidence::Low
    });
    assert!(
        has_low_graph_derived || ai.evidence.iter().all(|ev| ev.source_file.is_some()),
        "graph-derived evidence without source_file must be Low confidence"
    );
}

// M13 compact 结构验证测试

#[test]
fn test_query_page_logic_compact_truncated_structure() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/actions_test.spg",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    let summary = ai.summary.as_object().expect("summary must be object");
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    for key in [
        "action_flows",
        "entrypoints",
        "data_sources",
        "write_targets",
        "navigation",
    ] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            assert!(
                arr_struct.contains_key("total_count"),
                "{} must have total_count",
                key
            );
            assert!(
                arr_struct.contains_key("shown_count"),
                "{} must have shown_count",
                key
            );
            assert!(
                arr_struct.contains_key("truncated"),
                "{} must have truncated",
                key
            );
            assert!(
                arr_struct.contains_key("remaining_count"),
                "{} must have remaining_count",
                key
            );
            assert!(arr_struct.contains_key("items"), "{} must have items", key);
        }
    }
    let has_truncated = ai.diagnostics.iter().any(|d| d.code == "OUTPUT_TRUNCATED");
    assert!(
        has_truncated,
        "compact large page must have OUTPUT_TRUNCATED"
    );
}

#[test]
fn test_cli_query_page_logic_m19_prerequisites_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/page_relations.spg",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    assert_ai_output_contract(&ai);

    let summary = ai.summary.as_object().expect("summary must be object");
    // M19 summary 计数字段
    assert!(
        summary.contains_key("display_prerequisites_count"),
        "summary must have display_prerequisites_count"
    );
    assert!(
        summary.contains_key("data_prerequisites_count"),
        "summary must have data_prerequisites_count"
    );
    assert!(
        summary.contains_key("action_prerequisites_count"),
        "summary must have action_prerequisites_count"
    );
    assert!(
        summary.contains_key("primary_paths_count"),
        "summary must have primary_paths_count"
    );
    assert!(
        summary.contains_key("related_context_count"),
        "summary must have related_context_count"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // normal 模式下 prerequisites 和 primary_paths 为普通数组
    for key in [
        "display_prerequisites",
        "data_prerequisites",
        "action_prerequisites",
        "primary_paths",
    ] {
        assert!(
            details.get(key).and_then(|v| v.as_array()).is_some(),
            "{} must be an array in normal mode",
            key
        );
    }

    // related_context 在 normal 模式下也是数组，但可能包含 summary 字段
    assert!(
        details
            .get("related_context")
            .and_then(|v| v.as_array())
            .is_some(),
        "related_context must be an array in normal mode"
    );

    // M19 summary 可读字段：Top-N prerequisites 和 key_primary_paths
    assert!(
        summary.contains_key("top_display_prerequisites"),
        "summary must have top_display_prerequisites for readable AI output"
    );
    assert!(
        summary.contains_key("top_data_prerequisites"),
        "summary must have top_data_prerequisites for readable AI output"
    );
    assert!(
        summary.contains_key("top_action_prerequisites"),
        "summary must have top_action_prerequisites for readable AI output"
    );
    assert!(
        summary.contains_key("key_primary_paths"),
        "summary must have key_primary_paths for readable AI output"
    );
}

#[test]
fn test_cli_query_page_logic_m19_compact_truncation() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-page-logic",
        "page:app/page_relations.spg",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // compact 模式下 prerequisites 数组应被截断到合理长度
    for key in [
        "display_prerequisites",
        "data_prerequisites",
        "action_prerequisites",
    ] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            let items = arr_struct
                .get("items")
                .and_then(|v| v.as_array())
                .expect("items must be array");
            let shown_count = arr_struct
                .get("shown_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            assert_eq!(
                items.len() as u64,
                shown_count,
                "{} items length must match shown_count",
                key
            );
            assert!(
                shown_count <= 5,
                "compact {} shown_count must be <= 5, got {}",
                key,
                shown_count
            );
        }
    }
}
#[test]
fn test_query_model_compact_brief_structure() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("what_is_it"),
        "compact model query must have what_is_it"
    );
    assert!(
        summary.contains_key("consumed_by_dataflow_count"),
        "compact model query must have consumed_by_dataflow_count"
    );
    assert!(
        summary.contains_key("produced_by_count"),
        "compact model query must have produced_by_count"
    );
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    for key in ["readers", "writers", "dataflow_inputs", "dataflow_outputs"] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            assert!(
                arr_struct.contains_key("total_count"),
                "{} must have total_count",
                key
            );
            assert!(
                arr_struct.contains_key("shown_count"),
                "{} must have shown_count",
                key
            );
            assert!(
                arr_struct.contains_key("truncated"),
                "{} must have truncated",
                key
            );
            assert!(
                arr_struct.contains_key("remaining_count"),
                "{} must have remaining_count",
                key
            );
            assert!(arr_struct.contains_key("items"), "{} must have items", key);
        }
    }
}

#[test]
fn test_tbl_compact_truncated_structure() {
    let path = Path::new("tests/fixtures/test_project/app/app_table.tbl");
    let meta = parse_file(path).expect("parse should succeed");
    let tbl = meta.tbl.expect("should have tbl metadata");
    let out = metadata_checker::tbl_single::build_tbl_output(&tbl, "compact");
    assert_eq!(out.kind, OutputKind::Table);
    let summary = out.summary.as_object().expect("summary must be object");
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );

    let details_val = out.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    for key in ["fields", "field_lineage"] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            assert!(
                arr_struct.contains_key("total_count"),
                "{} must have total_count",
                key
            );
            assert!(
                arr_struct.contains_key("shown_count"),
                "{} must have shown_count",
                key
            );
            assert!(
                arr_struct.contains_key("truncated"),
                "{} must have truncated",
                key
            );
            assert!(
                arr_struct.contains_key("remaining_count"),
                "{} must have remaining_count",
                key
            );
            assert!(arr_struct.contains_key("items"), "{} must have items", key);
        }
    }
    // 小文件可能不触发截断，结构验证已通过
}

#[test]
fn test_dataflow_compact_truncated_structure() {
    let path = Path::new("tests/fixtures/test_project/app/dataflow_output.tbl");
    let meta = parse_file(path).expect("parse should succeed");
    let tbl = meta.tbl.expect("should have tbl metadata");
    let out = metadata_checker::tbl_single::build_tbl_output(&tbl, "compact");
    assert_eq!(out.kind, OutputKind::DataFlow);
    let summary = out.summary.as_object().expect("summary must be object");
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );

    let details_val = out.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    for key in [
        "fields",
        "field_lineage",
        "dataflow_inputs",
        "dataflow_outputs",
        "internal_nodes",
    ] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            assert!(
                arr_struct.contains_key("total_count"),
                "{} must have total_count",
                key
            );
            assert!(
                arr_struct.contains_key("shown_count"),
                "{} must have shown_count",
                key
            );
            assert!(
                arr_struct.contains_key("truncated"),
                "{} must have truncated",
                key
            );
            assert!(
                arr_struct.contains_key("remaining_count"),
                "{} must have remaining_count",
                key
            );
            assert!(arr_struct.contains_key("items"), "{} must have items", key);
        }
    }
    // 小文件可能不触发截断，结构验证已通过
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_query_page_logic_compact() {
    let graph_db_path = "/tmp/m13_test_xiaoshouyi.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-page-logic",
        "page:app/售后.app/首页.spg",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);
    let summary = ai.summary.as_object().expect("summary must be object");
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );

    let details_val = ai.details.expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    for key in [
        "action_flows",
        "entrypoints",
        "data_sources",
        "write_targets",
        "navigation",
    ] {
        if let Some(arr_struct) = details.get(key).and_then(|v| v.as_object()) {
            assert!(
                arr_struct.contains_key("total_count"),
                "{} must have total_count",
                key
            );
            assert!(
                arr_struct.contains_key("shown_count"),
                "{} must have shown_count",
                key
            );
            assert!(
                arr_struct.contains_key("truncated"),
                "{} must have truncated",
                key
            );
            assert!(
                arr_struct.contains_key("remaining_count"),
                "{} must have remaining_count",
                key
            );
            assert!(arr_struct.contains_key("items"), "{} must have items", key);
        }
    }
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_query_model_compact() {
    let graph_db_path = "/tmp/m13_test_xiaoshouyi.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-model",
        "model:fact_saleContract",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    let summary = ai.summary.as_object().expect("summary must be object");
    let key_findings = summary.get("key_findings").and_then(|v| v.as_array());
    assert!(key_findings.is_some(), "compact must have key_findings");
    assert!(
        !key_findings.unwrap().is_empty(),
        "key_findings must not be empty"
    );
    let evidence_summary = summary.get("evidence_summary").and_then(|v| v.as_object());
    assert!(
        evidence_summary.is_some(),
        "compact must have evidence_summary"
    );
}

// M14 回归测试：目标定位与命令规范防错

#[test]
fn test_quote_cli_arg_shell_safe() {
    use metadata_checker::output::schema::quote_cli_arg;
    assert_eq!(quote_cli_arg("simple"), "simple");
    assert_eq!(quote_cli_arg("model1"), "model1");
    assert_eq!(
        quote_cli_arg("comp:app/售后.app/首页.spg|button1"),
        "'comp:app/售后.app/首页.spg|button1'"
    );
    assert_eq!(
        quote_cli_arg("page:app/售后.app/首页.spg"),
        "'page:app/售后.app/首页.spg'"
    );
    assert_eq!(quote_cli_arg("model:test(1)"), "'model:test(1)'");
    assert_eq!(quote_cli_arg("model:test$var"), "'model:test$var'");
    assert_eq!(quote_cli_arg(""), "''");
}

#[test]
fn test_format_next_query_shell_safe() {
    use metadata_checker::output::schema::format_next_query;
    assert_eq!(
        format_next_query("--explain {} for summary", "comp:app/a.spg|b1"),
        "--explain 'comp:app/a.spg|b1' for summary"
    );
    assert_eq!(
        format_next_query("--query-model {}", "model1"),
        "--query-model model1"
    );
}

#[test]
fn test_cli_query_model_bare_vs_prefix() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out_bare = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
    ]);
    let out_prefix = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model:model1",
    ]);
    let ai_bare: AiOutput = serde_json::from_str(&out_bare).expect("bare must parse");
    let ai_prefix: AiOutput = serde_json::from_str(&out_prefix).expect("prefix must parse");
    assert_eq!(ai_bare.query_target, ai_prefix.query_target);
    assert_eq!(
        ai_bare.query_target,
        Some("model:model1".to_string()),
        "query_target should be normalized to model:model1"
    );
    assert_eq!(ai_bare.kind, ai_prefix.kind);
}

#[test]
fn test_cli_find_page_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--find-page",
        "page",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("find-page must parse");
    assert_eq!(ai.kind, OutputKind::PageQuery);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("match_count"),
        "find-page summary must have match_count"
    );
    let details_val = ai.details.expect("details must be object");
    let details = details_val.as_object().expect("details must be object");
    assert!(
        details.contains_key("matches"),
        "find-page details must have matches"
    );
}

#[test]
fn test_cli_find_model_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--find-model",
        "model",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("find-model must parse");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("match_count"),
        "find-model summary must have match_count"
    );
}

#[test]
fn test_cli_find_component_contract() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--find-component",
        "button",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("find-component must parse");
    assert_eq!(ai.kind, OutputKind::ComponentQuery);
    let summary = ai.summary.as_object().expect("summary must be object");
    assert!(
        summary.contains_key("match_count"),
        "find-component summary must have match_count"
    );
}

#[test]
fn test_cli_target_not_found_candidates() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model:nonexistent_xyz",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("not-found must parse");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    let has_target_not_found = ai.diagnostics.iter().any(|d| d.code == "TARGET_NOT_FOUND");
    assert!(
        has_target_not_found,
        "must have TARGET_NOT_FOUND diagnostic, got {:?}",
        ai.diagnostics
    );
    let details_val = ai.details.expect("details must be object");
    let details = details_val.as_object().expect("details must be object");
    assert!(
        details.contains_key("candidate_targets"),
        "not-found must include candidate_targets"
    );
    assert!(
        !ai.next_queries.is_empty(),
        "not-found must provide next_queries"
    );
}

#[test]
fn test_cli_resolve_model_in_page_single() {
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--resolve-model-page",
        "page:app/page_relations.spg",
        "--resolve-model",
        "model1",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("resolve must parse");
    let summary = ai.summary.as_object().expect("summary must be object");
    let resolved_count = summary
        .get("resolved_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        resolved_count >= 1 || !ai.diagnostics.is_empty(),
        "resolve should return at least one candidate or a diagnostic"
    );
}

#[test]
fn test_quote_cli_arg_inner_quote() {
    use metadata_checker::output::schema::quote_cli_arg;
    // 内部单引号必须转义为 '\''
    assert_eq!(quote_cli_arg("a'b"), "'a'\\''b'");
    assert_eq!(quote_cli_arg("it's"), "'it'\\''s'");
}

// M15-E: next_queries shell 安全回归评测（真实项目输出扫描）

/// 验证真实项目 PageLogic 输出的 next_queries 中所有含特殊字符的 target 都被单引号包裹
#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_next_queries_shell_safe() {
    let graph_db_path = "/tmp/m15_test_nextqueries.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    // 1. 中文页面路径
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-page-logic",
        "page:app/售后.app/首页.spg",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    for q in &ai.next_queries {
        // 如果 query 包含中文路径或 |，target 必须用单引号包裹
        if q.contains("page:app/售后.app") || q.contains("|") {
            assert!(
                q.contains("'page:app/售后.app") || q.contains("'comp:app/售后.app"),
                "next_query with Chinese path must use single quotes: {}",
                q
            );
        }
    }

    // 2. 含 | 的 component target
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain",
        "comp:app/售后.app/首页.spg|button1",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    for q in &ai.next_queries {
        if q.contains("|") {
            assert!(
                q.contains("'"),
                "next_query with | must use single quotes: {}",
                q
            );
        }
    }

    // 3. --query-model 裸名与前缀等价不再测试 double prefix（已有 test_cli_query_model_bare_vs_prefix）
    // 这里验证 target 不确定时推荐 --find-*
    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-model",
        "model:nonexistent_xyz",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    let has_find_recommendation = ai.next_queries.iter().any(|q| {
        q.contains("--find-model") || q.contains("--find-page") || q.contains("--find-component")
    });
    assert!(
        has_find_recommendation,
        "TARGET_NOT_FOUND output must recommend --find-* commands, got next_queries: {:?}",
        ai.next_queries
    );

    let _ = std::fs::remove_file(graph_db_path);
}

/// 验证 quote_cli_arg 对各类特殊字符的正确处理
#[test]
fn test_quote_cli_arg_special_chars() {
    use metadata_checker::output::schema::quote_cli_arg;
    // 空格
    assert_eq!(quote_cli_arg("hello world"), "'hello world'");
    // 括号
    assert_eq!(quote_cli_arg("model:test(1)"), "'model:test(1)'");
    // $
    assert_eq!(quote_cli_arg("model:test$var"), "'model:test$var'");
    // 空字符串
    assert_eq!(quote_cli_arg(""), "''");
    // 内部单引号
    assert_eq!(quote_cli_arg("a'b"), "'a'\\''b'");
}

// M16: SKILL.md 关键术语静态测试

fn skill_md_paths_for_protocol_check() -> Vec<std::path::PathBuf> {
    let mut paths = vec![std::path::PathBuf::from("SKILL.md")];
    let deployed_skill_path = std::env::var("METADATA_CHECKER_SKILL_PATH").unwrap_or_else(|_| {
        "/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md".to_string()
    });
    let deployed_skill = std::path::PathBuf::from(deployed_skill_path);
    if deployed_skill.exists() {
        paths.push(deployed_skill);
    }
    paths
}

#[test]
fn test_skill_md_has_key_terms() {
    let required_terms = [
        "GRAPH_DB_LOCKED",
        "--graph-lock-timeout-ms",
        "<graph-edge-derived>",
        "node_id=\"?\"",
        "--budget compact",
        "--find-model",
        "--resolve-model",
        "--human",
        "consumed_by_dataflow_count",
        "dataflow_role",
        "单文件 fallback",
        "证据强弱分级",
        "summary 与 details 冲突",
        "目标定位协议",
        "真实项目故障处理",
    ];

    for skill_path in skill_md_paths_for_protocol_check() {
        let skill_md = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("{} must be readable: {}", skill_path.display(), err));
        let mut missing = Vec::new();
        for term in &required_terms {
            if !skill_md.contains(term) {
                missing.push(term);
            }
        }
        assert!(
            missing.is_empty(),
            "{} 缺少以下关键术语: {:?}",
            skill_path.display(),
            missing
        );
    }
}

#[test]
fn test_skill_md_has_evidence_rules() {
    for skill_path in skill_md_paths_for_protocol_check() {
        let skill_md = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("{} must be readable: {}", skill_path.display(), err));
        // 必须包含禁止把 graph-edge-derived 当强证据的规则
        assert!(
            skill_md.contains("graph-edge-derived")
                && (skill_md.contains("弱证据") || skill_md.contains("medium/low")),
            "{} 必须定义 <graph-edge-derived> 为弱证据",
            skill_path.display()
        );
        // 必须包含 budget 协议
        assert!(
            skill_md.contains("--budget compact") && skill_md.contains("--budget full"),
            "{} 必须定义 budget 协议",
            skill_path.display()
        );
        // 必须包含单引号规则
        assert!(
            skill_md.contains("单引号") || skill_md.contains("single quotes"),
            "{} 必须定义单引号包裹规则",
            skill_path.display()
        );
    }
}

#[test]
fn test_function_calling_runtime_has_m29_tool_contract() {
    let doc = std::fs::read_to_string("docs/function-calling-runtime.md")
        .expect("function calling runtime doc must be readable");
    let required_terms = [
        "metadata_explain_condition",
        "metadata_explain",
        "metadata_context",
        "metadata_query_model",
        "metadata_query_page_logic",
        "metadata_runtime_status",
        "metadata_runtime_reload",
        "timing` 只能用于性能判断",
        "业务回答优先读取 `result.summary`",
        "related_context",
        "不是必要条件",
        "field:fact_qwSidebar.phoneNumber",
        "page:app/销售.app/销售/合同协议.spg",
    ];

    for term in required_terms {
        assert!(
            doc.contains(term),
            "docs/function-calling-runtime.md 缺少 M29 契约术语: {}",
            term
        );
    }
}

#[test]
fn test_schema_has_m29_function_calling_contract() {
    let schema = std::fs::read_to_string("docs/schema.md").expect("schema doc must be readable");
    let required_terms = [
        "Function Calling 工具层约束",
        "metadata_explain_condition",
        "metadata_query_model",
        "metadata_query_page_logic",
        "metadata_runtime_status",
        "timing` 只能用于性能判断",
        "业务回答优先读取 `result.summary`",
        "related_context",
    ];

    for term in required_terms {
        assert!(
            schema.contains(term),
            "docs/schema.md 缺少 M29 契约术语: {}",
            term
        );
    }
}

#[test]
fn test_skill_md_has_m29_function_calling_rules() {
    let required_terms = [
        "Function Calling 工具层",
        "metadata_explain_condition",
        "metadata_explain",
        "metadata_context",
        "metadata_query_model",
        "metadata_query_page_logic",
        "metadata_runtime_status",
        "metadata_runtime_reload",
        "timing` 只能用于性能判断",
        "业务回答优先读取 `result.summary`",
        "related_context",
        "不是必要条件",
    ];

    for skill_path in skill_md_paths_for_protocol_check() {
        let skill_md = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("{} must be readable: {}", skill_path.display(), err));
        let mut missing = Vec::new();
        for term in &required_terms {
            if !skill_md.contains(term) {
                missing.push(term);
            }
        }
        assert!(
            missing.is_empty(),
            "{} 缺少 M29 function calling 规则: {:?}",
            skill_path.display(),
            missing
        );
    }
}

#[test]
fn test_m30_docs_define_stdio_timing_and_capacity_contract() {
    let schema = std::fs::read_to_string("docs/schema.md").expect("schema doc must be readable");
    let runtime_doc = std::fs::read_to_string("docs/function-calling-runtime.md")
        .expect("function calling runtime doc must be readable");
    let baseline = std::fs::read_to_string("docs/performance-baseline.md")
        .expect("baseline doc must be readable");

    for (name, doc) in [
        ("docs/schema.md", schema.as_str()),
        ("docs/function-calling-runtime.md", runtime_doc.as_str()),
        ("docs/performance-baseline.md", baseline.as_str()),
    ] {
        for term in [
            "output_size_bytes",
            "graph_load_ms",
            "query_compute_ms",
            "serialize_ms",
            "total_ms",
        ] {
            assert!(
                doc.contains(term),
                "{} 缺少 M30 timing 字段: {}",
                name,
                term
            );
        }
    }

    assert!(
        baseline.contains("explain_condition input3")
            && baseline.contains("context input3 depth=2")
            && baseline.contains("query_model fact_qwSidebar")
            && baseline.contains("query_page_logic 合同协议.spg"),
        "performance baseline 必须列出 M30 真实项目基线查询"
    );
}

#[test]
fn test_skill_md_mentions_m30_output_size_capacity_rule() {
    for skill_path in skill_md_paths_for_protocol_check() {
        let skill_md = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|err| panic!("{} must be readable: {}", skill_path.display(), err));
        assert!(
            skill_md.contains("timing.output_size_bytes") && skill_md.contains("容量治理"),
            "{} 必须说明 timing.output_size_bytes 的容量治理用途",
            skill_path.display()
        );
    }
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_query_page_logic_input3_chain() {
    let graph_db_path = "/tmp/m19_test_xiaoshouyi.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-page-logic",
        "page:app/销售.app/销售/合同协议.spg",
        "--budget",
        "compact",
    ]);
    let err = run_cli_stderr(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--query-page-logic",
        "page:app/销售.app/销售/合同协议.spg",
        "--budget",
        "compact",
    ]);
    assert!(!err.contains("[DEBUG]"), "stderr 不应包含 [DEBUG] 调试输出");
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::PageLogic);

    let summary = ai.summary.as_object().expect("summary must be object");
    let key_primary_paths = summary
        .get("key_primary_paths")
        .and_then(|v| v.as_array())
        .expect("key_primary_paths must be array");

    // 精确 segments 断言：必须存在三段路径 input3 -> field:model22.phoneNumber -> field:fact_qwSidebar.phoneNumber <- action
    let mut found_input3_chain = false;
    let mut found_action4_write = false;
    let mut found_action1_write = false;
    for p in key_primary_paths.iter() {
        let empty: Vec<serde_json::Value> = Vec::new();
        let segs = p
            .get("segments")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        // 找 input3 主链路
        if segs.len() >= 3 {
            let to0 = segs[0]
                .get("to")
                .and_then(|v| v.get("node_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let et1 = segs[1]
                .get("edge")
                .and_then(|v| v.get("edge_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let to1 = segs[1]
                .get("to")
                .and_then(|v| v.get("node_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let from2 = segs[2]
                .get("from")
                .and_then(|v| v.get("node_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let et2 = segs[2]
                .get("edge")
                .and_then(|v| v.get("edge_type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let fp2 = segs[2]
                .get("edge")
                .and_then(|v| v.get("field_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if to0 == "field:model22.phoneNumber"
                && et1 == "FieldAlias"
                && to1 == "field:fact_qwSidebar.phoneNumber"
                && from2 == "field:fact_qwSidebar.phoneNumber"
                && et2 == "FieldWrite"
                && fp2.ends_with("phoneNumber")
            {
                found_input3_chain = true;
                let action_id = segs[2]
                    .get("to")
                    .and_then(|v| v.get("node_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if action_id.contains("action4") {
                    found_action4_write = true;
                }
                if action_id.contains("action1") {
                    found_action1_write = true;
                }
            }
        }
    }
    assert!(
        found_input3_chain,
        "key_primary_paths 必须包含三段链路: input3 -> field:model22.phoneNumber -> field:fact_qwSidebar.phoneNumber <- action"
    );
    assert!(
        found_action4_write,
        "key_primary_paths 必须包含 action4 对 fact_qwSidebar.phoneNumber 的 FieldWrite"
    );
    assert!(
        found_action1_write,
        "key_primary_paths 必须包含 action1 对 fact_qwSidebar.phoneNumber 的 FieldWrite"
    );

    // source_expr 透传断言
    let mut action1_source_expr: Option<String> = None;
    let mut action4_source_expr: Option<String> = None;
    let mut phone_time_in_chain = false;
    let mut cross_page_writer_true = false;
    for p in key_primary_paths.iter() {
        let empty: Vec<serde_json::Value> = Vec::new();
        let segs = p
            .get("segments")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        for s in segs.iter() {
            let action_id = s
                .get("to")
                .and_then(|v| v.get("node_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source_expr = s
                .get("edge")
                .and_then(|v| v.get("source_expr"))
                .and_then(|v| v.as_str());
            let fp = s
                .get("edge")
                .and_then(|v| v.get("field_path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if action_id.contains("action1") && fp.ends_with("phoneNumber") {
                action1_source_expr = source_expr.map(|s| s.to_string());
            }
            if action_id.contains("action4") && fp.ends_with("phoneNumber") {
                action4_source_expr = source_expr.map(|s| s.to_string());
            }
            if fp.contains("phone_time") {
                phone_time_in_chain = true;
            }
        }
        if let Some(rf) = p.get("rank_features") {
            if rf
                .get("contains_cross_page_writer")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                cross_page_writer_true = true;
            }
        }
    }
    assert_eq!(
        action1_source_expr.as_deref(),
        Some("input2"),
        "action1 FieldWrite 的 source_expr 应为 input2"
    );
    assert_eq!(
        action4_source_expr.as_deref(),
        Some("NULL"),
        "action4 FieldWrite 的 source_expr 应为 NULL"
    );
    assert!(!phone_time_in_chain, "input3 主链路不应包含 phone_time");
    assert!(
        cross_page_writer_true,
        "至少一条 key_primary_paths 的 rank_features.contains_cross_page_writer 应为 true"
    );

    // 前 3 条中至少有一条是 input3 链路
    let top3_has_input3 = key_primary_paths.iter().take(3).any(|p| {
        let empty: Vec<serde_json::Value> = Vec::new();
        let segs = p
            .get("segments")
            .and_then(|v| v.as_array())
            .unwrap_or(&empty);
        segs.iter().any(|s| {
            s.get("to")
                .and_then(|v| v.get("node_id"))
                .and_then(|v| v.as_str())
                .map_or(false, |id| id == "field:model22.phoneNumber")
        })
    });
    assert!(
        top3_has_input3,
        "key_primary_paths 前 3 条必须包含 input3 三段链路，避免无关噪声排在前位"
    );

    // summary.related_context_count 与 details.related_context_summary.total_count 必须一致
    let summary_rc = summary
        .get("related_context_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let details_val = ai.details.as_ref().expect("details must exist");
    let details_obj = details_val.as_object().expect("details must be object");
    let details_rc_total = details_obj
        .get("related_context_summary")
        .and_then(|v| v.get("total_count"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert_eq!(
        summary_rc, details_rc_total,
        "summary.related_context_count ({}) 必须等于 details.related_context_summary.total_count ({})",
        summary_rc, details_rc_total
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_explain_condition_input3() {
    let graph_db_path = "/tmp/m20_test_xiaoshouyi.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "comp:app/销售.app/销售/合同协议.spg|input3",
        "--intent",
        "writer",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);

    let summary = ai.summary.as_object().expect("summary must be object");
    let details_val = ai.details.as_ref().expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // 必须包含 primary_reason
    assert!(
        summary.contains_key("primary_reason"),
        "summary 必须包含 primary_reason"
    );

    // compact 模式不展开 primary_path，writer_facts 必须包含 input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber 链路
    let primary_path_len = details
        .get("primary_path")
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(usize::MAX);
    assert_eq!(
        primary_path_len, 0,
        "M33 compact 输出不应展开 primary_path 明细"
    );
    let writer_paths = details
        .get("answer_facts")
        .and_then(|v| v.get("writer_facts"))
        .and_then(|v| v.get("paths"))
        .and_then(|v| v.as_array())
        .expect("writer_facts.paths must be array");
    let has_input3_chain = writer_paths.iter().any(|p| {
        let path_id = p.get("result").and_then(|v| v.as_str()).unwrap_or("");
        path_id.contains("input3")
            && path_id.contains("model22.phoneNumber")
            && path_id.contains("fact_qwSidebar.phoneNumber")
    });
    assert!(
        has_input3_chain,
        "writer_facts 必须包含 input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber 链路"
    );

    // writer_facts 前 5 条不能全是无关模型（如 model19.brand）
    let top5 = writer_paths.iter().take(5).collect::<Vec<_>>();
    let all_noise = top5.iter().all(|p| {
        let path_id = p.get("result").and_then(|v| v.as_str()).unwrap_or("");
        !path_id.contains("input3") && !path_id.contains("model22")
    });
    assert!(
        !all_noise,
        "writer_facts 前 5 条不能全部是无关噪声，至少应包含 input3 链路"
    );

    // blocking_conditions 应包含 visibleCondition（input3 影响 text45 显示）
    let empty: Vec<serde_json::Value> = Vec::new();
    let blocking = details
        .get("blocking_conditions")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let has_visible_condition = blocking.iter().any(|c| {
        let ct = c
            .get("condition_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        ct == "VisibleCondition"
    });
    assert!(
        has_visible_condition,
        "input3 的 explain-condition 应包含关联的 VisibleCondition"
    );

    // blocking_conditions 或 data_empty_gates 不应包含 phone_time
    let gates = details
        .get("data_empty_gates")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    for cond in blocking.iter().chain(gates.iter()) {
        let raw = cond.get("raw_expr").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            !raw.contains("phone_time"),
            "input3 的 why 解释不应包含 phone_time"
        );
    }

    // 主区 blocking_conditions / data_empty_gates 不得带旁路 note
    for cond in blocking.iter().chain(gates.iter()) {
        if let Some(note) = cond.get("note").and_then(|v| v.as_str()) {
            assert!(
                note != "非当前页面必要条件",
                "input3 的主条件不应包含旁路 note"
            );
        }
    }

    let related_len = details
        .get("related_context")
        .and_then(|v| v.as_array())
        .map(|items| items.len())
        .unwrap_or(0);
    assert_eq!(
        related_len, 0,
        "M33 compact 输出不应展开大量 related_context 明细"
    );
    assert!(
        details
            .get("primary_path_summary")
            .and_then(|v| v.get("hidden"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        "M33 compact 输出必须用 primary_path_summary 标明主路径明细已隐藏"
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_explain_condition_model22_filter() {
    let graph_db_path = "/tmp/m20_test_xiaoshouyi_model22.db";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "model:model22",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);

    let summary = ai.summary.as_object().expect("summary must be object");
    let details_val = ai.details.as_ref().expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // model22 应该有 data_empty_gates（filter 条件）
    let data_empty_gates = details
        .get("data_empty_gates")
        .and_then(|v| v.as_array())
        .expect("data_empty_gates must be array");
    assert!(
        !data_empty_gates.is_empty(),
        "model:model22 的 explain-condition 必须包含 data_empty_gates（filter 条件）"
    );

    // 至少有一个 filter 条件是 SourceFilterExp
    let has_source_filter = data_empty_gates.iter().any(|c| {
        let ct = c
            .get("condition_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        ct == "SourceFilterExp"
    });
    assert!(
        has_source_filter,
        "model:model22 必须包含 SourceFilterExp 类型的 filter 条件"
    );

    // summary 中 data_empty_gates_count > 0
    let gates_count = summary
        .get("data_empty_gates_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(gates_count > 0, "summary.data_empty_gates_count 必须大于 0");

    // data_empty_gates 必须只包含当前页面（合同协议.spg）的条件
    for g in data_empty_gates.iter() {
        let sf = g.get("source_file").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            sf.contains("合同协议"),
            "model:model22 的 data_empty_gates 只能包含合同协议.spg 的条件，实际: {}",
            sf
        );
    }

    // related_context 必须包含其他页面的条件
    let related = details
        .get("related_context")
        .and_then(|v| v.as_array())
        .expect("related_context must be array");
    let has_other_page = related.iter().any(|r| {
        let sf = r.get("source_file").and_then(|v| v.as_str()).unwrap_or("");
        !sf.contains("合同协议")
    });
    assert!(
        has_other_page,
        "model:model22 的 related_context 必须包含其他页面的同名 model22 条件"
    );

    // primary_reason 应提到数据门控
    let primary_reason = summary
        .get("primary_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        primary_reason.contains("数据门控") || primary_reason.contains("filter"),
        "model:model22 的 primary_reason 应说明数据门控或 filter 影响"
    );

    // 主区 data_empty_gates 不得带旁路 note
    for g in data_empty_gates.iter() {
        if let Some(note) = g.get("note").and_then(|v| v.as_str()) {
            assert!(
                note != "非当前页面必要条件",
                "model:model22 的主 data_empty_gates 不应包含旁路 note"
            );
        }
    }

    // related_context 必须全部有旁路 note
    for r in related.iter() {
        let note = r.get("note").and_then(|v| v.as_str()).unwrap_or("");
        assert!(
            note.contains("非当前页面") || note.contains("非必要"),
            "model:model22 的 related_context 每项必须有旁路 note，实际: {:?}",
            r
        );
    }
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_m31_text41_display_chain_expands_model11_filter() {
    let graph_db_path = "/tmp/m31_test_xiaoshouyi_text41.db";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "comp:app/售后.app/绑定车辆/会员已注册.spg|text41",
        "--budget",
        "normal",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);

    let details_val = ai.details.as_ref().expect("details must exist");
    let details = details_val.as_object().expect("details must be object");
    let blocking = details
        .get("blocking_conditions")
        .and_then(|v| v.as_array())
        .expect("blocking_conditions must be array");
    assert!(
        blocking.iter().any(|c| {
            c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited")
                && c.get("condition_id")
                    .and_then(|v| v.as_str())
                    .map_or(false, |id| id.contains("panel35#visibleCondition"))
                && c.get("raw_expr")
                    .and_then(|v| v.as_str())
                    .map_or(false, |raw| raw.contains("model11.totalRowCount__ > 0"))
        }),
        "text41 必须继承 panel35 的 model11.totalRowCount__ 显示门控"
    );

    let gates = details
        .get("data_empty_gates")
        .and_then(|v| v.as_array())
        .expect("data_empty_gates must be array");
    assert!(
        gates.iter().any(|g| {
            g.get("condition_scope").and_then(|v| v.as_str())
                == Some("expanded_from_total_row_count")
                && g.get("condition_id")
                    .and_then(|v| v.as_str())
                    .map_or(false, |id| id.contains("model11#filter"))
        }),
        "text41 的 totalRowCount__ 门控必须展开当前页面 model11 filter"
    );
    assert!(
        !gates.iter().any(|g| {
            g.get("condition_id")
                .and_then(|v| v.as_str())
                .map_or(false, |id| id.contains("model2#filter"))
        }),
        "model2.name=text41 是引用 text41 的旁路 filter，不应作为 text41 显示必要门控"
    );

    let value_context = details
        .get("value_source_context")
        .and_then(|v| v.as_object())
        .expect("text41 必须输出裸字段值来源上下文");
    assert_eq!(
        value_context.get("bare_symbol").and_then(|v| v.as_str()),
        Some("CUSTOMAUTOMYAUTOLIST"),
        "text41.value 的裸字段名必须被识别"
    );
    assert_eq!(
        value_context
            .get("nearest_data_context")
            .and_then(|v| v.get("component_id"))
            .and_then(|v| v.as_str()),
        Some("sliderpanel2"),
        "text41.value 必须继承 sliderpanel2 的 dataSet 上下文"
    );
    assert_eq!(
        value_context
            .get("nearest_data_context")
            .and_then(|v| v.get("dataSet"))
            .and_then(|v| v.as_str()),
        Some("model11"),
        "sliderpanel2 的 dataSet 必须解析为 model11"
    );
    assert!(
        value_context
            .get("table_source_path")
            .and_then(|v| v.as_str())
            .map_or(false, |path| path.contains("加工表/小程序/绑车.tbl")),
        "text41 的 dataSet 表路径必须来自 model11 的 绑车.tbl，实际: {:?}",
        value_context.get("table_source_path")
    );
    assert!(
        value_context
            .get("dataflow_field_origin")
            .and_then(|v| v.get("module_table_path"))
            .and_then(|v| v.as_str())
            .map_or(false, |path| path.contains("fact_autoCustomerAutoRel.tbl")),
        "CUSTOMAUTOMYAUTOLIST 字段来源应追到 DataFlow 原始输入 fact_autoCustomerAutoRel.tbl，实际: {:?}",
        value_context.get("dataflow_field_origin")
    );

    let answer_facts = details
        .get("answer_facts")
        .and_then(|v| v.as_object())
        .expect("M33 answer_facts must exist");
    let display_facts = answer_facts
        .get("display_facts")
        .and_then(|v| v.as_object())
        .expect("display_facts must exist");
    assert_eq!(
        display_facts
            .get("has_direct_condition")
            .and_then(|v| v.as_bool()),
        Some(false),
        "text41 自身没有 direct visibleCondition"
    );
    assert!(
        display_facts
            .get("inherited_conditions")
            .and_then(|v| v.as_array())
            .map_or(false, |items| serde_json::Value::Array(items.clone())
                .to_string()
                .contains("panel35#visibleCondition")),
        "display_facts 必须包含 panel35 inherited condition"
    );
    assert!(
        display_facts
            .get("expanded_data_gates")
            .and_then(|v| v.as_array())
            .map_or(false, |items| serde_json::Value::Array(items.clone())
                .to_string()
                .contains("model11.ISSHOW == 1")),
        "display_facts 必须包含 model11.ISSHOW == 1 展开门控"
    );

    let value_facts = answer_facts
        .get("value_source_facts")
        .and_then(|v| v.as_object())
        .expect("value_source_facts must exist");
    assert_eq!(
        value_facts.get("bare_symbol").and_then(|v| v.as_str()),
        Some("CUSTOMAUTOMYAUTOLIST"),
        "value_source_facts 必须保留裸字段名"
    );
    assert_eq!(
        value_facts
            .get("nearest_data_context")
            .and_then(|v| v.as_str()),
        Some("sliderpanel2"),
        "value_source_facts 必须保留 sliderpanel2 数据容器"
    );
    assert!(
        value_facts
            .get("proven_physical_input")
            .and_then(|v| v.as_str())
            .map_or(false, |path| path.contains("fact_autoCustomerAutoRel.tbl")),
        "value_source_facts 必须直接给出 proven physical input"
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_m33_text41_compact_display_intent_is_low_noise() {
    let graph_db_path = "/tmp/m33_text41_display.graphdb";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "comp:app/售后.app/绑定车辆/会员已注册.spg|text41",
        "--intent",
        "display",
        "--budget",
        "compact",
    ]);
    assert!(
        out.len() < 12_000,
        "text41 display compact 输出应保持低噪声，实际 {} bytes",
        out.len()
    );

    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    let summary = ai.summary.as_object().expect("summary must be object");
    let details = ai
        .details
        .as_ref()
        .and_then(|v| v.as_object())
        .expect("details must be object");
    assert_eq!(
        summary.get("intent").and_then(|v| v.as_str()),
        Some("display")
    );
    assert_eq!(
        summary.get("answer_facts_count").and_then(|v| v.as_u64()),
        Some(1),
        "display intent 只能激活 display_facts"
    );
    let facts = details
        .get("answer_facts")
        .and_then(|v| v.as_object())
        .expect("answer_facts must be object");
    assert!(facts.get("display_facts").is_some());
    assert!(
        facts.get("value_source_facts").is_none(),
        "display intent 不应输出 active value_source_facts"
    );
    assert_eq!(
        details
            .get("primary_path")
            .and_then(|v| v.as_array())
            .map(|items| items.len()),
        Some(0),
        "compact display intent 不应展开 primary_path"
    );
    assert!(
        details
            .get("value_source_context")
            .map_or(false, |v| v.is_null()),
        "compact display intent 不应展开 value_source_context"
    );
    assert!(
        details
            .get("rejected_paths_summary")
            .and_then(|v| v.get("total_count"))
            .and_then(|v| v.as_u64())
            .map_or(false, |count| count > 0),
        "display intent 应把非显示路径计入 rejected_paths_summary"
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_explain_condition_page_合同协议() {
    let graph_db_path = "/tmp/m20_test_xiaoshouyi_page.db";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "page:app/销售.app/销售/合同协议.spg",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);

    let summary = ai.summary.as_object().expect("summary must be object");
    let details_val = ai.details.as_ref().expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // 不能返回空 summary
    let gates_count = summary
        .get("data_empty_gates_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let blocking_count = summary
        .get("blocking_conditions_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        gates_count > 0 || blocking_count > 0,
        "page:合同协议.spg 的 explain-condition 不能为空"
    );

    // 必须至少包含一个 SourceFilterExp
    let empty: Vec<serde_json::Value> = Vec::new();
    let gates = details
        .get("data_empty_gates")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let has_filter = gates.iter().any(|c| {
        let ct = c
            .get("condition_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        ct == "SourceFilterExp"
    });
    assert!(
        has_filter,
        "page:合同协议.spg 必须包含至少一个 SourceFilterExp"
    );

    // 必须至少包含一个 VisibleCondition
    let blocking = details
        .get("blocking_conditions")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty);
    let has_visible = blocking.iter().any(|c| {
        let ct = c
            .get("condition_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        ct == "VisibleCondition"
    });
    assert!(
        has_visible,
        "page:合同协议.spg 必须包含至少一个 VisibleCondition"
    );
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_explain_condition_input33_candidates() {
    let graph_db_path = "/tmp/m20_test_xiaoshouyi_input33.db";
    let _ = std::fs::remove_file(graph_db_path);
    let _ = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--build-graph",
    ]);

    let out = run_cli(&[
        "--project-dir",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
        "--graph-db-path",
        graph_db_path,
        "--explain-condition",
        "comp:app/销售.app/销售/合同协议.spg|input33",
        "--budget",
        "compact",
    ]);
    let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::Explain);

    let summary = ai.summary.as_object().expect("summary must be object");
    let details_val = ai.details.as_ref().expect("details must exist");
    let details = details_val.as_object().expect("details must be object");

    // candidate_count > 0
    let candidate_count = summary
        .get("candidate_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(candidate_count > 0, "input33 应返回至少 1 个候选");

    // 候选包含 input3
    let candidates = details
        .get("candidate_targets")
        .and_then(|v| v.as_array())
        .expect("candidate_targets must be array");
    let has_input3 = candidates.iter().any(|c| {
        let id = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
        id.ends_with("|input3")
    });
    assert!(has_input3, "input33 的候选必须包含 input3");

    // next_queries shell-safe（target ID 被单引号包裹）
    let next_queries = ai.next_queries;
    for q in &next_queries {
        assert!(
            q.contains("'") && q.split("'").nth(1).is_some(),
            "next_queries 必须 shell-safe（target ID 被单引号包裹）: {}",
            q
        );
    }
}
