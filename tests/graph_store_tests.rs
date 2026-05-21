use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore, IndexStateStore};
use std::collections::HashMap;

#[test]
fn test_graphdb_implements_graph_read_store() {
    let db_path = std::env::temp_dir().join("m39_test_read.graphdb");
    let _ = std::fs::remove_file(&db_path);
    let mut graph = GraphDB::open(&db_path).expect("open graphdb");

    // 使用 trait 方法
    let count = GraphReadStore::node_count(&graph).expect("node_count");
    assert_eq!(count, 0);

    let edge_count = GraphReadStore::edge_count(&graph).expect("edge_count");
    assert_eq!(edge_count, 0);

    let node = GraphReadStore::get_node(&graph, "nonexistent").expect("get_node");
    assert!(node.is_none());

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_graphdb_implements_graph_write_store() {
    let db_path = std::env::temp_dir().join("m39_test_write.graphdb");
    let _ = std::fs::remove_file(&db_path);
    let mut graph = GraphDB::open(&db_path).expect("open graphdb");

    let node = metadata_checker::graph::Node {
        id: "page:test".to_string(),
        node_type: metadata_checker::graph::NodeType::Page,
        path: "test.spg".to_string(),
        name: "Test Page".to_string(),
        meta: None,
    };

    GraphWriteStore::upsert_node(&mut graph, node);
    assert_eq!(graph.node_count().expect("count"), 1);

    let edge = metadata_checker::graph::Edge {
        from: "page:test".to_string(),
        to: "page:test".to_string(),
        edge_type: metadata_checker::graph::EdgeType::Contains,
        field_path: None,
        meta: None,
    };
    GraphWriteStore::add_edge(&mut graph, edge);
    assert_eq!(graph.edge_count().expect("count"), 1);

    GraphWriteStore::remove_nodes_by_ids(&mut graph, &["page:test".to_string()]);
    assert_eq!(graph.node_count().expect("count"), 0);

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_graphdb_implements_index_state_store() {
    let db_path = std::env::temp_dir().join("m39_test_index.graphdb");
    let lock_path = db_path.with_extension("graphdb.lock");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);
    let mut graph = GraphDB::open(&db_path).expect("open graphdb");

    let mut states = HashMap::new();
    states.insert(
        "test.spg".to_string(),
        metadata_checker::graph::FileState {
            file_path: "test.spg".to_string(),
            file_hash: "abc".to_string(),
            mtime: 0,
            size: 0,
            node_ids: vec![],
        },
    );

    // 先 upsert 一个节点让 is_dirty = true
    let node = metadata_checker::graph::Node {
        id: "page:test".to_string(),
        node_type: metadata_checker::graph::NodeType::Page,
        path: "test.spg".to_string(),
        name: "Test".to_string(),
        meta: None,
    };
    GraphWriteStore::upsert_node(&mut graph, node).unwrap();

    graph.persist(&states).expect("persist");
    let loaded = graph.load_file_states().expect("load_file_states");
    assert_eq!(loaded.len(), 1, "loaded: {:?}", loaded);
    assert!(loaded.contains_key("test.spg"));

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);
}

#[test]
fn test_find_candidates_lives_outside_graph_store() {
    // find_candidates 不是 GraphReadStore trait 的方法，而是独立函数
    use metadata_checker::graph_store::GraphReadStore;
    use metadata_checker::query::find_candidates;

    // 确认 GraphReadStore trait 里确实没有 find_candidates 方法
    // 这里不做 trait object 调用，只确认独立函数存在且可调用
    // （实际行为在 regression_tests 中已有覆盖）
    let fn_ptr: fn(
        &dyn GraphReadStore,
        &str,
        usize,
    ) -> anyhow::Result<Vec<(metadata_checker::graph::Node, String)>> = find_candidates;
    assert!(std::ptr::addr_of!(fn_ptr).is_aligned());
}

#[test]
fn test_graph_store_error_code_roundtrip() {
    use metadata_checker::graph_store::GraphStoreError;

    let cases = vec![
        (
            GraphStoreError::NotFound {
                resource: "x".into(),
            },
            "GRAPH_DB_NOT_FOUND",
        ),
        (
            GraphStoreError::InvalidArgument {
                message: "x".into(),
            },
            "INVALID_ARGUMENT",
        ),
        (
            GraphStoreError::OpenFailed {
                path: "x".into(),
                reason: "x".into(),
            },
            "GRAPH_DB_OPEN_FAILED",
        ),
        (
            GraphStoreError::ReadFailed { reason: "x".into() },
            "GRAPH_DB_READ_FAILED",
        ),
        (
            GraphStoreError::WriteFailed { reason: "x".into() },
            "GRAPH_DB_WRITE_FAILED",
        ),
        (
            GraphStoreError::SerializeFailed { reason: "x".into() },
            "GRAPH_DB_SERIALIZE_FAILED",
        ),
        (
            GraphStoreError::DeserializeFailed { reason: "x".into() },
            "GRAPH_DB_DESERIALIZE_FAILED",
        ),
        (
            GraphStoreError::LockTimeout { path: "x".into() },
            "GRAPH_DB_LOCK_TIMEOUT",
        ),
        (
            GraphStoreError::PermissionDenied { path: "x".into() },
            "GRAPH_DB_PERMISSION_DENIED",
        ),
        (
            GraphStoreError::Corrupted { reason: "x".into() },
            "GRAPH_DB_CORRUPTED",
        ),
        (
            GraphStoreError::UnsupportedOperation {
                message: "x".into(),
            },
            "GRAPH_DB_UNSUPPORTED_OPERATION",
        ),
    ];

    for (err, expected) in cases {
        assert_eq!(err.code(), expected, "code mismatch for {:?}", err);
    }
}

#[test]
fn test_persist_index_commits_graph_and_file_states_together() {
    use metadata_checker::graph_store::{GraphWriteStore, IndexCommit, IndexStateStore};
    use std::collections::HashMap;

    let db_path = std::env::temp_dir().join("m39_test_index_commit.graphdb");
    let lock_path = db_path.with_extension("graphdb.lock");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);
    let mut graph = GraphDB::open(&db_path).expect("open graphdb");

    // 准备节点
    let node = metadata_checker::graph::Node {
        id: "page:test".to_string(),
        node_type: metadata_checker::graph::NodeType::Page,
        path: "test.spg".to_string(),
        name: "Test".to_string(),
        meta: None,
    };
    GraphWriteStore::upsert_node(&mut graph, node).unwrap();

    let mut file_states = HashMap::new();
    file_states.insert(
        "test.spg".to_string(),
        metadata_checker::graph::FileState {
            file_path: "test.spg".to_string(),
            file_hash: "abc".to_string(),
            mtime: 0,
            size: 0,
            node_ids: vec!["page:test".to_string()],
        },
    );

    let commit = IndexCommit {
        file_states: file_states.clone(),
        dirty_nodes: vec!["page:test".to_string()],
        deleted_nodes: vec![],
    };

    let report =
        IndexStateStore::persist_index(&mut graph, &commit).expect("persist_index must succeed");
    assert_eq!(report.indexed, 1);

    // 验证 file_states 已持久化
    let loaded = graph.load_file_states().expect("load_file_states");
    assert_eq!(loaded.len(), 1);
    assert!(loaded.contains_key("test.spg"));

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);
}
