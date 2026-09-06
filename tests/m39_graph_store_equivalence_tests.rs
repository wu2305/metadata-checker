#![cfg(feature = "cli-local")]

#[path = "common/memory_graph_store.rs"]
mod memory_graph_store;

use memory_graph_store::MemoryGraphStore;
use metadata_checker::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};
use serde_json::Value;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn build_contract_graph_for_backend<S: GraphWriteStore>(graph: &mut S) {
    GraphWriteStore::upsert_node(
        graph,
        Node {
            id: "page:app/home.spg".to_string(),
            node_type: NodeType::Page,
            path: "app/home.spg".to_string(),
            name: "首页".to_string(),
            meta: None,
        },
    )
    .expect("insert page node");

    GraphWriteStore::upsert_node(
        graph,
        Node {
            id: "comp:app/home.spg|input1".to_string(),
            node_type: NodeType::Component,
            path: "app/home.spg".to_string(),
            name: "input1".to_string(),
            meta: Some(serde_json::json!({
                "component_type": "Input",
            })),
        },
    )
    .expect("insert component node");

    GraphWriteStore::upsert_node(
        graph,
        Node {
            id: "action:app/home.spg|input1|action1".to_string(),
            node_type: NodeType::Action,
            path: "app/home.spg".to_string(),
            name: "setParamValue:action1".to_string(),
            meta: None,
        },
    )
    .expect("insert action node");

    GraphWriteStore::upsert_node(
        graph,
        Node {
            id: "model:user".to_string(),
            node_type: NodeType::Model,
            path: "tables/user.tbl".to_string(),
            name: "用户".to_string(),
            meta: None,
        },
    )
    .expect("insert model node");

    GraphWriteStore::add_edge(
        graph,
        Edge {
            from: "page:app/home.spg".to_string(),
            to: "comp:app/home.spg|input1".to_string(),
            edge_type: EdgeType::Contains,
            field_path: None,
            meta: None,
        },
    )
    .expect("add contains edge");

    GraphWriteStore::add_edge(
        graph,
        Edge {
            from: "comp:app/home.spg|input1".to_string(),
            to: "model:user".to_string(),
            edge_type: EdgeType::Reads,
            field_path: Some("user.name".to_string()),
            meta: Some(serde_json::json!({
                "target_model": "user",
            })),
        },
    )
    .expect("add read edge");

    GraphWriteStore::add_edge(
        graph,
        Edge {
            from: "comp:app/home.spg|input1".to_string(),
            to: "action:app/home.spg|input1|action1".to_string(),
            edge_type: EdgeType::Triggers,
            field_path: None,
            meta: None,
        },
    )
    .expect("add trigger edge");

    GraphWriteStore::add_edge(
        graph,
        Edge {
            from: "action:app/home.spg|input1|action1".to_string(),
            to: "model:user".to_string(),
            edge_type: EdgeType::ActionWrites,
            field_path: Some("user.name".to_string()),
            meta: Some(serde_json::json!({"write_target": "user.name"})),
        },
    )
    .expect("add write edge");
}

fn build_memory_graph() -> MemoryGraphStore {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:app/home.spg", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node(
        "comp:app/home.spg|input1",
        "input1",
        NodeType::Component,
        "app/home.spg",
    );
    store.add_test_node(
        "action:app/home.spg|input1|action1",
        "setParamValue:action1",
        NodeType::Action,
        "app/home.spg",
    );
    store.add_test_node("model:user", "用户", NodeType::Model, "tables/user.tbl");

    store.add_test_edge(
        "page:app/home.spg",
        "comp:app/home.spg|input1",
        EdgeType::Contains,
        None,
    );
    store.add_test_edge(
        "comp:app/home.spg|input1",
        "model:user",
        EdgeType::Reads,
        Some("user.name"),
    );
    store.add_test_edge(
        "comp:app/home.spg|input1",
        "action:app/home.spg|input1|action1",
        EdgeType::Triggers,
        None,
    );
    store.add_test_edge(
        "action:app/home.spg|input1|action1",
        "model:user",
        EdgeType::ActionWrites,
        Some("user.name"),
    );
    store
}

fn build_graphdb_graph() -> (GraphDB, PathBuf) {
    let db_path = std::env::temp_dir().join(format!(
        "m39_eq_{}_{}.graphdb",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path);
    let mut graph = GraphDB::open(&db_path).expect("open graphdb");
    build_contract_graph_for_backend(&mut graph);
    (graph, db_path)
}

fn cleanup_graphdb_path(db_path: &PathBuf) {
    let lock_path = db_path.with_extension("graphdb.lock");
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(lock_path);
}

fn assert_truncated_array_or_array_total_count(value: &Value, fallback_key: &str) -> usize {
    if let Some(obj) = value.as_object() {
        return obj
            .get("total_count")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                obj.get("items")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len() as u64)
            })
            .unwrap_or(0) as usize;
    }
    if let Some(arr) = value.as_array() {
        return arr.len();
    }
    panic!("unexpected category value for {fallback_key}");
}

#[test]
fn test_m39_query_model_equivalence_between_memory_graph_and_graphdb() {
    let memory_graph = build_memory_graph();
    let memory_output = metadata_checker::query::build_query_model_output(
        &memory_graph as &dyn GraphReadStore,
        "model:user",
        "compact",
    )
    .expect("query model from memory graph");

    let (graph_db, db_path) = build_graphdb_graph();
    let graphdb_output = metadata_checker::query::build_query_model_output(
        &graph_db as &dyn GraphReadStore,
        "model:user",
        "compact",
    )
    .expect("query model from graphdb");

    assert_eq!(memory_output.get("kind"), graphdb_output.get("kind"));
    assert_eq!(
        memory_output.get("query_target"),
        graphdb_output.get("query_target")
    );
    let memory_summary = memory_output
        .get("summary")
        .expect("memory summary exists")
        .as_object()
        .expect("summary object");
    let graphdb_summary = graphdb_output
        .get("summary")
        .expect("graphdb summary exists")
        .as_object()
        .expect("summary object");

    for key in [
        "model_id",
        "read_by_count",
        "written_by_count",
        "consumed_by_dataflow_count",
        "produced_by_count",
        "dataflow_input_count",
        "dataflow_output_count",
        "dataflow_role",
    ] {
        assert_eq!(
            memory_summary.get(key),
            graphdb_summary.get(key),
            "summary field mismatch: {key}"
        );
    }

    let memory_details = memory_output
        .get("details")
        .expect("memory details exists")
        .as_object()
        .expect("details object");
    let graphdb_details = graphdb_output
        .get("details")
        .expect("graphdb details exists")
        .as_object()
        .expect("details object");

    for key in [
        "readers",
        "writers",
        "dataflow_inputs",
        "dataflow_outputs",
        "produced_by",
        "consumed_by_dataflows",
        "upstream_dependencies",
        "downstream_outputs",
    ] {
        assert_eq!(
            assert_truncated_array_or_array_total_count(
                memory_details.get(key).expect("memory key"),
                key
            ),
            assert_truncated_array_or_array_total_count(
                graphdb_details.get(key).expect("graphdb key"),
                key
            ),
            "detail total_count mismatch for {key}"
        );
    }

    assert_eq!(
        memory_output
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .expect("memory diagnostics")
            .len(),
        graphdb_output
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .expect("graphdb diagnostics")
            .len()
    );
    assert_eq!(
        memory_output
            .get("next_queries")
            .and_then(|v| v.as_array())
            .expect("memory next queries")
            .len(),
        graphdb_output
            .get("next_queries")
            .and_then(|v| v.as_array())
            .expect("graphdb next queries")
            .len()
    );

    cleanup_graphdb_path(&db_path);
}

#[test]
fn test_m39_query_page_logic_equivalence_between_memory_graph_and_graphdb() {
    let memory_graph = build_memory_graph();
    let memory_output = metadata_checker::query::build_query_page_logic_output(
        &memory_graph as &dyn GraphReadStore,
        "page:app/home.spg",
        None,
        "compact",
    )
    .expect("query page logic from memory graph");

    let (graph_db, db_path) = build_graphdb_graph();
    let graphdb_output = metadata_checker::query::build_query_page_logic_output(
        &graph_db as &dyn GraphReadStore,
        "page:app/home.spg",
        None,
        "compact",
    )
    .expect("query page logic from graphdb");

    assert_eq!(memory_output.get("kind"), graphdb_output.get("kind"));
    assert_eq!(
        memory_output.get("query_target"),
        graphdb_output.get("query_target")
    );

    let memory_summary = memory_output
        .get("summary")
        .expect("memory summary exists")
        .as_object()
        .expect("summary object");
    let graphdb_summary = graphdb_output
        .get("summary")
        .expect("graphdb summary exists")
        .as_object()
        .expect("summary object");
    for key in [
        "page_id",
        "page_name",
        "entrypoint_count",
        "data_source_count",
        "write_target_count",
        "navigation_count",
        "page_jump_count",
        "page_embed_count",
        "risk_count",
    ] {
        assert_eq!(
            memory_summary.get(key),
            graphdb_summary.get(key),
            "summary field mismatch: {key}"
        );
    }

    let memory_details = memory_output
        .get("details")
        .expect("memory details exists")
        .as_object()
        .expect("details object");
    let graphdb_details = graphdb_output
        .get("details")
        .expect("graphdb details exists")
        .as_object()
        .expect("details object");
    for key in [
        "page_inputs",
        "data_sources",
        "write_targets",
        "entrypoints",
        "action_flows",
        "visibility_rules",
        "navigation",
    ] {
        assert_eq!(
            assert_truncated_array_or_array_total_count(
                memory_details.get(key).expect("memory key"),
                key
            ),
            assert_truncated_array_or_array_total_count(
                graphdb_details.get(key).expect("graphdb key"),
                key
            ),
            "page logic detail key mismatch: {key}"
        );
    }

    assert_eq!(
        memory_output
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .expect("memory diagnostics")
            .len(),
        graphdb_output
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .expect("graphdb diagnostics")
            .len()
    );
    assert_eq!(
        memory_output
            .get("next_queries")
            .and_then(|v| v.as_array())
            .expect("memory next queries")
            .len(),
        graphdb_output
            .get("next_queries")
            .and_then(|v| v.as_array())
            .expect("graphdb next queries")
            .len()
    );

    cleanup_graphdb_path(&db_path);
}
