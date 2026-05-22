use anyhow::{Context, Result, anyhow};
use std::collections::HashMap;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use twox_hash::XxHash64;

use super::{process_spg_file_from_value, process_tbl_file_from_string};
use crate::graph::{FileState, GraphDB};
use crate::graph_store::{GraphWriteStore, IndexCommit, IndexReport, IndexStateStore};
use crate::parsed_content::ParsedContent;
use crate::source_id::{ProjectRef, SourceId};
use crate::storage_provider::{DocumentProvider, LocalStorageProvider};
use serde_json::Value;

/// 发现的文件数量（阶段内部使用，不暴露字段级细节）

/// 脏文件（需要重新解析）
pub type DirtyFile = (String, PathBuf, Option<Vec<u8>>);
/// 已删除文件记录
pub type DeletedFile = (String, Vec<String>);

/// 索引计划
#[derive(Debug)]
pub struct IndexPlan {
    pub discovered_count: usize,
    pub dirty: Vec<DirtyFile>,
    pub deleted: Vec<DeletedFile>,
}

/// 已解析待写图更新项
#[derive(Debug)]
pub struct ParsedGraphUpdate {
    /// 逻辑路径（相对项目路径）
    pub logical_path: String,
    /// 物理路径
    pub physical_path: PathBuf,
    /// 文件哈希
    pub file_hash: String,
    /// 文件修改时间（Unix seconds）
    pub mtime: u64,
    /// 文件大小
    pub size: u64,
    /// 上次解析产生的节点 ID，更新前需要先移除
    pub previous_node_ids: Vec<String>,
    /// 解析后的文件内容
    pub content: ParsedGraphContent,
}

/// 已解析文件内容
#[derive(Debug)]
pub enum ParsedGraphContent {
    Spg(Value),
    Tbl(String),
}

/// 项目索引器
///
/// M39：把 scan_project 拆为可测试的阶段。
pub struct ProjectIndexer;

impl ProjectIndexer {
    /// 阶段 1：递归发现项目目录下所有 .spg 和 .tbl 文件
    ///
    /// 仅做本地路径发现，不读取文件内容。
    pub fn discover_files(project_dir: &Path) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        Self::collect_files(project_dir, project_dir, &mut files)?;
        Ok(files)
    }

    fn collect_files(base: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
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
    ///
    /// 本阶段读取已发现文件并生成脏文件计划，不负责目录遍历。
    pub fn diff_file_states(
        discovered_files: &[PathBuf],
        prev_states: &HashMap<String, FileState>,
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<IndexPlan> {
        let mut discovered_count: usize = 0;
        let mut dirty = Vec::new();
        let mut current_paths: HashMap<String, PathBuf> = HashMap::new();

        for path in discovered_files {
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
                    // 文件未变化，跳过后续解析。
                }
                _ => {
                    dirty.push((rel, path.clone(), Some(content_bytes)));
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

    /// 阶段 3：解析脏文件内容，返回待写图更新列表
    ///
    /// 该阶段仅负责内容解析，不执行图读写；上游只在内容确认为可解析后再应用更新。
    pub fn parse_dirty_files(
        prev_states: &HashMap<String, FileState>,
        dirty_files: &[DirtyFile],
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<Vec<ParsedGraphUpdate>> {
        let mut updates = Vec::with_capacity(dirty_files.len());

        for (rel, path, staged_bytes) in dirty_files {
            let content_bytes = if let Some(bytes) = staged_bytes {
                bytes.clone()
            } else {
                provider.read_bytes(path)?
            };

            let (file_hash, mtime, size) = {
                let mut hasher = XxHash64::default();
                hasher.write(&content_bytes);
                let hash = format!("{:x}", hasher.finish());

                let metadata = provider.metadata(path)?;
                let mtime = metadata
                    .modified
                    .unwrap_or(SystemTime::UNIX_EPOCH)
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                (hash, mtime, metadata.size)
            };

            let previous_node_ids = prev_states
                .get(rel)
                .map(|state| state.node_ids.clone())
                .unwrap_or_default();

            let content = if path.extension().map(|e| e == "spg").unwrap_or(false) {
                let source =
                    SourceId::from_local_path(ProjectRef::new("default"), path, Some(project_dir))
                        .with_context(|| format!("Failed to build SourceId for {}", rel))?;
                let parsed = ParsedContent::from_bytes(source, content_bytes.clone());
                let raw_value = parsed
                    .json()
                    .with_context(|| format!("Failed to parse JSON for {}", rel))?;
                ParsedGraphContent::Spg((*raw_value).clone())
            } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
                let content = String::from_utf8_lossy(&content_bytes);
                ParsedGraphContent::Tbl(content.to_string())
            } else {
                continue;
            };

            updates.push(ParsedGraphUpdate {
                logical_path: rel.clone(),
                physical_path: path.clone(),
                file_hash,
                mtime,
                size,
                previous_node_ids,
                content,
            });
        }

        Ok(updates)
    }

    /// 阶段 4：应用解析后的脏文件更新到图数据库
    pub fn apply_graph_updates(
        graph: &mut dyn GraphWriteStore,
        updates: &[ParsedGraphUpdate],
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut touched_nodes = HashMap::new();

        for update in updates {
            if !update.previous_node_ids.is_empty() {
                graph.remove_nodes_by_ids(&update.previous_node_ids)?;
            }

            let node_ids = match &update.content {
                ParsedGraphContent::Spg(value) => {
                    process_spg_file_from_value(graph, &update.logical_path, value.clone())?
                }
                ParsedGraphContent::Tbl(content) => {
                    process_tbl_file_from_string(graph, &update.logical_path, content)?
                }
            };
            touched_nodes.insert(update.logical_path.clone(), node_ids);
        }

        Ok(touched_nodes)
    }

    /// 阶段 5：应用删除操作
    pub fn apply_deletions(
        graph: &mut dyn GraphWriteStore,
        new_states: &mut HashMap<String, FileState>,
        deleted: &[DeletedFile],
    ) -> Result<()> {
        for (rel, node_ids) in deleted {
            graph.remove_nodes_by_ids(node_ids)?;
            new_states.remove(rel);
        }
        Ok(())
    }

    /// 阶段 6：持久化索引结果
    pub fn persist_index(
        graph: &mut dyn IndexStateStore,
        commit: IndexCommit,
    ) -> Result<IndexReport> {
        IndexStateStore::persist_index(graph, commit)
            .map_err(|e| anyhow!("persist_index failed: {}", e))
    }

    /// 全量索引入口（替代 scan_project）
    pub fn scan(project_dir: &Path, db_path: &Path) -> Result<IndexReport> {
        let mut graph = GraphDB::open(db_path)?;
        let prev_states = graph.load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        let mut new_states = prev_states.clone();
        Self::apply_deletions(&mut graph, &mut new_states, &plan.deleted)?;

        if !plan.dirty.is_empty() || !plan.deleted.is_empty() {
            let updates =
                Self::parse_dirty_files(&prev_states, &plan.dirty, project_dir, &provider)?;
            let parsed_nodes = Self::apply_graph_updates(&mut graph, &updates)?;

            for update in updates {
                let logical_path = update.logical_path.clone();
                let node_ids = parsed_nodes
                    .get(logical_path.as_str())
                    .cloned()
                    .unwrap_or_default();
                new_states.insert(
                    logical_path.clone(),
                    FileState {
                        file_path: logical_path,
                        file_hash: update.file_hash,
                        mtime: update.mtime,
                        size: update.size,
                        node_ids,
                    },
                );
            }

            let commit = IndexCommit {
                file_states: new_states.clone(),
                dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
                deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
            };
            let report = Self::persist_index(&mut graph, commit)?;
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
