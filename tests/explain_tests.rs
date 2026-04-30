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

#[test]
fn test_explain_model_graph_semantics() {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-explain-model-test.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let result = metadata_checker::explain::explain_node_graph(&graph, "model:model1", false);
    assert!(
        result.is_ok(),
        "explain_node_graph should succeed for model"
    );
}

#[test]
fn test_explain_component_graph_semantics() {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-explain-comp-test.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let result = metadata_checker::explain::explain_node_graph(
        &graph,
        "comp:app/page_relations.spg|button1",
        false,
    );
    assert!(
        result.is_ok(),
        "explain_node_graph should succeed for component"
    );
}

#[test]
fn test_explain_action_graph_semantics() {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-explain-action-test.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let result = metadata_checker::explain::explain_node_graph(
        &graph,
        "action:app/page_relations.spg|button1|action1",
        false,
    );
    assert!(
        result.is_ok(),
        "explain_node_graph should succeed for action"
    );
}

#[test]
fn test_explain_page_graph_semantics() {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-explain-page-test.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let result =
        metadata_checker::explain::explain_node_graph(&graph, "page:app/page_relations.spg", false);
    assert!(result.is_ok(), "explain_node_graph should succeed for page");
}

#[test]
fn test_explain_not_found_returns_error() {
    use metadata_checker::graph::GraphDB;
    use metadata_checker::scanner::scan_project;
    use std::path::Path;

    let db_path = std::env::temp_dir().join("metadata-checker-explain-nf-test.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");
    scan_project(project_dir, &db_path).expect("scan_project failed");
    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let result = metadata_checker::explain::explain_node_graph(&graph, "model:nonexistent", false);
    assert!(
        result.is_err(),
        "explain_node_graph should fail for nonexistent node"
    );
}
