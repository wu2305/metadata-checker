use clap::Parser;
use metadata_checker::dependency::DependencyGraph;
use metadata_checker::output;
use metadata_checker::parser;
use std::path::PathBuf;

// ============================================================
// 一、parser.rs 测试
// ============================================================

#[test]
fn test_parse_file_superpage() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse test_superpage.spg");

    assert!(meta.superpage.is_some(), "Should detect SuperPage format");
    let spg = meta.superpage.unwrap();
    assert_eq!(spg.version, Some("4.19.7".to_string()));
    assert_eq!(spg.components.len(), 8);
    assert_eq!(spg.expressions.len(), 7);
}

#[test]
fn test_parse_file_not_found() {
    let path = PathBuf::from("tests/fixtures/nonexistent.spg");
    let result = parser::parse_file(&path);
    assert!(result.is_err(), "Should fail for nonexistent file");
}

#[test]
fn test_parse_file_invalid_json() {
    let path = PathBuf::from("tests/fixtures/invalid_json.spg");
    let result = parser::parse_file(&path);
    assert!(result.is_err(), "Should fail for invalid JSON");
}

#[test]
fn test_parse_file_empty() {
    let path = PathBuf::from("tests/fixtures/empty_file.spg");
    let result = parser::parse_file(&path);
    assert!(result.is_err(), "Should fail for empty file");
}

// ============================================================
// 二、output.rs print_human / print_non_human 测试
// ============================================================

#[test]
fn test_print_human_superpage() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    output::print_human_to(&meta, &mut buf).expect("print_human_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    assert!(
        s.contains("=== SuperPage Metadata Report ==="),
        "Should contain SuperPage header"
    );
    assert!(
        s.contains("Version      : 4.19.7"),
        "Should contain version"
    );
    assert!(
        s.contains("Components   : 8"),
        "Should contain component count"
    );
    assert!(s.contains("input1"), "Should mention input1 component");
    assert!(
        s.contains("=== End of Report ==="),
        "Should end with footer"
    );
}

#[test]
fn test_print_non_human_superpage() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    output::print_non_human_to(&meta, None, &mut buf).expect("print_non_human_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let json: serde_json::Value = serde_json::from_str(&s).expect("Should be valid JSON");
    assert_eq!(
        json.get("schema_version").and_then(|v| v.as_str()),
        Some("1.0")
    );
    assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("SuperPage"));
    assert!(json.get("summary").is_some(), "Should have summary");
    assert!(json.get("details").is_some(), "Should have details");
    let details = json.get("details").unwrap();
    assert!(
        details.get("components").is_some(),
        "Should have components array"
    );
    assert!(
        details.get("expressions").is_some(),
        "Should have expressions array"
    );
    assert!(
        details.get("dependency_order").is_some(),
        "Should have dependency_order"
    );
}

// ============================================================
// 三、output.rs component query 测试
// ============================================================

#[test]
fn test_print_component_query_human() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");
    let graph = DependencyGraph::new(&spg);

    let mut buf: Vec<u8> = Vec::new();
    output::print_component_query_human_to(&spg, &graph, "input1", false, &mut buf)
        .expect("Should print component query");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    assert!(
        s.contains("=== Component: input1 ==="),
        "Should contain component header"
    );
    assert!(s.contains("Type: input"), "Should contain component type");
    assert!(s.contains("value"), "Should mention value field");
}

#[test]
fn test_print_component_query_human_not_found() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");
    let graph = DependencyGraph::new(&spg);

    let mut buf: Vec<u8> = Vec::new();
    let result =
        output::print_component_query_human_to(&spg, &graph, "nonexistent", false, &mut buf);
    assert!(result.is_err(), "Should fail for nonexistent component");
}

#[test]
fn test_print_component_query_json() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");
    let graph = DependencyGraph::new(&spg);

    let mut buf: Vec<u8> = Vec::new();
    output::print_component_query_json_to(&spg, &graph, "input3", false, &mut buf)
        .expect("Should print JSON query");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let json: serde_json::Value = serde_json::from_str(&s).expect("Should be valid JSON");
    assert_eq!(
        json.get("schema_version").and_then(|v| v.as_str()),
        Some("1.0")
    );
    assert_eq!(
        json.get("kind").and_then(|v| v.as_str()),
        Some("ComponentQuery")
    );
    let summary = json.get("summary").expect("Should have summary");
    assert_eq!(
        summary.get("component_id").and_then(|v| v.as_str()),
        Some("input3")
    );
    let details = json.get("details").expect("Should have details");
    assert!(
        details.get("expressions").is_some(),
        "Should have expressions"
    );
    assert!(
        details.get("value_trace").is_some(),
        "Should have value_trace"
    );
}

// ============================================================
// 四、summary / detail JSON 契约测试
// ============================================================

#[test]
fn test_print_summary_is_valid_json() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    output::print_summary_to(&meta, None, &mut buf).expect("print_summary_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let json: serde_json::Value = serde_json::from_str(&s).expect("Summary should be valid JSON");
    assert_eq!(
        json.get("schema_version").and_then(|v| v.as_str()),
        Some("1.0")
    );
    assert_eq!(json.get("kind").and_then(|v| v.as_str()), Some("SuperPage"));
    assert!(json.get("summary").is_some(), "Should have summary object");
    let summary = json.get("summary").expect("Should have summary");
    assert!(
        summary.get("important_components").is_some(),
        "Should have important_components"
    );
    assert!(json.get("diagnostics").is_some(), "Should have diagnostics");
    // Summary should NOT contain full components/expressions arrays
    assert!(
        json.get("details").is_none() || json.get("details").unwrap().is_null(),
        "Summary should not contain details"
    );
}

#[test]
fn test_print_summary_with_priority_is_valid_json() {
    use metadata_checker::priority;

    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.as_ref().expect("Should be SuperPage");
    let analyses = priority::analyze_priority(spg);

    let mut buf: Vec<u8> = Vec::new();
    output::print_summary_to(&meta, Some(&analyses), &mut buf)
        .expect("print_summary_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let json: serde_json::Value = serde_json::from_str(&s).expect("Summary should be valid JSON");
    assert_eq!(
        json.get("schema_version").and_then(|v| v.as_str()),
        Some("1.0")
    );
    // In compact summary, priority_analysis is in details which is null
    // We just verify the schema is valid
    assert!(
        json.get("schema_version").is_some(),
        "Should have schema_version"
    );
    assert!(json.get("kind").is_some(), "Should have kind");
}

#[test]
fn test_detail_output_contains_resolved_refs() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parser::parse_file(&path).expect("Failed to parse");

    let mut buf: Vec<u8> = Vec::new();
    output::print_non_human_to(&meta, None, &mut buf).expect("print_non_human_to should succeed");
    let s = String::from_utf8(buf).expect("Valid UTF-8");

    let json: serde_json::Value = serde_json::from_str(&s).expect("Should be valid JSON");
    let details = json.get("details").expect("Should have details");
    let expressions = details
        .get("expressions")
        .and_then(|v| v.as_array())
        .expect("Should have expressions array");
    assert!(!expressions.is_empty(), "Should have expressions");

    let first_expr = &expressions[0];
    assert!(
        first_expr.get("resolved_refs").is_some(),
        "Detail output should contain resolved_refs"
    );
}

// ============================================================
// 五、CLI 行为测试
// ============================================================

#[test]
fn test_cli_human_and_interactive_both_enter_repl() {
    // 模拟 --human 和 --interactive 都进入 REPL
    let cli_human = metadata_checker::cli::Cli::parse_from(["metadata-checker", "--human"]);
    assert!(
        cli_human.is_human(),
        "--human should set is_human() to true"
    );

    let cli_interactive =
        metadata_checker::cli::Cli::parse_from(["metadata-checker", "--interactive"]);
    assert!(
        cli_interactive.is_human(),
        "--interactive should set is_human() to true"
    );
    assert!(
        cli_interactive.is_interactive(),
        "--interactive should set is_interactive() to true"
    );

    let cli_none = metadata_checker::cli::Cli::parse_from(["metadata-checker"]);
    assert!(!cli_none.is_human(), "Default should not be human");
}

// ============================================================
// 六、REPL 集成测试
// ============================================================

#[test]
#[ignore = "Requires compiled binary; run manually or in CI"]
fn test_repl_interactive_priority() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let bin = std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker");
    if !bin.exists() {
        eprintln!("Binary not found, skipping integration test");
        return;
    }

    let mut child = Command::new(&bin)
        .args(["tests/fixtures/test_superpage.spg", "--human", "--priority"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn metadata-checker");

    {
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(b"input3\n").unwrap();
        stdin.write_all(b"all\n").unwrap();
        stdin.write_all(b"q\n").unwrap();
    }

    let output = child.wait_with_output().expect("Failed to read output");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("=== SuperPage Interactive Mode ==="),
        "Should enter REPL"
    );
    assert!(
        stdout.contains("Component: input3"),
        "Should query component input3"
    );
    assert!(
        stdout.contains("=== SuperPage Metadata Report ==="),
        "Should print full report on 'all'"
    );
    assert!(
        stdout.contains("组件计算优先级分析"),
        "Should include priority analysis when --priority is set"
    );
}
