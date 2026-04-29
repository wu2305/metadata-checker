use metadata_checker::graph::GraphDB;
use metadata_checker::scanner::scan_project;
use std::path::{Path, PathBuf};

fn unique_temp_dir(test_name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("metadata-checker-{}", test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_fixture_project(temp_dir: &Path) {
    let src = PathBuf::from("tests/fixtures/test_project");
    for entry in std::fs::read_dir(&src).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == ".metadata-checker.graphdb" {
            continue;
        }
        let dest = temp_dir.join(entry.file_name());
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn test_df_a_dataflow_outputs_contains_physical_x() {
    let temp_dir = unique_temp_dir("df-a-outputs");
    copy_fixture_project(&temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");

    scan_project(&temp_dir, &db_path).expect("scan should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open");

    let outputs = graph.find_dataflow_outputs("model:df_a");
    let has_physical_x = outputs.iter().any(|(n, _)| n.id == "model:physical_x");
    assert!(
        has_physical_x,
        "df_a dataflow_outputs should contain physical_x"
    );
}

#[test]
fn test_df_b_dataflow_inputs_contains_physical_x() {
    let temp_dir = unique_temp_dir("df-b-inputs");
    copy_fixture_project(&temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");

    scan_project(&temp_dir, &db_path).expect("scan should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open");

    let inputs = graph.find_dataflow_inputs("model:df_b");
    let has_physical_x = inputs.iter().any(|(n, _)| n.id == "model:physical_x");
    assert!(
        has_physical_x,
        "df_b dataflow_inputs should contain physical_x"
    );
}

#[test]
fn test_physical_x_produced_by_contains_df_a() {
    let temp_dir = unique_temp_dir("physical-x-produced");
    copy_fixture_project(&temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");

    scan_project(&temp_dir, &db_path).expect("scan should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open");

    let producers = graph.find_produced_by("model:physical_x");
    let has_df_a = producers.iter().any(|(n, _)| n.id == "model:df_a");
    assert!(has_df_a, "physical_x produced_by should contain df_a");
}

#[test]
fn test_physical_x_consumed_by_dataflows_contains_df_b() {
    let temp_dir = unique_temp_dir("physical-x-consumed");
    copy_fixture_project(&temp_dir);
    let db_path = temp_dir.join(".metadata-checker.graphdb");

    scan_project(&temp_dir, &db_path).expect("scan should succeed");
    let graph = GraphDB::open(&db_path).expect("graph should open");

    let consumers = graph.find_consumed_by_dataflows("model:physical_x");
    let has_df_b = consumers.iter().any(|(n, _)| n.id == "model:df_b");
    assert!(
        has_df_b,
        "physical_x consumed_by_dataflows should contain df_b"
    );
}
