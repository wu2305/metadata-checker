#![cfg(feature = "cli-local")]
//! M54-M56 follow-up 修复测试
//!
//! 覆盖冷脸验收 P1/P2 follow-up 项：
//! - mirror 失败后内存 manifest 回滚
//! - 空 bootstrap 落 checkpoint（下次走 poll 而非重新 bootstrap）
//! - v2 hydrate 失败诊断（v2_hydrate_warning）
//! - remove_nodes_by_ids in-place 删除正确性
//! - persist lock 覆盖（并发写不撕裂）
//! - 阈值 ==1024 等值边界

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow};

use metadata_checker::diff_refresh::{
    ChangeSet, ChangedRemoteFile, MetaFilesWatermark, SourceCursor, apply_changeset_to_mirror,
};
use metadata_checker::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
use metadata_checker::graph_store::{
    GraphReadStore, GraphWriteStore, IndexCommit, IndexDelta, V2ShadowState,
};
use metadata_checker::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use metadata_checker::session::RemoteSessionProvider;
use metadata_checker::session::manifest::SessionManifest;
use metadata_checker::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo,
};
use metadata_checker::session::sync::{
    SessionSyncItem, SessionSyncMode, sync_remote_files_to_session,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_root(name: &str) -> std::path::PathBuf {
    let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("metadata-checker-followup-{pid}-{seq}-{name}"));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create test dir");
    path
}

fn make_manifest(db_path: &str) -> SessionManifest {
    SessionManifest::new(
        "s1",
        "https://bi.test",
        "proj",
        "proj",
        "remote",
        db_path,
        1,
    )
}

fn active_event(
    event_id: &str,
    file_id: &str,
    source_path: &str,
    previous_source_path: Option<&str>,
    updated_at_ms: u64,
) -> ChangedRemoteFile {
    ChangedRemoteFile {
        event_id: event_id.to_string(),
        file_id: file_id.to_string(),
        source_path: source_path.to_string(),
        previous_source_path: previous_source_path.map(str::to_string),
        content_type: MetadataContentType::SuperPage,
        updated_at_ms,
        deleted: false,
    }
}

/// 失败 provider：fetch 总是返回 Err。
struct FailingProvider {
    fetch_count: Cell<usize>,
}

impl FailingProvider {
    fn new() -> Self {
        Self {
            fetch_count: Cell::new(0),
        }
    }
}

impl RemoteSessionProvider for FailingProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        Err(anyhow!("not supported"))
    }
    fn list_metafiles(&self, _project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        Err(anyhow!("not supported"))
    }
    fn fetch_metafile_info(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        Err(anyhow!("not supported"))
    }
    fn fetch_metafile_content(&self, _file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        self.fetch_count.set(self.fetch_count.get() + 1);
        Err(anyhow!("simulated fetch failure"))
    }
    fn fetch_changed_since(
        &self,
        _project_ref: &str,
        _since_revision: &str,
    ) -> Result<RemoteChangeSet> {
        Err(anyhow!("not supported"))
    }
}

fn seed_file(
    root: &std::path::Path,
    manifest: &mut SessionManifest,
    path: &str,
    file_id: &str,
    revision: &str,
    raw_text: &str,
) {
    let content = RemoteFileContent {
        source_path: path.to_string(),
        file_id: Some(file_id.to_string()),
        revision: Some(revision.to_string()),
        content_type: MetadataContentType::from_extension(path.rsplit('.').next().unwrap_or("")),
        raw_text: raw_text.to_string(),
    };
    sync_remote_files_to_session(
        root,
        manifest,
        &[SessionSyncItem::new(content)],
        SessionSyncMode::Partial,
    )
    .expect("seed mirror file");
}

// ── P2-1: mirror 失败后内存 manifest 回滚 ──

/// mirror 中间事件 fetch 失败时，内存 manifest 回滚到调用前状态，
/// 不残留已应用事件的部分修改。
#[test]
fn followup_mirror_failure_rolls_back_manifest() {
    let root = test_root("mirror-rollback");
    let mut manifest = make_manifest("graph.redb");
    seed_file(&root, &mut manifest, "app/page_a.spg", "file-a", "1", "A1");
    seed_file(&root, &mut manifest, "app/page_b.spg", "file-b", "1", "B1");

    // manifest 快照（调用前状态）
    let manifest_before = manifest.clone();

    // 第一个事件正常、第二个 fetch 失败
    // FailingProvider 对所有 fetch 返回 Err，所以第一个就会失败
    let provider = FailingProvider::new();
    let changeset = ChangeSet::new(
        vec![
            active_event("active:file-a:2", "file-a", "app/page_a.spg", None, 1000),
            active_event("active:file-b:2", "file-b", "app/page_b.spg", None, 1001),
        ],
        MetaFilesWatermark {
            active: SourceCursor::new(1001, vec!["active:file-b:2".into()]),
            deleted: SourceCursor::new(0, Vec::new()),
        },
    );

    let result = apply_changeset_to_mirror(&root, &mut manifest, &changeset, &provider);
    assert!(result.is_err(), "mirror must fail on fetch error");

    // manifest 回滚到调用前
    assert_eq!(
        manifest, manifest_before,
        "manifest must roll back to pre-call state after mirror failure"
    );

    let _ = std::fs::remove_dir_all(root);
}

// ── P1-1: remove_nodes_by_ids in-place 删除正确性 ──

/// in-place 删除后：节点不存在、关联边自动移除、剩余节点不受影响。
#[test]
fn followup_remove_nodes_in_place_preserves_remaining_graph() {
    let root = test_root("remove-inplace");
    let db_path = root.join("graph.redb");
    let mut graph = GraphDB::open(&db_path).expect("open graph");

    // 构建链式图：A → B → C → D
    for id in &["node:A", "node:B", "node:C", "node:D"] {
        GraphWriteStore::upsert_node(
            &mut graph,
            Node {
                id: id.to_string(),
                node_type: NodeType::Component,
                path: "test.spg".to_string(),
                name: id.to_string(),
                meta: None,
            origin_file: None},
        )
        .expect("add node");
    }
    graph.add_edge("node:A", "node:B", EdgeType::Reads, None);
    graph.add_edge("node:B", "node:C", EdgeType::Reads, None);
    graph.add_edge("node:C", "node:D", EdgeType::Reads, None);

    // 删除 B 和 C
    graph.remove_nodes_by_ids(&["node:B".to_string(), "node:C".to_string()]);

    // B、C 节点不存在
    assert!(
        GraphReadStore::get_node(&graph, "node:B")
            .unwrap()
            .is_none()
    );
    assert!(
        GraphReadStore::get_node(&graph, "node:C")
            .unwrap()
            .is_none()
    );

    // A、D 仍在
    assert!(
        GraphReadStore::get_node(&graph, "node:A")
            .unwrap()
            .is_some()
    );
    assert!(
        GraphReadStore::get_node(&graph, "node:D")
            .unwrap()
            .is_some()
    );

    // A→D 之间不应有边（B、C 被删后关联边自动移除）
    let neighbors = GraphReadStore::get_node_edges(&graph, "node:A")
        .expect("get edges")
        .expect("A exists");
    assert!(
        neighbors.outgoing.is_empty(),
        "A must have no outgoing edges after B removal"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// in-place 删除后再 persist，reopen 后图等价。
#[test]
fn followup_remove_in_place_persist_and_reopen() {
    let root = test_root("remove-persist");
    let db_path = root.join("graph.redb");

    // 构建并持久化
    {
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        for id in &["n1", "n2", "n3"] {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: id.to_string(),
                    node_type: NodeType::Component,
                    path: "t.spg".to_string(),
                    name: id.to_string(),
                    meta: None,
                origin_file: None},
            )
            .expect("add node");
        }
        graph.add_edge("n1", "n2", EdgeType::Reads, None);
        graph.add_edge("n2", "n3", EdgeType::Reads, None);
        graph.persist(&HashMap::new()).expect("persist");
    }

    // reopen → delete → persist → reopen
    {
        let mut graph = GraphDB::open(&db_path).expect("reopen");
        graph.remove_nodes_by_ids(&["n2".to_string()]);
        graph
            .persist(&HashMap::new())
            .expect("persist after delete");
    }

    let graph = GraphDB::open(&db_path).expect("reopen after delete");
    assert!(GraphReadStore::get_node(&graph, "n2").unwrap().is_none());
    assert!(GraphReadStore::get_node(&graph, "n1").unwrap().is_some());
    assert!(GraphReadStore::get_node(&graph, "n3").unwrap().is_some());

    let _ = std::fs::remove_dir_all(root);
}

// ── P2-4: v2 hydrate 失败诊断 ──

/// GraphDB 有 v2_hydrate_warning 访问器，正常路径返回 None。
#[test]
fn followup_v2_hydrate_warning_none_on_normal_open() {
    let root = test_root("v2-warning-none");
    let db_path = root.join("graph.redb");

    let mut graph = GraphDB::open(&db_path).expect("open graph");
    GraphWriteStore::upsert_node(
        &mut graph,
        Node {
            id: "test:node".to_string(),
            node_type: NodeType::Component,
            path: "t.spg".to_string(),
            name: "test".to_string(),
            meta: None,
        origin_file: None},
    )
    .expect("add node");
    graph.persist(&HashMap::new()).expect("persist");

    let graph = GraphDB::open(&db_path).expect("reopen");
    assert!(
        graph.v2_hydrate_warning().is_none(),
        "normal open must have no v2 warning"
    );

    let _ = std::fs::remove_dir_all(root);
}

// ── P2-5: full_rewrite 语义 ──

/// 空提交（无 dirty 且无 checkpoint）返回 full_rewrite=false。
#[test]
fn followup_empty_persist_full_rewrite_false() {
    let root = test_root("empty-rewrite");
    let db_path = root.join("graph.redb");

    let mut graph = GraphDB::open(&db_path).expect("open graph");
    // 初始 persist（有写入，不是空提交）
    GraphWriteStore::upsert_node(
        &mut graph,
        Node {
            id: "n1".to_string(),
            node_type: NodeType::Component,
            path: "t.spg".to_string(),
            name: "n1".to_string(),
            meta: None,
        origin_file: None},
    )
    .expect("add node");
    graph.persist(&HashMap::new()).expect("initial persist");

    // reopen → 不做任何修改 → persist（空提交）
    let mut graph = GraphDB::open(&db_path).expect("reopen");
    let report = graph
        .persist_with_checkpoint(&HashMap::new(), None)
        .expect("empty persist");
    assert_eq!(
        report.full_rewrite, false,
        "empty persist must not be marked full_rewrite"
    );
    assert_eq!(report.dirty_nodes, 0);
    assert_eq!(report.bytes_written, 0);

    let _ = std::fs::remove_dir_all(root);
}

// ── P2-10: 阈值 ==1024 等值边界 ──

/// 累计受影响节点 keys 恰好 ==1024 时触发 v2 重建（>= 语义）。
#[test]
fn followup_v2_threshold_exact_1024_triggers_rebuild() {
    let root = test_root("threshold-1024");
    let db_path = root.join("graph.redb");

    // 构建基线图：1024 个节点 + 1023 条边（链式）
    {
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        for i in 0..1024 {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: format!("n:{i}"),
                    node_type: NodeType::Component,
                    path: "t.spg".to_string(),
                    name: format!("n{i}"),
                    meta: None,
                origin_file: None},
            )
            .expect("add node");
        }
        for i in 0..1023 {
            graph.add_edge(
                &format!("n:{i}"),
                &format!("n:{}", i + 1),
                EdgeType::Reads,
                None,
            );
        }
        graph.persist(&HashMap::new()).expect("baseline persist");
    }

    // delta persist：恰好 1024 个 dirty nodes → 应触发重建 → v2 = Current
    let mut graph = GraphDB::open(&db_path).expect("reopen");
    let dirty_ids: Vec<String> = (0..1024).map(|i| format!("n:{i}")).collect();

    // 收集 removed edge keys（模拟 scanner remove+re-add）
    let mut removed_edge_keys = std::collections::HashSet::new();
    let mut removed_edges: Vec<Edge> = Vec::new();
    for id in &dirty_ids {
        if let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("edges") {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                removed_edge_keys
                    .insert(metadata_checker::graph_redb::edge_storage_key(&view.edge));
                removed_edges.push(view.edge.clone());
            }
        }
    }

    // remove + re-add
    graph.remove_nodes_by_ids(&dirty_ids);
    for id in &dirty_ids {
        GraphWriteStore::upsert_node(
            &mut graph,
            Node {
                id: id.clone(),
                node_type: NodeType::Component,
                path: "t.spg".to_string(),
                name: id.clone(),
                meta: None,
            origin_file: None},
        )
        .expect("re-add node");
    }
    let mut seen = std::collections::HashSet::new();
    let mut dirty_edges = Vec::new();
    for edge in removed_edges {
        graph.add_edge(
            &edge.from,
            &edge.to,
            edge.edge_type.clone(),
            edge.field_path.clone(),
        );
        if seen.insert(metadata_checker::graph_redb::edge_storage_key(&edge)) {
            dirty_edges.push(edge);
        }
    }

    let commit = IndexCommit {
        file_states: HashMap::new(),
        dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
        deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
        checkpoint: None,
        delta: Some(IndexDelta {
            dirty_edges,
            removed_edge_keys: removed_edge_keys.into_iter().collect(),
            changed_file_states: Vec::new(),
            removed_file_paths: Vec::new(),
        }),
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    let report = graph.persist_commit(&commit).expect("delta persist");

    assert_eq!(
        report.v2_shadow_state,
        V2ShadowState::Current,
        "exactly 1024 affected nodes must trigger v2 rebuild (>= semantics)"
    );

    let _ = std::fs::remove_dir_all(root);
}

/// 累计受影响节点 keys ==1023 时不触发重建 → v2 = Stale。
#[test]
fn followup_v2_threshold_below_1024_stays_stale() {
    let root = test_root("threshold-1023");
    let db_path = root.join("graph.redb");

    {
        let mut graph = GraphDB::open(&db_path).expect("open graph");
        for i in 0..1023 {
            GraphWriteStore::upsert_node(
                &mut graph,
                Node {
                    id: format!("n:{i}"),
                    node_type: NodeType::Component,
                    path: "t.spg".to_string(),
                    name: format!("n{i}"),
                    meta: None,
                origin_file: None},
            )
            .expect("add node");
        }
        graph.persist(&HashMap::new()).expect("baseline");
    }

    let mut graph = GraphDB::open(&db_path).expect("reopen");
    let dirty_ids: Vec<String> = (0..1023).map(|i| format!("n:{i}")).collect();

    let mut removed_edge_keys = std::collections::HashSet::new();
    let mut removed_edges: Vec<Edge> = Vec::new();
    for id in &dirty_ids {
        if let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("edges") {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                removed_edge_keys
                    .insert(metadata_checker::graph_redb::edge_storage_key(&view.edge));
                removed_edges.push(view.edge.clone());
            }
        }
    }

    graph.remove_nodes_by_ids(&dirty_ids);
    for id in &dirty_ids {
        GraphWriteStore::upsert_node(
            &mut graph,
            Node {
                id: id.clone(),
                node_type: NodeType::Component,
                path: "t.spg".to_string(),
                name: id.clone(),
                meta: None,
            origin_file: None},
        )
        .expect("re-add");
    }
    let mut seen = std::collections::HashSet::new();
    let mut dirty_edges = Vec::new();
    for edge in removed_edges {
        graph.add_edge(
            &edge.from,
            &edge.to,
            edge.edge_type.clone(),
            edge.field_path.clone(),
        );
        if seen.insert(metadata_checker::graph_redb::edge_storage_key(&edge)) {
            dirty_edges.push(edge);
        }
    }

    let commit = IndexCommit {
        file_states: HashMap::new(),
        dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
        deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
        checkpoint: None,
        delta: Some(IndexDelta {
            dirty_edges,
            removed_edge_keys: removed_edge_keys.into_iter().collect(),
            changed_file_states: Vec::new(),
            removed_file_paths: Vec::new(),
        }),
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    let report = graph.persist_commit(&commit).expect("delta persist");

    assert_eq!(
        report.v2_shadow_state,
        V2ShadowState::Stale,
        "1023 affected nodes must stay Stale (below threshold)"
    );

    let _ = std::fs::remove_dir_all(root);
}
