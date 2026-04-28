use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::hash::Hasher;
use std::path::Path;
use std::time::SystemTime;
use twox_hash::XxHash64;

use crate::graph::{FileState, GraphDB};

/// 项目目录扫描模块
///
/// 递归扫描项目目录下的所有 .spg 和 .tbl 文件：
/// - .spg：解析组件、表达式、动作，构建 Page/Component/Model/Field 节点
/// - .tbl：解析 App 类型（可写）和 DataFlow 类型（只读加工流程）
///
/// 支持增量更新：对比文件 mtime/size/hash，只重新处理变更文件。
/// Scan a project directory and build/update the graph database.
pub fn scan_project(project_dir: &Path, db_path: &Path) -> Result<()> {
    let mut graph = GraphDB::open(db_path)?;
    let prev_states = graph.load_file_states().unwrap_or_default();

    // Collect all .spg and .tbl files
    let mut files = Vec::new();
    collect_files(project_dir, project_dir, &mut files)?;

    // Determine which files changed or were deleted
    let mut dirty_files = Vec::new();
    let mut current_paths = HashMap::new();
    for path in &files {
        let rel = path
            .strip_prefix(project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        current_paths.insert(rel.clone(), path.clone());

        let content_bytes = fs::read(path)?;
        let mut hasher = XxHash64::default();
        hasher.write(&content_bytes);
        let file_hash = format!("{:x}", hasher.finish());

        match prev_states.get(&rel) {
            Some(state) if state.file_hash == file_hash => {
                // File unchanged, skip
            }
            _ => {
                dirty_files.push((rel, path.clone(), content_bytes));
            }
        }
    }

    // Find deleted files
    let mut deleted_files = Vec::new();
    for (rel, state) in &prev_states {
        if !current_paths.contains_key(rel) {
            deleted_files.push((rel.clone(), state.node_ids.clone()));
        }
    }

    eprintln!(
        "Indexed {} files | Unchanged: {} | Dirty: {} | Deleted: {}",
        files.len(),
        files.len() - dirty_files.len(),
        dirty_files.len(),
        deleted_files.len()
    );

    // Process dirty files
    let mut new_states = prev_states.clone();
    for (rel, node_ids) in &deleted_files {
        graph.remove_nodes_by_ids(node_ids);
        new_states.remove(rel);
    }

    if dirty_files.is_empty() && deleted_files.is_empty() {
        eprintln!("No graph changes detected; skip persistence");
        return Ok(());
    }

    for (rel, path, content_bytes) in &dirty_files {
        // Remove old nodes if updating
        if let Some(old_state) = prev_states.get(rel) {
            graph.remove_nodes_by_ids(&old_state.node_ids);
        }

        let node_ids = if path.extension().map(|e| e == "spg").unwrap_or(false) {
            let raw_value: serde_json::Value = serde_json::from_slice(content_bytes)?;
            process_spg_file_from_value(&mut graph, rel, raw_value)?
        } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
            let content = String::from_utf8_lossy(content_bytes);
            process_tbl_file_from_string(&mut graph, rel, &content)?
        } else {
            Vec::new()
        };

        // Reuse hash from scan loop
        let mut hasher = XxHash64::default();
        hasher.write(content_bytes);
        let file_hash = format!("{:x}", hasher.finish());

        let metadata = fs::metadata(path)?;
        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let size = metadata.len();

        new_states.insert(
            rel.clone(),
            FileState {
                file_path: rel.clone(),
                file_hash,
                mtime,
                size,
                node_ids,
            },
        );
    }

    // Persist graph and file states
    graph.persist(&new_states)?;
    Ok(())
}

mod spg;
mod tbl;
/// 递归收集目录下所有 .spg 和 .tbl 文件
mod utils;

pub use spg::process_spg_file_from_value;
pub use tbl::process_tbl_file_from_string;
use utils::*;
