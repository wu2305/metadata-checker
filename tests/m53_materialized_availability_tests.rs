#![cfg(feature = "cli-local")]

use metadata_checker::graph::GraphDB;
use metadata_checker::query::{
    MaterializedAvailabilityFactsIndex, build_query_page_logic_output_profiled,
    build_query_page_logic_output_profiled_with_materialized_availability,
};
use metadata_checker::scanner::scan_project;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph(test_name: &str) -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-{test_name}-{}-{nanos}.db",
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

/// 验证物化索引与 baseline 输出等价，且命中 materialized condition 缓存。
#[test]
fn m53_materialized_index_matches_baseline_output() -> anyhow::Result<()> {
    let graph = fixture_graph("materialized-equiv")?;
    let page_id = "page:app/actions_test.spg";
    let index = MaterializedAvailabilityFactsIndex::build(&graph)?;
    assert!(
        index.node_count > 0,
        "fixture graph should precollect at least one conditioned node"
    );

    for budget in ["normal", "compact"] {
        let (mut baseline, _) =
            build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;
        let (mut indexed, profile) =
            build_query_page_logic_output_profiled_with_materialized_availability(
                &graph,
                Some(&index),
                page_id,
                None,
                budget,
            )?;
        canonicalize_value(&mut baseline);
        canonicalize_value(&mut indexed);
        assert_eq!(
            baseline, indexed,
            "{budget} output must match with materialized availability index"
        );
        assert!(
            profile.counter("availability_materialized_hits") > 0,
            "{budget} should record materialized condition cache hits"
        );
    }
    Ok(())
}
