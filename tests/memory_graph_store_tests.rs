#[path = "common/memory_graph_store.rs"]
mod memory_graph_store;
use memory_graph_store::MemoryGraphStore;
use metadata_checker::graph::{Edge, EdgeType, FileState, NodeType};
use metadata_checker::graph_store::{
    GraphReadStore, GraphWriteStore, IndexCommit, IndexStateStore,
};
use metadata_checker::query::find_candidates;

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
fn test_memory_graph_store_get_node_edges_missing_node_returns_none() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");

    let missing = store.get_node_edges("page:missing").unwrap();
    assert!(missing.is_none(), "不存在的节点应返回 None");
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

#[test]
fn test_memory_graph_store_write_trait_edge_direction_parity() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");

    GraphWriteStore::add_edge(
        &mut store,
        Edge {
            from: "page:home".to_string(),
            to: "comp:btn1".to_string(),
            edge_type: EdgeType::Contains,
            field_path: None,
            meta: None,
        },
    )
    .expect("add_edge should succeed");

    let outgoing = store.get_node_edges("page:home").unwrap().unwrap();
    assert_eq!(outgoing.outgoing.len(), 1);
    assert_eq!(outgoing.outgoing[0].node.id, "comp:btn1");
    assert_eq!(outgoing.outgoing[0].edge.to, "comp:btn1");

    let incoming = store.get_node_edges("comp:btn1").unwrap().unwrap();
    assert_eq!(incoming.incoming.len(), 1);
    assert_eq!(incoming.incoming[0].node.id, "page:home");
    assert_eq!(incoming.incoming[0].edge.from, "page:home");
}

#[test]
fn test_memory_graph_store_find_candidates_through_graph_read_store() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node(
        "comp:hero_btn",
        "首页按钮",
        NodeType::Component,
        "app/home.spg",
    );
    store.add_test_node("model:user", "用户", NodeType::Model, "models/user.tbl");

    let graph: &dyn GraphReadStore = &store;
    let candidates = find_candidates(graph, "user", 10).expect("find_candidates should work");
    let contains_user = candidates.iter().any(|(node, _)| node.id == "model:user");
    assert!(
        contains_user,
        "find_candidates 应该在 trait 对象上可用并返回命中节点"
    );
}

#[test]
fn test_query_page_logic_runs_on_memory_graph_store() {
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

    let graph: &dyn GraphReadStore = &store;
    let output = metadata_checker::query::build_query_page_logic_output(
        graph,
        "page:app/home.spg",
        None,
        "compact",
    )
    .expect("query_page_logic should run on MemoryGraphStore");

    assert_eq!(
        output
            .get("query_target")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        "page:app/home.spg"
    );
    let data_sources = output
        .get("details")
        .and_then(|v| v.get("data_sources"))
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
        .expect("data_sources should be present");
    assert_eq!(data_sources.len(), 1);
}

#[test]
fn test_query_page_cross_context_and_resolve_run_on_memory_graph_store() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:a.spg", "页面A", NodeType::Page, "a.spg");
    store.add_test_node("page:b.spg", "页面B", NodeType::Page, "b.spg");
    store.add_test_node("model:user", "用户", NodeType::Model, "tables/user.tbl");
    store.add_test_edge("page:a.spg", "model:user", EdgeType::Reads, None);
    store.add_test_edge("page:b.spg", "model:user", EdgeType::Reads, None);

    let graph: &dyn GraphReadStore = &store;
    let page = metadata_checker::query::build_query_page_output(graph, "page:a.spg")
        .expect("query page should run on MemoryGraphStore");
    assert_eq!(
        page.get("query_target").and_then(|v| v.as_str()),
        Some("page:a.spg")
    );

    let cross =
        metadata_checker::query::build_query_cross_output(graph, "page:a.spg", "page:b.spg")
            .expect("query cross should run on MemoryGraphStore");
    assert_eq!(
        cross
            .get("summary")
            .and_then(|v| v.get("path_count"))
            .and_then(|v| v.as_u64()),
        Some(1)
    );

    let context =
        metadata_checker::context::build_context_output(graph, "model:user", 1, "compact")
            .expect("context should run on MemoryGraphStore");
    assert_eq!(
        context.get("query_target").and_then(|v| v.as_str()),
        Some("model:user")
    );

    let resolved = metadata_checker::query::resolve_model_in_page(graph, "page:a.spg", "user");
    assert_eq!(
        resolved
            .summary
            .get("resolved_count")
            .and_then(|v| v.as_u64()),
        Some(1)
    );

    let explain = metadata_checker::explain::build_explain_output(graph, "model:user")
        .expect("explain should run on MemoryGraphStore");
    assert_eq!(
        explain.get("query_target").and_then(|v| v.as_str()),
        Some("model:user")
    );

    let explain_condition =
        metadata_checker::explain::build_explain_condition_output(graph, "model:user", "compact")
            .expect("explain-condition should run on MemoryGraphStore");
    assert_eq!(
        explain_condition
            .get("query_target")
            .and_then(|v| v.as_str()),
        Some("model:user")
    );
}

#[test]
fn test_memory_graph_store_remove_nodes_removes_incident_edges() {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:home", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("comp:btn1", "按钮1", NodeType::Component, "app/home.spg");
    store.add_test_node("model:user", "用户", NodeType::Model, "app/user.tbl");
    store.add_test_edge("page:home", "comp:btn1", EdgeType::Contains, None);
    store.add_test_edge("comp:btn1", "model:user", EdgeType::Writes, None);

    assert_eq!(store.edge_count().unwrap(), 2);
    assert_eq!(store.node_count().unwrap(), 3);

    GraphWriteStore::remove_nodes_by_ids(&mut store, &["comp:btn1".to_string()])
        .expect("remove node should succeed");

    assert_eq!(store.node_count().unwrap(), 2);
    assert_eq!(store.edge_count().unwrap(), 0);
    assert!(
        store
            .get_node_edges("page:home")
            .unwrap()
            .expect("node should still exist")
            .outgoing
            .is_empty()
    );
    assert!(
        store
            .get_node_edges("model:user")
            .unwrap()
            .expect("node should still exist")
            .incoming
            .is_empty()
    );
    assert!(
        store.get_node_edges("comp:btn1").unwrap().is_none(),
        "删除节点后 get_node_edges 应返回 None"
    );
}

#[test]
fn test_memory_graph_store_persist_index_returns_commit_stats() {
    let mut store = MemoryGraphStore::new();
    let file_states = vec![
        (
            "app/home.spg".to_string(),
            FileState {
                file_path: "app/home.spg".to_string(),
                file_hash: "hash-home".to_string(),
                mtime: 100,
                size: 10,
                node_ids: vec!["page:home".to_string()],
            },
        ),
        (
            "app/about.spg".to_string(),
            FileState {
                file_path: "app/about.spg".to_string(),
                file_hash: "hash-about".to_string(),
                mtime: 200,
                size: 20,
                node_ids: vec!["page:about".to_string()],
            },
        ),
        (
            "models/user.tbl".to_string(),
            FileState {
                file_path: "models/user.tbl".to_string(),
                file_hash: "hash-user".to_string(),
                mtime: 300,
                size: 30,
                node_ids: vec!["model:user".to_string()],
            },
        ),
    ]
    .into_iter()
    .collect();

    let commit = IndexCommit {
        file_states,
        dirty_nodes: vec!["page:home".to_string(), "model:user".to_string()],
        deleted_nodes: vec!["old:removed".to_string()],
    };

    let report =
        IndexStateStore::persist_index(&mut store, commit).expect("persist_index should succeed");
    assert_eq!(report.indexed, 3);
    assert_eq!(report.dirty, 2);
    assert_eq!(report.deleted, 1);
    assert_eq!(report.unchanged, 1);

    let loaded = store
        .load_file_states()
        .expect("load_file_states should succeed");
    assert_eq!(loaded.len(), 3);
}
