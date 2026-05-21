mod common;
use common::memory_graph_store::MemoryGraphStore;
use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_store::GraphReadStore;

#[test]
fn test_memory_graph_store_node_count() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    assert_eq!(store.node_count().unwrap(), 2);
    assert_eq!(store.edge_count().unwrap(), 0);
}

#[test]
fn test_memory_graph_store_get_node_edges() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    store.add_test_edge("page:home", "comp:btn1", EdgeType::Contains, None);

    let neighbors = store.get_node_edges("page:home").unwrap().unwrap();
    assert_eq!(neighbors.outgoing.len(), 1);
    assert_eq!(neighbors.outgoing[0].node.id, "comp:btn1");
    assert_eq!(neighbors.outgoing[0].edge.to, "comp:btn1");
    assert_eq!(neighbors.incoming.len(), 0);

    let neighbors2 = store.get_node_edges("comp:btn1").unwrap().unwrap();
    assert_eq!(neighbors2.outgoing.len(), 0);
    assert_eq!(neighbors2.incoming.len(), 1);
    assert_eq!(neighbors2.incoming[0].node.id, "page:home");
    assert_eq!(neighbors2.incoming[0].edge.from, "page:home");
}

#[test]
fn test_memory_graph_store_iter_nodes_via_trait() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    store.add_test_node("model:user", "用户", NodeType::Model, "app/user.tbl");

    let graph: &dyn GraphReadStore = &store;
    let count = graph.iter_nodes().unwrap().count();
    assert_eq!(count, 3);
}

#[test]
fn test_memory_graph_store_proves_query_can_leave_redb() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    store.add_test_node("model:user", "用户", NodeType::Model, "app/user.tbl");
    store.add_test_edge("page:home", "model:user", EdgeType::Reads, None);

    let count = store.iter_nodes().unwrap().count();
    assert_eq!(count, 3);

    let home_edges = store.get_node_edges("page:home").unwrap().unwrap();
    assert_eq!(home_edges.outgoing.len(), 1);
    assert_eq!(home_edges.incoming.len(), 0);

    let user_edges = store.get_node_edges("model:user").unwrap().unwrap();
    assert_eq!(user_edges.outgoing.len(), 0);
    assert_eq!(user_edges.incoming.len(), 1);
}

#[test]
fn test_memory_graph_store_edge_direction_parity() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    // outgoing from page:home to comp:btn1
    store.add_test_edge("page:home", "comp:btn1", EdgeType::Contains, None);

    let neighbors = store.get_node_edges("page:home").unwrap().unwrap();
    // outgoing should contain the TARGET node (comp:btn1)
    assert_eq!(neighbors.outgoing.len(), 1);
    assert_eq!(neighbors.outgoing[0].node.id, "comp:btn1");
    assert_eq!(neighbors.outgoing[0].edge.edge_type, EdgeType::Contains);

    let reverse = store.get_node_edges("comp:btn1").unwrap().unwrap();
    // incoming should contain the SOURCE node (page:home)
    assert_eq!(reverse.incoming.len(), 1);
    assert_eq!(reverse.incoming[0].node.id, "page:home");
    assert_eq!(reverse.incoming[0].edge.edge_type, EdgeType::Contains);
}
