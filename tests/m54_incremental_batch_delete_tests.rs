#![cfg(feature = "cli-local")]
//! M54 Task 5：多 dirty/deleted 文件只做一次图批删
//!
//! 用计数 test double 断言 `apply_incremental_changes` 对
//! remove_nodes_by_ids 的调用次数与合并去重后的删除集合。

use std::collections::HashMap;
use std::path::Path;

use metadata_checker::graph::{Edge, FileState, Node};
use metadata_checker::graph_store::{GraphStoreResult, GraphWriteStore};
use metadata_checker::scanner::indexer::{
    DeletedFile, ParsedGraphContent, ParsedGraphUpdate, ProjectIndexer,
};

/// 记录 remove_nodes_by_ids 调用次数与删除集合的写存储 test double。
#[derive(Default)]
struct CountingWriteStore {
    remove_calls: usize,
    removed: Vec<String>,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

impl GraphWriteStore for CountingWriteStore {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        self.nodes.push(node);
        Ok(())
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        self.edges.push(edge);
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        self.remove_calls += 1;
        self.removed.extend(node_ids.iter().cloned());
        Ok(())
    }
}

/// 构造一个带 previous_node_ids 的 TBL 解析更新。
fn dirty_update(index: usize) -> ParsedGraphUpdate {
    ParsedGraphUpdate {
        logical_path: format!("tables/t{index}.tbl"),
        physical_path: Path::new("tables").join(format!("t{index}.tbl")),
        file_hash: format!("hash-{index}"),
        mtime: 0,
        size: 1,
        previous_node_ids: vec![format!("model:old_{index}")],
        content: ParsedGraphContent::Tbl(
            serde_json::json!({
                "dimensions": [
                    {"name": "id"},
                    {"name": format!("field_{index}")}
                ]
            })
            .to_string(),
        ),
    }
}

fn file_state(rel: &str, node_ids: Vec<String>) -> FileState {
    FileState {
        file_path: rel.to_string(),
        file_hash: "hash".to_string(),
        mtime: 0,
        size: 1,
        node_ids,
    }
}

/// 20 个 dirty 文件合并为一次批删，删除集合为全部 previous_node_ids。
#[test]
fn m54_batch_delete_twenty_dirty_files_single_remove_call() {
    let updates: Vec<ParsedGraphUpdate> = (0..20).map(dirty_update).collect();
    let mut store = CountingWriteStore::default();
    let mut states = HashMap::new();

    let touched = ProjectIndexer::apply_incremental_changes(&mut store, &mut states, &updates, &[])
        .expect("apply incremental changes");

    assert_eq!(store.remove_calls, 1, "20 个 dirty 文件只允许一次批删");
    let expected: Vec<String> = (0..20).map(|index| format!("model:old_{index}")).collect();
    assert_eq!(store.removed, expected);
    // 每个 dirty 文件都返回了新增节点
    assert_eq!(touched.len(), 20);
    assert!(!store.nodes.is_empty(), "批删后仍应写入新增节点");
}

/// 20 个 deleted 文件合并为一次批删，并移除对应 file states。
#[test]
fn m54_batch_delete_twenty_deleted_files_single_remove_call() {
    let deleted: Vec<DeletedFile> = (0..20)
        .map(|index| {
            (
                format!("tables/d{index}.tbl"),
                vec![format!("model:del_{index}")],
            )
        })
        .collect();
    let mut states: HashMap<String, FileState> = deleted
        .iter()
        .map(|(rel, node_ids)| (rel.clone(), file_state(rel, node_ids.clone())))
        .collect();
    let mut store = CountingWriteStore::default();

    ProjectIndexer::apply_incremental_changes(&mut store, &mut states, &[], &deleted)
        .expect("apply incremental changes");

    assert_eq!(store.remove_calls, 1, "20 个 deleted 文件只允许一次批删");
    let expected: Vec<String> = (0..20).map(|index| format!("model:del_{index}")).collect();
    assert_eq!(store.removed, expected);
    assert!(states.is_empty(), "deleted 文件的 file states 应被移除");
}

/// dirty + deleted 混合仍只调一次批删，重叠 node ID 去重。
#[test]
fn m54_batch_delete_mixed_dirty_and_deleted_single_deduped_remove_call() {
    let updates: Vec<ParsedGraphUpdate> = (0..10).map(dirty_update).collect();
    // deleted IDs 与 dirty previous IDs 重叠 old_0..old_4，外加 del_5..del_9
    let deleted: Vec<DeletedFile> = (0..10)
        .map(|index| {
            let node_id = if index < 5 {
                format!("model:old_{index}")
            } else {
                format!("model:del_{index}")
            };
            (format!("tables/d{index}.tbl"), vec![node_id])
        })
        .collect();
    let mut states: HashMap<String, FileState> = deleted
        .iter()
        .map(|(rel, node_ids)| (rel.clone(), file_state(rel, node_ids.clone())))
        .collect();
    let mut store = CountingWriteStore::default();

    let touched =
        ProjectIndexer::apply_incremental_changes(&mut store, &mut states, &updates, &deleted)
            .expect("apply incremental changes");

    assert_eq!(store.remove_calls, 1, "dirty+deleted 混合只允许一次批删");
    // 合并去重：old_0..old_9（10 个）+ del_5..del_9（5 个）= 15 个，保持首次出现顺序
    let mut expected: Vec<String> = (0..10).map(|index| format!("model:old_{index}")).collect();
    expected.extend((5..10).map(|index| format!("model:del_{index}")));
    assert_eq!(store.removed, expected);
    assert_eq!(touched.len(), 10);
    assert!(states.is_empty());
}

/// 兼容入口行为一致：apply_graph_updates 对多 dirty 文件同样只做一次批删。
#[test]
fn m54_apply_graph_updates_compat_entry_batches_removal() {
    let updates: Vec<ParsedGraphUpdate> = (0..20).map(dirty_update).collect();
    let mut store = CountingWriteStore::default();

    ProjectIndexer::apply_graph_updates(&mut store, &updates).expect("apply graph updates");

    assert_eq!(store.remove_calls, 1);
    assert_eq!(store.removed.len(), 20);
}

/// 兼容入口行为一致：apply_deletions 对多 deleted 文件同样只做一次批删。
#[test]
fn m54_apply_deletions_compat_entry_batches_removal() {
    let deleted: Vec<DeletedFile> = (0..20)
        .map(|index| {
            (
                format!("tables/d{index}.tbl"),
                vec![format!("model:del_{index}")],
            )
        })
        .collect();
    let mut states: HashMap<String, FileState> = deleted
        .iter()
        .map(|(rel, node_ids)| (rel.clone(), file_state(rel, node_ids.clone())))
        .collect();
    let mut store = CountingWriteStore::default();

    ProjectIndexer::apply_deletions(&mut store, &mut states, &deleted).expect("apply deletions");

    assert_eq!(store.remove_calls, 1);
    assert_eq!(store.removed.len(), 20);
    assert!(states.is_empty());
}
