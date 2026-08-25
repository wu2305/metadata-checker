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

/// 单文件扫描诊断计数的持久化镜像（M58.3 PR1 refix，F2）。
///
/// `ScanDiagnostics` 定义在 `scanner::spg`（只读模块边界，不加 serde derive），
/// 序列化格式由 indexer 侧拥有；graph_redb 只存取原始 bytes，不参与序列化/合并。
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct FileScanDiagnostics {
    unrecognized_container_key: usize,
    duplicate_component_id: usize,
    sample_unrecognized_location: Option<crate::output::Location>,
    sample_duplicate_location: Option<crate::output::Location>,
}

impl FileScanDiagnostics {
    /// 从 spg 侧计数结构拷贝为可序列化镜像。
    fn from_scan(counts: &crate::scanner::spg::ScanDiagnostics) -> Self {
        Self {
            unrecognized_container_key: counts.unrecognized_container_key,
            duplicate_component_id: counts.duplicate_component_id,
            sample_unrecognized_location: counts.sample_unrecognized_location.clone(),
            sample_duplicate_location: counts.sample_duplicate_location.clone(),
        }
    }

    /// 还原为 spg 侧计数结构，供 `ScanDiagnostics::merge` 聚合。
    fn into_scan(self) -> crate::scanner::spg::ScanDiagnostics {
        crate::scanner::spg::ScanDiagnostics {
            unrecognized_container_key: self.unrecognized_container_key,
            duplicate_component_id: self.duplicate_component_id,
            sample_unrecognized_location: self.sample_unrecognized_location,
            sample_duplicate_location: self.sample_duplicate_location,
        }
    }
}

/// M58.3 PR1 refix（F2）：计算单个 update 的 per-file 扫描诊断并序列化为
/// `(logical_path, bytes)` entry，供落库。
///
/// 样例的 `source_file` 在 per-file 层面回填为本文件的 logical_path
/// （样例本就来自该文件，与旧聚合点「首个非空样例胜出」回填语义等价）。
/// 计数为零的脏 SPG 文件也写 entry：覆盖旧值，正确淘汰已修复文件的计数。
fn per_file_scan_diagnostic_entry(update: &ParsedGraphUpdate) -> Result<Option<(String, Vec<u8>)>> {
    if let ParsedGraphContent::Spg(value) = &update.content {
        let mut counts = crate::scanner::spg::scan_raw_counts(value);
        if let Some(loc) = counts.sample_unrecognized_location.as_mut() {
            loc.source_file = Some(update.logical_path.clone());
        }
        if let Some(loc) = counts.sample_duplicate_location.as_mut() {
            loc.source_file = Some(update.logical_path.clone());
        }
        let bytes =
            serde_json::to_vec(&FileScanDiagnostics::from_scan(&counts)).with_context(|| {
                format!(
                    "Failed to serialize scanner diagnostics for {}",
                    update.logical_path
                )
            })?;
        Ok(Some((update.logical_path.clone(), bytes)))
    } else {
        Ok(None)
    }
}

/// 合并多文件待删节点 ID 并去重（保持首次出现顺序）
fn merge_removed_node_ids<'a>(sources: impl Iterator<Item = &'a [String]>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut merged = Vec::new();
    for node_ids in sources {
        for node_id in node_ids {
            if seen.insert(node_id.clone()) {
                merged.push(node_id.clone());
            }
        }
    }
    merged
}

/// M56：apply 前快照——收集被删节点的 incident edge keys（去重）。
///
/// 与 persist 的 `edge_storage_key` 共用同一键格式；persist 只消费，
/// 不在提交阶段扫描全图 edges。
fn collect_incident_edge_keys(graph: &GraphDB, node_ids: &[String]) -> Vec<String> {
    let mut keys = std::collections::HashSet::new();
    for node_id in node_ids {
        if let Ok(Some(neighbors)) =
            crate::graph_store::GraphReadStore::get_node_edges(graph, node_id)
        {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                keys.insert(crate::graph_redb::edge_storage_key(&view.edge));
            }
        }
    }
    keys.into_iter().collect()
}

/// M56：apply 后快照——收集新增/变更节点的 incident edges（按键去重）。
fn collect_incident_edges(graph: &GraphDB, node_ids: &[String]) -> Vec<crate::graph::Edge> {
    let mut seen = std::collections::HashSet::new();
    let mut edges = Vec::new();
    for node_id in node_ids {
        if let Ok(Some(neighbors)) =
            crate::graph_store::GraphReadStore::get_node_edges(graph, node_id)
        {
            for view in neighbors.outgoing.iter().chain(neighbors.incoming.iter()) {
                if seen.insert(crate::graph_redb::edge_storage_key(&view.edge)) {
                    edges.push(view.edge.clone());
                }
            }
        }
    }
    edges
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
    ///
    /// 委托 `apply_incremental_changes`（空 deleted），共享同一套批删逻辑。
    pub fn apply_graph_updates(
        graph: &mut dyn GraphWriteStore,
        updates: &[ParsedGraphUpdate],
    ) -> Result<HashMap<String, Vec<String>>> {
        let mut unused_states = HashMap::new();
        Self::apply_incremental_changes(graph, &mut unused_states, updates, &[])
    }

    /// 阶段 5：应用删除操作
    ///
    /// 委托 `apply_incremental_changes`（空 updates），共享同一套批删逻辑。
    pub fn apply_deletions(
        graph: &mut dyn GraphWriteStore,
        new_states: &mut HashMap<String, FileState>,
        deleted: &[DeletedFile],
    ) -> Result<()> {
        Self::apply_incremental_changes(graph, new_states, &[], deleted).map(|_| ())
    }

    /// 阶段 4+5 合并入口：一批解析更新 + 删除记录，每轮 apply 最多一次图批删
    ///
    /// 合并 dirty 文件的 `previous_node_ids` 与 deleted 文件的 node IDs，
    /// 去重后单次 `remove_nodes_by_ids`，再应用所有新增节点并移除已删除
    /// 文件的 file states。indexer 持有的是 `GraphWriteStore` trait 对象，
    /// 批删固定走 trait 层；`GraphDB` inherent 的同名方法不另建批删路径。
    pub fn apply_incremental_changes(
        graph: &mut dyn GraphWriteStore,
        new_states: &mut HashMap<String, FileState>,
        updates: &[ParsedGraphUpdate],
        deleted: &[DeletedFile],
    ) -> Result<HashMap<String, Vec<String>>> {
        let merged_removed = merge_removed_node_ids(
            updates
                .iter()
                .map(|update| update.previous_node_ids.as_slice())
                .chain(deleted.iter().map(|(_, node_ids)| node_ids.as_slice())),
        );
        if !merged_removed.is_empty() {
            graph.remove_nodes_by_ids(&merged_removed)?;
        }

        let mut touched_nodes = HashMap::new();
        for update in updates {
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

        for (rel, _) in deleted {
            new_states.remove(rel);
        }

        Ok(touched_nodes)
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
        Self::scan_with_diagnostics(project_dir, db_path).map(|with| with.report)
    }

    /// M58.3 PR1 refix（F2）：合并 redb 中的 per-file scanner 诊断计数 entry，
    /// 产出全库口径的信封诊断。
    ///
    /// 按 key（文件 logical_path）字典序合并（确定性），样例取「首个非空胜出」；
    /// 供构建报告出口与 runtime 加载诊断共用，同一 code 只经此一处进入 runtime。
    pub fn merge_scanner_diagnostic_entries(
        entries: &[(String, Vec<u8>)],
    ) -> Result<Vec<crate::output::Diagnostic>> {
        let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut acc = crate::scanner::spg::ScanDiagnostics::default();
        for (path, bytes) in sorted {
            let file_counts: FileScanDiagnostics =
                serde_json::from_slice(bytes).with_context(|| {
                    format!("Failed to decode scanner diagnostics entry for {path}")
                })?;
            acc.merge(&file_counts.into_scan());
        }
        Ok(acc.to_diagnostics())
    }

    /// 全量索引并返回扫描诊断（未识别容器键 / 重复组件 id 的跨文件聚合）。
    pub fn scan_with_diagnostics(
        project_dir: &Path,
        db_path: &Path,
    ) -> Result<crate::scanner::IndexReportWithDiagnostics> {
        let mut graph = GraphDB::open(db_path)?;
        let prev_states = graph.load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        let mut new_states = prev_states.clone();

        if !plan.dirty.is_empty() || !plan.deleted.is_empty() {
            let updates =
                Self::parse_dirty_files(&prev_states, &plan.dirty, project_dir, &provider)?;
            // M58.3 PR1 refix（F2）：per-file 诊断计数序列化，persist 后落库
            let mut scanner_entries = Vec::new();
            for update in &updates {
                if let Some(entry) = per_file_scan_diagnostic_entry(update)? {
                    scanner_entries.push(entry);
                }
            }
            // M56：apply 前收集被删节点的 incident edge keys（persist 只消费 delta）
            let merged_removed = merge_removed_node_ids(
                updates
                    .iter()
                    .map(|update| update.previous_node_ids.as_slice())
                    .chain(plan.deleted.iter().map(|(_, node_ids)| node_ids.as_slice())),
            );
            let removed_edge_keys = collect_incident_edge_keys(&graph, &merged_removed);
            // M54：dirty previous IDs 与 deleted IDs 合并去重后只做一次图批删
            let parsed_nodes = Self::apply_incremental_changes(
                &mut graph,
                &mut new_states,
                &updates,
                &plan.deleted,
            )?;
            // M56：apply 后收集新增/变更节点的 incident edges
            let new_node_ids: Vec<String> = parsed_nodes
                .values()
                .flat_map(|node_ids| node_ids.iter().cloned())
                .collect();
            let dirty_edges = collect_incident_edges(&graph, &new_node_ids);

            let changed_file_states: Vec<String> = updates
                .iter()
                .map(|update| update.logical_path.clone())
                .collect();
            let removed_file_paths: Vec<String> =
                plan.deleted.iter().map(|(rel, _)| rel.clone()).collect();

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
                checkpoint: None,
                // M56：初始全量构建（无 prev states）走显式 full rebuild 路径
                //（v2 置 Current）；增量提交走 delta 路径（v2 置 Stale）
                delta: if prev_states.is_empty() {
                    None
                } else {
                    Some(crate::graph_store::IndexDelta {
                        dirty_edges,
                        removed_edge_keys,
                        changed_file_states,
                        removed_file_paths,
                    })
                },
            };
            let mut report = Self::persist_index(&mut graph, commit)?;
            // M58.3 PR1 refix（F2）：persist 成功后落库 per-file 诊断计数
            //（脏文件覆盖 entry，删除文件移除 entry），随后重新 load 合并全量——
            // 报告 envelope 反映全库口径，而非仅本轮脏文件。
            let deleted_paths: Vec<String> =
                plan.deleted.iter().map(|(rel, _)| rel.clone()).collect();
            graph.save_scanner_diagnostic_entries(&scanner_entries, &deleted_paths)?;
            let scanner_diagnostics =
                Self::merge_scanner_diagnostic_entries(&graph.load_scanner_diagnostic_entries()?)?;
            // M58.3 PR1 refix（F6）：IndexReport 统一为文件口径。
            // store 层 persist_index 只能从 commit 拿到节点数（dirty_nodes/
            // deleted_nodes），文件数只有 diff 阶段的 plan 知道，因此在报告
            // 出口层覆盖，与下方 no-op 路径口径一致。
            report.indexed = plan.discovered_count;
            report.dirty = plan.dirty.len();
            report.deleted = plan.deleted.len();
            report.unchanged = plan.discovered_count.saturating_sub(plan.dirty.len());
            return Ok(crate::scanner::IndexReportWithDiagnostics {
                report,
                diagnostics: scanner_diagnostics,
            });
        }

        Ok(crate::scanner::IndexReportWithDiagnostics {
            report: IndexReport {
                indexed: plan.discovered_count,
                unchanged: plan.discovered_count - plan.dirty.len(),
                dirty: plan.dirty.len(),
                deleted: plan.deleted.len(),
            },
            // M58.3 PR1 refix（F2）：no-op 路径从库里 load 合并，
            // 持久化的 scanner 诊断不再随无变更构建消失。
            diagnostics: Self::merge_scanner_diagnostic_entries(
                &graph.load_scanner_diagnostic_entries()?,
            )?,
        })
    }

    /// M54：候选准备入口 — 打开候选 GraphDB、完成 diff/parse/apply、
    /// 构造 commit，但不调用 `persist_index`（不写盘）。
    ///
    /// 与 `scan` 复用同一套阶段函数；graphdb 文件在 prepare 前后保持不变，
    /// 由调用方决定何时把 `commit`（可附加 diff-refresh checkpoint）落盘。
    /// M58.3 PR2：per-file scanner 诊断 entries 与删除路径随返回值透传，
    /// 调用方须在 commit 落库成功后写 SCANNER_* 诊断（见 `PreparedIndexUpdate`）。
    pub fn prepare(project_dir: &Path, db_path: &Path) -> Result<PreparedIndexUpdate> {
        let mut graph = GraphDB::open(db_path)?;
        let prev_states = graph.load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        let mut new_states = prev_states.clone();
        let updates = Self::parse_dirty_files(&prev_states, &plan.dirty, project_dir, &provider)?;
        // M58.3 PR2：与 scan 相同逻辑收集本轮 per-file scanner 诊断 entries，
        // 随 PreparedIndexUpdate 透传给调用方（diff-refresh 编排器）落库，
        // 候选图不落盘不代表诊断可以丢——持久化责任移交调用方。
        let mut scanner_entries = Vec::new();
        for update in &updates {
            if let Some(entry) = per_file_scan_diagnostic_entry(update)? {
                scanner_entries.push(entry);
            }
        }
        // M56：apply 前收集被删节点的 incident edge keys（persist 只消费 delta）
        let merged_removed = merge_removed_node_ids(
            updates
                .iter()
                .map(|update| update.previous_node_ids.as_slice())
                .chain(plan.deleted.iter().map(|(_, node_ids)| node_ids.as_slice())),
        );
        let removed_edge_keys = collect_incident_edge_keys(&graph, &merged_removed);
        // dirty previous IDs 与 deleted IDs 合并去重后只做一次图批删（与 scan 一致）
        let parsed_nodes =
            Self::apply_incremental_changes(&mut graph, &mut new_states, &updates, &plan.deleted)?;
        // M56：apply 后收集新增/变更节点的 incident edges
        let new_node_ids: Vec<String> = parsed_nodes
            .values()
            .flat_map(|node_ids| node_ids.iter().cloned())
            .collect();
        let dirty_edges = collect_incident_edges(&graph, &new_node_ids);

        // 完整 dirty IDs：dirty 文件旧节点 ∪ 新增节点（页面失效需覆盖两侧）
        let mut dirty_node_ids = merge_removed_node_ids(
            updates
                .iter()
                .map(|update| update.previous_node_ids.as_slice()),
        );
        let mut seen_dirty: std::collections::HashSet<String> =
            dirty_node_ids.iter().cloned().collect();
        for node_ids in parsed_nodes.values() {
            for node_id in node_ids {
                if seen_dirty.insert(node_id.clone()) {
                    dirty_node_ids.push(node_id.clone());
                }
            }
        }
        let deleted_node_ids =
            merge_removed_node_ids(plan.deleted.iter().map(|(_, node_ids)| node_ids.as_slice()));

        let changed_file_states: Vec<String> = updates
            .iter()
            .map(|update| update.logical_path.clone())
            .collect();
        let removed_file_paths: Vec<String> =
            plan.deleted.iter().map(|(rel, _)| rel.clone()).collect();
        // M58.3 PR2：本轮删除文件的 logical_path，落库时需移除其诊断 entry
        let scanner_deleted_paths = removed_file_paths.clone();

        for update in &updates {
            let logical_path = update.logical_path.clone();
            let node_ids = parsed_nodes
                .get(logical_path.as_str())
                .cloned()
                .unwrap_or_default();
            new_states.insert(
                logical_path.clone(),
                FileState {
                    file_path: logical_path,
                    file_hash: update.file_hash.clone(),
                    mtime: update.mtime,
                    size: update.size,
                    node_ids,
                },
            );
        }

        let commit = IndexCommit {
            file_states: new_states,
            dirty_nodes: graph.dirty_nodes_set().iter().cloned().collect(),
            deleted_nodes: graph.removed_nodes_set().iter().cloned().collect(),
            checkpoint: None,
            // M56：初始全量构建（无 prev states）走显式 full rebuild 路径
            //（v2 置 Current）；增量提交走 delta 路径（v2 置 Stale）
            delta: if prev_states.is_empty() {
                None
            } else {
                Some(crate::graph_store::IndexDelta {
                    dirty_edges,
                    removed_edge_keys,
                    changed_file_states,
                    removed_file_paths,
                })
            },
        };
        Ok(PreparedIndexUpdate {
            graph,
            commit,
            dirty_node_ids,
            deleted_node_ids,
            scanner_entries,
            scanner_deleted_paths,
        })
    }
}

/// M54：候选索引更新（`ProjectIndexer::prepare` 产物，不落盘）
///
/// 不实现 Debug：`GraphDB` 未实现 Debug，且候选图体积不适合日志输出。
pub struct PreparedIndexUpdate {
    /// 完成 diff/parse/apply 后的候选图（内存态，尚未持久化）
    pub graph: GraphDB,
    /// 提交单元；调用方可附加 diff-refresh checkpoint 后交给 `persist_index`
    pub commit: IndexCommit,
    /// 完整 dirty 节点 ID：dirty 文件旧节点与新增节点的并集（去重）
    pub dirty_node_ids: Vec<String>,
    /// 完整 deleted 节点 ID：已删除文件节点的并集（去重）
    pub deleted_node_ids: Vec<String>,
    /// M58.3 PR2：本轮脏 SPG 文件的 per-file scanner 诊断 entries
    /// （序列化 bytes；计数为零的修复文件也携带 entry 以覆盖旧值）。
    /// 生命周期：调用方在 commit 持久化成功后交给
    /// `GraphDB::save_scanner_diagnostic_entries` 落库；deferred 模式
    /// 由编排器跨轮合并（脏覆盖、删除移除），随 pending commit 一起落库。
    pub scanner_entries: Vec<(String, Vec<u8>)>,
    /// M58.3 PR2：本轮删除文件的 logical_path，落库时移除其诊断 entry
    pub scanner_deleted_paths: Vec<String>,
}
