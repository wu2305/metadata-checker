use anyhow::{Context, Result};
use std::collections::HashMap;
use std::hash::Hasher;
use std::path::Path;
use std::time::SystemTime;
use twox_hash::XxHash64;

use super::{process_spg_file_from_value, process_tbl_file_from_string};
use crate::graph::{FileState, GraphDB};
use crate::graph_store::{GraphStore, IndexCommit, IndexReport, IndexStateStore};
use crate::parsed_content::ParsedContent;
use crate::source_id::{ProjectRef, SourceId};
use crate::storage_provider::{DocumentProvider, LocalStorageProvider};

/// 发现的文件数量（阶段内部使用，不暴露字段级细节）

/// 脏文件（需要重新解析）
pub type DirtyFile = (String, std::path::PathBuf, Vec<u8>);
/// 已删除文件记录
pub type DeletedFile = (String, Vec<String>);

/// 索引计划
#[derive(Debug)]
pub struct IndexPlan {
    pub discovered_count: usize,
    pub dirty: Vec<DirtyFile>,
    pub deleted: Vec<DeletedFile>,
}

/// 项目索引器
///
/// M39：把 scan_project 拆为可测试的阶段。
pub struct ProjectIndexer;

impl ProjectIndexer {
    /// 阶段 1：递归发现项目目录下所有 .spg 和 .tbl 文件
    pub fn discover_files(project_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
        let mut files = Vec::new();
        Self::collect_files(project_dir, project_dir, &mut files)?;
        Ok(files)
    }

    fn collect_files(
        base: &Path,
        current: &Path,
        files: &mut Vec<std::path::PathBuf>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                Self::collect_files(base, &path, files)?;
            } else if path
                .extension()
                .map(|e| e == "spg" || e == "tbl")
                .unwrap_or(false)
            {
                files.push(path);
            }
        }
        Ok(())
    }

    /// 阶段 2：对比文件 hash，找出脏文件和已删除文件
    pub fn diff_file_states(
        files: &[std::path::PathBuf],
        prev_states: &HashMap<String, FileState>,
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<IndexPlan> {
        let mut discovered_count: usize = 0;
        let mut dirty = Vec::new();
        let mut current_paths: HashMap<String, std::path::PathBuf> = HashMap::new();

        for path in files {
            let rel = path
                .strip_prefix(project_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            current_paths.insert(rel.clone(), path.clone());

            let content_bytes = provider.read_bytes(path)?;
            let mut hasher = XxHash64::default();
            hasher.write(&content_bytes);
            let file_hash = format!("{:x}", hasher.finish());

            discovered_count += 1;

            match prev_states.get(&rel) {
                Some(state) if state.file_hash == file_hash => {
                    // File unchanged, skip
                }
                _ => {
                    dirty.push((rel, path.clone(), content_bytes));
                }
            }
        }

        let mut deleted = Vec::new();
        for (rel, state) in prev_states {
            if !current_paths.contains_key(rel) {
                deleted.push((rel.clone(), state.node_ids.clone()));
            }
        }

        Ok(IndexPlan {
            discovered_count,
            dirty,
            deleted,
        })
    }

    /// 阶段 3：解析脏文件内容并更新图存储
    pub fn parse_dirty_files(
        graph: &mut dyn GraphStore,
        prev_states: &HashMap<String, FileState>,
        dirty_files: &[DirtyFile],
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<HashMap<String, FileState>> {
        let mut new_states = prev_states.clone();

        for (rel, path, content_bytes) in dirty_files {
            // Remove old nodes if updating
            if let Some(old_state) = prev_states.get(rel) {
                let _ = graph.remove_nodes_by_ids(&old_state.node_ids);
                new_states.remove(rel);
            }

            let node_ids = if path.extension().map(|e| e == "spg").unwrap_or(false) {
                let source =
                    SourceId::from_local_path(ProjectRef::new("default"), path, Some(project_dir))
                        .with_context(|| format!("Failed to build SourceId for {}", rel))?;
                let parsed = ParsedContent::from_bytes(source, content_bytes.clone());
                let raw_value = parsed
                    .json()
                    .with_context(|| format!("Failed to parse JSON for {}", rel))?;
                process_spg_file_from_value(graph, rel, (*raw_value).clone())?
            } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
                let content = String::from_utf8_lossy(content_bytes);
                process_tbl_file_from_string(graph, rel, &content)?
            } else {
                Vec::new()
            };

            let mut hasher = XxHash64::default();
            hasher.write(content_bytes);
            let file_hash = format!("{:x}", hasher.finish());

            let metadata = provider.metadata(path)?;
            let mtime = metadata
                .modified
                .unwrap_or(SystemTime::UNIX_EPOCH)
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let size = metadata.size;

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

        Ok(new_states)
    }

    /// 阶段 4：应用删除操作
    pub fn apply_deletions(
        graph: &mut dyn GraphStore,
        new_states: &mut HashMap<String, FileState>,
        deleted: &[DeletedFile],
    ) {
        for (rel, node_ids) in deleted {
            graph
                .remove_nodes_by_ids(node_ids)
                .expect("remove_nodes_by_ids must succeed");
            new_states.remove(rel);
        }
    }

    /// 阶段 5：持久化索引结果
    pub fn persist_index(graph: &mut GraphDB, commit: &IndexCommit) -> Result<IndexReport> {
        IndexStateStore::persist_index(graph, commit)
            .map_err(|e| anyhow::anyhow!("persist_index failed: {}", e))
    }

    /// 全量索引入口（替代 scan_project）
    pub fn scan(project_dir: &Path, db_path: &Path) -> Result<IndexReport> {
        let mut graph = GraphDB::open(db_path)?;
        let prev_states = graph.load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        let mut new_states = prev_states.clone();
        Self::apply_deletions(&mut graph, &mut new_states, &plan.deleted);

        if !plan.dirty.is_empty() || !plan.deleted.is_empty() {
            new_states = Self::parse_dirty_files(
                &mut graph,
                &prev_states,
                &plan.dirty,
                project_dir,
                &provider,
            )?;

            let commit = IndexCommit {
                file_states: new_states.clone(),
                dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
                deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
            };
            let report = Self::persist_index(&mut graph, &commit)?;
            return Ok(report);
        }

        Ok(IndexReport {
            indexed: plan.discovered_count,
            unchanged: plan.discovered_count - plan.dirty.len(),
            dirty: plan.dirty.len(),
            deleted: plan.deleted.len(),
        })
    }
}
