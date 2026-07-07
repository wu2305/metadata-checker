#![cfg(feature = "cli-local")]

use metadata_checker::dense_graph::DenseGraphSnapshot;
use metadata_checker::graph::GraphDB;
use metadata_checker::query::{
    build_query_page_logic_output_profiled,
    build_query_page_logic_output_profiled_with_dense_snapshot,
};
use metadata_checker::scanner::scan_project;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph(test_name: &str) -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-dense-{test_name}-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    GraphDB::open(&db_path)
}

fn canonicalize_value(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                canonicalize_value(item);
            }
            items.sort_by_key(|item| item.to_string());
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                canonicalize_value(item);
            }
        }
        _ => {}
    }
}

/// dense-native path finder 输出必须与 baseline 等价，并记录 CSR 邻接命中。
#[test]
fn m53_dense_path_finder_matches_baseline_and_records_hits() -> anyhow::Result<()> {
    let graph = fixture_graph("path-equiv")?;
    let page_id = "page:app/actions_test.spg";
    let dense = DenseGraphSnapshot::from_graph(&graph)?;

    for budget in ["normal", "compact"] {
        let (mut baseline, _) =
            build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;
        let (mut dense_output, profile) = build_query_page_logic_output_profiled_with_dense_snapshot(
            &graph,
            Some(&dense),
            page_id,
            None,
            budget,
        )?;
        canonicalize_value(&mut baseline);
        canonicalize_value(&mut dense_output);
        assert_eq!(
            baseline, dense_output,
            "{budget} dense path summary must match baseline"
        );
        assert_eq!(
            profile.counter("path_dense_graph_used"),
            1,
            "{budget} must mark dense graph usage"
        );
        assert!(
            profile.counter("path_dense_adjacency_hits") > 0,
            "{budget} must record CSR adjacency hits during candidate search"
        );
    }
    Ok(())
}
