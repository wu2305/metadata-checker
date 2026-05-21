mod common;

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::scan_project;
use metadata_checker::storage_provider::LocalStorageProvider;
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

/// 测试 ProjectIndexer 的 diff_file_states 能正确区分 dirty、deleted 和 unchanged
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
    let plan = ProjectIndexer::diff_file_states(
        &remaining_files,
        &prev_states,
        &temp_dir,
        &LocalStorageProvider,
    )
    .expect("diff must succeed");

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

/// 测试 scan_project 调用 ProjectIndexer 后行为不变（graph 能正常构建）
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
    use common::memory_graph_store::MemoryGraphStore;

    let mut store = MemoryGraphStore::new();
    let node_a = metadata_checker::graph::Node {
        id: "page:a".to_string(),
        name: "Page A".to_string(),
        node_type: metadata_checker::graph::NodeType::Page,
        path: "a.spg".to_string(),
        meta: None,
    };
    let node_b = metadata_checker::graph::Node {
        id: "page:b".to_string(),
        name: "Page B".to_string(),
        node_type: metadata_checker::graph::NodeType::Page,
        path: "b.spg".to_string(),
        meta: None,
    };
    let node_model = metadata_checker::graph::Node {
        id: "model:m1".to_string(),
        name: "Model 1".to_string(),
        node_type: metadata_checker::graph::NodeType::Model,
        path: "m1.tbl".to_string(),
        meta: None,
    };

    store.upsert_node(node_a.clone()).unwrap();
    store.upsert_node(node_b.clone()).unwrap();
    store.upsert_node(node_model.clone()).unwrap();

    store
        .add_edge(metadata_checker::graph::Edge {
            from: "page:a".to_string(),
            to: "model:m1".to_string(),
            edge_type: metadata_checker::graph::EdgeType::Reads,
            field_path: None,
            meta: None,
        })
        .unwrap();

    store
        .add_edge(metadata_checker::graph::Edge {
            from: "page:b".to_string(),
            to: "model:m1".to_string(),
            edge_type: metadata_checker::graph::EdgeType::Reads,
            field_path: None,
            meta: None,
        })
        .unwrap();

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
