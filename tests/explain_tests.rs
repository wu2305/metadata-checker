use metadata_checker::explain::explain_component_spg;
use metadata_checker::parser::parse_file;
use std::path::PathBuf;

#[test]
fn test_explain_component_json() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");

    let _buf: Vec<u8> = Vec::new();
    explain_component_spg(&spg, "input1", false).expect("Should explain component");
    // Since explain_component_spg prints to stdout for non-human, we can't easily capture it
    // without modifying the function. For now, just verify it doesn't panic.
}

#[test]
fn test_explain_component_found() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");

    let result = explain_component_spg(&spg, "input1", false);
    assert!(result.is_ok(), "Should explain existing component");
}

#[test]
fn test_explain_component_not_found() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_file(&path).expect("Failed to parse");
    let spg = meta.superpage.expect("Should be SuperPage");

    let result = explain_component_spg(&spg, "nonexistent", false);
    assert!(result.is_err(), "Should fail for nonexistent component");
}
