#[path = "common/memory_graph_store.rs"]
mod memory_graph_store;

use memory_graph_store::MemoryGraphStore;
use metadata_checker::graph::{EdgeType, Node, NodeType};
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};

fn build_contract_graph() -> MemoryGraphStore {
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
    store.add_test_node(
        "field:user.name",
        "name",
        NodeType::Field,
        "tables/user.tbl",
    );

    GraphWriteStore::upsert_node(
        &mut store,
        Node {
            id: "model:df_user".to_string(),
            node_type: NodeType::Model,
            path: "tables/df_user.tbl".to_string(),
            name: "df_user".to_string(),
            meta: Some(serde_json::json!({
                "modelType": "DataFlow",
                "dimensions": [
                    {"name": "name", "dbfield": "name", "inputField": "name"}
                ]
            })),
            origin_file: None,
        },
    )
    .expect("insert dataflow node");
    store.add_test_node(
        "model:df_user_output",
        "df_user_output",
        NodeType::Model,
        "tables/df_user_output.tbl",
    );

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
        "field:user.name",
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
    store.add_test_edge("model:user", "field:user.name", EdgeType::Contains, None);
    store.add_test_edge(
        "model:df_user",
        "model:user",
        EdgeType::DataflowInput,
        Some("tables/user.tbl"),
    );
    store.add_test_edge(
        "model:df_user",
        "model:df_user_output",
        EdgeType::OutputsTo,
        Some("df_user_output"),
    );

    store
}

fn assert_ai_output_contract(value: &serde_json::Value, expected_target: &str) {
    assert!(value.get("kind").and_then(|v| v.as_str()).is_some());
    assert_eq!(
        value.get("query_target").and_then(|v| v.as_str()),
        Some(expected_target)
    );
    assert!(value.get("summary").and_then(|v| v.as_object()).is_some());
    assert!(value.get("details").is_some());
    assert!(
        value
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .is_some()
    );
    assert!(value.get("evidence").and_then(|v| v.as_array()).is_some());
    assert!(
        value
            .get("next_queries")
            .and_then(|v| v.as_array())
            .is_some()
    );
}

fn assert_target_not_found_contract(
    value: &serde_json::Value,
    expected_target: &str,
    expected_kind: &str,
) {
    assert_eq!(
        value.get("kind").and_then(|v| v.as_str()),
        Some(expected_kind)
    );
    assert_eq!(
        value.get("query_target").and_then(|v| v.as_str()),
        Some(expected_target)
    );
    assert_eq!(
        value
            .get("summary")
            .and_then(|v| v.get("target_id"))
            .and_then(|v| v.as_str()),
        Some(expected_target)
    );

    let diagnostics = value
        .get("diagnostics")
        .and_then(|v| v.as_array())
        .expect("diagnostics should exist");
    assert!(
        !diagnostics.is_empty(),
        "target-not-found path should carry diagnostics"
    );
    let has_target_not_found = diagnostics
        .iter()
        .any(|diag| diag.get("code").and_then(|v| v.as_str()) == Some("TARGET_NOT_FOUND"));
    assert!(
        has_target_not_found,
        "target-not-found should emit TARGET_NOT_FOUND diagnostic"
    );
}

#[test]
fn test_m39_query_model_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::query::build_query_model_output(graph, "model:user", "compact")
        .expect("query_model should run on GraphReadStore");

    assert_ai_output_contract(&value, "model:user");
}

#[test]
fn test_m39_query_dataflow_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::query::build_query_dataflow_output(graph, "model:df_user")
        .expect("query_dataflow should run on GraphReadStore");

    assert_ai_output_contract(&value, "model:df_user");
}

#[test]
fn test_m39_query_page_logic_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::query::build_query_page_logic_output(
        graph,
        "page:app/home.spg",
        None,
        "compact",
    )
    .expect("query_page_logic should run on GraphReadStore");

    assert_ai_output_contract(&value, "page:app/home.spg");
    assert!(
        value
            .get("details")
            .and_then(|v| v.get("data_sources"))
            .and_then(|v| v.get("items"))
            .and_then(|v| v.as_array())
            .is_some(),
        "compact data_sources should keep truncated array structure"
    );
}

#[test]
fn test_m39_context_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::context::build_context_output(graph, "model:user", 1, "compact")
        .expect("context should run on GraphReadStore");

    assert_ai_output_contract(&value, "model:user");
}

#[test]
fn test_m39_explain_condition_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value =
        metadata_checker::explain::build_explain_condition_output(graph, "model:user", "compact")
            .expect("explain_condition should run on GraphReadStore");

    assert_ai_output_contract(&value, "model:user");
}

#[test]
fn test_m39_explain_graph_store_schema_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::explain::build_explain_output(graph, "model:user")
        .expect("explain should run on GraphReadStore");

    assert_ai_output_contract(&value, "model:user");
}

#[test]
fn test_m39_query_model_target_not_found_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value =
        metadata_checker::query::build_query_model_output(graph, "model:not_exists", "compact")
            .expect("query_model should handle missing target without panic");

    assert_target_not_found_contract(&value, "model:not_exists", "ModelQuery");
}

#[test]
fn test_m39_query_dataflow_target_not_found_or_non_dataflow_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;

    let missing = metadata_checker::query::build_query_dataflow_output(graph, "model:not_exists")
        .expect("query_dataflow should handle missing target without panic");
    assert_target_not_found_contract(&missing, "model:not_exists", "ModelQuery");

    let non_dataflow =
        metadata_checker::query::build_query_dataflow_output(graph, "page:app/home.spg")
            .expect("query_dataflow should handle non-DataFlow target without panic");
    assert_ai_output_contract(&non_dataflow, "page:app/home.spg");
    assert_eq!(
        non_dataflow.get("kind").and_then(|v| v.as_str()),
        Some("DataFlowQuery")
    );
    assert_eq!(
        non_dataflow
            .get("summary")
            .and_then(|v| v.get("dataflow_id"))
            .and_then(|v| v.as_str()),
        Some("page:app/home.spg")
    );
}

#[test]
fn test_m39_query_page_logic_missing_page_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::query::build_query_page_logic_output(
        graph,
        "page:not_exists",
        None,
        "compact",
    )
    .expect("query_page_logic should handle missing page without panic");

    assert_target_not_found_contract(&value, "page:not_exists", "PageQuery");
}

#[test]
fn test_m39_context_target_not_found_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value =
        metadata_checker::context::build_context_output(graph, "model:not_exists", 1, "compact")
            .expect("context should handle missing target without panic");

    assert_target_not_found_contract(&value, "model:not_exists", "Context");
}

#[test]
fn test_m39_explain_condition_target_not_found_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::explain::build_explain_condition_output(
        graph,
        "model:not_exists",
        "compact",
    )
    .expect("explain_condition should handle missing target without panic");

    assert_target_not_found_contract(&value, "model:not_exists", "Explain");
}

#[test]
fn test_m39_explain_target_not_found_contract() {
    let store = build_contract_graph();
    let graph: &dyn GraphReadStore = &store;
    let value = metadata_checker::explain::build_explain_output(graph, "model:not_exists")
        .expect("explain should handle missing target without panic");

    assert_target_not_found_contract(&value, "model:not_exists", "Explain");
}
