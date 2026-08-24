//! redb 持久化图数据库实现
//!
//! M40.1：从 graph.rs 拆出，只在 cli-local feature 下编译。
//! 包含 GraphDB struct、redb table 定义、lock file、open/load/persist/check。

use crate::graph::{Edge, EdgeType, FileState, Node};
use crate::graph_store::{
    GraphEdgeView, GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
    GraphWriteStore, IndexCommit, IndexReport, IndexStateStore,
};
use crate::output::schema::{
    AiOutput, Confidence, DiagnosticSeverity, Evidence, Location, OutputKind,
    format_next_query,
};
use anyhow::{Context, Result};
use fs2::FileExt;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// 图数据库锁等待超时（毫秒），进程级可配置
static GRAPH_LOCK_TIMEOUT_MS: AtomicU64 = AtomicU64::new(3000);

/// 设置 graphdb 锁等待超时（毫秒）
pub fn set_graph_lock_timeout_ms(ms: u64) {
    GRAPH_LOCK_TIMEOUT_MS.store(ms, Ordering::Relaxed);
}

const NODES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("nodes");
const EDGES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("edges");
const FILE_STATES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("file_states");
const META_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("meta");
/// M54：diff-refresh checkpoint 在 META_TABLE 中的键
const META_DIFF_REFRESH_CHECKPOINT_KEY: &str = "diff_refresh_checkpoint";

/// M56：edge 在 EDGES_TABLE 的存储键（存储格式契约）。
///
/// scanner/bench 收集 delta 与 persist 全量/增量写共用同一格式，避免两套键。
pub fn edge_storage_key(edge: &Edge) -> String {
    let type_str = serde_json::to_string(&edge.edge_type).unwrap_or_default();
    format!(
        "{}|{}|{}|{}",
        edge.from,
        edge.to,
        type_str,
        edge.field_path.as_deref().unwrap_or("")
    )
}

/// M56 P1：Stale 累计重建阈值（受影响节点 keys 数）。
///
/// 真图测量（release、77,077 节点）：v1 hydrate 1604ms / v2 hydrate 1408ms
/// （每轮差 196ms），v2 全量重建 8575ms —— 每轮重建（A/C）要 ~44 轮小 delta
/// 才回本，不划算；取 1024 节点 keys 作为阈值，超过时本轮同事务重建 v2。
const V2_STALE_REBUILD_THRESHOLD_NODES: u64 = 1024;

/// 进程间文件锁（用于串行化 graphdb 访问）
///
/// redb 同一文件不支持多进程并发打开，因此通过辅助锁文件实现串行。
fn acquire_graph_lock(db_path: &Path) -> Result<std::fs::File> {
    let lock_path = db_path.with_extension("graphdb.lock");
    let timeout_ms = GRAPH_LOCK_TIMEOUT_MS.load(Ordering::Relaxed);
    let interval_ms = 100u64;
    let max_attempts = timeout_ms.div_ceil(interval_ms).max(1);
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .with_context(|| format!("无法打开 graphdb 锁文件 {}", lock_path.display()))?;

    for attempt in 0..max_attempts {
        match lock_file.try_lock_exclusive() {
            Ok(()) => return Ok(lock_file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if attempt + 1 < max_attempts {
                    std::thread::sleep(std::time::Duration::from_millis(interval_ms));
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("无法锁定 graphdb 锁文件 {}", lock_path.display()));
            }
        }
    }
    anyhow::bail!(
        "Cannot acquire graphdb lock after {}ms: {:?}",
        timeout_ms,
        lock_path
    )
}

/// GraphDB 锁守卫，Drop 时释放与 GraphDB 相同的锁文件
pub(crate) struct GraphDbLockGuard {
    lock_file: Option<std::fs::File>,
}

impl Drop for GraphDbLockGuard {
    fn drop(&mut self) {
        if let Some(lock_file) = self.lock_file.take() {
            let _ = lock_file.unlock();
        }
    }
}

/// 获取 GraphDB 共享锁语义，供同一 graphdb 文件的附加持久化表复用
pub(crate) fn acquire_graph_db_lock(db_path: &Path) -> Result<GraphDbLockGuard> {
    let lock_file = acquire_graph_lock(db_path)?;
    Ok(GraphDbLockGuard {
        lock_file: Some(lock_file),
    })
}

/// 图数据库（内存图 + redb 持久化）
pub struct GraphDB {
    pub graph: DiGraph<Node, Edge>,
    pub node_indices: HashMap<String, NodeIndex>,
    pub db_path: String,
    is_dirty: bool,
    seen_edges: HashSet<(String, String, EdgeType, Option<String>)>,
    dirty_nodes: HashSet<String>,
    removed_nodes: HashSet<String>,
    topology_dirty: bool,
    /// v2 hydrate 失败时的诊断信息（fallback 到 v1 后仍可正常使用，
    /// 调用方可读取此字段决定是否上报 diagnostic）。
    v2_hydrate_warning: Option<String>,
    /// hydrate 阶段的结构化诊断统计（PR1 引入，供 runtime 统一信封透出）
    hydrate_diagnostics: crate::diagnostics::HydrateDiagnostics,
}

type NodeEdgePair<'a> = (&'a Node, &'a Edge);

impl GraphDB {
    /// 打开或创建图数据库
    pub fn open(db_path: &Path) -> Result<Self> {
        let _lock = acquire_graph_db_lock(db_path)?;
        Self::open_inner(db_path)
    }

    /// 从已构建的内存图创建 GraphDB，供 v2 hydrate 与 shadow compare 使用。
    pub(crate) fn from_memory_graph(
        graph: DiGraph<Node, Edge>,
        node_indices: HashMap<String, NodeIndex>,
        db_path: String,
    ) -> Self {
        let mut seen_edges = HashSet::new();
        for edge_ref in graph.edge_references() {
            let edge = edge_ref.weight();
            seen_edges.insert((
                edge.from.clone(),
                edge.to.clone(),
                edge.edge_type.clone(),
                edge.field_path.clone(),
            ));
        }
        Self {
            graph,
            node_indices,
            db_path,
            is_dirty: false,
            seen_edges,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
            topology_dirty: false,
            v2_hydrate_warning: None,
            hydrate_diagnostics: crate::diagnostics::HydrateDiagnostics::default(),
        }
    }

    fn open_inner(db_path: &Path) -> Result<Self> {
        Self::ensure_redb_tables(db_path)?;

        let mut pending_hydrate_diagnostics = crate::diagnostics::HydrateDiagnostics::default();

        // 检测 v2 layout 可读性：read_v2_layout 报错/返回 None 均视为一次 V2_LAYOUT_UNREADABLE，
        // 与后续 v1 hydrate 的细粒度计数并行记录（不折叠）。
        let v2_layout_probe = crate::graph_redb_v2::read_v2_layout(db_path);
        let v2_layout_had_error = matches!(&v2_layout_probe, Err(_));
        if v2_layout_had_error {
            pending_hydrate_diagnostics.v2_layout_unreadable = 1;
        }

        // M56：v2 shadow 为 Stale 时跳过 v2，从增量更新后的 v1 hydrate；
        // Current 或旧库无记录（视为 Current，向后兼容）时维持 v2 优先。
        if crate::graph_redb_v2::read_v2_shadow_state(db_path)?
            != crate::graph_redb_v2::V2ShadowState::Stale
        {
            if let Ok(Some(layout)) = v2_layout_probe {
                match crate::graph_redb_v2::hydrate_graph_from_v2(
                    &layout,
                    &db_path.to_string_lossy(),
                ) {
                    Ok(mut graph) => {
                        graph.hydrate_diagnostics = pending_hydrate_diagnostics;
                        return Ok(graph);
                    }
                    Err(error) => {
                        // v2 hydrate 失败时 fallback v1（始终正确），但记录诊断供调用方上报
                        let mut graph = Self::open_inner_v1(db_path)?;
                        graph.v2_hydrate_warning = Some(format!(
                            "v2 shadow hydrate failed, fell back to v1: {error:#}"
                        ));
                        if graph.hydrate_diagnostics.v2_hydrate_warning.is_none() {
                            graph.hydrate_diagnostics.v2_hydrate_warning =
                                graph.v2_hydrate_warning.clone();
                        }
                        if graph.hydrate_diagnostics.v2_layout_unreadable == 0
                            && pending_hydrate_diagnostics.v2_layout_unreadable > 0
                        {
                            graph.hydrate_diagnostics.v2_layout_unreadable =
                                pending_hydrate_diagnostics.v2_layout_unreadable;
                        }
                        return Ok(graph);
                    }
                }
            }
        }

        let mut graph = Self::open_inner_v1(db_path)?;
        // 合并未通过 v2 路径的 v2_layout 探针诊断
        if graph.hydrate_diagnostics.v2_layout_unreadable == 0
            && pending_hydrate_diagnostics.v2_layout_unreadable > 0
        {
            graph.hydrate_diagnostics.v2_layout_unreadable =
                pending_hydrate_diagnostics.v2_layout_unreadable;
        }
        Ok(graph)
    }

    /// 确保 redb 基础表存在。
    fn ensure_redb_tables(db_path: &Path) -> Result<()> {
        let db = Database::create(db_path)
            .with_context(|| format!("Failed to create/open database at {:?}", db_path))?;
        let write_txn = db.begin_write()?;
        {
            let _ = write_txn.open_table(NODES_TABLE)?;
            let _ = write_txn.open_table(EDGES_TABLE)?;
            let _ = write_txn.open_table(FILE_STATES_TABLE)?;
            let _ = write_txn.open_table(META_TABLE)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    /// 从 v1 节点/边表 hydrate 内存图。
    fn open_inner_v1(db_path: &Path) -> Result<Self> {
        let db = Database::create(db_path)
            .with_context(|| format!("Failed to create/open database at {:?}", db_path))?;

        let mut graph = DiGraph::new();
        let mut node_indices = HashMap::new();
        let mut hydrate_diagnostics = crate::diagnostics::HydrateDiagnostics::default();

        let read_txn = db.begin_read()?;
        let nodes_table = read_txn.open_table(NODES_TABLE)?;
        for item in nodes_table.iter()? {
            let (key, value) = item?;
            let id = key.value();
            match serde_json::from_slice::<Node>(value.value().as_slice()) {
                Ok(node) => {
                    let idx = graph.add_node(node);
                    node_indices.insert(id.to_string(), idx);
                }
                Err(_) => {
                    hydrate_diagnostics.node_decode_failed += 1;
                    if hydrate_diagnostics.sample_node_location.is_none() {
                        hydrate_diagnostics.sample_node_location = Some(crate::output::Location {
                            source_file: Some(db_path.to_string_lossy().to_string()),
                            node_id: Some(id.to_string()),
                            json_path: None,
                        });
                    }
                }
            }
        }

        let mut seen_edges: HashSet<(String, String, EdgeType, Option<String>)> = HashSet::new();
        let edges_table = read_txn.open_table(EDGES_TABLE)?;
        for item in edges_table.iter()? {
            let (key, value) = item?;
            let raw_key = key.value().to_string();
            match serde_json::from_slice::<Edge>(value.value().as_slice()) {
                Ok(edge) => {
                    if let (Some(&from_idx), Some(&to_idx)) =
                        (node_indices.get(&edge.from), node_indices.get(&edge.to))
                    {
                        graph.add_edge(from_idx, to_idx, edge.clone());
                        seen_edges.insert((
                            edge.from.clone(),
                            edge.to.clone(),
                            edge.edge_type.clone(),
                            edge.field_path.clone(),
                        ));
                    } else {
                        hydrate_diagnostics.dangling_edge += 1;
                        if hydrate_diagnostics.sample_dangling_location.is_none() {
                            hydrate_diagnostics.sample_dangling_location =
                                Some(crate::output::Location {
                                    source_file: Some(db_path.to_string_lossy().to_string()),
                                    node_id: Some(edge.from.clone()),
                                    json_path: Some(raw_key.clone()),
                                });
                        }
                    }
                }
                Err(_) => {
                    hydrate_diagnostics.edge_decode_failed += 1;
                    if hydrate_diagnostics.sample_edge_location.is_none() {
                        hydrate_diagnostics.sample_edge_location = Some(crate::output::Location {
                            source_file: Some(db_path.to_string_lossy().to_string()),
                            node_id: None,
                            json_path: Some(raw_key.clone()),
                        });
                    }
                }
            }
        }

        Ok(GraphDB {
            graph,
            node_indices,
            db_path: db_path.to_string_lossy().to_string(),
            is_dirty: false,
            seen_edges,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
            topology_dirty: false,
            v2_hydrate_warning: None,
            hydrate_diagnostics,
        })
    }

    /// 读取 v2 hydrate 诊断信息（fallback 到 v1 时记录的失败原因）。
    ///
    /// 调用方可据此决定是否上报 diagnostic；`None` 表示 v2 hydrate 正常或未尝试。
    pub fn v2_hydrate_warning(&self) -> Option<&str> {
        self.v2_hydrate_warning.as_deref()
    }

    /// 读取 hydrate 结构化诊断统计
    pub fn hydrate_diagnostics(&self) -> &crate::diagnostics::HydrateDiagnostics {
        &self.hydrate_diagnostics
    }

    /// 检查图数据库状态，返回结构化 AiOutput（不 panic）
    pub fn check_graph_db(db_path: &Path) -> AiOutput {
        let mut out = AiOutput::new(
            OutputKind::GraphDbCheck,
            serde_json::json!({
                "db_path": db_path.to_string_lossy().to_string(),
                "exists": false,
                "readable": false,
                "writable": false,
                "needs_rebuild": true,
            }),
        );
        out.query_target = Some(db_path.to_string_lossy().to_string());

        if !db_path.exists() {
            let mut diag = crate::diagnostics::envelope_diagnostic(
                "GRAPH_DB_NOT_FOUND",
                1,
                Location {
                    source_file: Some(db_path.to_string_lossy().to_string()),
                    node_id: None,
                    json_path: None,
                },
                format!("Graph database not found at {:?}", db_path),
            );
            diag.severity = DiagnosticSeverity::Error;
            diag.suggestion = Some("Run metadata-checker --project-dir <DIR> --build-graph to create it".to_string());
            out.diagnostics.push(diag);
            out.next_queries.push(format_next_query(
                "metadata-checker --project-dir <DIR> --build-graph --graph-db-path {}",
                &db_path.to_string_lossy(),
            ));
            return out;
        }

        let file_readable = std::fs::File::open(db_path).is_ok();
        let writable = std::fs::OpenOptions::new()
            .write(true)
            .open(db_path)
            .is_ok();

        out.summary["exists"] = serde_json::json!(true);
        out.summary["readable"] = serde_json::json!(file_readable);
        out.summary["writable"] = serde_json::json!(writable);

        let _lock = match acquire_graph_lock(db_path) {
            Ok(l) => l,
            Err(e) => {
                let mut diag = crate::diagnostics::envelope_diagnostic(
                    "GRAPH_DB_LOCKED",
                    1,
                    Location {
                        source_file: Some(db_path.to_string_lossy().to_string()),
                        node_id: None,
                        json_path: None,
                    },
                    format!("Cannot acquire graphdb lock: {}", e),
                );
                diag.severity = DiagnosticSeverity::Error;
                diag.suggestion = Some("Wait for other process to finish, or use a different --graph-db-path".to_string());
                out.diagnostics.push(diag);
                out.next_queries.push(format_next_query(
                    "metadata-checker --graph-db-path {} --graph-lock-timeout-ms <MS>",
                    &db_path.to_string_lossy(),
                ));
                return out;
            }
        };
        let db_result = Database::open(db_path);
        match db_result {
            Ok(db) => {
                let read_txn = db.begin_read();
                let tables_ok = read_txn
                    .map(|tx| {
                        tx.open_table(NODES_TABLE).is_ok() && tx.open_table(EDGES_TABLE).is_ok()
                    })
                    .unwrap_or(false);
                out.summary["needs_rebuild"] = serde_json::json!(!tables_ok);
                out.evidence.push(
                    Evidence::new("Graph database opened successfully", "redb open + read")
                        .with_source_file(db_path.to_string_lossy().to_string())
                        .with_confidence(Confidence::High),
                );
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("lock")
                    || msg.contains("already open")
                    || msg.contains("Cannot acquire")
                {
                    out.diagnostics.push({
                        let mut diag = crate::diagnostics::envelope_diagnostic(
                            "GRAPH_DB_LOCKED",
                            1,
                            Location {
                                source_file: Some(db_path.to_string_lossy().to_string()),
                                node_id: None,
                                json_path: None,
                            },
                            format!("Graph database is locked by another process: {}", msg),
                        );
                        diag.severity = DiagnosticSeverity::Error;
                        diag.suggestion = Some("Wait for other process to finish, or use a different --graph-db-path".to_string());
                        diag
                    });
                } else if msg.contains("permission")
                    || msg.contains("denied")
                    || msg.contains("read-only")
                {
                    if file_readable && !writable {
                        out.diagnostics.push({
                            let mut diag = crate::diagnostics::envelope_diagnostic(
                                "GRAPH_DB_READ_ONLY",
                                1,
                                Location {
                                    source_file: Some(db_path.to_string_lossy().to_string()),
                                    node_id: None,
                                    json_path: None,
                                },
                                format!("Graph database file is read-only: {}", msg),
                            );
                            diag.severity = DiagnosticSeverity::Info;
                            diag.suggestion = Some("redb requires write access even for read. Copy to a writable path with --graph-db-path".to_string());
                            diag
                        });
                    } else {
                        out.diagnostics.push({
                            let mut diag = crate::diagnostics::envelope_diagnostic(
                                "GRAPH_DB_PERMISSION_DENIED",
                                1,
                                Location {
                                    source_file: Some(db_path.to_string_lossy().to_string()),
                                    node_id: None,
                                    json_path: None,
                                },
                                format!("Graph database permission denied: {}", msg),
                            );
                            diag.severity = DiagnosticSeverity::Error;
                            diag.suggestion = Some("Use --graph-db-path pointing to a writable directory".to_string());
                            diag
                        });
                    }
                } else {
                    out.diagnostics.push({
                        let mut diag = crate::diagnostics::envelope_diagnostic(
                            "GRAPH_DB_OPEN_ERROR",
                            1,
                            Location {
                                source_file: Some(db_path.to_string_lossy().to_string()),
                                node_id: None,
                                json_path: None,
                            },
                            format!("Failed to open graph database: {}", msg),
                        );
                        diag.severity = DiagnosticSeverity::Error;
                        diag.suggestion = Some("Try --build-graph to rebuild".to_string());
                        diag
                    });
                }
                out.summary["needs_rebuild"] = serde_json::json!(true);
            }
        }

        out
    }

    /// 尝试打开图数据库，失败时返回结构化 AiOutput 错误
    pub fn open_or_diagnostic(db_path: &Path) -> Result<Self, Box<AiOutput>> {
        if !db_path.exists() {
            let mut out = AiOutput::new(
                OutputKind::GraphDbCheck,
                serde_json::json!({
                    "db_path": db_path.to_string_lossy().to_string(),
                    "exists": false,
                }),
            );
            out.query_target = Some(db_path.to_string_lossy().to_string());
            out.diagnostics.push({
                let mut diag = crate::diagnostics::envelope_diagnostic(
                    "GRAPH_DB_NOT_FOUND",
                    1,
                    Location {
                        source_file: Some(db_path.to_string_lossy().to_string()),
                        node_id: None,
                        json_path: None,
                    },
                    format!("Graph database not found at {:?}", db_path),
                );
                diag.severity = DiagnosticSeverity::Error;
                diag.suggestion = Some("Run metadata-checker --project-dir <DIR> --build-graph to create it".to_string());
                diag
            });
            out.next_queries.push(format_next_query(
                "metadata-checker --project-dir <DIR> --build-graph --graph-db-path {}",
                &db_path.to_string_lossy(),
            ));
            return Err(Box::new(out));
        }

        match Self::open_readonly(db_path) {
            Ok(g) => Ok(g),
            Err(e) => {
                let msg = e.to_string();
                let mut out = AiOutput::new(
                    OutputKind::GraphDbCheck,
                    serde_json::json!({
                        "db_path": db_path.to_string_lossy().to_string(),
                        "exists": true,
                    }),
                );
                out.query_target = Some(db_path.to_string_lossy().to_string());
                if msg.contains("lock")
                    || msg.contains("already open")
                    || msg.contains("Cannot acquire")
                {
                    out.diagnostics.push({
                        let mut diag = crate::diagnostics::envelope_diagnostic(
                            "GRAPH_DB_LOCKED",
                            1,
                            Location {
                                source_file: Some(db_path.to_string_lossy().to_string()),
                                node_id: None,
                                json_path: None,
                            },
                            format!("Graph database locked: {}", msg),
                        );
                        diag.severity = DiagnosticSeverity::Error;
                        diag.suggestion = Some("Use --graph-db-path to a separate path, wait for other process, or increase --graph-lock-timeout-ms".to_string());
                        diag
                    });
                    out.next_queries.push(format_next_query(
                        "metadata-checker --project-dir <DIR> --query-model <MODEL> --graph-db-path {} --graph-lock-timeout-ms 30000",
                        &db_path.to_string_lossy(),
                    ));
                } else if msg.contains("permission")
                    || msg.contains("denied")
                    || msg.contains("read-only")
                {
                    out.diagnostics.push({
                        let mut diag = crate::diagnostics::envelope_diagnostic(
                            "GRAPH_DB_PERMISSION_DENIED",
                            1,
                            Location {
                                source_file: Some(db_path.to_string_lossy().to_string()),
                                node_id: None,
                                json_path: None,
                            },
                            format!("Graph database permission denied: {}", msg),
                        );
                        diag.severity = DiagnosticSeverity::Error;
                        diag.suggestion = Some("Use --graph-db-path pointing to a writable directory".to_string());
                        diag
                    });
                } else {
                    out.diagnostics.push({
                        let mut diag = crate::diagnostics::envelope_diagnostic(
                            "GRAPH_DB_OPEN_ERROR",
                            1,
                            Location {
                                source_file: Some(db_path.to_string_lossy().to_string()),
                                node_id: None,
                                json_path: None,
                            },
                            format!("Failed to open graph database: {}", msg),
                        );
                        diag.severity = DiagnosticSeverity::Error;
                        diag.suggestion = Some("Try --build-graph to rebuild".to_string());
                        diag
                    });
                }
                Err(Box::new(out))
            }
        }
    }

    /// 以只读模式打开图数据库（不创建表，不持有写锁）
    pub fn open_readonly(db_path: &Path) -> Result<Self> {
        let _lock = acquire_graph_db_lock(db_path)?;
        Self::open_readonly_inner(db_path)
    }

    fn open_readonly_inner(db_path: &Path) -> Result<Self> {
        if !db_path.exists() {
            anyhow::bail!("Graph database not found at {:?}", db_path);
        }
        let mut last_err = None;
        for attempt in 0..3 {
            match Database::open(db_path) {
                Ok(db) => {
                    return Self::load_from_db(db, db_path);
                }
                Err(e) => {
                    let msg = e.to_string();
                    if (msg.contains("lock")
                        || msg.contains("already open")
                        || msg.contains("Cannot acquire"))
                        && attempt < 2
                    {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        continue;
                    }
                    last_err = Some(e);
                    break;
                }
            }
        }
        let err = last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "Unknown redb error".to_string());
        Err(anyhow::anyhow!(
            "Failed to open database at {:?}: {}",
            db_path,
            err
        ))
    }

    fn load_from_db(db: Database, db_path: &Path) -> Result<Self> {
        let mut graph = DiGraph::new();
        let mut node_indices = HashMap::new();
        let mut hydrate_diagnostics = crate::diagnostics::HydrateDiagnostics::default();

        let read_txn = db.begin_read()?;
        let nodes_table = read_txn.open_table(NODES_TABLE)?;
        for item in nodes_table.iter()? {
            let (key, value) = item?;
            let id = key.value();
            match serde_json::from_slice::<Node>(value.value().as_slice()) {
                Ok(node) => {
                    let idx = graph.add_node(node);
                    node_indices.insert(id.to_string(), idx);
                }
                Err(_) => {
                    hydrate_diagnostics.node_decode_failed += 1;
                    if hydrate_diagnostics.sample_node_location.is_none() {
                        hydrate_diagnostics.sample_node_location = Some(crate::output::Location {
                            source_file: Some(db_path.to_string_lossy().to_string()),
                            node_id: Some(id.to_string()),
                            json_path: None,
                        });
                    }
                }
            }
        }

        let mut seen_edges: HashSet<(String, String, EdgeType, Option<String>)> = HashSet::new();
        let edges_table = read_txn.open_table(EDGES_TABLE)?;
        for item in edges_table.iter()? {
            let (key, value) = item?;
            let raw_key = key.value().to_string();
            match serde_json::from_slice::<Edge>(value.value().as_slice()) {
                Ok(edge) => {
                    if let (Some(&from_idx), Some(&to_idx)) =
                        (node_indices.get(&edge.from), node_indices.get(&edge.to))
                    {
                        graph.add_edge(from_idx, to_idx, edge.clone());
                        seen_edges.insert((
                            edge.from.clone(),
                            edge.to.clone(),
                            edge.edge_type.clone(),
                            edge.field_path.clone(),
                        ));
                    } else {
                        hydrate_diagnostics.dangling_edge += 1;
                        if hydrate_diagnostics.sample_dangling_location.is_none() {
                            hydrate_diagnostics.sample_dangling_location =
                                Some(crate::output::Location {
                                    source_file: Some(db_path.to_string_lossy().to_string()),
                                    node_id: Some(edge.from.clone()),
                                    json_path: Some(raw_key.clone()),
                                });
                        }
                    }
                }
                Err(_) => {
                    hydrate_diagnostics.edge_decode_failed += 1;
                    if hydrate_diagnostics.sample_edge_location.is_none() {
                        hydrate_diagnostics.sample_edge_location = Some(crate::output::Location {
                            source_file: Some(db_path.to_string_lossy().to_string()),
                            node_id: None,
                            json_path: Some(raw_key.clone()),
                        });
                    }
                }
            }
        }

        Ok(GraphDB {
            graph,
            node_indices,
            db_path: db_path.to_string_lossy().to_string(),
            is_dirty: false,
            seen_edges,
            dirty_nodes: HashSet::new(),
            removed_nodes: HashSet::new(),
            topology_dirty: false,
            v2_hydrate_warning: None,
            hydrate_diagnostics,
        })
    }

    /// 添加节点
    pub fn add_node(
        &mut self,
        id: String,
        node_type: crate::graph::NodeType,
        path: String,
        name: String,
        meta: Option<serde_json::Value>,
    ) -> NodeIndex {
        if let Some(&idx) = self.node_indices.get(&id) {
            if let Some(existing) = self.graph.node_weight_mut(idx) {
                if meta.is_some() {
                    existing.meta = meta;
                }
                existing.node_type = node_type;
                existing.path = path;
                existing.name = name;
            }
            self.is_dirty = true;
            self.dirty_nodes.insert(id.clone());
            self.removed_nodes.remove(&id);
            return idx;
        }
        let node = Node {
            id: id.clone(),
            node_type,
            path,
            name,
            meta,
        };
        let idx = self.graph.add_node(node);
        self.is_dirty = true;
        self.topology_dirty = true;
        self.dirty_nodes.insert(id.clone());
        self.removed_nodes.remove(&id);
        self.node_indices.insert(id, idx);
        idx
    }

    /// 添加有向边
    pub fn add_edge(
        &mut self,
        from: &str,
        to: &str,
        edge_type: EdgeType,
        field_path: Option<String>,
    ) {
        self.add_edge_with_meta(from, to, edge_type, field_path, None)
    }

    pub fn add_edge_with_meta(
        &mut self,
        from: &str,
        to: &str,
        edge_type: EdgeType,
        field_path: Option<String>,
        meta: Option<serde_json::Value>,
    ) {
        if let (Some(&from_idx), Some(&to_idx)) =
            (self.node_indices.get(from), self.node_indices.get(to))
        {
            let key = (
                from.to_string(),
                to.to_string(),
                edge_type.clone(),
                field_path.clone(),
            );
            if !self.seen_edges.insert(key) {
                return;
            }
            let edge = Edge {
                from: from.to_string(),
                to: to.to_string(),
                edge_type,
                field_path,
                meta,
            };
            self.is_dirty = true;
            self.topology_dirty = true;
            self.graph.add_edge(from_idx, to_idx, edge);
        }
    }

    /// 批量删除节点（in-place，不重建整图）
    ///
    /// 使用 petgraph 的 `remove_node` 逐个删除（swap_remove 语义），
    /// 同时维护 `node_indices` 和 `seen_edges`。每次删除后更新被 swap
    /// 的节点的索引映射。O(k * avg_degree) 其中 k 为待删节点数，
    /// 不随全图规模放大。
    pub fn remove_nodes_by_ids(&mut self, node_ids: &[String]) {
        if node_ids.is_empty() {
            return;
        }
        self.is_dirty = true;
        self.topology_dirty = true;

        for id in node_ids {
            self.removed_nodes.insert(id.clone());
            self.dirty_nodes.remove(id);

            // 从 seen_edges 移除涉及该节点的所有边
            self.seen_edges
                .retain(|(from, to, _, _)| from != id && to != id);

            // 从内存图删除节点（petgraph swap_remove 自动移除关联边，
            // 但会把末尾节点 swap 到被删位置，需更新其索引）
            if let Some(&idx) = self.node_indices.get(id) {
                // petgraph 的 remove_node 用 swap_remove：末尾节点被移到 idx 位置。
                // 删除前记录末尾节点 ID，删除后更新其索引。
                let last_node_id = self.graph.node_weights().last().map(|n| n.id.clone());
                self.graph.remove_node(idx);

                // 末尾节点被 swap 到 idx 位置，更新其索引映射
                // （被删节点本身是末尾时 last_node_id 为 None，无需更新）
                if let Some(swap_id) = last_node_id {
                    // swap_id 不应等于被删 id（删除前它还存在于图中）
                    if swap_id != *id {
                        self.node_indices.insert(swap_id, idx);
                    }
                }
            }
            self.node_indices.remove(id);
        }
    }

    /// 为 persist 选择 v2 layout：仅当 dirty 节点全是既有 v2 节点且拓扑未删改时做 meta patch。
    fn build_v2_layout_for_persist(
        graph: &GraphDB,
        file_states: &HashMap<String, FileState>,
        db_path: &std::path::Path,
    ) -> Result<crate::graph_redb_v2::RedbV2Layout> {
        const INCREMENTAL_V2_MAX_DIRTY: usize = 32;
        let can_try_incremental = !graph.topology_dirty
            && graph.removed_nodes.is_empty()
            && !graph.dirty_nodes.is_empty()
            && graph.dirty_nodes.len() <= INCREMENTAL_V2_MAX_DIRTY;
        if can_try_incremental {
            if let Ok(Some(mut existing)) = crate::graph_redb_v2::read_v2_layout(db_path) {
                let dirty_subset_of_v2 = graph.dirty_nodes.iter().all(|node_id| {
                    existing
                        .node_ids
                        .iter()
                        .any(|existing_id| existing_id == node_id)
                });
                if dirty_subset_of_v2 {
                    crate::graph_redb_v2::patch_v2_layout_node_meta(
                        &mut existing,
                        graph,
                        &graph.dirty_nodes,
                        file_states,
                    )
                    .context("incremental patch v2 layout")?;
                    return Ok(existing);
                }
            }
        }
        crate::graph_redb_v2::build_v2_layout(graph, file_states)
            .context("build redb v2 shadow layout")
    }

    pub fn persist(&mut self, file_states: &HashMap<String, FileState>) -> Result<()> {
        self.persist_with_checkpoint(file_states, None)?;
        Ok(())
    }

    /// M54：持久化图与 file states，并在同一 write transaction 写入
    /// diff-refresh checkpoint（`META_TABLE`）。
    ///
    /// checkpoint-only commit（graph 无 dirty、checkpoint 为 Some）也必须
    /// 写入 `META_TABLE`；graph 与 checkpoint 要么同时生效要么同时保持旧值。
    pub fn persist_with_checkpoint(
        &mut self,
        file_states: &HashMap<String, FileState>,
        checkpoint: Option<&crate::diff_refresh::DiffRefreshCheckpoint>,
    ) -> Result<crate::graph_store::PersistReport> {
        self.persist_internal(file_states, checkpoint, None)
    }

    /// M56：按 IndexCommit 持久化并返回本次提交的成本报告。
    ///
    /// `commit.delta` 为 `Some` 时走增量路径：edges/file_states 只写受影响
    /// keys，topology dirty 时只把 v2 shadow 标记 `Stale`（不写全量 v2 blobs）；
    /// 为 `None` 时维持全量重写（显式 full rebuild/compaction 路径，v2 置 Current）。
    pub fn persist_commit(
        &mut self,
        commit: &crate::graph_store::IndexCommit,
    ) -> Result<crate::graph_store::PersistReport> {
        self.persist_internal(
            &commit.file_states,
            commit.checkpoint.as_ref(),
            commit.delta.as_ref(),
        )
    }

    fn persist_internal(
        &mut self,
        file_states: &HashMap<String, FileState>,
        checkpoint: Option<&crate::diff_refresh::DiffRefreshCheckpoint>,
        delta: Option<&crate::graph_store::IndexDelta>,
    ) -> Result<crate::graph_store::PersistReport> {
        let commit_started = std::time::Instant::now();
        // P1 修复：persist 全程持有 graph lock，覆盖预读→写入→提交，
        // 防止并发 CLI/--build-graph 在 mutate→persist 间隙写入不一致边集
        let _lock = acquire_graph_db_lock(std::path::Path::new(&self.db_path))?;

        if !self.is_dirty && checkpoint.is_none() {
            // 空提交：无写入，report 全零（v2 状态读当前值）
            return Ok(crate::graph_store::PersistReport {
                dirty_nodes: 0,
                dirty_edges: 0,
                changed_file_states: 0,
                bytes_written: 0,
                commit_ms: commit_started.elapsed().as_millis(),
                full_rewrite: false,
                v2_shadow_state: crate::graph_redb_v2::read_v2_shadow_state(std::path::Path::new(
                    &self.db_path,
                ))?,
            });
        }
        let was_dirty = self.is_dirty;
        let mut bytes_written: u64 = 0;
        // M56 P1：在打开写库之前预读 stale 状态（redb 单文件单实例，
        // persist 过程中不能再开读库）
        let stale_base = if delta.is_some() && was_dirty {
            crate::graph_redb_v2::read_v2_stale_dirty_nodes(std::path::Path::new(&self.db_path))?
        } else {
            0
        };
        let prev_shadow_state = if delta.is_some() && !was_dirty {
            Some(crate::graph_redb_v2::read_v2_shadow_state(
                std::path::Path::new(&self.db_path),
            )?)
        } else {
            None
        };

        // delta 路径不构建 v2 layout（topology dirty 只标记 Stale）；
        // 全量路径维持现状：构建/patch v2 layout 并写 v2 blobs（Current）
        let layout = if delta.is_none() {
            let db_path = std::path::Path::new(&self.db_path);
            Some(Self::build_v2_layout_for_persist(
                self,
                file_states,
                db_path,
            )?)
        } else {
            None
        };

        let db = Database::create(&self.db_path)?;
        let write_txn = db.begin_write()?;

        // nodes：两路径相同（removed/dirty 集合增量写）
        if !self.removed_nodes.is_empty() {
            let mut nodes_table = write_txn.open_table(NODES_TABLE)?;
            for id in &self.removed_nodes {
                nodes_table.remove(id.as_str())?;
            }
        }
        if !self.dirty_nodes.is_empty() {
            let mut nodes_table = write_txn.open_table(NODES_TABLE)?;
            for id in &self.dirty_nodes {
                if let Some(&idx) = self.node_indices.get(id)
                    && let Some(node) = self.graph.node_weight(idx)
                {
                    let bytes = serde_json::to_vec(node)
                        .with_context(|| format!("Failed to serialize node {}", id))?;
                    bytes_written += (id.len() + bytes.len()) as u64;
                    nodes_table.insert(id.as_str(), bytes)?;
                }
            }
        }

        let mut dirty_edge_count = 0_usize;
        let mut changed_state_count = 0_usize;
        match delta {
            Some(delta) => {
                // M56 edges delta：只删/写受影响 keys，不扫描全图
                {
                    let mut edges_table = write_txn.open_table(EDGES_TABLE)?;
                    for key in &delta.removed_edge_keys {
                        edges_table.remove(key.as_str())?;
                    }
                    for edge in &delta.dirty_edges {
                        let bytes =
                            serde_json::to_vec(edge).with_context(|| "Failed to serialize edge")?;
                        let key = edge_storage_key(edge);
                        bytes_written += (key.len() + bytes.len()) as u64;
                        edges_table.insert(key.as_str(), bytes)?;
                    }
                    dirty_edge_count = delta.removed_edge_keys.len() + delta.dirty_edges.len();
                }
                // M56 file_states delta：只删/写受影响 paths
                {
                    let mut states_table = write_txn.open_table(FILE_STATES_TABLE)?;
                    for path in &delta.removed_file_paths {
                        states_table.remove(path.as_str())?;
                    }
                    for path in &delta.changed_file_states {
                        let state = file_states
                            .get(path)
                            .with_context(|| format!("delta changed file state missing: {path}"))?;
                        let bytes = serde_json::to_vec(state)
                            .with_context(|| format!("Failed to serialize file state {path}"))?;
                        bytes_written += (path.len() + bytes.len()) as u64;
                        states_table.insert(path.as_str(), bytes)?;
                    }
                    changed_state_count =
                        delta.changed_file_states.len() + delta.removed_file_paths.len();
                }
            }
            None => {
                {
                    let mut edges_table = write_txn.open_table(EDGES_TABLE)?;
                    edges_table.retain(|_, _| false)?;
                    for edge_ref in self.graph.edge_references() {
                        let edge = edge_ref.weight();
                        let key = edge_storage_key(edge);
                        let bytes =
                            serde_json::to_vec(edge).with_context(|| "Failed to serialize edge")?;
                        bytes_written += (key.len() + bytes.len()) as u64;
                        edges_table.insert(key.as_str(), bytes)?;
                        dirty_edge_count += 1;
                    }
                }

                {
                    let mut states_table = write_txn.open_table(FILE_STATES_TABLE)?;
                    states_table.retain(|_, _| false)?;
                    for (path, state) in file_states {
                        let bytes = serde_json::to_vec(state)
                            .with_context(|| format!("Failed to serialize file state {}", path))?;
                        bytes_written += (path.len() + bytes.len()) as u64;
                        states_table.insert(path.as_str(), bytes)?;
                        changed_state_count += 1;
                    }
                }
            }
        }

        if let Some(checkpoint) = checkpoint {
            let bytes = serde_json::to_vec(checkpoint)
                .context("Failed to serialize diff refresh checkpoint")?;
            bytes_written += (META_DIFF_REFRESH_CHECKPOINT_KEY.len() + bytes.len()) as u64;
            let mut meta_table = write_txn.open_table(META_TABLE)?;
            meta_table.insert(META_DIFF_REFRESH_CHECKPOINT_KEY, bytes)?;
        }

        let v2_shadow_state = match delta {
            Some(_) => {
                // graph 未变化（checkpoint-only）时不动 v2 状态
                if was_dirty {
                    // M56 P1：累计 stale 受影响节点 keys；超阈值时本轮同事务
                    // 重建 v2 并置 Current（每轮重建不回本，见阈值常量注释）
                    let affected = (self.dirty_nodes.len() + self.removed_nodes.len()) as u64;
                    let accumulated = stale_base + affected;
                    if accumulated >= V2_STALE_REBUILD_THRESHOLD_NODES {
                        let layout = crate::graph_redb_v2::build_v2_layout(self, file_states)
                            .context("rebuild redb v2 shadow layout after stale threshold")?;
                        crate::graph_redb_v2::write_v2_shadow_tables(&write_txn, &layout)
                            .context("write redb v2 shadow tables after stale threshold")?;
                        crate::graph_redb_v2::write_v2_stale_dirty_nodes(&write_txn, 0)?;
                        crate::graph_store::V2ShadowState::Current
                    } else {
                        crate::graph_redb_v2::write_v2_shadow_state(
                            &write_txn,
                            crate::graph_redb_v2::V2ShadowState::Stale,
                        )
                        .context("mark v2 shadow stale")?;
                        crate::graph_redb_v2::write_v2_stale_dirty_nodes(&write_txn, accumulated)?;
                        crate::graph_store::V2ShadowState::Stale
                    }
                } else {
                    prev_shadow_state.expect("prev shadow state captured before opening write db")
                }
            }
            None => {
                crate::graph_redb_v2::write_v2_shadow_tables(
                    &write_txn,
                    layout.as_ref().expect("layout built for full persist path"),
                )
                .context("write redb v2 shadow tables")?;
                // 全量 layout 写入后无 stale 累计
                crate::graph_redb_v2::write_v2_stale_dirty_nodes(&write_txn, 0)?;
                crate::graph_store::V2ShadowState::Current
            }
        };

        write_txn.commit()?;

        self.is_dirty = false;
        let dirty_node_count = self.dirty_nodes.len() + self.removed_nodes.len();
        self.dirty_nodes.clear();
        self.removed_nodes.clear();
        self.topology_dirty = false;
        Ok(crate::graph_store::PersistReport {
            dirty_nodes: dirty_node_count,
            dirty_edges: dirty_edge_count,
            changed_file_states: changed_state_count,
            bytes_written,
            commit_ms: commit_started.elapsed().as_millis(),
            full_rewrite: delta.is_none(),
            v2_shadow_state,
        })
    }

    /// 从 redb 加载文件状态
    pub fn load_file_states(&self) -> Result<HashMap<String, FileState>> {
        let db = Database::create(&self.db_path)?;
        let read_txn = db.begin_read()?;
        let table = read_txn.open_table(FILE_STATES_TABLE)?;
        let mut states = HashMap::new();
        for item in table.iter()? {
            let (key, value) = item?;
            if let Ok(state) = serde_json::from_slice::<FileState>(value.value().as_slice()) {
                states.insert(key.value().to_string(), state);
            }
        }
        Ok(states)
    }

    /// M54：加载 diff-refresh checkpoint。
    ///
    /// 旧 graphdb 无 `META_TABLE`（或表内无该键）时返回 `None`，
    /// 由调用方走 bootstrap 初始化路径。
    pub fn load_diff_refresh_checkpoint(
        &self,
    ) -> Result<Option<crate::diff_refresh::DiffRefreshCheckpoint>> {
        let db = Database::create(&self.db_path)?;
        let read_txn = db.begin_read()?;
        let table = match read_txn.open_table(META_TABLE) {
            Ok(table) => table,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let Some(value) = table.get(META_DIFF_REFRESH_CHECKPOINT_KEY)? else {
            return Ok(None);
        };
        let checkpoint = serde_json::from_slice(value.value().as_slice())
            .context("Failed to deserialize diff refresh checkpoint")?;
        Ok(Some(checkpoint))
    }

    /// 查找读取指定模型的所有节点
    pub fn find_readers<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&model_idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(model_idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::Reads)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查找写入指定模型的所有节点
    pub fn find_writers<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&model_idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(model_idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::Writes | EdgeType::ActionWrites)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查找两个页面之间的共同依赖路径
    pub fn find_cross_relations<'a>(
        &'a self,
        page_a: &str,
        page_b: &str,
    ) -> Vec<Vec<NodeEdgePair<'a>>> {
        let mut paths = Vec::new();
        if let (Some(&a_idx), Some(&b_idx)) =
            (self.node_indices.get(page_a), self.node_indices.get(page_b))
        {
            let a_neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(a_idx, petgraph::Direction::Outgoing)
                .collect();
            let b_neighbors: Vec<NodeIndex> = self
                .graph
                .neighbors_directed(b_idx, petgraph::Direction::Outgoing)
                .collect();

            for a_n in &a_neighbors {
                if b_neighbors.contains(a_n)
                    && let (Some(a_node), Some(mid_node)) =
                        (self.graph.node_weight(a_idx), self.graph.node_weight(*a_n))
                    && let Some(edge_a) = self.graph.edges_connecting(a_idx, *a_n).next()
                    && let Some(edge_b) = self.graph.edges_connecting(b_idx, *a_n).next()
                {
                    paths.push(vec![(a_node, edge_a.weight()), (mid_node, edge_b.weight())]);
                }
            }
        }
        paths
    }

    /// 查询 DataFlow 的输入依赖
    pub fn find_dataflow_inputs<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Outgoing)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::DataflowInput)
                    && let Some(node) = self.graph.node_weight(edge_ref.target())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查询 DataFlow 的输出目标
    pub fn find_dataflow_outputs<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Outgoing)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::OutputsTo)
                    && let Some(node) = self.graph.node_weight(edge_ref.target())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查询物理表的生产者
    pub fn find_produced_by<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::OutputsTo)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 查询哪些 DataFlow 消费了该输入表
    pub fn find_consumed_by_dataflows<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        let mut results = Vec::new();
        if let Some(&idx) = self.node_indices.get(model_id) {
            for edge_ref in self
                .graph
                .edges_directed(idx, petgraph::Direction::Incoming)
            {
                let edge = edge_ref.weight();
                if matches!(edge.edge_type, EdgeType::DataflowInput)
                    && let Some(node) = self.graph.node_weight(edge_ref.source())
                {
                    results.push((node, edge));
                }
            }
        }
        results
    }

    /// 兼容性保留
    pub fn find_upstream_dependencies<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        self.find_consumed_by_dataflows(model_id)
    }

    /// 兼容性保留
    pub fn find_downstream_outputs<'a>(&'a self, model_id: &str) -> Vec<NodeEdgePair<'a>> {
        self.find_dataflow_outputs(model_id)
    }

    pub fn get_node(&self, node_id: &str) -> Option<Node> {
        self.node_indices
            .get(node_id)
            .and_then(|&idx| self.graph.node_weight(idx))
            .cloned()
    }

    /// 按 ID 获取节点边
    pub fn get_node_edges<'a>(
        &'a self,
        node_id: &str,
    ) -> Option<(Vec<NodeEdgePair<'a>>, Vec<NodeEdgePair<'a>>)> {
        let idx = self.node_indices.get(node_id)?;

        let outgoing: Vec<NodeEdgePair<'a>> = self
            .graph
            .edges_directed(*idx, petgraph::Direction::Outgoing)
            .filter_map(|e| self.graph.node_weight(e.target()).map(|n| (n, e.weight())))
            .collect();

        let incoming: Vec<NodeEdgePair<'a>> = self
            .graph
            .edges_directed(*idx, petgraph::Direction::Incoming)
            .filter_map(|e| self.graph.node_weight(e.source()).map(|n| (n, e.weight())))
            .collect();

        Some((outgoing, incoming))
    }

    /// 返回当前脏节点 ID 集合的引用
    pub fn dirty_nodes_set(&self) -> &std::collections::HashSet<String> {
        &self.dirty_nodes
    }

    /// 返回当前已移除节点 ID 集合的引用
    pub fn removed_nodes_set(&self) -> &std::collections::HashSet<String> {
        &self.removed_nodes
    }

    /// 搜索与目标 ID 相似的候选节点
    pub fn find_candidates(&self, target_id: &str, limit: usize) -> Vec<(Node, String)> {
        crate::candidate::rank_candidates(
            self.node_indices
                .values()
                .filter_map(|index| self.graph.node_weight(*index).cloned()),
            target_id,
            limit,
        )
    }
}

impl GraphReadStore for GraphDB {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        Ok(self
            .node_indices
            .get(node_id)
            .and_then(|&idx| self.graph.node_weight(idx))
            .cloned())
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        let idx = match self.node_indices.get(node_id) {
            Some(i) => *i,
            None => return Ok(None),
        };

        let outgoing: Vec<GraphEdgeView> = self
            .graph
            .edges_directed(idx, petgraph::Direction::Outgoing)
            .filter_map(|e| {
                self.graph.node_weight(e.target()).map(|n| GraphEdgeView {
                    node: n.clone(),
                    edge: e.weight().clone(),
                })
            })
            .collect();

        let incoming: Vec<GraphEdgeView> = self
            .graph
            .edges_directed(idx, petgraph::Direction::Incoming)
            .filter_map(|e| {
                self.graph.node_weight(e.source()).map(|n| GraphEdgeView {
                    node: n.clone(),
                    edge: e.weight().clone(),
                })
            })
            .collect();

        Ok(Some(GraphNeighbors { outgoing, incoming }))
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        Ok(self.graph.node_count())
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        Ok(self.graph.edge_count())
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        Ok(Box::new(self.graph.node_weights().cloned()))
    }
}

impl GraphWriteStore for GraphDB {
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()> {
        let node_id = node.id.clone();
        if let Some(idx) = self.node_indices.get(&node_id).copied() {
            let preserve_meta = merge_upsert_meta(self.graph[idx].meta.clone(), node.meta);
            self.graph[idx] = Node {
                meta: preserve_meta,
                ..node
            };
        } else {
            let idx = self.graph.add_node(node);
            self.node_indices.insert(node_id.clone(), idx);
            self.topology_dirty = true;
        }
        self.is_dirty = true;
        self.dirty_nodes.insert(node_id.clone());
        self.removed_nodes.remove(&node_id);
        Ok(())
    }

    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()> {
        GraphDB::add_edge_with_meta(
            self,
            &edge.from,
            &edge.to,
            edge.edge_type,
            edge.field_path,
            edge.meta,
        );
        Ok(())
    }

    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()> {
        GraphDB::remove_nodes_by_ids(self, &node_ids.to_vec());
        Ok(())
    }
}

/// 合并写入节点 meta
fn merge_upsert_meta(
    existing_meta: Option<serde_json::Value>,
    incoming_meta: Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    let Some(incoming_meta) = incoming_meta else {
        return existing_meta;
    };
    let Some(existing_meta) = existing_meta else {
        return Some(incoming_meta);
    };

    let existing_model_type = existing_meta.get("modelType").and_then(|v| v.as_str());
    let incoming_model_type = incoming_meta.get("modelType").and_then(|v| v.as_str());
    if incoming_model_type == Some("PhysicalTable")
        && matches!(existing_model_type, Some("DataFlow" | "App"))
    {
        return Some(existing_meta);
    }

    Some(incoming_meta)
}

impl IndexStateStore for GraphDB {
    fn load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>> {
        GraphDB::load_file_states(self).map_err(|e| GraphStoreError::ReadFailed {
            reason: format!("{}", e),
        })
    }

    fn persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport> {
        GraphDB::persist_commit(self, &commit).map_err(|e| GraphStoreError::WriteFailed {
            reason: format!("{}", e),
        })?;
        Ok(IndexReport {
            indexed: commit.file_states.len(),
            unchanged: commit
                .file_states
                .len()
                .saturating_sub(commit.dirty_nodes.len()),
            dirty: commit.dirty_nodes.len(),
            deleted: commit.deleted_nodes.len(),
        })
    }
}
