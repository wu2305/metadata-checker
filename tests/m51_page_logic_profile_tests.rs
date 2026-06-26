#![cfg(feature = "cli-local")]

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphNeighbors, GraphReadStore, GraphStoreResult};
use metadata_checker::query::{
    build_query_page_logic_output, build_query_page_logic_output_profiled,
};
use metadata_checker::scanner::scan_project;
use serde_json::Value;
use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

struct CountingGraphStore<'a> {
    inner: &'a GraphDB,
    edge_reads: Cell<usize>,
}

impl<'a> CountingGraphStore<'a> {
    fn new(inner: &'a GraphDB) -> Self {
        Self {
            inner,
            edge_reads: Cell::new(0),
        }
    }

    fn edge_reads(&self) -> usize {
        self.edge_reads.get()
    }
}

impl GraphReadStore for CountingGraphStore<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<metadata_checker::graph::Node>> {
        GraphReadStore::get_node(self.inner, node_id)
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        self.edge_reads.set(self.edge_reads.get() + 1);
        GraphReadStore::get_node_edges(self.inner, node_id)
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        GraphReadStore::node_count(self.inner)
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        GraphReadStore::edge_count(self.inner)
    }

    fn iter_nodes(
        &self,
    ) -> GraphStoreResult<Box<dyn Iterator<Item = metadata_checker::graph::Node> + '_>> {
        GraphReadStore::iter_nodes(self.inner)
    }
}

fn fixture_graph(test_name: &str) -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m51-{test_name}-{}-{nanos}.db",
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
fn m51_profiled_page_logic_matches_default_output_for_all_budgets() -> anyhow::Result<()> {
    let graph = fixture_graph("budget-output")?;
    let page_id = "page:app/page_relations.spg";

    for budget in ["compact", "normal", "full"] {
        let mut baseline = build_query_page_logic_output(&graph, page_id, None, budget)?;
        let (mut profiled, profile) =
            build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;

        canonicalize_value(&mut baseline);
        canonicalize_value(&mut profiled);
        assert_eq!(
            profiled, baseline,
            "profiled output changed {budget} output"
        );
        assert_eq!(profile.capability, "query_page_logic");
        assert!(profile.total_duration_ms() <= profile.wall_duration_ms);
    }
    Ok(())
}

#[test]
fn m51_profiled_page_logic_records_expected_stage_contract() -> anyhow::Result<()> {
    let graph = fixture_graph("stage-contract")?;
    let page_id = "page:app/page_relations.spg";

    let (_profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "compact")?;

    for stage in [
        "target_lookup",
        "collect_page_nodes",
        "load_page_metadata",
        "entrypoint_scan",
        "edge_scan",
        "action_flow_build",
        "prerequisites",
        "path_summary",
        "diagnostics",
        "key_model_availability",
        "output_build",
        "evidence_build",
        "validation",
    ] {
        assert!(
            profile.stage(stage).is_some(),
            "profile must record {stage}"
        );
    }
    assert!(
        profile.counter("child_components") > 0,
        "fixture page must expose component count in profile counters"
    );
    assert!(
        profile.counter("edges_scanned") > 0,
        "fixture page must expose scanned edge count in profile counters"
    );
    Ok(())
}

#[test]
fn m51_profiled_page_logic_records_path_summary_breakdown() -> anyhow::Result<()> {
    let graph = fixture_graph("path-summary-breakdown")?;
    let page_id = "page:app/page_relations.spg";

    let (_profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "compact")?;

    for counter in [
        "path_anchor_extract_ms",
        "path_candidate_search_ms",
        "path_field_candidate_build_ms",
        "path_dedup_ms",
        "path_classification_ms",
        "path_json_build_ms",
        "path_side_context_ms",
        "path_sort_ms",
    ] {
        assert!(
            profile.counters.contains_key(counter),
            "path summary profile must record {counter}"
        );
    }
    assert!(
        profile.counter("path_candidates_before_dedup") > 0,
        "path summary must expose candidate volume before dedup"
    );
    assert!(
        profile.counter("path_candidates_after_dedup") > 0,
        "path summary must expose candidate volume after dedup"
    );
    Ok(())
}

#[test]
fn m51_profiled_page_logic_records_prerequisite_breakdown() -> anyhow::Result<()> {
    let graph = fixture_graph("prerequisite-breakdown")?;
    let page_id = "page:app/actions_test.spg";

    let (_profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "compact")?;

    for counter in [
        "prerequisites_component_scan_ms",
        "prerequisites_action_scan_ms",
        "prerequisites_data_source_scan_ms",
        "prerequisites_sort_ms",
        "prerequisites_component_nodes",
        "prerequisites_action_nodes",
        "prerequisites_data_source_nodes",
        "prerequisites_unique_conditions",
    ] {
        assert!(
            profile.counters.contains_key(counter),
            "prerequisites profile must record {counter}"
        );
    }
    assert!(
        profile.counter("prerequisites_unique_conditions") > 0,
        "fixture page must expose collected prerequisite conditions"
    );
    Ok(())
}

#[test]
fn m51_profiled_page_logic_records_target_lookup_for_missing_page() -> anyhow::Result<()> {
    let graph = fixture_graph("missing-target")?;
    let page_id = "page:app/__missing_page__.spg";

    let baseline = build_query_page_logic_output(&graph, page_id, None, "compact")?;
    let (profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "compact")?;

    assert_eq!(profiled, baseline);
    assert!(
        profile.stage("target_lookup").is_some(),
        "missing target path must still record target lookup"
    );
    assert_eq!(profile.stages.len(), 1);
    Ok(())
}

#[test]
fn m51_compact_page_logic_only_expands_visible_key_model_availability() -> anyhow::Result<()> {
    let graph = fixture_graph("compact-key-model-limit")?;
    let page_id = "page:app/actions_test.spg";

    let (profiled, profile) =
        build_query_page_logic_output_profiled(&graph, page_id, None, "compact")?;
    let availability = profiled
        .get("details")
        .and_then(|details| details.get("key_model_availability"))
        .expect("compact output must keep key_model_availability");

    assert_eq!(
        availability
            .get("total_count")
            .and_then(|value| value.as_u64()),
        Some(5),
        "compact output must still expose total key model count"
    );
    assert_eq!(
        availability
            .get("shown_count")
            .and_then(|value| value.as_u64()),
        Some(3),
        "compact output should only show the first three key model availability entries"
    );
    assert_eq!(profile.counter("key_models"), 5);
    assert_eq!(
        profile.counter("key_model_availability"),
        3,
        "compact profile should count only expanded availability entries"
    );
    Ok(())
}

#[test]
fn m51_key_model_availability_uses_fast_path_for_expanded_models() -> anyhow::Result<()> {
    let graph = fixture_graph("availability-fast-path")?;
    let page_id = "page:app/actions_test.spg";

    for budget in ["normal", "full"] {
        let (profiled, profile) =
            build_query_page_logic_output_profiled(&graph, page_id, None, budget)?;
        let entries = profiled
            .get("details")
            .and_then(|details| details.get("key_model_availability"))
            .and_then(|value| value.as_array())
            .expect("normal/full page logic output must expand availability entries");

        assert!(
            entries.iter().all(|entry| entry
                .get("availability_summary")
                .and_then(|value| value.get("result"))
                .and_then(|value| value.as_str())
                .is_some()),
            "fast path must preserve availability facts for every expanded model"
        );
        assert_eq!(
            profile.counter("key_model_availability_fast_path"),
            profile.counter("key_model_availability"),
            "{budget} should build every expanded availability entry through the fast path"
        );
    }
    Ok(())
}

#[test]
fn m51_availability_intent_uses_incoming_model_edges_for_page_scope() -> anyhow::Result<()> {
    let graph = fixture_graph("availability-neighbor-walk")?;
    let counting_graph = CountingGraphStore::new(&graph);

    let output = metadata_checker::explain::build_explain_condition_output_with_intent(
        &counting_graph,
        "model:app/actions_test.spg|model1",
        "compact",
        metadata_checker::explain::TraversalIntent::Availability,
    )?;

    assert!(
        output
            .get("details")
            .and_then(|details| details.get("answer_facts"))
            .and_then(|facts| facts.get("availability_facts"))
            .is_some(),
        "availability intent must still return availability facts"
    );
    assert!(
        counting_graph.edge_reads() <= 8,
        "availability intent should avoid repeated page descendant scans, got {} edge reads",
        counting_graph.edge_reads()
    );
    Ok(())
}
