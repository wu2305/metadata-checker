//! M40.8 持久化提供者行为测试

use metadata_checker::graph::{Edge, EdgeType, FileState, Node, NodeType};
use metadata_checker::persistence::types::PersistenceError;
use metadata_checker::persistence::{
    GraphPersistenceProvider,
    memory::MemoryPersistenceProvider,
    types::{CachedDocument, GraphMeta, GraphSnapshot, PERSISTENCE_SCHEMA_VERSION},
};

#[cfg(feature = "cli-local")]
mod common;

#[cfg(feature = "cli-local")]
use metadata_checker::graph_redb::GraphDB;

#[cfg(feature = "cli-local")]
use metadata_checker::persistence::redb::RedbPersistenceProvider;

#[cfg(feature = "browser-wasm")]
use metadata_checker::persistence::indexeddb::IndexedDbPersistenceProvider;

#[cfg(feature = "cli-local")]
fn unique_graph_db_path(name: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-persistence-{}-{}-{}.graphdb",
        name,
        std::process::id(),
        nanos
    ))
}

fn make_graph_snapshot() -> GraphSnapshot {
    GraphSnapshot {
        schema_version: PERSISTENCE_SCHEMA_VERSION.to_string(),
        created_at: 1700000000,
        nodes: vec![Node {
            id: "page:app/home.spg".to_string(),
            node_type: NodeType::Page,
            path: "app/home.spg".to_string(),
            name: "home".to_string(),
            meta: None,
            origin_file: None,
        }],
        edges: vec![Edge {
            from: "page:app/home.spg".to_string(),
            to: "page:app/home.spg".to_string(),
            edge_type: EdgeType::Contains,
            field_path: Some("edge_field".to_string()),
            meta: None,
            origin_file: None,
        }],
    }
}

fn make_graph_meta(graph_ref: &str) -> GraphMeta {
    GraphMeta {
        schema_version: PERSISTENCE_SCHEMA_VERSION.to_string(),
        updated_at: 1700000000,
        project_ref: "project:m40_8".to_string(),
        graph_ref: graph_ref.to_string(),
    }
}

fn make_cached_document(content: &str, sensitive: bool) -> CachedDocument {
    CachedDocument {
        content_hash: format!("hash-{}", content),
        fetched_at: 1700000000,
        content: content.to_string(),
        sensitive,
    }
}

fn make_file_state(source_path: &str) -> FileState {
    FileState {
        file_path: source_path.to_string(),
        file_hash: format!("hash-{}", source_path),
        mtime: 100,
        size: 200,
        node_ids: vec!["node-1".to_string()],
    }
}

fn assert_not_found(err: PersistenceError) {
    assert!(
        matches!(err, PersistenceError::NotFound { .. }),
        "expect NotFound, got {err}"
    );
}

#[test]
fn test_memory_persistence_provider_roundtrip() {
    let mut provider = MemoryPersistenceProvider::new();

    let project_ref = "project:m40_8";
    let graph_ref = "graph:memory";
    let source_path = "app/home.spg";

    let snapshot = make_graph_snapshot();
    let snapshot_out = snapshot.clone();
    provider
        .save_graph_snapshot(project_ref, graph_ref, &snapshot)
        .expect("save_graph_snapshot should succeed");
    let loaded_snapshot = provider
        .load_graph_snapshot(project_ref, graph_ref)
        .expect("load_graph_snapshot should succeed");
    assert_eq!(loaded_snapshot.schema_version, snapshot_out.schema_version);
    assert_eq!(loaded_snapshot.nodes.len(), snapshot_out.nodes.len());
    assert_eq!(loaded_snapshot.nodes[0].id, snapshot_out.nodes[0].id);
    assert_eq!(loaded_snapshot.nodes[0].name, snapshot_out.nodes[0].name);
    assert_eq!(loaded_snapshot.nodes[0].path, snapshot_out.nodes[0].path);
    assert!(matches!(loaded_snapshot.nodes[0].node_type, NodeType::Page));
    assert_eq!(loaded_snapshot.edges.len(), snapshot_out.edges.len());
    assert_eq!(loaded_snapshot.edges[0].from, snapshot_out.edges[0].from);
    assert_eq!(loaded_snapshot.edges[0].to, snapshot_out.edges[0].to);
    assert_eq!(
        loaded_snapshot.edges[0].field_path,
        snapshot_out.edges[0].field_path
    );
    assert!(matches!(
        loaded_snapshot.edges[0].edge_type,
        EdgeType::Contains
    ));

    let document = make_cached_document("cache body", false);
    provider
        .save_document_cache(project_ref, source_path, &document)
        .expect("save_document_cache should succeed");
    let loaded_document = provider
        .load_document_cache(project_ref, source_path)
        .expect("load_document_cache should succeed");
    assert_eq!(loaded_document.content_hash, document.content_hash);
    assert_eq!(loaded_document.content, document.content);
    assert!(!loaded_document.sensitive);

    let sensitive_document = make_cached_document("sensitive", true);
    provider
        .save_document_cache(project_ref, "app/secret.spg", &sensitive_document)
        .expect("save_sensitive should clear cache entry");
    assert_not_found(
        provider
            .load_document_cache(project_ref, "app/secret.spg")
            .expect_err("sensitive cache should not be loadable"),
    );

    let file_state = make_file_state(source_path);
    provider
        .save_file_state(project_ref, source_path, &file_state)
        .expect("save_file_state should succeed");
    let loaded_file_state = provider
        .load_file_state(project_ref, source_path)
        .expect("load_file_state should succeed");
    assert_eq!(loaded_file_state.file_path, file_state.file_path);
    assert_eq!(loaded_file_state.file_hash, file_state.file_hash);
    assert_eq!(loaded_file_state.size, file_state.size);

    let meta = make_graph_meta(graph_ref);
    let meta_out = meta.clone();
    provider
        .save_graph_meta(project_ref, graph_ref, &meta)
        .expect("save_graph_meta should succeed");
    let loaded_meta = provider
        .load_graph_meta(project_ref, graph_ref)
        .expect("load_graph_meta should succeed");
    assert_eq!(loaded_meta.schema_version, meta_out.schema_version);
    assert_eq!(loaded_meta.project_ref, meta_out.project_ref);
    assert_eq!(loaded_meta.graph_ref, meta_out.graph_ref);
}

#[test]
fn test_memory_persistence_provider_rejects_version_mismatch() {
    let mut provider = MemoryPersistenceProvider::new();

    let mut snapshot = make_graph_snapshot();
    snapshot.schema_version = "old-schema".to_string();

    let err = provider
        .save_graph_snapshot("project:m40_8", "graph:old", &snapshot)
        .expect_err("old snapshot schema should be rejected");
    assert!(matches!(err, PersistenceError::VersionMismatch { .. }));
    assert_eq!(err.code(), "PERSISTENCE_VERSION_MISMATCH");

    let mut meta = make_graph_meta("graph:old");
    meta.schema_version = "old-schema".to_string();
    let err = provider
        .save_graph_meta("project:m40_8", "graph:old", &meta)
        .expect_err("old meta schema should be rejected");
    assert!(matches!(err, PersistenceError::VersionMismatch { .. }));
}

#[cfg(feature = "cli-local")]
#[test]
fn test_redb_provider_new_is_lazy_for_missing_db() {
    let db_path = unique_graph_db_path("lazy-missing");
    let _ = std::fs::remove_file(&db_path);

    let provider = RedbPersistenceProvider::new(&db_path).expect("provider init should be lazy");
    assert!(
        !db_path.exists(),
        "new should not create a missing graphdb file"
    );

    assert_not_found(
        provider
            .load_document_cache("project:m40_8", "app/missing.spg")
            .expect_err("missing db document cache read should be NotFound"),
    );
    assert!(
        !db_path.exists(),
        "document cache load should not create a missing graphdb file"
    );

    assert_not_found(
        provider
            .load_graph_meta("project:m40_8", "graph:missing")
            .expect_err("missing db graph meta read should be NotFound"),
    );
    assert!(
        !db_path.exists(),
        "graph meta load should not create a missing graphdb file"
    );
}

/// 图内容指纹：节点数 + 边数 + file_states 数。
///
/// 为什么不用文件字节大小：探针实测（M58.3 复核返修）redb 在 Database
/// open+close 时会非确定性地落盘内部 pending/trim 状态（文件缩小约 265KB，
/// 时序相关、可复现但不可控），与图内容无关；「不修改 db」的真实语义是
/// 图内容不变。load 合法触碰文件的先例见 ef03a6d 的守卫测试。
#[cfg(feature = "cli-local")]
fn graph_content_fingerprint(db_path: &std::path::Path) -> (usize, usize, usize) {
    let graph_db = GraphDB::open_readonly(db_path).expect("readonly GraphDB open should work");
    let file_states = graph_db
        .load_file_states()
        .expect("should load file states");
    (
        graph_db.graph.node_count(),
        graph_db.graph.edge_count(),
        file_states.len(),
    )
}

#[cfg(feature = "cli-local")]
#[test]
fn test_redb_provider_new_and_missing_cache_load_do_not_modify_existing_db() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let before = graph_content_fingerprint(&db_path);

    let provider = RedbPersistenceProvider::new(&db_path).expect("provider init should be lazy");
    assert_eq!(
        graph_content_fingerprint(&db_path),
        before,
        "provider new 不得改动图内容"
    );

    assert_not_found(
        provider
            .load_document_cache("project:m40_8", "app/not-yet-cached.spg")
            .expect_err("missing document cache table should be NotFound"),
    );
    assert_not_found(
        provider
            .load_graph_meta("project:m40_8", "graph:not-yet-cached")
            .expect_err("missing graph meta table should be NotFound"),
    );

    assert_eq!(
        graph_content_fingerprint(&db_path),
        before,
        "缺失缓存的读操作不得改动图内容"
    );
}

#[cfg(feature = "cli-local")]
#[test]
fn test_redb_load_graph_snapshot_reads_existing_graphdb_readonly() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();
    let before = graph_content_fingerprint(&db_path);
    let (baseline_node_count, baseline_edge_count) = (before.0, before.1);
    assert!(baseline_node_count > 0, "fixture graphdb should have nodes");

    let provider = RedbPersistenceProvider::new(&db_path).expect("provider init should be lazy");
    let snapshot = provider
        .load_graph_snapshot("project:m40_8_fixture", "graph:m40_8_fixture")
        .expect("snapshot load should read existing nodes and edges");

    assert_eq!(snapshot.schema_version, PERSISTENCE_SCHEMA_VERSION);
    assert_eq!(snapshot.nodes.len(), baseline_node_count);
    assert_eq!(snapshot.edges.len(), baseline_edge_count);
    // 内容级「未修改」断言（字节大小受 redb 内部 housekeeping 影响，见
    // graph_content_fingerprint 注释）
    assert_eq!(
        graph_content_fingerprint(&db_path),
        before,
        "快照读取不得改动图内容"
    );
}

#[cfg(feature = "cli-local")]
#[test]
fn test_redb_persistence_provider_reuses_existing_graph_db_file_states() {
    let (_temp_dir, db_path) = common::build_fixture_graphdb();

    let baseline_file_states = {
        let graph_db = GraphDB::open(&db_path).expect("GraphDB::open should succeed");
        graph_db
            .load_file_states()
            .expect("should load baseline file states")
    };
    assert!(
        !baseline_file_states.is_empty(),
        "fixture graphdb should have states"
    );

    let project_ref = "project:m40_8_fixture";
    let graph_ref = "graph:m40_8_fixture";
    let (baseline_node_count, baseline_edge_count) = {
        let graph_db = GraphDB::open(&db_path).expect("GraphDB::open should succeed");
        (graph_db.graph.node_count(), graph_db.graph.edge_count())
    };
    assert!(baseline_node_count > 0, "fixture graphdb should have nodes");

    let mut provider =
        RedbPersistenceProvider::new(&db_path).expect("provider init should succeed");

    let snapshot = make_graph_snapshot();
    let save_snapshot_err = provider
        .save_graph_snapshot(project_ref, graph_ref, &snapshot)
        .expect_err("redb provider should not write a second graph snapshot store");
    assert!(matches!(
        save_snapshot_err,
        PersistenceError::Unsupported { .. }
    ));
    assert_eq!(save_snapshot_err.code(), "PERSISTENCE_UNSUPPORTED");

    let document = make_cached_document("fixture cache", false);
    provider
        .save_document_cache(project_ref, "app/fetch.spg", &document)
        .expect("save_document_cache should succeed in redb provider");

    let meta = make_graph_meta(graph_ref);
    provider
        .save_graph_meta(project_ref, graph_ref, &meta)
        .expect("save_graph_meta should succeed in redb provider");

    let loaded_snapshot = provider
        .load_graph_snapshot(project_ref, graph_ref)
        .expect("load_graph_snapshot should map existing GraphDB nodes and edges");
    assert_eq!(loaded_snapshot.schema_version, PERSISTENCE_SCHEMA_VERSION);
    assert_eq!(loaded_snapshot.nodes.len(), baseline_node_count);
    assert_eq!(loaded_snapshot.edges.len(), baseline_edge_count);

    let loaded_document = provider
        .load_document_cache(project_ref, "app/fetch.spg")
        .expect("load_document_cache should succeed in redb provider");
    assert_eq!(loaded_document.content, document.content);

    let sensitive_document = make_cached_document("token=secret-cookie-password", true);
    provider
        .save_document_cache(project_ref, "app/fetch.spg", &sensitive_document)
        .expect("saving sensitive cache should remove existing cache entry");
    assert_not_found(
        provider
            .load_document_cache(project_ref, "app/fetch.spg")
            .expect_err("sensitive cache should not be loadable from redb"),
    );

    let loaded_meta = provider
        .load_graph_meta(project_ref, graph_ref)
        .expect("load_graph_meta should succeed in redb provider");
    assert_eq!(loaded_meta.updated_at, meta.updated_at);

    let (state_path, state_sample) = baseline_file_states
        .iter()
        .next()
        .map(|(path, state)| (path.clone(), state.clone()))
        .expect("baseline states should have at least one item");

    let retained_state = provider
        .load_file_state(project_ref, state_path.as_str())
        .expect("load_file_state should still read existing graphdb state");
    assert_eq!(retained_state.file_path, state_sample.file_path);
    assert_eq!(retained_state.file_hash, state_sample.file_hash);

    let after_states = {
        let graph_db =
            GraphDB::open(&db_path).expect("GraphDB::open should succeed after persistence writes");
        graph_db
            .load_file_states()
            .expect("should load file states after persistence writes")
    };

    assert_eq!(after_states.len(), baseline_file_states.len());
    assert_eq!(after_states[&state_path].file_hash, state_sample.file_hash);
}

#[cfg(feature = "browser-wasm")]
#[test]
fn test_indexed_db_persistence_provider_is_unsupported_stub() {
    let mut provider = IndexedDbPersistenceProvider::new();

    let snapshot = make_graph_snapshot();
    let document = make_cached_document("browser cache", false);
    let state = make_file_state("app/wasm.spg");
    let meta = make_graph_meta("graph:wasm");

    let err = provider.load_graph_snapshot("p", "g").unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider
        .save_graph_snapshot("p", "g", &snapshot)
        .unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider.load_document_cache("p", "d").unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider
        .save_document_cache("p", "d", &document)
        .unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider.load_file_state("p", "s").unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider.save_file_state("p", "s", &state).unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider.load_graph_meta("p", "g").unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");

    let err = provider.save_graph_meta("p", "g", &meta).unwrap_err();
    assert!(matches!(err, PersistenceError::Unsupported { .. }));
    assert_eq!(err.code(), "PERSISTENCE_UNSUPPORTED");
}
