#![cfg(feature = "cli-local")]

use metadata_checker::dense_graph::DenseGraphSnapshot;
use metadata_checker::graph::GraphDB;
use metadata_checker::query::{
    build_page_logic_availability_cache, build_query_page_logic_output_profiled,
    build_query_page_logic_output_profiled_with_availability_cache,
    build_query_page_logic_output_profiled_with_dense_snapshot,
};
use metadata_checker::scanner::scan_project;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph(test_name: &str) -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m52-{test_name}-{}-{nanos}.db",
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

#[test]
fn m52_page_logic_records_bulk_availability_counters() -> anyhow::Result<()> {
    let graph = fixture_graph("bulk-availability-counters")?;
    let page_id = "page:app/actions_test.spg";

    for budget in ["normal", "full"] {
        let (profiled, profile) =
            build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;
        let entries = profiled
            .get("details")
            .and_then(|details| details.get("key_model_availability"))
            .and_then(|value| value.as_array())
            .expect("normal/full page logic must expand key model availability");

        let expanded_count = entries.len() as u64;
        assert_eq!(
            entries.is_empty(),
            false,
            "fixture must exercise availability"
        );
        assert_eq!(
            profile.counter("key_model_availability_fast_path"),
            expanded_count,
            "every emitted model must use the availability fast path"
        );
        assert_eq!(
            profile.counter("availability_models_batched"),
            expanded_count,
            "{budget} should report every expanded availability entry as batched"
        );
        assert_eq!(
            profile
                .counters
                .contains_key("availability_context_build_ms"),
            true,
            "{budget} should record page-local availability context build time"
        );
        assert!(
            profile.counter("availability_condition_groups") >= expanded_count,
            "{budget} should record condition groups considered by availability context"
        );
        let fallback_count = entries
            .iter()
            .filter(|entry| {
                entry["scope_warning"].as_str()
                    == Some("page_scoped_target_not_resolved_fallback_to_global_model")
            })
            .count() as u64;
        assert_eq!(
            profile.counter("availability_fallback_models"),
            fallback_count,
            "{budget} fallback count must match emitted model scope warnings"
        );
        assert_eq!(
            profile.counters.contains_key("availability_index_build_ms"),
            true,
            "{budget} should record availability index build time"
        );
        assert_eq!(
            profile
                .counters
                .contains_key("availability_index_projection_ms"),
            true,
            "{budget} should record availability index projection time"
        );
        assert_eq!(
            profile.counter("availability_index_fallback_models"),
            profile.counter("availability_fallback_models"),
            "{budget} availability index fallback count must match projected fallback count"
        );
    }
    Ok(())
}

#[test]
fn m52_page_logic_records_path_context_cache_counters() -> anyhow::Result<()> {
    let graph = fixture_graph("path-context-cache")?;
    let page_id = "page:app/actions_test.spg";

    let (_profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "normal")?;

    assert!(
        profile.counter("path_graph_cache_hits") > 0,
        "path_summary should report reused graph reads from its page-local context"
    );
    assert_eq!(
        profile.counters.contains_key("path_context_build_ms"),
        true,
        "path_summary should record page-local path context build time"
    );
    assert_eq!(
        profile.counter("path_dense_graph_used"),
        0,
        "default query path must not build a full dense snapshot per request"
    );
    assert_eq!(
        profile.counter("dense_snapshot_build_ms"),
        0,
        "dense snapshot build cost belongs to runtime/load, not query-time path_summary"
    );
    Ok(())
}

#[test]
fn m52_page_logic_uses_prebuilt_dense_snapshot_without_changing_output() -> anyhow::Result<()> {
    let graph = fixture_graph("path-dense-override")?;
    let page_id = "page:app/actions_test.spg";
    let dense_snapshot = DenseGraphSnapshot::from_graph(&graph)?;

    let (mut baseline, _) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "normal")?;
    let (mut profiled, profile) = build_query_page_logic_output_profiled_with_dense_snapshot(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        "normal",
    )?;

    canonicalize_value(&mut baseline);
    canonicalize_value(&mut profiled);
    assert_eq!(
        profiled, baseline,
        "prebuilt dense snapshot must not change page logic output"
    );
    assert_eq!(
        profile.counter("path_dense_graph_used"),
        1,
        "path_summary should use the provided dense snapshot"
    );
    assert_eq!(
        profile.counter("dense_snapshot_nodes"),
        dense_snapshot.dense_node_count() as u64
    );
    assert_eq!(
        profile.counter("dense_snapshot_edges"),
        dense_snapshot.dense_edge_count() as u64
    );
    Ok(())
}

#[test]
fn m52_page_logic_uses_warmed_availability_cache_without_changing_output() -> anyhow::Result<()> {
    let graph = fixture_graph("availability-cache-override")?;
    let page_id = "page:app/actions_test.spg";
    let budget = "normal";
    let availability_cache =
        build_page_logic_availability_cache(&graph, None, page_id, None, budget, None)?;

    let (mut baseline, baseline_profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;
    let (mut cached, cached_profile) =
        build_query_page_logic_output_profiled_with_availability_cache(
            &graph,
            None,
            Some(&availability_cache),
            None,
            page_id,
            None,
            budget,
        )?;

    canonicalize_value(&mut baseline);
    canonicalize_value(&mut cached);
    assert_eq!(
        cached, baseline,
        "warmed availability cache must not change page logic output"
    );
    assert_eq!(
        cached_profile.counter("availability_read_model_used"),
        1,
        "page logic must consume warmed availability cache"
    );
    assert_eq!(
        cached_profile.counter("availability_index_build_ms"),
        0,
        "cached availability should remove query-time availability index build"
    );
    assert_eq!(
        cached_profile.counter("key_model_availability"),
        baseline_profile.counter("key_model_availability"),
        "cached availability should preserve expanded model count"
    );
    assert!(
        availability_cache.warm_stage_ms("path_summary").is_some(),
        "availability warm must materialize path_summary read model"
    );
    assert!(
        availability_cache.warm_stage_ms("prerequisites").is_some(),
        "availability warm must materialize prerequisites read model"
    );
    assert!(
        availability_cache
            .warm_stage_ms("availability_materialize")
            .is_some(),
        "availability warm should record materialized facts stage"
    );
    assert_eq!(
        cached_profile.counter("prerequisites_read_model_used"),
        1,
        "page logic must consume warmed prerequisites cache"
    );
    assert_eq!(
        cached_profile.counter("path_read_model_used"),
        1,
        "page logic must consume warmed path_summary cache"
    );
    Ok(())
}

#[test]
fn m52_page_logic_extended_warm_cache_preserves_compact_output() -> anyhow::Result<()> {
    let graph = fixture_graph("extended-warm-compact")?;
    let page_id = "page:app/actions_test.spg";
    let budget = "compact";
    let dense_snapshot = DenseGraphSnapshot::from_graph(&graph)?;
    let availability_cache = build_page_logic_availability_cache(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        budget,
        None,
    )?;

    let (mut baseline, _) = build_query_page_logic_output_profiled_with_dense_snapshot(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        budget,
    )?;
    let (mut cached, cached_profile) =
        build_query_page_logic_output_profiled_with_availability_cache(
            &graph,
            Some(&dense_snapshot),
            Some(&availability_cache),
            None,
            page_id,
            None,
            budget,
        )?;

    canonicalize_value(&mut baseline);
    canonicalize_value(&mut cached);
    assert_eq!(
        cached, baseline,
        "extended warm cache must not change compact page logic output"
    );
    assert_eq!(cached_profile.counter("availability_read_model_used"), 1);
    assert_eq!(cached_profile.counter("prerequisites_read_model_used"), 1);
    assert_eq!(cached_profile.counter("path_read_model_used"), 1);
    Ok(())
}
