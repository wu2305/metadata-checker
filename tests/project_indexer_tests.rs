#[path = "common/memory_graph_store.rs"]
mod memory_graph_store;

use anyhow::{Result, bail};
use metadata_checker::graph::FileState;
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{
    GraphReadStore, GraphStoreResult, IndexCommit, IndexReport, IndexStateStore,
};
use metadata_checker::scanner::indexer::{DirtyFile, ProjectIndexer};
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

/// 测试 parse_dirty_files 在 dirty 文件已预携带 bytes 时不会重复读取
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
    let db_path = temp_dir.join(".metadata-checker.graphdb");
    let mut graph = GraphDB::open(&db_path).expect("open graph must succeed");

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
    let dirty: Vec<DirtyFile> = vec![(rel, spg_file.clone(), Some(staged_bytes))];
    let provider = TestDocumentProvider::new(false);

    let new_states = ProjectIndexer::parse_dirty_files(
        &mut graph,
        &HashMap::new(),
        &dirty,
        &temp_dir,
        &provider,
    )
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
    assert!(new_states.len() >= 1, "应有解析后的文件状态记录");

    let _ = std::fs::remove_dir_all(&temp_dir);
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
