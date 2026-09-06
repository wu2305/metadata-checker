//! 图存储抽象模块
//!
//! M39：把图查询、图写入、索引状态提交拆开，让 query 不再绑定 redb。
//! 读写分离：GraphReadStore / GraphWriteStore / IndexStateStore。

use crate::graph::{Edge, FileState, Node};
use std::collections::HashMap;
use std::fmt;

/// 图存储操作结果
pub type GraphStoreResult<T> = Result<T, GraphStoreError>;

/// 图存储错误枚举
///
/// 内部图存储错误的稳定 code 来源，不直接复用 ToolErrorCode，但预留统一映射函数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphStoreError {
    /// 节点或资源不存在
    NotFound { resource: String },
    /// 参数非法
    InvalidArgument { message: String },
    /// 打开数据库失败
    OpenFailed { path: String, reason: String },
    /// 读取失败
    ReadFailed { reason: String },
    /// 写入失败
    WriteFailed { reason: String },
    /// 序列化失败
    SerializeFailed { reason: String },
    /// 反序列化失败
    DeserializeFailed { reason: String },
    /// 获取锁超时
    LockTimeout { path: String },
    /// 权限不足
    PermissionDenied { path: String },
    /// 数据损坏
    Corrupted { reason: String },
    /// 不支持的操作
    UnsupportedOperation { message: String },
}

impl GraphStoreError {
    /// 返回稳定错误码字符串
    pub fn code(&self) -> &'static str {
        match self {
            GraphStoreError::NotFound { .. } => "GRAPH_DB_NOT_FOUND",
            GraphStoreError::InvalidArgument { .. } => "INVALID_ARGUMENT",
            GraphStoreError::OpenFailed { .. } => "GRAPH_DB_OPEN_FAILED",
            GraphStoreError::ReadFailed { .. } => "GRAPH_DB_READ_FAILED",
            GraphStoreError::WriteFailed { .. } => "GRAPH_DB_WRITE_FAILED",
            GraphStoreError::SerializeFailed { .. } => "GRAPH_DB_SERIALIZE_FAILED",
            GraphStoreError::DeserializeFailed { .. } => "GRAPH_DB_DESERIALIZE_FAILED",
            GraphStoreError::LockTimeout { .. } => "GRAPH_DB_LOCK_TIMEOUT",
            GraphStoreError::PermissionDenied { .. } => "GRAPH_DB_PERMISSION_DENIED",
            GraphStoreError::Corrupted { .. } => "GRAPH_DB_CORRUPTED",
            GraphStoreError::UnsupportedOperation { .. } => "GRAPH_DB_UNSUPPORTED_OPERATION",
        }
    }
}

impl fmt::Display for GraphStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphStoreError::NotFound { resource } => {
                write!(f, "Graph store resource not found: {}", resource)
            }
            GraphStoreError::InvalidArgument { message } => {
                write!(f, "Invalid graph store argument: {}", message)
            }
            GraphStoreError::OpenFailed { path, reason } => {
                write!(f, "Failed to open graph store at {}: {}", path, reason)
            }
            GraphStoreError::ReadFailed { reason } => {
                write!(f, "Graph store read failed: {}", reason)
            }
            GraphStoreError::WriteFailed { reason } => {
                write!(f, "Graph store write failed: {}", reason)
            }
            GraphStoreError::SerializeFailed { reason } => {
                write!(f, "Graph store serialize failed: {}", reason)
            }
            GraphStoreError::DeserializeFailed { reason } => {
                write!(f, "Graph store deserialize failed: {}", reason)
            }
            GraphStoreError::LockTimeout { path } => {
                write!(f, "Graph store lock timeout at: {}", path)
            }
            GraphStoreError::PermissionDenied { path } => {
                write!(f, "Graph store permission denied at: {}", path)
            }
            GraphStoreError::Corrupted { reason } => {
                write!(f, "Graph store corrupted: {}", reason)
            }
            GraphStoreError::UnsupportedOperation { message } => {
                write!(f, "Graph store unsupported operation: {}", message)
            }
        }
    }
}

impl std::error::Error for GraphStoreError {}

/// 节点边的视图（owned，避免 trait 生命周期复杂化）
#[derive(Debug, Clone)]
pub struct GraphEdgeView {
    pub node: Node,
    pub edge: Edge,
}

/// 节点的邻居（出边和入边）
#[derive(Debug, Clone)]
pub struct GraphNeighbors {
    pub outgoing: Vec<GraphEdgeView>,
    pub incoming: Vec<GraphEdgeView>,
}

/// 图只读存储 trait
///
/// 不包含 reader/writer/dataflow 这类业务 helper，只表达低语义图读取能力。
pub trait GraphReadStore {
    /// 按 ID 获取节点（返回克隆）
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>>;

    /// 按 ID 获取节点的出边和入边
    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>>;

    /// 节点总数
    fn node_count(&self) -> GraphStoreResult<usize>;

    /// 边总数
    fn edge_count(&self) -> GraphStoreResult<usize>;

    /// 遍历所有节点（返回 owned Node，避免生命周期复杂化）
    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>>;
}

/// 图写入存储 trait
///
/// 写接口接收完整 Node / Edge，方便后续自定义新增节点和边。
/// 不继承 GraphReadStore，读写可独立组合。
pub trait GraphWriteStore {
    /// 插入或更新节点
    fn upsert_node(&mut self, node: Node) -> GraphStoreResult<()>;

    /// 添加边
    fn add_edge(&mut self, edge: Edge) -> GraphStoreResult<()>;

    /// 按 ID 列表删除节点
    fn remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()>;
}

/// 组合读写 trait（用于需要同时读写的场景）
pub trait GraphStore: GraphReadStore + GraphWriteStore {}

impl<T: GraphReadStore + GraphWriteStore + ?Sized> GraphStore for T {}
/// 索引提交单元
///
/// 一次索引操作产生的图变更 + 文件状态快照。
/// `checkpoint` 仅 diff-refresh 通路设置，与 graph/file states 在同一
/// write transaction 提交；其余调用点显式传 `None`。
/// `delta` 为 M56 增量持久化负载：`Some` 时 persist 只写受影响
/// node/edge/file-state keys 并把 v2 shadow 标记 Stale；
/// `None` 时维持全量重写（显式 full rebuild/compaction 路径，v2 置 Current）。
#[derive(Debug, Clone)]
pub struct IndexCommit {
    pub file_states: HashMap<String, FileState>,
    pub dirty_nodes: Vec<String>,
    pub deleted_nodes: Vec<String>,
    /// diff-refresh checkpoint（active/deleted 双 cursor 水位）
    pub checkpoint: Option<crate::diff_refresh::DiffRefreshCheckpoint>,
    /// M56 增量持久化 delta；`None` 表示走全量重写路径
    pub delta: Option<IndexDelta>,
    /// M58.3 复核返修：scanner 诊断载荷，与 nodes/edges/file_states/checkpoint
    /// 在同一 write transaction 落库（同生共死，避免 checkpoint 已推进但诊断
    /// 陈旧的持久不一致）。
    /// `scanner_entries`：本轮脏文件的 `(logical_path, 序列化计数 bytes)`，
    /// 覆盖同 key 旧值；计数为零的修复文件也携带 entry 以覆盖旧值。
    /// `scanner_deleted_paths`：已删除文件的 logical_path，移除其 entry。
    /// 两者皆空表示本次提交不涉及 scanner 诊断变更。
    pub scanner_entries: Vec<(String, Vec<u8>)>,
    /// 已删除文件的 logical_path 列表，落库时移除其 scanner 诊断 entry
    pub scanner_deleted_paths: Vec<String>,
}

/// M56：增量持久化 delta（由 scanner 在 remove/re-add 时收集，persist 只消费）。
#[derive(Debug, Clone)]
pub struct IndexDelta {
    /// 新增/变更节点的 incident edges（apply 后快照，按键去重）
    pub dirty_edges: Vec<crate::graph::Edge>,
    /// 被删节点的 incident edge keys（apply 前快照）
    pub removed_edge_keys: Vec<String>,
    /// file state 发生变化的文件路径
    pub changed_file_states: Vec<String>,
    /// 已删除文件的路径
    pub removed_file_paths: Vec<String>,
}

/// v2 shadow 状态。
///
/// `Current`：v2 与 v1 一致，`GraphDB::open` 优先 v2 hydrate；
/// `Stale`：v1 已有增量提交而 v2 未重建，open 必须跳过 v2 走 v1 hydrate。
/// 类型放在 graph_store（ungated），供 PersistReport 跨 feature 引用。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V2ShadowState {
    Current,
    Stale,
}

/// M56：一次 persist 提交的成本报告。
///
/// 由 `GraphDB::persist_commit` 返回；`IndexStateStore::persist_index`
/// 的 `IndexReport` 保持高层摘要不变。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersistReport {
    /// 受影响的节点 keys 数（removed + dirty）
    pub dirty_nodes: usize,
    /// 受影响的 edge keys 数（delta 路径为 removed+dirty；全量路径为全图边数）
    pub dirty_edges: usize,
    /// 受影响的 file-state keys 数（delta 路径为 changed+removed；全量路径为全量 states）
    pub changed_file_states: usize,
    /// 本轮实际写入 redb 的字节数（insert 的 key+value 合计；remove 不计）
    pub bytes_written: u64,
    /// persist 提交耗时（毫秒）
    pub commit_ms: u128,
    /// 是否走了全量 durable rewrite（delta=None 的显式 full rebuild 路径）。
    /// 空提交（无 dirty 且无 checkpoint）标记 false，因为没有实际写入。
    pub full_rewrite: bool,
    /// 提交后的 v2 shadow 状态
    pub v2_shadow_state: V2ShadowState,
}

/// 索引报告
///
/// 一次索引提交后的结果摘要。**四个字段均为文件口径**（不是节点数）：
/// `indexed` = 发现的文件总数，`dirty` = 本轮重新解析的脏文件数，
/// `deleted` = 本轮删除的文件数，`unchanged` = `indexed - dirty`。
///
/// 注意：`IndexStateStore::persist_index` 的 store 实现（redb/memory）
/// 只能从 `IndexCommit` 拿到节点数，返回的是节点口径的近似值；
/// 文件口径由 `ProjectIndexer::scan_with_diagnostics` 在报告出口层
/// 用 diff 计划覆盖（M58.3 PR1 refix，F6）。直接消费 store 层
/// `persist_index` 返回值的调用方（如 perf profile）读到的仍是节点口径。
#[derive(Debug, Clone)]
pub struct IndexReport {
    /// 发现的文件总数
    pub indexed: usize,
    /// 未变更文件数（`indexed - dirty`）
    pub unchanged: usize,
    /// 本轮重新解析的脏文件数
    pub dirty: usize,
    /// 本轮删除的文件数
    pub deleted: usize,
}

/// 索引状态存储 trait
///
/// 负责 file states 与一次索引结果提交。
pub trait IndexStateStore {
    /// 加载已持久化的文件状态
    fn load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>>;

    /// 持久化索引结果（IndexCommit 为唯一提交边界）
    fn persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport>;
}

/// 把 GraphStoreError 映射为 ToolError（统一错误出口）
impl From<GraphStoreError> for crate::tool_contract::ToolError {
    fn from(err: GraphStoreError) -> Self {
        let code = match err.code() {
            "GRAPH_DB_NOT_FOUND" => crate::tool_contract::ToolErrorCode::GraphDbNotFound,
            "INVALID_ARGUMENT" => crate::tool_contract::ToolErrorCode::InvalidArgument,
            "GRAPH_DB_OPEN_FAILED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_READ_FAILED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_WRITE_FAILED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_SERIALIZE_FAILED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_DESERIALIZE_FAILED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_LOCK_TIMEOUT" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_PERMISSION_DENIED" => crate::tool_contract::ToolErrorCode::InternalError,
            "GRAPH_DB_CORRUPTED" => crate::tool_contract::ToolErrorCode::InternalError,
            _ => crate::tool_contract::ToolErrorCode::InternalError,
        };
        crate::tool_contract::ToolError::new(code, err.to_string())
    }
}

/// 合并写入节点 meta —— **GraphStore 契约的一部分**，所有实现共用这一个函数。
///
/// M59-B1：本函数原先私有于 `graph_redb.rs`，而 `MemoryGraphStore::upsert_node`
/// 直接 `insert` 覆盖，于是同一组写入在两个实现上产出不同的 meta。规则一旦分头
/// 实现就必然分叉，所以提到 trait 模块，`graph_redb` 与 `memory_graph_store`
/// 都调这一份；将来的 Grafeo 实现同样调它，而不是再抄一遍。
///
/// 规则：
/// 1. 写入方没带 meta ⇒ **保留既有 meta**。扫描器会为「尚未见到定义的引用目标」
///    先建占位节点，之后真正的定义再 upsert 上来；反过来也有先定义后被引用的
///    顺序。无条件覆盖会让后到的、信息更少的那次写入抹掉已有事实。
/// 2. 既有为空 ⇒ 采用写入方的 meta。
/// 3. **占位节点不得被降级**：`PhysicalTable` 是引用侧推断出的保守类型，
///    不能覆盖已经确认的 `DataFlow` / `App`。其余情况以写入方为准。
pub fn merge_upsert_meta(
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

/// 边的去重键：`(from, to, edge_type, field_path)`。
///
/// M59-B1：`graph_redb` 用这个四元组去重（`seen_edges`），`MemoryGraphStore`
/// 原先**完全不去重**，同一条边写两次就出现两次。`field_path` 参与键是刻意的：
/// 同一对端点上不同字段产生的引用是**不同的事实**，不能合并。
pub fn edge_dedup_key(edge: &Edge) -> (String, String, crate::graph::EdgeType, Option<String>) {
    (
        edge.from.clone(),
        edge.to.clone(),
        edge.edge_type.clone(),
        edge.field_path.clone(),
    )
}
