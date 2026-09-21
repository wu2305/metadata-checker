#![cfg(feature = "cli-local")]

use metadata_checker::graph::{GraphDB, NodeType};
use metadata_checker::ownership::ProjectBinding;
use std::collections::HashMap;
use std::path::PathBuf;

fn temp_db(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m59-ownership-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn empty_store_initializes_ownership_binding_and_reopens() {
    let db_path = temp_db("init");
    let binding = ProjectBinding::new("project-a").expect("valid binding");

    let mut graph = GraphDB::open_for_project(&db_path, &binding).expect("initialize store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph.persist(&HashMap::new()).expect("persist graph");

    GraphDB::open_for_project(&db_path, &binding).expect("reopen with original binding");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn populated_legacy_store_cannot_receive_ownership_marker() {
    let db_path = temp_db("legacy");
    let binding = ProjectBinding::new("project-a").expect("valid binding");

    let mut graph = GraphDB::open(&db_path).expect("create legacy store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph
        .persist(&HashMap::new())
        .expect("persist legacy graph");

    let error = match GraphDB::open_for_project(&db_path, &binding) {
        Ok(_) => panic!("legacy populated store must require rebuild"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("GRAPH_OWNERSHIP_SCHEMA_STALE"));
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn project_binding_mismatch_is_rejected_after_restart() {
    let db_path = temp_db("mismatch");
    let first = ProjectBinding::new("project-a").expect("valid binding");
    let second = ProjectBinding::new("project-b").expect("valid binding");

    let mut graph = GraphDB::open_for_project(&db_path, &first).expect("initialize store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph.persist(&HashMap::new()).expect("persist graph");

    let error = match GraphDB::open_for_project(&db_path, &second) {
        Ok(_) => panic!("different project must not open the store"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("GRAPH_PROJECT_BINDING_MISMATCH"));
    let _ = std::fs::remove_file(db_path);
}
