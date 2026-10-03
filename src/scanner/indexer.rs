use anyhow::{Context, Result, anyhow};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use twox_hash::XxHash64;

use super::{
    process_spg_file_from_value, process_spg_file_from_value_with_identity,
    process_tbl_file_from_string,
};
use crate::graph::{FileState, GraphDB, Node, NodeType};
use crate::graph_store::{
    GraphReadStore, GraphStoreResult, GraphWriteStore, IndexCommit, IndexReport, IndexStateStore,
};
use crate::ownership::{
    ContributionKind, EdgeContribution, EntityContribution, FileContributionLedger, ProjectBinding,
};
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

/// 将单文件解析结果转换为可撤销的来源账本。
///
/// M59-2 根因修复：身份**在建节点之前**就已确定（见
/// [`crate::scanner::spg::PageIdentityMode`]）。旧实现先用旧全局 ID 建一张临时
/// 图、再按名字事后改写 ID——局部 source 与同名物理表在临时图里已经塌成同一个
/// 节点（`path`/`meta` 后写覆盖前写），此后无论怎么转换都只能靠猜。现在的临时
/// 图里两类实体天然是两个节点，账本只做归属分类，不再做身份猜测。
fn ledger_from_parsed_content(
    logical_path: &str,
    content: &ParsedGraphContent,
) -> Result<FileContributionLedger> {
    let mut temporary = crate::memory_graph_store::MemoryGraphStore::new();
    match content {
        ParsedGraphContent::Spg(value) => {
            process_spg_file_from_value_with_identity(
                &mut temporary,
                logical_path,
                value.clone(),
                crate::scanner::spg::PageIdentityMode::OwnershipPageLocal,
            )?;
        }
        ParsedGraphContent::Tbl(text) => {
            process_tbl_file_from_string(&mut temporary, logical_path, text)?;
        }
    }
    let mut ledger = FileContributionLedger::new(logical_path, "");
    let nodes: Vec<Node> = temporary.iter_nodes()?.collect();

    let is_spg = logical_path.ends_with(".spg");
    let is_tbl = logical_path.ends_with(".tbl");

    // 1. 本页面的身份前缀：`model:<page>|` / `field:<page>|`。
    //    页面局部实体与物理实体在扫描侧已分好类，这里只按 id 判定归属，
    //    不再按名字反推（名字会撞：局部 source 名可以等于另一个 source 的物理表名）。
    let page_key = if is_spg {
        Some(crate::graph_identity::normalize_project_path(
            &logical_path.replace('\\', "/"),
        )?)
    } else {
        None
    };
    let is_page_local = |id: &str| -> bool {
        match page_key.as_deref() {
            Some(page) => match id.split_once('|') {
                Some((head, _)) => match head.split_once(':') {
                    Some((_, page_segment)) => page_segment == page,
                    None => false,
                },
                None => false,
            },
            None => false,
        }
    };

    // 2. TBL 主模型名称：解析器的全局身份取**文件 stem**（tbl.rs“Use file stem as
    //    model identifier”），不是 JSON `name`。这里必须与身份规则一致，否则主模型
    //    会被误判为 Reference 并触发占位降级、同 stem 冲突也无法报告。
    let tbl_primary_model = if is_tbl {
        Path::new(logical_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    } else {
        None
    };

    // 3. 判定 Definition vs Reference
    // 页面/组件/条件/动作只有 path 等于本文件时才是本源定义；embedsuperpage/link
    // 为目标页创建的 stub（path 是目标页）必须是 Reference，否则 embedder 会在
    // 重建时覆盖目标页自身的 origin_file 溯源（冷脸验收 P1）。
    // 模型/字段按 id 判定：带本页页面段的是本页定义的局部实体；全局 id 是物理
    // 实体或被嵌目标，属 Reference。
    for mut node in nodes {
        let is_definition = if is_spg {
            match node.node_type {
                NodeType::Page | NodeType::Component | NodeType::Condition | NodeType::Action => {
                    node.path == logical_path
                }
                NodeType::Model | NodeType::Field => is_page_local(&node.id),
            }
        } else if is_tbl {
            if let Some(ref primary) = tbl_primary_model {
                let primary_model_id = format!("model:{}", primary);
                let primary_field_prefix = format!("field:{}.", primary);
                node.id == primary_model_id || node.id.starts_with(&primary_field_prefix)
            } else {
                node.path == logical_path
            }
        } else {
            node.path == logical_path
        };

        node.origin_file = Some(logical_path.to_string());
        ledger.entities.push(EntityContribution {
            origin_file: logical_path.to_string(),
            node,
            kind: if is_definition {
                ContributionKind::Definition
            } else {
                ContributionKind::Reference
            },
        });
    }

    let mut edge_ids: Vec<String> = ledger
        .entities
        .iter()
        .map(|contribution| contribution.node.id.clone())
        .collect();
    edge_ids.sort();
    edge_ids.dedup();
    for node_id in edge_ids {
        let Some(neighbors) = temporary.get_node_edges(&node_id)? else {
            continue;
        };
        for view in neighbors.outgoing {
            let mut edge = view.edge;
            // 端点 ID 在扫描侧已按归属构造完成（局部带页面段、物理恒全局），
            // 这里保持原样——任何事后改写都会把两个不同实体并成一个。
            edge.origin_file = Some(logical_path.to_string());
            ledger.edges.push(EdgeContribution {
                origin_file: logical_path.to_string(),
                edge,
            });
        }
    }
    Ok(ledger)
}

fn apply_ownership_changes(
    graph: &mut GraphDB,
    ledgers: &mut BTreeMap<String, FileContributionLedger>,
    updates: &[ParsedGraphUpdate],
    deleted: &[DeletedFile],
) -> Result<(
    HashMap<String, Vec<String>>,
    Vec<crate::ownership::OwnershipConflict>,
)> {
    for update in updates {
        let mut ledger = ledger_from_parsed_content(&update.logical_path, &update.content)?;
        ledger.revision = update.file_hash.clone();
        ledgers.insert(update.logical_path.clone(), ledger);
    }
    for (logical_path, _) in deleted {
        ledgers.remove(logical_path);
    }
    let conflicts = crate::ownership::rebuild_graph_from_ledgers(graph, ledgers)?;
    graph.replace_ownership_ledgers(ledgers.clone())?;
    let node_ids = updates
        .iter()
        .map(|update| {
            let node_ids = ledgers
                .get(&update.logical_path)
                .map(|ledger| {
                    ledger
                        .entities
                        .iter()
                        .map(|contribution| contribution.node.id.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (update.logical_path.clone(), node_ids)
        })
        .collect();
    Ok((node_ids, conflicts))
}

/// 把账本重建发现的来源冲突转为报告诊断（spec：必须报告，不得静默择一）。
pub fn ownership_conflict_diagnostics(
    conflicts: &[crate::ownership::OwnershipConflict],
) -> Vec<crate::output::Diagnostic> {
    conflicts
        .iter()
        .map(|conflict| {
            crate::diagnostics::envelope_diagnostic(
                crate::diagnostics::CODE_GRAPH_OWNERSHIP_CONFLICT,
                conflict.definition_origins.len(),
                crate::output::Location {
                    source_file: conflict.definition_origins.first().cloned(),
                    node_id: Some(conflict.node_id.clone()),
                    json_path: None,
                },
                format!(
                    "node '{}' has conflicting definitions from {}",
                    conflict.node_id,
                    conflict.definition_origins.join(", ")
                ),
            )
        })
        .collect()
}

/// M59-B3：本轮解析失败、**未进候选图**的源文件.
///
/// 旧行为是 `tbl.rs` 把非法 JSON 静默转成 `Ok(空集)`：配合「先删旧节点再重建」，
/// 一个写坏的 `.tbl` 会让旧模型图被删、新图为空，而 file hash 照常记录——
/// 下次内容不变直接跳过，「读不出来」就此变成「模型不存在」。
///
/// 现在解析失败的文件**整个跳过**：
/// - 不进 `updates` ⇒ `previous_node_ids` 不进 `merged_removed` ⇒ 旧图原样保留；
/// - 不写 `new_states` ⇒ 旧 `file_hash` 保留 ⇒ 文件下一轮仍是脏的，会重试；
/// - 写一条 `SCANNER_FILE_PARSE_FAILED` 诊断 entry ⇒ 图里这部分是**陈旧**而非缺失，
///   查询方能区分这两件事。
#[derive(Debug, Clone)]
pub struct ParseFailure {
    /// 逻辑路径（相对项目路径）
    pub logical_path: String,
    /// 失败原因（UTF-8 / serde 报错原文）
    pub reason: String,
}

/// 逐次扫描诊断的合并结果，见 [`ProjectIndexer::merge_scanner_occurrence_entries`]。
#[derive(Debug, Default, Clone)]
pub struct ScannerOccurrenceReport {
    /// 全库逐次记录（路径字典序、文件内出现顺序）
    pub occurrences: Vec<crate::scanner::ScanOccurrence>,
    /// 只有计数、没有逐次记录的文件（旧版本写入；重新解析后补齐）
    pub legacy_files: Vec<String>,
}

/// 单文件扫描诊断计数的持久化镜像（M58.3 PR1 refix，F2）。
///
/// `ScanDiagnostics` 定义在 `scanner::spg`（只读模块边界，不加 serde derive），
/// 序列化格式由 indexer 侧拥有；graph_redb 只存取原始 bytes，不参与序列化/合并。
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct FileScanDiagnostics {
    unrecognized_container_key: usize,
    duplicate_component_id: usize,
    /// M59-B3：本文件解析失败、未进候选图。旧库里的 entry 没有这个字段，
    /// `#[serde(default)]` 让它们照常反序列化为 0（不需要 schema 版本升级）。
    #[serde(default)]
    parse_failed: usize,
    sample_unrecognized_location: Option<crate::output::Location>,
    sample_duplicate_location: Option<crate::output::Location>,
    #[serde(default)]
    sample_parse_failed_location: Option<crate::output::Location>,
    #[serde(default)]
    parse_failed_reason: Option<String>,
    /// 逐次出现的记录。旧库 entry 没有这个字段（`#[serde(default)]` → 空），
    /// 此时计数大于记录数，见 [`FileScanDiagnostics::lacks_occurrences`]。
    #[serde(default)]
    occurrences: Vec<crate::scanner::ScanOccurrence>,
}

impl FileScanDiagnostics {
    /// 旧版本写入的 entry：有计数却没有逐次记录。聚合信封仍可用，
    /// 但不能据此把诊断挂到具体节点；重新解析该文件后才会补齐。
    fn lacks_occurrences(&self) -> bool {
        self.unrecognized_container_key + self.duplicate_component_id + self.parse_failed
            > self.occurrences.len()
    }

    /// 从 spg 侧计数结构拷贝为可序列化镜像。
    fn from_scan(counts: &crate::scanner::spg::ScanDiagnostics) -> Self {
        Self {
            unrecognized_container_key: counts.unrecognized_container_key,
            duplicate_component_id: counts.duplicate_component_id,
            parse_failed: counts.parse_failed,
            sample_unrecognized_location: counts.sample_unrecognized_location.clone(),
            sample_duplicate_location: counts.sample_duplicate_location.clone(),
            sample_parse_failed_location: counts.sample_parse_failed_location.clone(),
            parse_failed_reason: counts.parse_failed_reason.clone(),
            occurrences: counts.occurrences.clone(),
        }
    }

    /// 还原为 spg 侧计数结构，供 `ScanDiagnostics::merge` 聚合。
    fn into_scan(self) -> crate::scanner::spg::ScanDiagnostics {
        crate::scanner::spg::ScanDiagnostics {
            unrecognized_container_key: self.unrecognized_container_key,
            duplicate_component_id: self.duplicate_component_id,
            parse_failed: self.parse_failed,
            sample_unrecognized_location: self.sample_unrecognized_location,
            sample_duplicate_location: self.sample_duplicate_location,
            sample_parse_failed_location: self.sample_parse_failed_location,
            parse_failed_reason: self.parse_failed_reason,
            occurrences: self.occurrences,
            ..Default::default()
        }
    }
}

/// M58.3 PR1 refix（F2）：计算单个 update 的 per-file 扫描诊断并序列化为
/// `(logical_path, bytes)` entry，供落库。
///
/// 样例的 `source_file` 在 per-file 层面回填为本文件的 logical_path
/// （样例本就来自该文件，与旧聚合点「首个非空样例胜出」回填语义等价）。
/// 计数为零的脏 SPG 文件也写 entry：覆盖旧值，正确淘汰已修复文件的计数。
///
/// M59-B3：脏 TBL 文件同样写一条**全零** entry。它没有 SPG 那两项计数，但必须
/// 覆盖同路径上可能残留的 `parse_failed` 标记——文件修好了、这轮解析成功了，
/// 陈旧标记就得当场消失，否则会一直挂着一条永不退休的警告。
fn per_file_scan_diagnostic_entry(update: &ParsedGraphUpdate) -> Result<Option<(String, Vec<u8>)>> {
    let counts = match &update.content {
        ParsedGraphContent::Spg(value) => {
            let mut counts = crate::scanner::spg::scan_raw_counts(value);
            if let Some(loc) = counts.sample_unrecognized_location.as_mut() {
                loc.source_file = Some(update.logical_path.clone());
            }
            if let Some(loc) = counts.sample_duplicate_location.as_mut() {
                loc.source_file = Some(update.logical_path.clone());
            }
            for occurrence in &mut counts.occurrences {
                occurrence.location.source_file = Some(update.logical_path.clone());
            }
            counts
        }
        ParsedGraphContent::Tbl(_) => crate::scanner::spg::ScanDiagnostics::default(),
    };
    let bytes =
        serde_json::to_vec(&FileScanDiagnostics::from_scan(&counts)).with_context(|| {
            format!(
                "Failed to serialize scanner diagnostics for {}",
                update.logical_path
            )
        })?;
    Ok(Some((update.logical_path.clone(), bytes)))
}

/// M59-B3：把一次解析失败序列化成 per-file 诊断 entry。
///
/// 与 `per_file_scan_diagnostic_entry` 共用同一张 `scanner_entries` 表、同一条
/// 「按 logical_path 覆盖」语义——所以下一轮解析成功时上面那个函数写的全零 entry
/// 会自动把这条抹掉，不需要额外的清理路径。
fn parse_failure_diagnostic_entry(failure: &ParseFailure) -> Result<(String, Vec<u8>)> {
    let counts =
        crate::scanner::spg::ScanDiagnostics::parse_failure(&failure.logical_path, &failure.reason);
    let bytes =
        serde_json::to_vec(&FileScanDiagnostics::from_scan(&counts)).with_context(|| {
            format!(
                "Failed to serialize parse failure diagnostics for {}",
                failure.logical_path
            )
        })?;
    Ok((failure.logical_path.clone(), bytes))
}

/// 磁盘路径 → 诊断/`FileState` 使用的 logical_path。
///
/// `diff_file_states` 与诊断对账必须用**同一个**派生，否则两边算出的路径集合会
/// 悄悄错开，对账要么漏删要么误删。
fn logical_path_of(path: &Path, project_dir: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

/// M59 codex 复审返修：把已落库的 per-file scanner 诊断与**当前文件集**对账，
/// 返回本轮应当移除的诊断路径。
///
/// 既有的淘汰机制是「文件这轮被解析 ⇒ 按 logical_path 覆盖它的 entry」
/// （见 [`per_file_scan_diagnostic_entry`]）。它有两个够不着的角落，两者都会让
/// `SCANNER_FILE_PARSE_FAILED` **无限期挂在一个已经不存在问题的路径上**：
///
/// 1. **孤儿**：文件已从磁盘消失。首次解析就失败的文件只写了诊断、**没写
///    `FileState`**（`parse_dirty_files_with_failures` 让失败文件整个跳过），
///    所以删掉它之后它既不在 discovered 里、也进不了 `plan.deleted`，
///    没有任何路径会去碰它的 entry。
/// 2. **陈旧失败**：文件内容被改回**上一次解析成功时的字节**。此时 hash 与保留
///    下来的 `FileState` 相同 ⇒ 文件不脏 ⇒ 不重新解析 ⇒ 覆盖机制不触发。
///    修好了，警告却还在。
///
/// 第 2 类直接删 entry 而不重新解析，依据是本模块自己维持的不变量：
/// `FileState.file_hash` **只**从 `updates` 里写入，而 `updates` 只含解析成功的
/// 文件。所以「hash 与已存 `FileState` 相同」本身就等价于「这份字节解析得通」。
/// 反过来若强制重新解析，会把一个内容未变的文件推进 apply 路径、连带删除并重建
/// 它的节点——在 §2.1 的跨文件边缺陷尚未修复前，那等于为了清一条警告去触发
/// 一次真实的丢边。
///
/// **解码不出来的 entry 一律不碰**，哪怕它的路径已经不存在。损坏有它自己的响亮
/// 信号（`runtime` 的 `SCANNER_DIAGNOSTICS_REFRESH_FAILED`）；本函数只做生命周期
/// 记账，删掉损坏 entry 等于毁掉证据、把损坏悄悄抹平——正是 B3 要根除的那种
/// 「失败被转成非事件」。同理也不向上抛错：一条坏 entry 不该让整轮索引失败。
fn stale_scanner_diagnostic_paths(
    entries: &[(String, Vec<u8>)],
    discovered: &HashSet<String>,
    prev_states: &HashMap<String, FileState>,
    dirty: &[DirtyFile],
) -> Vec<String> {
    let dirty_paths: HashSet<&str> = dirty.iter().map(|(rel, _, _)| rel.as_str()).collect();
    let mut stale = Vec::new();
    for (path, bytes) in entries {
        let Ok(counts) = serde_json::from_slice::<FileScanDiagnostics>(bytes) else {
            continue;
        };
        if !discovered.contains(path.as_str()) {
            // 孤儿：文件没了。已在 `plan.deleted` 里的路径由调用方去重。
            stale.push(path.clone());
            continue;
        }
        if dirty_paths.contains(path.as_str()) || !prev_states.contains_key(path) {
            // 这轮会重新解析，覆盖机制自会处理；不要抢在解析结果之前删。
            continue;
        }
        if counts.parse_failed > 0 {
            stale.push(path.clone());
        }
    }
    stale
}

/// 把本轮删除文件的路径与对账出的陈旧诊断路径合并去重。
fn merge_scanner_deleted_paths(deleted: &[DeletedFile], stale: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for path in deleted
        .iter()
        .map(|(rel, _)| rel)
        .chain(stale.iter())
        .cloned()
    {
        if seen.insert(path.clone()) {
            merged.push(path);
        }
    }
    merged
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
fn collect_incident_edge_keys(graph: &dyn GraphReadStore, node_ids: &[String]) -> Vec<String> {
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
fn collect_incident_edges(
    graph: &dyn GraphReadStore,
    node_ids: &[String],
) -> Vec<crate::graph::Edge> {
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

/// 扫描编排所需的图 + 索引侧状态能力（M59-3 C2）。
///
/// 非 ownership 扫描路径只依赖这组接口：图读写契约、`IndexStateStore`
/// 提交、per-file scanner 诊断读写、本轮待定节点账（dirty/removed）。
/// `GraphDB`（redb）与 `GrafeoGraphStore` 各自实现，共享同一条编排代码；
/// ownership 应用层仍是 `GraphDB` 具体 API（M59-2 未交付完成），由调用方
/// 在需要时把 `ScanBackend` 匹配回具体类型。
pub(crate) trait IndexScanStore: GraphReadStore + GraphWriteStore + IndexStateStore {
    /// 全库 per-file scanner 诊断计数 entry
    /// （`logical_path → FileScanDiagnostics` JSON bytes）
    fn load_scanner_diagnostic_entries(&self) -> GraphStoreResult<Vec<(String, Vec<u8>)>>;

    /// 本轮已 upsert、尚未 persist 的节点 id
    fn pending_dirty_nodes(&self) -> Vec<String>;

    /// 本轮已删除、尚未 persist 的节点 id
    fn pending_removed_nodes(&self) -> Vec<String>;

    /// 是否需要收集 M56 增量 delta 载荷（edge-key 快照供 redb v2 shadow
    /// 置 Stale）。直写后端没有影子层，返回 false 让编排层跳过两遍
    /// 全邻接收集。
    fn wants_index_delta(&self) -> bool;
}

/// 扫描打开的图后端：默认 redb；`.grafeo` 扩展名显式选择 Grafeo。
///
/// 枚举而不是裸 `&mut dyn IndexScanStore`：ownership 应用层仍是 `GraphDB`
/// 具体 API，需要时回退具体类型；非 ownership 路径统一走 `store()` 的
/// trait 对象。
enum ScanBackend {
    Redb(GraphDB),
    #[cfg(feature = "grafeo-store")]
    Grafeo(crate::graph_grafeo::GrafeoGraphStore),
}

impl ScanBackend {
    fn store(&mut self) -> &mut dyn IndexScanStore {
        match self {
            Self::Redb(graph) => graph,
            #[cfg(feature = "grafeo-store")]
            Self::Grafeo(store) => store,
        }
    }
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
        // read_dir 的枚举序由文件系统决定（ext4 / APFS / overlayfs 各不相同）。
        // 跨页同名局部实体（如各页的 `model:model2`）按扫描序后写覆盖先写，
        // 序不固定会让建图结果随平台漂移，所以按路径排序后再处理。
        let mut paths = std::fs::read_dir(current)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<PathBuf>>>()?;
        paths.sort();
        for path in paths {
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
            let rel = logical_path_of(path, project_dir);
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
    ///
    /// 便捷包装：丢弃解析失败清单。需要把失败落成诊断的调用点用
    /// [`Self::parse_dirty_files_with_failures`]。
    pub fn parse_dirty_files(
        prev_states: &HashMap<String, FileState>,
        dirty_files: &[DirtyFile],
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<Vec<ParsedGraphUpdate>> {
        Self::parse_dirty_files_with_failures(prev_states, dirty_files, project_dir, provider)
            .map(|(updates, _)| updates)
    }

    /// 阶段 3（M59-B3 完整版）：解析脏文件，同时返回**解析失败**的文件清单。
    ///
    /// 失败的文件不出现在返回的 updates 里，因此后续 apply 既不会删它的旧节点、
    /// 也不会给它写新的 `FileState`——旧图保留、文件保持脏、下轮重试。
    /// 见 [`ParseFailure`]。
    pub fn parse_dirty_files_with_failures(
        prev_states: &HashMap<String, FileState>,
        dirty_files: &[DirtyFile],
        project_dir: &Path,
        provider: &dyn DocumentProvider,
    ) -> Result<(Vec<ParsedGraphUpdate>, Vec<ParseFailure>)> {
        let mut updates = Vec::with_capacity(dirty_files.len());
        let mut failures: Vec<ParseFailure> = Vec::new();

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
                // SPG 解析失败仍然向上抛错（整轮索引失败），不走 ParseFailure：
                // 它本来就是**响亮**的失败，从没有「静默转成空结果」的问题。
                // B3 要修的是 TBL 那条静默路径，不是把 SPG 也降级成部分成功。
                let raw_value = parsed
                    .json()
                    .with_context(|| format!("Failed to parse JSON for {}", rel))?;
                ParsedGraphContent::Spg((*raw_value).clone())
            } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
                // M59-B3：这里是**唯一**能把「读不出来」和「模型不存在」分开的地方。
                // 过了这一关才允许动候选图；`from_utf8_lossy` 会把坏字节抹成 U+FFFD
                // 再一路当成合法内容走下去，所以必须用严格版。
                let text = match String::from_utf8(content_bytes.clone()) {
                    Ok(text) => text,
                    Err(err) => {
                        failures.push(ParseFailure {
                            logical_path: rel.clone(),
                            reason: format!("invalid UTF-8: {err}"),
                        });
                        continue;
                    }
                };
                // 内容层校验与 `tbl::process_tbl_file_from_string` 同口径：空文件和
                // 非法 JSON 都不是「没有模型」，是解析没成功。
                if let Err(err) = serde_json::from_str::<Value>(&text) {
                    failures.push(ParseFailure {
                        logical_path: rel.clone(),
                        reason: format!("invalid JSON: {err}"),
                    });
                    continue;
                }
                ParsedGraphContent::Tbl(text)
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

        Ok((updates, failures))
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

    /// 全量索引入口（替代 scan_project）。
    ///
    /// 未传入项目绑定时保持旧低层兼容路径；启用 M59-2 ownership 的调用方
    /// 必须使用 [`Self::scan_for_project`]，避免把 machine-specific 路径当身份。
    pub fn scan(project_dir: &Path, db_path: &Path) -> Result<IndexReport> {
        Self::scan_with_diagnostics(project_dir, db_path).map(|with| with.report)
    }

    /// 按稳定项目绑定执行一次索引，并在扫描前校验 ownership schema。
    pub fn scan_for_project(
        project_dir: &Path,
        db_path: &Path,
        project_binding: &ProjectBinding,
    ) -> Result<IndexReport> {
        Self::scan_with_diagnostics_for_project(project_dir, db_path, project_binding)
            .map(|with| with.report)
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

    /// 合并 per-file scanner 诊断 entry 的**逐次记录**（不聚合）。
    ///
    /// 按文件 logical_path 字典序、文件内按出现顺序，输出确定。聚合信封请用
    /// [`Self::merge_scanner_diagnostic_entries`]。旧版本写入的 entry 没有逐次记录，
    /// 其路径列入 `legacy_files` 而不是静默当作「没有诊断」。
    pub fn merge_scanner_occurrence_entries(
        entries: &[(String, Vec<u8>)],
    ) -> Result<ScannerOccurrenceReport> {
        let mut sorted: Vec<&(String, Vec<u8>)> = entries.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut report = ScannerOccurrenceReport::default();
        for (path, bytes) in sorted {
            let file: FileScanDiagnostics = serde_json::from_slice(bytes).with_context(|| {
                format!("Failed to decode scanner diagnostics entry for {path}")
            })?;
            if file.lacks_occurrences() {
                report.legacy_files.push(path.clone());
            }
            report.occurrences.extend(file.occurrences);
        }
        Ok(report)
    }

    /// 全量索引并返回扫描诊断（未识别容器键 / 重复组件 id 的跨文件聚合）。
    pub fn scan_with_diagnostics(
        project_dir: &Path,
        db_path: &Path,
    ) -> Result<crate::scanner::IndexReportWithDiagnostics> {
        Self::scan_with_diagnostics_internal(project_dir, db_path, None, false)
    }

    /// 按稳定项目绑定执行 ownership 扫描，绑定与来源账本在同一入口启用。
    pub fn scan_with_diagnostics_for_project(
        project_dir: &Path,
        db_path: &Path,
        project_binding: &ProjectBinding,
    ) -> Result<crate::scanner::IndexReportWithDiagnostics> {
        Self::scan_with_diagnostics_internal(project_dir, db_path, Some(project_binding), true)
    }

    fn scan_with_diagnostics_internal(
        project_dir: &Path,
        db_path: &Path,
        project_binding: Option<&ProjectBinding>,
        ownership_enabled: bool,
    ) -> Result<crate::scanner::IndexReportWithDiagnostics> {
        // M59-3 C2：`.grafeo` 扩展名显式选择 Grafeo 直写后端。
        if crate::graph_store::is_grafeo_db_path(db_path) {
            // ownership 应用层是 GraphDB 具体 API（M59-2 未完成）；project_binding
            // 只服务于 ownership 语义，grafeo 侧没有对应物——静默忽略 binding 会
            // 产生一个「看着像带项目绑定、实际没有」的库，fail-closed 更诚实。
            anyhow::ensure!(
                !ownership_enabled && project_binding.is_none(),
                "GRAPH_BACKEND_UNSUPPORTED: grafeo 后端尚未接线 ownership 扫描（M59-2 未完成）"
            );
            #[cfg(feature = "grafeo-store")]
            {
                let store = crate::graph_grafeo::GrafeoGraphStore::open(db_path)
                    .map_err(|err| anyhow!("打开 grafeo 图库失败：{err}"))?;
                return Self::scan_store_body(
                    ScanBackend::Grafeo(store),
                    project_dir,
                    ownership_enabled,
                );
            }
            #[cfg(not(feature = "grafeo-store"))]
            anyhow::bail!(
                "GRAPH_BACKEND_UNSUPPORTED: 本构建未启用 grafeo-store，无法打开 .grafeo 图库"
            );
        }
        let backend = ScanBackend::Redb(match (project_binding, ownership_enabled) {
            (Some(binding), true) => GraphDB::open_with_ownership(db_path, binding)?,
            (Some(binding), false) => GraphDB::open_for_project(db_path, binding)?,
            (None, false) => GraphDB::open(db_path)?,
            (None, true) => anyhow::bail!(
                "GRAPH_PROJECT_BINDING_REQUIRED: ownership scanning requires a project binding"
            ),
        });
        Self::scan_store_body(backend, project_dir, ownership_enabled)
    }

    /// 共享扫描编排体：diff → 解析 → apply → commit → 诊断聚合。
    ///
    /// 非 ownership 路径在 `&mut dyn IndexScanStore` 上运行，redb / grafeo
    /// 两个后端走同一份代码；ownership 应用仍是 `GraphDB` 具体 API，在
    /// `ScanBackend::Redb` 分支上保留。
    fn scan_store_body(
        mut backend: ScanBackend,
        project_dir: &Path,
        ownership_enabled: bool,
    ) -> Result<crate::scanner::IndexReportWithDiagnostics> {
        let prev_states = backend.store().load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        // M59 codex 复审返修：与当前文件集对账，找出既有覆盖机制够不着的陈旧诊断
        // （孤儿 entry / 内容已复原但警告仍在）。见 `stale_scanner_diagnostic_paths`。
        let discovered: HashSet<String> = files
            .iter()
            .map(|path| logical_path_of(path, project_dir))
            .collect();
        let stale_diagnostic_paths = stale_scanner_diagnostic_paths(
            &backend
                .store()
                .load_scanner_diagnostic_entries()
                .map_err(|err| anyhow!("{err}"))?,
            &discovered,
            &prev_states,
            &plan.dirty,
        );
        let wants_delta = backend.store().wants_index_delta();

        let mut new_states = prev_states.clone();

        // 只有陈旧诊断要清时也得进这个分支：否则清理提交永远没有机会发生。
        if !plan.dirty.is_empty() || !plan.deleted.is_empty() || !stale_diagnostic_paths.is_empty()
        {
            let (updates, parse_failures) = Self::parse_dirty_files_with_failures(
                &prev_states,
                &plan.dirty,
                project_dir,
                &provider,
            )?;
            // M58.3 PR1 refix（F2）：per-file 诊断计数序列化，随 commit 载荷
            // 与图/file states 在同一事务落库（M58.3 复核返修：原子化）
            let mut scanner_entries = Vec::new();
            for update in &updates {
                if let Some(entry) = per_file_scan_diagnostic_entry(update)? {
                    scanner_entries.push(entry);
                }
            }
            // M59-B3：解析失败的文件也占一条 entry。顺序在成功项之后无所谓——
            // 两者路径互斥（一个文件这轮要么成功要么失败），不会互相覆盖。
            for failure in &parse_failures {
                scanner_entries.push(parse_failure_diagnostic_entry(failure)?);
            }
            let mut ownership_conflicts: Vec<crate::ownership::OwnershipConflict> = Vec::new();
            let (parsed_nodes, removed_edge_keys, dirty_edges) = match &mut backend {
                // ownership 应用层是 GraphDB 具体 API（M59-2 未完成）；
                // grafeo+ownership 已在入口 bail。
                ScanBackend::Redb(graph) if ownership_enabled => {
                    let mut ledgers = graph.ownership_ledgers().cloned().unwrap_or_default();
                    let (parsed_nodes, conflicts) =
                        apply_ownership_changes(graph, &mut ledgers, &updates, &plan.deleted)?;
                    ownership_conflicts = conflicts;
                    for (logical_path, _) in &plan.deleted {
                        new_states.remove(logical_path);
                    }
                    (parsed_nodes, Vec::new(), Vec::new())
                }
                backend => {
                    let store = backend.store();
                    // M56：apply 前收集被删节点的 incident edge keys
                    //（persist 只消费 delta；grafeo 等不消费 delta 的后端直接跳过）
                    let merged_removed = merge_removed_node_ids(
                        updates
                            .iter()
                            .map(|update| update.previous_node_ids.as_slice())
                            .chain(plan.deleted.iter().map(|(_, node_ids)| node_ids.as_slice())),
                    );
                    let removed_edge_keys = if wants_delta {
                        collect_incident_edge_keys(&*store, &merged_removed)
                    } else {
                        Vec::new()
                    };
                    // M54：dirty previous IDs 与 deleted IDs 合并去重后只做一次图批删
                    let parsed_nodes = Self::apply_incremental_changes(
                        &mut *store,
                        &mut new_states,
                        &updates,
                        &plan.deleted,
                    )?;
                    // M56：apply 后收集新增/变更节点的 incident edges
                    let new_node_ids: Vec<String> = parsed_nodes
                        .values()
                        .flat_map(|node_ids| node_ids.iter().cloned())
                        .collect();
                    let dirty_edges = if wants_delta {
                        collect_incident_edges(&*store, &new_node_ids)
                    } else {
                        Vec::new()
                    };
                    (parsed_nodes, removed_edge_keys, dirty_edges)
                }
            };

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

            // M58.3 复核返修：本轮删除文件的 logical_path 随 commit 载荷落库，
            // 同事务移除其 scanner 诊断 entry；
            // M59 codex 复审返修：对账出的陈旧路径一并移除。
            let scanner_deleted_paths =
                merge_scanner_deleted_paths(&plan.deleted, &stale_diagnostic_paths);
            let commit = IndexCommit {
                file_states: new_states.clone(),
                dirty_nodes: backend.store().pending_dirty_nodes(),
                deleted_nodes: backend.store().pending_removed_nodes(),
                checkpoint: None,
                // M56：初始全量构建（无 prev states）走显式 full rebuild 路径
                //（v2 置 Current）；增量提交走 delta 路径（v2 置 Stale）；
                // 直写后端（grafeo）没有影子层，不携带 delta。
                delta: if prev_states.is_empty() || ownership_enabled || !wants_delta {
                    None
                } else {
                    Some(crate::graph_store::IndexDelta {
                        dirty_edges,
                        removed_edge_keys,
                        changed_file_states,
                        removed_file_paths,
                    })
                },
                scanner_entries,
                scanner_deleted_paths,
            };
            let mut report = Self::persist_index(backend.store(), commit)?;
            // M58.3 复核返修：per-file 诊断计数已随 commit 同事务落库
            //（脏文件覆盖 entry，删除文件移除 entry），persist 成功后重新
            // load 合并全量——报告 envelope 反映全库口径，而非仅本轮脏文件。
            let scanner_diagnostics = Self::merge_scanner_diagnostic_entries(
                &backend
                    .store()
                    .load_scanner_diagnostic_entries()
                    .map_err(|err| anyhow!("{err}"))?,
            )?;
            let mut diagnostics = scanner_diagnostics;
            diagnostics.extend(ownership_conflict_diagnostics(&ownership_conflicts));
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
                diagnostics,
                node_count: backend.store().node_count().map_err(|e| anyhow!("{e}"))?,
                edge_count: backend.store().edge_count().map_err(|e| anyhow!("{e}"))?,
            });
        }

        let mut diagnostics = Self::merge_scanner_diagnostic_entries(
            &backend
                .store()
                .load_scanner_diagnostic_entries()
                .map_err(|err| anyhow!("{err}"))?,
        )?;
        // no-op 轮也要如实报告当前账本状态中仍然存在的来源冲突
        if ownership_enabled {
            // grafeo+ownership 已在入口 bail，这里必然落在 redb 分支。
            if let ScanBackend::Redb(graph) = &mut backend {
                let ledgers = graph.ownership_ledgers().cloned().unwrap_or_default();
                diagnostics.extend(ownership_conflict_diagnostics(
                    &crate::ownership::ledger_definition_conflicts(&ledgers),
                ));
            }
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
            diagnostics,
            node_count: backend.store().node_count().map_err(|e| anyhow!("{e}"))?,
            edge_count: backend.store().edge_count().map_err(|e| anyhow!("{e}"))?,
        })
    }

    /// M54：候选准备入口 — 打开候选 GraphDB、完成 diff/parse/apply、
    /// 构造 commit，但不调用 `persist_index`（不写盘）。
    ///
    /// 与 `scan` 复用同一套阶段函数；graphdb 文件在 prepare 前后保持不变，
    /// 由调用方决定何时把 `commit`（可附加 diff-refresh checkpoint）落盘。
    /// M58.3 PR2：per-file scanner 诊断 entries 与删除路径随返回值透传；
    /// M58.3 复核返修：调用方须先把它们挂到 commit 的 scanner 载荷上
    /// （可与 pending 累积合并），再 persist，保证与图同事务落库。
    pub fn prepare(project_dir: &Path, db_path: &Path) -> Result<PreparedIndexUpdate> {
        Self::prepare_internal(project_dir, db_path, None)
    }

    /// 按稳定项目绑定准备 ownership 候选图，供 session/diff-refresh 生产路径使用。
    pub fn prepare_for_project(
        project_dir: &Path,
        db_path: &Path,
        project_binding: &ProjectBinding,
    ) -> Result<PreparedIndexUpdate> {
        Self::prepare_internal(project_dir, db_path, Some(project_binding))
    }

    fn prepare_internal(
        project_dir: &Path,
        db_path: &Path,
        project_binding: Option<&ProjectBinding>,
    ) -> Result<PreparedIndexUpdate> {
        // M59-3 C2：diff-refresh 候选 prepare 还没接线 grafeo（PreparedIndexUpdate
        // 内嵌 GraphDB 候选图，属 diff-refresh 通路范围，不是 C2）。
        // 显式拒绝比让 redb 打开一个 grafeo 文件报格式错更清楚。
        if crate::graph_store::is_grafeo_db_path(db_path) {
            anyhow::bail!(
                "GRAPH_BACKEND_UNSUPPORTED: 候选 prepare（diff-refresh）尚未支持 grafeo 后端"
            );
        }
        let ownership_enabled = project_binding.is_some();
        let mut graph = match project_binding {
            Some(binding) => GraphDB::open_with_ownership(db_path, binding)?,
            None => GraphDB::open(db_path)?,
        };
        let prev_states = graph.load_file_states().unwrap_or_default();

        let files = Self::discover_files(project_dir)?;
        let provider = LocalStorageProvider;
        let plan = Self::diff_file_states(&files, &prev_states, project_dir, &provider)?;

        // M59 codex 复审返修：与 scan 同一套对账（prepare 不落盘，路径随
        // PreparedIndexUpdate 透传给编排器）。
        let discovered: HashSet<String> = files
            .iter()
            .map(|path| logical_path_of(path, project_dir))
            .collect();
        let stale_diagnostic_paths = stale_scanner_diagnostic_paths(
            &graph.load_scanner_diagnostic_entries()?,
            &discovered,
            &prev_states,
            &plan.dirty,
        );

        let mut new_states = prev_states.clone();
        let (updates, parse_failures) = Self::parse_dirty_files_with_failures(
            &prev_states,
            &plan.dirty,
            project_dir,
            &provider,
        )?;
        // M58.3 PR2：与 scan 相同逻辑收集本轮 per-file scanner 诊断 entries，
        // 随 PreparedIndexUpdate 透传给调用方（diff-refresh 编排器）落库，
        // 候选图不落盘不代表诊断可以丢——持久化责任移交调用方。
        let mut scanner_entries = Vec::new();
        for update in &updates {
            if let Some(entry) = per_file_scan_diagnostic_entry(update)? {
                scanner_entries.push(entry);
            }
        }
        // M59-B3：解析失败同样透传（与 scan 一致）
        for failure in &parse_failures {
            scanner_entries.push(parse_failure_diagnostic_entry(failure)?);
        }
        let mut prepare_conflicts: Vec<crate::ownership::OwnershipConflict> = Vec::new();
        let (parsed_nodes, removed_edge_keys, dirty_edges) = if ownership_enabled {
            let mut ledgers = graph.ownership_ledgers().cloned().unwrap_or_default();
            // M59-2：来源冲突随候选更新透出（不再丢弃）——调用方（diff-refresh
            // 编排器）据此把冲突写进诊断与 runtime 状态，使冲突在查询侧可见。
            let (parsed_nodes, conflicts) =
                apply_ownership_changes(&mut graph, &mut ledgers, &updates, &plan.deleted)?;
            prepare_conflicts = conflicts;
            for (logical_path, _) in &plan.deleted {
                new_states.remove(logical_path);
            }
            (parsed_nodes, Vec::new(), Vec::new())
        } else {
            // M56：apply 前收集被删节点的 incident edge keys（persist 只消费 delta）
            let merged_removed = merge_removed_node_ids(
                updates
                    .iter()
                    .map(|update| update.previous_node_ids.as_slice())
                    .chain(plan.deleted.iter().map(|(_, node_ids)| node_ids.as_slice())),
            );
            let removed_edge_keys = collect_incident_edge_keys(&graph, &merged_removed);
            // dirty previous IDs 与 deleted IDs 合并去重后只做一次图批删（与 scan 一致）
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
            (parsed_nodes, removed_edge_keys, dirty_edges)
        };

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
        // M58.3 PR2：本轮删除文件的 logical_path，落库时需移除其诊断 entry；
        // M59 codex 复审返修：并上对账出的陈旧路径。
        let scanner_deleted_paths =
            merge_scanner_deleted_paths(&plan.deleted, &stale_diagnostic_paths);

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
            delta: if prev_states.is_empty() || ownership_enabled {
                None
            } else {
                Some(crate::graph_store::IndexDelta {
                    dirty_edges,
                    removed_edge_keys,
                    changed_file_states,
                    removed_file_paths,
                })
            },
            // M58.3 复核返修：prepare 不落盘，scanner 载荷由调用方
            // （diff-refresh 编排器）合并 pending 累积后挂到 commit 再 persist
            scanner_entries: Vec::new(),
            scanner_deleted_paths: Vec::new(),
        };
        Ok(PreparedIndexUpdate {
            graph,
            commit,
            dirty_node_ids,
            deleted_node_ids,
            scanner_entries,
            scanner_deleted_paths,
            // M59-2 B：本轮解析失败但**已消费变更事件**的源文件。
            // 非空 ⇒ 本轮是部分失败：必须保留旧贡献（已完成）、保持文件脏
            // （可重试），且**不得推进 diff-refresh checkpoint**。
            parse_failures,
            ownership_conflicts: prepare_conflicts,
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
    /// 生命周期（M58.3 复核返修）：调用方把本字段挂到 commit 的 scanner
    /// 载荷上随 commit 同事务落库；deferred 模式由编排器跨轮合并
    /// （脏覆盖、删除移除），随 pending commit 一起原子落库。
    pub scanner_entries: Vec<(String, Vec<u8>)>,
    /// M58.3 PR2：本轮删除文件的 logical_path，落库时移除其诊断 entry
    pub scanner_deleted_paths: Vec<String>,
    /// M59-2 B：本轮解析失败的文件（事件已被消费，但内容未进候选图）。
    ///
    /// 旧图与旧贡献原样保留、file hash 不推进，因此文件下一轮仍是脏的、
    /// 会重试。非空即表示本轮是**部分失败**：调用方不得把 checkpoint 推进到
    /// 本轮水位——否则这些事件被永久消费掉，只能等未来新事件偶然触发重试。
    pub parse_failures: Vec<ParseFailure>,
    /// M59-2 C：本轮账本中仍然存在的来源冲突。
    ///
    /// prepare 不落盘，但冲突不能随返回值蒸发；调用方须把它写进 runtime
    /// 诊断与持久化诊断，使冲突在 query/status 与重启后一致可见。
    pub ownership_conflicts: Vec<crate::ownership::OwnershipConflict>,
}

impl PreparedIndexUpdate {
    /// 本轮是否为部分失败（存在已消费但未入图的源文件）。
    pub fn has_parse_failure(&self) -> bool {
        !self.parse_failures.is_empty()
    }
}
