use metadata_checker::graph::{EdgeType, GraphDB};
use metadata_checker::scanner::scan_project;
use metadata_checker::superpage::{RefType, parse_superpage};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "metadata-checker-{}-{}-{}",
        test_name,
        std::process::id(),
        nanos
    ));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir should be created");
    }
    fs::write(path, content).expect("fixture file should be written");
}

fn simple_spg(model: &str, field: &str) -> String {
    format!(
        r#"{{
  "version": "4.19.7",
  "canvas": {{
    "id": "canvas",
    "type": "canvas",
    "components": [
      {{
        "id": "input1",
        "type": "input",
        "submitField": "{model}.{field}",
        "value": "={model}.{field}"
      }},
      {{
        "id": "text1",
        "type": "text",
        "value": "=input1.value"
      }}
    ]
  }}
}}"#
    )
}

fn page_has_contains_edge(graph: &GraphDB, page_id: &str, component_name: &str) -> bool {
    graph.get_node_edges(page_id).is_some_and(|(outgoing, _)| {
        outgoing.iter().any(|(node, edge)| {
            node.name == component_name && matches!(edge.edge_type, EdgeType::Contains)
        })
    })
}

fn model_has_writer(graph: &GraphDB, model_id: &str, writer_id: &str) -> bool {
    graph
        .find_writers(model_id)
        .iter()
        .any(|(node, _)| node.id == writer_id)
}

#[test]
fn test_incremental_deleted_file_removes_nodes_and_state() {
    let project_dir = unique_temp_dir("deleted-file");
    let db_path = project_dir.join(".metadata-checker.graphdb");
    let first_page = project_dir.join("app/main.spg");
    let deleted_page = project_dir.join("app/delete_me.spg");

    write_file(&first_page, &simple_spg("model1", "name"));
    write_file(&deleted_page, &simple_spg("model2", "status"));

    scan_project(&project_dir, &db_path).expect("initial scan should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open after initial scan");
    assert!(
        graph.get_node("page:app/delete_me.spg").is_some(),
        "delete_me page should exist after initial scan"
    );
    drop(graph);

    fs::remove_file(&deleted_page).expect("fixture page should be removable");
    scan_project(&project_dir, &db_path).expect("scan after deletion should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open after deletion scan");
    assert_eq!(
        graph.get_node("page:app/delete_me.spg").is_none(),
        true,
        "deleted page node should be removed from graph"
    );
    drop(graph);

    scan_project(&project_dir, &db_path).expect("second deletion scan should not repeat deletion");
    let graph = GraphDB::open(&db_path).expect("graph should open after second scan");
    assert!(
        page_has_contains_edge(&graph, "page:app/main.spg", "input1"),
        "unrelated page edges should remain after deleting another file"
    );

    let _ = fs::remove_dir_all(project_dir);
}

#[test]
fn test_project_local_graphdb_is_not_scanned_as_metadata() {
    let project_dir = unique_temp_dir("local-graphdb");
    let db_path = project_dir.join(".metadata-checker.graphdb");
    write_file(
        &project_dir.join("app/main.spg"),
        &simple_spg("model1", "name"),
    );

    scan_project(&project_dir, &db_path).expect("initial scan should succeed");
    scan_project(&project_dir, &db_path).expect("second scan should ignore graphdb file");

    let graph = GraphDB::open(&db_path).expect("graph should open after repeated scan");
    assert!(
        page_has_contains_edge(&graph, "page:app/main.spg", "input1"),
        "project-local graphdb must not interfere with page graph"
    );

    let _ = fs::remove_dir_all(project_dir);
}

#[test]
fn test_invalid_tbl_does_not_abort_project_scan() {
    let project_dir = unique_temp_dir("invalid-tbl");
    let db_path = project_dir.join(".metadata-checker.graphdb");
    write_file(
        &project_dir.join("app/main.spg"),
        &simple_spg("model1", "name"),
    );
    write_file(&project_dir.join("app/broken.tbl"), "{ invalid json");

    scan_project(&project_dir, &db_path).expect("invalid tbl should be skipped, not abort scan");
    let graph = GraphDB::open(&db_path).expect("graph should open after scan with invalid tbl");
    assert!(
        page_has_contains_edge(&graph, "page:app/main.spg", "text1"),
        "valid spg should still be indexed when another tbl is invalid"
    );

    let _ = fs::remove_dir_all(project_dir);
}

#[test]
fn test_expression_parser_handles_escaped_quotes_inside_string_literals() {
    let refs = metadata_checker::superpage::parse_expression_refs(
        r#"=IF(input1.value = 'it\'s model2.name', model1.amount, 0)"#,
    );

    assert!(
        refs.contains(&RefType::ComponentValue("input1".to_string())),
        "component reference outside string literal should be detected"
    );
    assert!(
        refs.iter()
            .any(|r| matches!(r, RefType::ModelField(model, field) if model == "model1" && field == "amount")),
        "model reference outside string literal should be detected"
    );
    assert!(
        !refs
            .iter()
            .any(|r| matches!(r, RefType::ModelField(model, field) if model == "model2" && field == "name")),
        "model reference inside escaped string literal should be ignored"
    );
}

#[test]
fn test_incremental_dirty_file_rebuild_preserves_edges() {
    let project_dir = unique_temp_dir("dirty-rebuild");
    let db_path = project_dir.join(".metadata-checker.graphdb");
    let page_path = project_dir.join("app/main.spg");

    write_file(&page_path, &simple_spg("model1", "name"));
    scan_project(&project_dir, &db_path).expect("initial scan should succeed");

    let graph = GraphDB::open(&db_path).expect("graph should open after initial scan");
    assert!(
        page_has_contains_edge(&graph, "page:app/main.spg", "input1"),
        "initial scan should create page -> component edge"
    );
    assert!(
        model_has_writer(&graph, "model:model1", "comp:app/main.spg|input1"),
        "initial scan should create submitField write edge"
    );
    drop(graph);

    write_file(&page_path, &simple_spg("model1", "name_with_suffix"));
    scan_project(&project_dir, &db_path).expect("dirty scan should succeed");

    let graph = GraphDB::open(&db_path).expect("graph should open after dirty scan");
    assert!(
        page_has_contains_edge(&graph, "page:app/main.spg", "input1"),
        "dirty rebuild must preserve page -> component edge"
    );
    assert!(
        model_has_writer(&graph, "model:model1", "comp:app/main.spg|input1"),
        "dirty rebuild must preserve submitField write edge"
    );

    let _ = fs::remove_dir_all(project_dir);
}

#[test]
fn test_additional_expression_fields_are_extracted() {
    let dir = unique_temp_dir("extra-fields");
    let path = dir.join("extra_fields.spg");
    write_file(
        &path,
        r#"{
  "version": "4.19.7",
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {
        "id": "input1",
        "type": "input",
        "value": "=model1.amount",
        "calcCondition": "=input2.value > 0",
        "itemFilter": "=model2.status",
        "validExp": "=input3.value != ''"
      },
      { "id": "input2", "type": "input", "value": "=1" },
      { "id": "input3", "type": "input", "value": "=2" }
    ]
  }
}"#,
    );

    let meta = parse_superpage(&path).expect("superpage with extra expression fields should parse");
    for field in ["value", "calcCondition", "itemFilter", "validExp"] {
        assert!(
            meta.expressions
                .iter()
                .any(|expr| expr.component_id == "input1" && expr.field == field),
            "{field} should be extracted as an expression field"
        );
    }

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_same_size_quick_file_change_is_detected() {
    let project_dir = unique_temp_dir("same-size-change");
    let db_path = project_dir.join(".metadata-checker.graphdb");
    let page_path = project_dir.join("app/main.spg");

    write_file(&page_path, &simple_spg("model1", "name"));
    scan_project(&project_dir, &db_path).expect("initial scan should succeed");

    let first_size = fs::metadata(&page_path)
        .expect("metadata should exist")
        .len();
    let replacement = simple_spg("model2", "code");
    assert_eq!(
        replacement.len() as u64,
        first_size,
        "fixture must stay same size to verify hash/mtime behavior"
    );
    write_file(&page_path, &replacement);
    scan_project(&project_dir, &db_path).expect("same-size change scan should succeed");

    let graph = GraphDB::open(&db_path).expect("graph should open after same-size scan");
    assert!(
        graph.get_node("model:model2").is_some(),
        "same-size content change should be detected and indexed"
    );

    let _ = fs::remove_dir_all(project_dir);
}

#[test]
fn debug_parse_refs() {
    let refs = metadata_checker::superpage::parse_expression_refs("=myinput1.value + input1.value");
    println!("refs: {:?}", refs);
    assert_eq!(refs.len(), 2);
}
