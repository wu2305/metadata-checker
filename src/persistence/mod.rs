#[cfg(feature = "browser-wasm")]
pub mod indexeddb;
pub mod memory;
#[cfg(feature = "cli-local")]
pub mod redb;
pub mod types;

use crate::graph::FileState;
use crate::persistence::types::{CachedDocument, GraphMeta, GraphSnapshot, PersistenceError};

/// 图持久化提供者 trait
///
/// 抽象 memory / redb / IndexedDB 三种存储后端。
/// 查询层保持同步，async 只隔离在 adapter 层。
pub trait GraphPersistenceProvider {
    /// 错误类型
    type Error: std::fmt::Display + std::fmt::Debug;

    // === Graph Snapshot ===

    /// 加载图快照
    fn load_graph_snapshot(
        &self,
        project_ref: &str,
        graph_ref: &str,
    ) -> Result<GraphSnapshot, Self::Error>;

    /// 保存图快照
    fn save_graph_snapshot(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        snapshot: &GraphSnapshot,
    ) -> Result<(), Self::Error>;

    // === Document Cache ===

    /// 加载文档缓存
    fn load_document_cache(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<CachedDocument, Self::Error>;

    /// 保存文档缓存
    fn save_document_cache(
        &mut self,
        project_ref: &str,
        source_path: &str,
        document: &CachedDocument,
    ) -> Result<(), Self::Error>;

    // === File State ===

    /// 加载文件状态
    fn load_file_state(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<FileState, Self::Error>;

    /// 保存文件状态
    fn save_file_state(
        &mut self,
        project_ref: &str,
        source_path: &str,
        state: &FileState,
    ) -> Result<(), Self::Error>;

    // === Graph Meta ===

    /// 加载图元数据
    fn load_graph_meta(&self, project_ref: &str, graph_ref: &str)
    -> Result<GraphMeta, Self::Error>;

    /// 保存图元数据
    fn save_graph_meta(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        meta: &GraphMeta,
    ) -> Result<(), Self::Error>;
}

/// 将 PersistenceError 转换为 GraphStoreError（用于与现有错误体系兼容）
impl From<PersistenceError> for crate::graph_store::GraphStoreError {
    fn from(err: PersistenceError) -> Self {
        match err {
            PersistenceError::NotFound { resource } => {
                crate::graph_store::GraphStoreError::NotFound { resource }
            }
            PersistenceError::InvalidArgument { message } => {
                crate::graph_store::GraphStoreError::InvalidArgument { message }
            }
            PersistenceError::SerializeFailed { reason } => {
                crate::graph_store::GraphStoreError::SerializeFailed { reason }
            }
            PersistenceError::DeserializeFailed { reason } => {
                crate::graph_store::GraphStoreError::DeserializeFailed { reason }
            }
            PersistenceError::Io { reason } => {
                crate::graph_store::GraphStoreError::ReadFailed { reason }
            }
            other => crate::graph_store::GraphStoreError::ReadFailed {
                reason: other.to_string(),
            },
        }
    }
}
