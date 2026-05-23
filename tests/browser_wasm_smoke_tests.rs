//! Browser WASM feature smoke 测试
//!
//! M40.1.6：验证 browser-wasm feature 下内存图 + 查询链路可编译并正确运行。
//! 此测试在 default feature 和 browser-wasm feature 下都应编译通过。

use metadata_checker::graph::Edge;
use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use serde_json::json;

#[test]
fn test_memory_graph_store_basic_node_edge() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node(
        "page:app/test.spg",
        "test_page",
        NodeType::Page,
        "app/test.spg",
    );
    store.add_test_node(
        "comp:app/test.spg|btn",
        "btn",
        NodeType::Component,
        "app/test.spg",
    );
    store.add_test_node(
        "model:app/test.spg|m1",
        "m1",
        NodeType::Model,
        "app/test.spg",
    );
    store.add_test_edge(
        "comp:app/test.spg|btn",
        "model:app/test.spg|m1",
        EdgeType::Reads,
        None,
    );

    let neighbors = store
        .get_node_edges("comp:app/test.spg|btn")
        .expect("get_node_edges must not fail")
        .expect("btn must have neighbors");

    assert_eq!(neighbors.outgoing.len(), 1);
    assert_eq!(neighbors.outgoing[0].node.id, "model:app/test.spg|m1");
    assert!(matches!(
        neighbors.outgoing[0].edge.edge_type,
        EdgeType::Reads
    ));
}

#[test]
fn test_memory_graph_store_multi_hop_path() {
    let mut store = MemoryGraphStore::new();
    // page -> component -> model -> field
    store.add_test_node(
        "page:app/test.spg",
        "test_page",
        NodeType::Page,
        "app/test.spg",
    );
    store.add_test_node(
        "comp:app/test.spg|form",
        "form",
        NodeType::Component,
        "app/test.spg",
    );
    store.add_test_node(
        "model:app/test.spg|m1",
        "m1",
        NodeType::Model,
        "app/test.spg",
    );
    store.add_test_node(
        "field:app/test.spg|m1.name",
        "name",
        NodeType::Field,
        "app/test.spg",
    );

    store.add_test_edge(
        "page:app/test.spg",
        "comp:app/test.spg|form",
        EdgeType::Contains,
        None,
    );
    store.add_test_edge(
        "comp:app/test.spg|form",
        "model:app/test.spg|m1",
        EdgeType::Reads,
        None,
    );
    store.add_test_edge(
        "model:app/test.spg|m1",
        "field:app/test.spg|m1.name",
        EdgeType::Contains,
        None,
    );

    // 从 field 反向追溯：field <- model <- component <- page
    let field_neighbors = store
        .get_node_edges("field:app/test.spg|m1.name")
        .expect("must not fail")
        .expect("field must have neighbors");
    assert_eq!(field_neighbors.incoming.len(), 1);
    assert_eq!(field_neighbors.incoming[0].node.id, "model:app/test.spg|m1");

    let model_neighbors = store
        .get_node_edges("model:app/test.spg|m1")
        .expect("must not fail")
        .expect("model must have neighbors");
    assert_eq!(model_neighbors.incoming.len(), 1);
    assert_eq!(
        model_neighbors.incoming[0].node.id,
        "comp:app/test.spg|form"
    );

    let comp_neighbors = store
        .get_node_edges("comp:app/test.spg|form")
        .expect("must not fail")
        .expect("component must have neighbors");
    assert_eq!(comp_neighbors.incoming.len(), 1);
    assert_eq!(comp_neighbors.incoming[0].node.id, "page:app/test.spg");
}

#[test]
fn test_memory_graph_store_missing_node() {
    let store = MemoryGraphStore::new();
    let result = store.get_node_edges("nonexistent").expect("must not fail");
    assert!(result.is_none());
}

#[test]
fn test_memory_graph_store_writes_edge() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node(
        "comp:app/test.spg|btn",
        "btn",
        NodeType::Component,
        "app/test.spg",
    );
    store.add_test_node(
        "model:app/test.spg|m1",
        "m1",
        NodeType::Model,
        "app/test.spg",
    );
    let edge_meta = json!({
        "source_file": "app/test.spg",
        "field_path_note": "写回字段字段"
    });
    GraphWriteStore::add_edge(
        &mut store,
        Edge {
            from: "comp:app/test.spg|btn".to_string(),
            to: "model:app/test.spg|m1".to_string(),
            edge_type: EdgeType::Writes,
            field_path: Some("field1".to_string()),
            meta: Some(edge_meta.clone()),
        },
    )
    .expect("add edge should succeed");

    let neighbors = store
        .get_node_edges("model:app/test.spg|m1")
        .expect("must not fail")
        .expect("model must have incoming");
    assert_eq!(neighbors.incoming.len(), 1);
    assert!(matches!(
        neighbors.incoming[0].edge.edge_type,
        EdgeType::Writes
    ));
    assert_eq!(
        neighbors.incoming[0].edge.field_path.as_deref(),
        Some("field1")
    );
    assert_eq!(neighbors.incoming[0].edge.meta, Some(edge_meta));
}

#[test]
fn test_memory_graph_store_component_mixed_edge_types_keep_direction_and_type() {
    let mut store = MemoryGraphStore::new();
    let page_id = "page:app/mixed.spg";
    let component_id = "comp:app/mixed.spg|btn";
    let model_read_id = "model:app/mixed.spg|m_read";
    let model_write_id = "model:app/mixed.spg|m_write";
    let model_action_write_id = "model:app/mixed.spg|m_action_write";

    store.add_test_node(page_id, "mixed_page", NodeType::Page, "app/mixed.spg");
    store.add_test_node(component_id, "btn", NodeType::Component, "app/mixed.spg");
    store.add_test_node(model_read_id, "m_read", NodeType::Model, "app/mixed.spg");
    store.add_test_node(model_write_id, "m_write", NodeType::Model, "app/mixed.spg");
    store.add_test_node(
        model_action_write_id,
        "m_action_write",
        NodeType::Model,
        "app/mixed.spg",
    );

    store.add_test_edge(page_id, component_id, EdgeType::Contains, None);
    store.add_test_edge(
        component_id,
        model_read_id,
        EdgeType::Reads,
        Some("read_field"),
    );
    store.add_test_edge(
        component_id,
        model_write_id,
        EdgeType::Writes,
        Some("write_field"),
    );
    store.add_test_edge(
        component_id,
        model_action_write_id,
        EdgeType::ActionWrites,
        Some("action_write_field"),
    );

    let component_edges = store
        .get_node_edges(component_id)
        .expect("must not fail")
        .expect("component must have neighbors");

    assert_eq!(component_edges.outgoing.len(), 3);
    assert_eq!(
        component_edges
            .outgoing
            .iter()
            .filter(|e| matches!(e.edge.edge_type, EdgeType::Reads))
            .count(),
        1
    );
    assert_eq!(
        component_edges
            .outgoing
            .iter()
            .filter(|e| matches!(e.edge.edge_type, EdgeType::Writes))
            .count(),
        1
    );
    assert_eq!(
        component_edges
            .outgoing
            .iter()
            .filter(|e| matches!(e.edge.edge_type, EdgeType::ActionWrites))
            .count(),
        1
    );

    let read_neighbors = store
        .get_node_edges(model_read_id)
        .expect("must not fail")
        .expect("read target must have neighbors");
    assert_eq!(read_neighbors.incoming.len(), 1);
    assert_eq!(read_neighbors.incoming[0].edge.from, component_id);
    assert!(matches!(
        read_neighbors.incoming[0].edge.edge_type,
        EdgeType::Reads
    ));

    let write_neighbors = store
        .get_node_edges(model_write_id)
        .expect("must not fail")
        .expect("write target must have neighbors");
    assert_eq!(write_neighbors.incoming.len(), 1);
    assert_eq!(write_neighbors.incoming[0].edge.from, component_id);
    assert!(matches!(
        write_neighbors.incoming[0].edge.edge_type,
        EdgeType::Writes
    ));

    let action_write_neighbors = store
        .get_node_edges(model_action_write_id)
        .expect("must not fail")
        .expect("action-write target must have neighbors");
    assert_eq!(action_write_neighbors.incoming.len(), 1);
    assert_eq!(action_write_neighbors.incoming[0].edge.from, component_id);
    assert!(matches!(
        action_write_neighbors.incoming[0].edge.edge_type,
        EdgeType::ActionWrites
    ));
}

#[test]
fn test_memory_graph_store_remove_nodes_cleans_incident_edges_for_page_and_model_queries() {
    let mut store = MemoryGraphStore::new();
    let page_id = "page:app/remove.spg";
    let component_id = "comp:app/remove.spg|remove_btn";
    let model_read_id = "model:app/remove.spg|m1";
    let model_write_id = "model:app/remove.spg|m2";
    let model_to_comp_id = "model:app/remove.spg|m3";

    store.add_test_node(page_id, "remove_page", NodeType::Page, "app/remove.spg");
    store.add_test_node(
        component_id,
        "remove_btn",
        NodeType::Component,
        "app/remove.spg",
    );
    store.add_test_node(model_read_id, "m1", NodeType::Model, "app/remove.spg");
    store.add_test_node(model_write_id, "m2", NodeType::Model, "app/remove.spg");
    store.add_test_node(model_to_comp_id, "m3", NodeType::Model, "app/remove.spg");

    store.add_test_edge(page_id, component_id, EdgeType::Contains, None);
    store.add_test_edge(
        component_id,
        model_read_id,
        EdgeType::Reads,
        Some("field_a"),
    );
    store.add_test_edge(
        component_id,
        model_write_id,
        EdgeType::Writes,
        Some("field_b"),
    );
    store.add_test_edge(
        model_to_comp_id,
        component_id,
        EdgeType::Reads,
        Some("field_c"),
    );

    assert_eq!(
        store.node_count().expect("node count should be queryable"),
        5
    );
    assert_eq!(
        store.edge_count().expect("edge count should be queryable"),
        4
    );

    GraphWriteStore::remove_nodes_by_ids(&mut store, &[component_id.to_string()])
        .expect("remove node should succeed");

    assert_eq!(
        store.node_count().expect("node count should be queryable"),
        4
    );
    assert_eq!(
        store.edge_count().expect("edge count should be queryable"),
        0
    );

    let page_neighbors = store
        .get_node_edges(page_id)
        .expect("must not fail")
        .expect("page should exist");
    assert!(page_neighbors.outgoing.is_empty());

    let model_read_neighbors = store
        .get_node_edges(model_read_id)
        .expect("must not fail")
        .expect("model should exist");
    assert!(model_read_neighbors.incoming.is_empty());

    let model_to_comp_neighbors = store
        .get_node_edges(model_to_comp_id)
        .expect("must not fail")
        .expect("sender model should exist");
    assert!(model_to_comp_neighbors.outgoing.is_empty());

    assert!(
        store
            .get_node_edges(component_id)
            .expect("must not fail")
            .is_none()
    );
}

#[test]
fn test_memory_graph_store_same_name_nodes_are_isolated_by_source_path() {
    let mut store = MemoryGraphStore::new();
    let page_a = "page:app/a.spg";
    let page_b = "page:app/b.spg";
    let comp_a = "comp:app/a.spg|btn";
    let comp_b = "comp:app/b.spg|btn";
    let model_a = "model:app/a.spg|m";
    let model_b = "model:app/b.spg|m";

    store.add_test_node(page_a, "page_a", NodeType::Page, "app/a.spg");
    store.add_test_node(page_b, "page_b", NodeType::Page, "app/b.spg");
    store.add_test_node(comp_a, "btn", NodeType::Component, "app/a.spg");
    store.add_test_node(comp_b, "btn", NodeType::Component, "app/b.spg");
    store.add_test_node(model_a, "m", NodeType::Model, "app/a.spg");
    store.add_test_node(model_b, "m", NodeType::Model, "app/b.spg");

    store.add_test_edge(page_a, comp_a, EdgeType::Contains, None);
    store.add_test_edge(page_b, comp_b, EdgeType::Contains, None);
    store.add_test_edge(comp_a, model_a, EdgeType::Reads, Some("shared_field"));
    store.add_test_edge(comp_b, model_b, EdgeType::Reads, Some("shared_field"));

    let comp_a_neighbors = store
        .get_node_edges(comp_a)
        .expect("must not fail")
        .expect("comp a should have neighbors");
    assert_eq!(comp_a_neighbors.outgoing.len(), 1);
    assert_eq!(comp_a_neighbors.outgoing[0].node.id, model_a);
    assert_eq!(
        comp_a_neighbors.outgoing[0].edge.field_path.as_deref(),
        Some("shared_field")
    );

    let comp_b_neighbors = store
        .get_node_edges(comp_b)
        .expect("must not fail")
        .expect("comp b should have neighbors");
    assert_eq!(comp_b_neighbors.outgoing.len(), 1);
    assert_eq!(comp_b_neighbors.outgoing[0].node.id, model_b);
    assert_eq!(
        comp_b_neighbors.outgoing[0].edge.field_path.as_deref(),
        Some("shared_field")
    );

    let model_a_neighbors = store
        .get_node_edges(model_a)
        .expect("must not fail")
        .expect("model a should have neighbors");
    assert_eq!(model_a_neighbors.incoming.len(), 1);
    assert_eq!(model_a_neighbors.incoming[0].node.id, comp_a);
    assert_eq!(model_a_neighbors.incoming[0].edge.to, model_a);

    let model_b_neighbors = store
        .get_node_edges(model_b)
        .expect("must not fail")
        .expect("model b should have neighbors");
    assert_eq!(model_b_neighbors.incoming.len(), 1);
    assert_eq!(model_b_neighbors.incoming[0].node.id, comp_b);
    assert_eq!(model_b_neighbors.incoming[0].edge.to, model_b);
}
