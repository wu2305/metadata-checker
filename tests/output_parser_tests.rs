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
    assert_eq!(json.get("version").and_then(|v| v.as_str()), Some("4.19.7"));
    assert!(
        json.get("components").is_some(),
        "Should have components array"
    );
    assert!(
        json.get("expressions").is_some(),
        "Should have expressions array"
    );
    assert!(
        json.get("dependency_order").is_some(),
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
        json.get("component_id").and_then(|v| v.as_str()),
        Some("input3")
    );
    assert!(json.get("expressions").is_some(), "Should have expressions");
    assert!(json.get("value_trace").is_some(), "Should have value_trace");
}
