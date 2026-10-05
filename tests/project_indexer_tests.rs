#![cfg(feature = "cli-local")]

#[path = "common/memory_graph_store.rs"]
mod memory_graph_store;

use anyhow::{Result, bail};
use metadata_checker::graph::{Edge, FileState, GraphDB, Node, NodeType};
use metadata_checker::graph_store::{
    GraphReadStore, GraphStoreResult, GraphWriteStore, IndexCommit, IndexReport, IndexStateStore,
};
use metadata_checker::scanner::indexer::{
    DirtyFile, ParsedGraphContent, ParsedGraphUpdate, ProjectIndexer,
};
use metadata_checker::scanner::scan_project;
use metadata_checker::storage_provider::{DocumentFileMetadata, DocumentProvider};
use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;

/// 测试 ProjectIndexer 能正确发现项目目录下的 .spg 和 .tbl 文件
#[test]
fn test_project_indexer_discovers_spg_tbl() {
    let project_dir = Path::new("tests/fixtures/test_project");
    let files = ProjectIndexer::discover_files(project_dir).expect("discover_files must succeed");

    let spg_count = files
        .iter()
        .filter(|p| p.extension().map(|e| e == "spg").unwrap_or(false))
        .count();
    let tbl_count = files
        .iter()
        .filter(|p| p.extension().map(|e| e == "tbl").unwrap_or(false))
        .count();

    assert!(
        spg_count > 0,
        "应至少发现一个 .spg 文件，实际发现 {}",
        spg_count
    );
    assert!(
        tbl_count > 0,
        "应至少发现一个 .tbl 文件，实际发现 {}",
        tbl_count
    );
}

/// discover_files 仅做路径发现，不需要 provider 入参。
#[test]
fn test_project_indexer_discover_files_without_provider() {
    let project_dir = Path::new("tests/fixtures/test_project");
    let _ = ProjectIndexer::discover_files(project_dir).expect("discover_files should succeed");
}

#[derive(Default)]
struct TestDocumentProvider {
    read_calls: Cell<usize>,
    metadata_calls: Cell<usize>,
    allow_read: bool,
}

impl TestDocumentProvider {
    fn new(allow_read: bool) -> Self {
        Self {
            read_calls: Cell::new(0),
            metadata_calls: Cell::new(0),
            allow_read,
        }
    }

    fn read_calls(&self) -> usize {
        self.read_calls.get()
    }
}

impl DocumentProvider for TestDocumentProvider {
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        self.read_calls.set(self.read_calls.get() + 1);
        if !self.allow_read {
            bail!("should not read bytes when staged dirty file provides bytes");
        }
        Ok(std::fs::read(path)?)
    }

    fn metadata(&self, path: &Path) -> Result<DocumentFileMetadata> {
        self.metadata_calls.set(self.metadata_calls.get() + 1);
        let metadata = std::fs::metadata(path)?;
        Ok(DocumentFileMetadata {
            modified: metadata.modified().ok(),
            size: metadata.len(),
        })
    }
}

/// 测试 ProjectIndexer 的 diff_file_states 需要 provider 处理已发现文件并生成脏文件计划
#[test]
fn test_project_indexer_diff_detects_dirty_deleted_unchanged() {
    let temp_dir = std::env::temp_dir().join(format!(
        "m39-diff-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let src = Path::new("tests/fixtures/test_project");
    copy_dir(src, &temp_dir);

    let db_path = temp_dir.join(".metadata-checker.graphdb");
    scan_project(&temp_dir, &db_path).expect("initial scan must succeed");

    let files = ProjectIndexer::discover_files(&temp_dir).expect("discover must succeed");
    let spg_files: Vec<_> = files
        .iter()
        .filter(|p| p.extension().map(|e| e == "spg").unwrap_or(false))
        .cloned()
        .collect();
    let tbl_files: Vec<_> = files
        .iter()
        .filter(|p| p.extension().map(|e| e == "tbl").unwrap_or(false))
        .cloned()
        .collect();

    assert!(!spg_files.is_empty(), "需要至少一个 spg 文件做 dirty 测试");
    assert!(
        !tbl_files.is_empty(),
        "需要至少一个 tbl 文件做 deleted 测试"
    );

    let dirty_spg = &spg_files[0];
    let deleted_tbl = &tbl_files[0];

    let original = std::fs::read_to_string(dirty_spg).unwrap();
    std::fs::write(dirty_spg, format!("{}\n", original)).unwrap();
    std::fs::remove_file(deleted_tbl).unwrap();

    let prev_states = GraphDB::open(&db_path)
        .unwrap()
        .load_file_states()
        .unwrap_or_default();

    let remaining_files: Vec<_> = files.into_iter().filter(|p| p != deleted_tbl).collect();
    let provider = TestDocumentProvider::new(true);
    let plan =
        ProjectIndexer::diff_file_states(&remaining_files, &prev_states, &temp_dir, &provider)
            .expect("diff must succeed");

    assert_eq!(
        provider.read_calls(),
        remaining_files.len(),
        "diff 阶段应读入每个已发现文件"
    );

    let dirty_rels: Vec<_> = plan.dirty.iter().map(|(rel, _, _)| rel.clone()).collect();
    let deleted_rels: Vec<_> = plan.deleted.iter().map(|(rel, _)| rel.clone()).collect();

    let dirty_rel = dirty_spg
        .strip_prefix(&temp_dir)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let deleted_rel = deleted_tbl
        .strip_prefix(&temp_dir)
        .unwrap()
        .to_string_lossy()
        .to_string();

    assert!(
        dirty_rels.contains(&dirty_rel),
        "dirty 列表应包含修改的文件 {}",
        dirty_rel
    );
    assert!(
        deleted_rels.contains(&deleted_rel),
        "deleted 列表应包含删除的文件 {}",
        deleted_rel
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 测试 parse_dirty_files 在 dirty 文件已预携带 bytes 时不会重复读取，并且不需要 Graph 写接口。
#[test]
fn test_project_indexer_parse_dirty_files_uses_staged_bytes() {
    let temp_dir = std::env::temp_dir().join(format!(
        "m39-parse-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let src = Path::new("tests/fixtures/test_project");
    copy_dir(src, &temp_dir);

    let files = ProjectIndexer::discover_files(&temp_dir).expect("discover files");
    let spg_file = files
        .iter()
        .find(|p| p.extension().map(|e| e == "spg").unwrap_or(false))
        .expect("at least one spg");
    let rel = spg_file
        .strip_prefix(&temp_dir)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let staged_bytes = std::fs::read(spg_file).unwrap();
    let dirty: Vec<DirtyFile> = vec![(rel.clone(), spg_file.clone(), Some(staged_bytes))];
    let provider = TestDocumentProvider::new(false);

    let updates = ProjectIndexer::parse_dirty_files(&HashMap::new(), &dirty, &temp_dir, &provider)
        .expect("parse should succeed");

    assert!(
        provider.metadata_calls.get() > 0,
        "parse 阶段应读取文件元数据"
    );
    assert_eq!(
        provider.read_calls.get(),
        0,
        "有 content 的 dirty 不应再次读取文件"
    );
    assert_eq!(updates.len(), 1, "应有一条解析更新");
    assert!(!updates[0].logical_path.is_empty(), "应有逻辑路径");
    assert_eq!(updates[0].logical_path, rel, "更新应对应同一文件");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 测试 parse 阶段产物可直接交给 apply 阶段写入图（仅 apply 需要图接口）。
#[test]
fn test_project_indexer_apply_graph_updates_uses_parsed_updates_to_write_graph() {
    let temp_dir = std::env::temp_dir().join(format!(
        "m39-apply-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let src = Path::new("tests/fixtures/test_project");
    copy_dir(src, &temp_dir);

    let files = ProjectIndexer::discover_files(&temp_dir).expect("discover files");
    let spg_file = files
        .iter()
        .find(|p| p.extension().map(|e| e == "spg").unwrap_or(false))
        .expect("at least one spg");
    let rel = spg_file
        .strip_prefix(&temp_dir)
        .unwrap()
        .to_string_lossy()
        .to_string();
    let staged_bytes = std::fs::read(spg_file).unwrap();
    let provider = TestDocumentProvider::new(false);

    let updates = ProjectIndexer::parse_dirty_files(
        &HashMap::new(),
        &vec![(rel.clone(), spg_file.clone(), Some(staged_bytes))],
        &temp_dir,
        &provider,
    )
    .expect("parse should succeed");

    let db_path = temp_dir.join(".metadata-checker.graphdb");
    let mut graph = GraphDB::open(&db_path).expect("open graph must succeed");
    let applied = ProjectIndexer::apply_graph_updates(&mut graph, &updates)
        .expect("apply graph updates should succeed");

    assert_eq!(
        applied.get(&rel).map(|nodes| nodes.len()).unwrap_or(0) > 0,
        true
    );
    assert!(
        GraphReadStore::node_count(&graph).unwrap() > 0,
        "apply 阶段写入后 graph 应有节点"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 测试 apply_graph_updates 只依赖 GraphWriteStore，不要求读接口或 IndexStateStore。
#[test]
fn test_project_indexer_apply_graph_updates_accepts_write_only_store() {
    #[derive(Default)]
    struct WriteOnlyGraphStore {
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        removed: Vec<String>,
    }

    impl GraphWriteStore for WriteOnlyGraphStore {
        fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
            self.nodes.push(node);
            Ok(())
        }

        fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
            self.edges.push(edge);
            Ok(())
        }

        fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
            self.removed.extend(node_ids.iter().cloned());
            Ok(())
        }

        fn node_type_of(&self, node_id: &str) -> GraphStoreResult<Option<NodeType>> {
            Ok(self
                .nodes
                .iter()
                .rev()
                .find(|node| node.id == node_id)
                .map(|node| node.node_type.clone()))
        }
    }

    let update = ParsedGraphUpdate {
        logical_path: "tables/user.tbl".to_string(),
        physical_path: Path::new("tables/user.tbl").to_path_buf(),
        file_hash: "hash".to_string(),
        mtime: 0,
        size: 1,
        previous_node_ids: vec!["model:old_user".to_string()],
        content: ParsedGraphContent::Tbl(
            serde_json::json!({
                "dimensions": [
                    {"name": "id"},
                    {"name": "name"}
                ]
            })
            .to_string(),
        ),
    };
    let mut store = WriteOnlyGraphStore::default();

    let applied = ProjectIndexer::apply_graph_updates(&mut store, &[update])
        .expect("apply_graph_updates should work with write-only store");

    assert_eq!(store.removed, vec!["model:old_user".to_string()]);
    assert!(
        store
            .nodes
            .iter()
            .any(|node| node.node_type == NodeType::Model),
        "apply 阶段应通过写接口插入模型节点"
    );
    assert!(!store.edges.is_empty(), "apply 阶段应通过写接口插入关系边");
    assert!(
        applied
            .get("tables/user.tbl")
            .map(|nodes| !nodes.is_empty())
            .unwrap_or(false),
        "apply 应返回逻辑文件对应的 node_ids"
    );
}

/// 测试 scan_project 行为不变（graph 能正常构建）
#[test]
fn test_scan_project_behavior_unchanged() {
    let temp_dir = std::env::temp_dir().join(format!(
        "m39-scan-test-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();

    let src = Path::new("tests/fixtures/test_project");
    copy_dir(src, &temp_dir);

    let db_path = temp_dir.join(".metadata-checker.graphdb");
    scan_project(&temp_dir, &db_path).expect("scan_project must succeed");

    let graph = GraphDB::open(&db_path).expect("open graph must succeed");
    assert!(
        GraphReadStore::node_count(&graph).unwrap() > 0,
        "scan_project 应构建出非空图"
    );

    let states = graph
        .load_file_states()
        .expect("load file states must succeed");
    assert!(!states.is_empty(), "scan_project 应持久化文件状态");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 测试 query 辅助函数不直接依赖 GraphDB 具体类型（通过 trait 对象也能工作）
#[test]
fn test_query_functions_do_not_require_graphdb() {
    use memory_graph_store::MemoryGraphStore;

    let mut store = MemoryGraphStore::new();
    store.add_test_node(
        "page:a",
        "Page A",
        metadata_checker::graph::NodeType::Page,
        "a.spg",
    );
    store.add_test_node(
        "page:b",
        "Page B",
        metadata_checker::graph::NodeType::Page,
        "b.spg",
    );
    store.add_test_node(
        "model:m1",
        "Model 1",
        metadata_checker::graph::NodeType::Model,
        "m1.tbl",
    );
    store.add_test_edge(
        "page:a",
        "model:m1",
        metadata_checker::graph::EdgeType::Reads,
        None,
    );
    store.add_test_edge(
        "page:b",
        "model:m1",
        metadata_checker::graph::EdgeType::Reads,
        None,
    );

    let read_store: &dyn GraphReadStore = &store;
    let edges = read_store
        .get_node_edges("model:m1")
        .expect("get_node_edges must succeed");
    assert!(edges.is_some(), "model:m1 应有 incoming edges");
    let neighbors = edges.unwrap();
    let readers: Vec<_> = neighbors
        .incoming
        .into_iter()
        .filter(|e| matches!(e.edge.edge_type, metadata_checker::graph::EdgeType::Reads))
        .map(|e| e.node)
        .collect();

    assert_eq!(readers.len(), 2, "应有 2 个 reader");
}

/// 验证 persist_index 的提交边界走 IndexStateStore::persist_index，不直接依赖 GraphDB 细节。
#[test]
fn test_project_indexer_persist_index_uses_index_state_store_boundary() {
    #[derive(Default)]
    struct TrackingIndexStateStore {
        persisted_commit: Option<IndexCommit>,
        persist_calls: usize,
    }

    impl IndexStateStore for TrackingIndexStateStore {
        fn load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>> {
            Ok(HashMap::new())
        }

        fn persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport> {
            self.persist_calls += 1;
            self.persisted_commit = Some(commit);
            let file_states = self.persisted_commit.as_ref().expect("commit stored");
            Ok(IndexReport {
                indexed: file_states.file_states.len(),
                unchanged: file_states
                    .file_states
                    .len()
                    .saturating_sub(file_states.dirty_nodes.len()),
                dirty: file_states.dirty_nodes.len(),
                deleted: file_states.deleted_nodes.len(),
            })
        }
    }

    let mut store = TrackingIndexStateStore::default();
    let mut file_states = HashMap::new();
    file_states.insert(
        "test.spg".to_string(),
        FileState {
            file_path: "test.spg".to_string(),
            file_hash: "hash".to_string(),
            mtime: 0,
            size: 1,
            node_ids: vec![],
        },
    );
    let commit = IndexCommit {
        file_states,
        dirty_nodes: vec!["page:test".to_string()],
        deleted_nodes: vec![],
        checkpoint: None,
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };

    let report = ProjectIndexer::persist_index(&mut store, commit.clone())
        .expect("persist_index should call IndexStateStore");

    assert_eq!(
        store.persist_calls, 1,
        "persist_index 应该走一次 IndexStateStore 入口"
    );
    assert_eq!(report.indexed, 1);
    assert_eq!(report.dirty, 1);
    assert_eq!(
        store
            .persisted_commit
            .as_ref()
            .expect("commit should be recorded")
            .dirty_nodes,
        commit.dirty_nodes,
    );
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}
