//! Browser WASM feature smoke 测试
//!
//! M40.1.6：验证 browser-wasm feature 下内存图 + 查询链路可编译并正确运行。
//! 此测试在 default feature 和 browser-wasm feature 下都应编译通过。

use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;

#[test]
fn test_memory_graph_store_basic_node_edge() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:app/test.spg", "test_page", NodeType::Page, "app/test.spg");
    store.add_test_node("comp:app/test.spg|btn", "btn", NodeType::Component, "app/test.spg");
    store.add_test_node("model:app/test.spg|m1", "m1", NodeType::Model, "app/test.spg");
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
    assert!(matches!(neighbors.outgoing[0].edge.edge_type, EdgeType::Reads));
}

#[test]
fn test_memory_graph_store_multi_hop_path() {
    let mut store = MemoryGraphStore::new();
    // page -> component -> model -> field
    store.add_test_node("page:app/test.spg", "test_page", NodeType::Page, "app/test.spg");
    store.add_test_node("comp:app/test.spg|form", "form", NodeType::Component, "app/test.spg");
    store.add_test_node("model:app/test.spg|m1", "m1", NodeType::Model, "app/test.spg");
    store.add_test_node("field:app/test.spg|m1.name", "name", NodeType::Field, "app/test.spg");

    store.add_test_edge("page:app/test.spg", "comp:app/test.spg|form", EdgeType::Contains, None);
    store.add_test_edge("comp:app/test.spg|form", "model:app/test.spg|m1", EdgeType::Reads, None);
    store.add_test_edge("model:app/test.spg|m1", "field:app/test.spg|m1.name", EdgeType::Contains, None);

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
    assert_eq!(model_neighbors.incoming[0].node.id, "comp:app/test.spg|form");

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
    store.add_test_node("comp:app/test.spg|btn", "btn", NodeType::Component, "app/test.spg");
    store.add_test_node("model:app/test.spg|m1", "m1", NodeType::Model, "app/test.spg");
    store.add_test_edge(
        "comp:app/test.spg|btn",
        "model:app/test.spg|m1",
        EdgeType::Writes,
        Some("field1"),
    );

    let neighbors = store
        .get_node_edges("model:app/test.spg|m1")
        .expect("must not fail")
        .expect("model must have incoming");
    assert_eq!(neighbors.incoming.len(), 1);
    assert!(matches!(neighbors.incoming[0].edge.edge_type, EdgeType::Writes));
    assert_eq!(neighbors.incoming[0].edge.field_path.as_deref(), Some("field1"));
}
