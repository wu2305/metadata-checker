use crate::graph::{Edge, Node};
use serde::{Deserialize, Serialize};

/// 当前持久化快照模式版本
pub const PERSISTENCE_SCHEMA_VERSION: &str = "m40.8";

/// 持久化错误类型
///
/// 为 GraphPersistenceProvider 提供稳定的错误码体系。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceError {
    /// 资源不存在
    NotFound { resource: String },
    /// 参数非法
    InvalidArgument { message: String },
    /// 序列化失败
    SerializeFailed { reason: String },
    /// 反序列化失败
    DeserializeFailed { reason: String },
    /// 版本不匹配
    VersionMismatch { expected: String, actual: String },
    /// 不支持的操作
    Unsupported { message: String },
    /// IO 错误
    Io { reason: String },
}

impl PersistenceError {
    /// 返回稳定错误码字符串
    pub fn code(&self) -> &'static str {
        match self {
            PersistenceError::NotFound { .. } => "PERSISTENCE_NOT_FOUND",
            PersistenceError::InvalidArgument { .. } => "PERSISTENCE_INVALID_ARGUMENT",
            PersistenceError::SerializeFailed { .. } => "PERSISTENCE_SERIALIZE_FAILED",
            PersistenceError::DeserializeFailed { .. } => "PERSISTENCE_DESERIALIZE_FAILED",
            PersistenceError::VersionMismatch { .. } => "PERSISTENCE_VERSION_MISMATCH",
            PersistenceError::Unsupported { .. } => "PERSISTENCE_UNSUPPORTED",
            PersistenceError::Io { .. } => "PERSISTENCE_IO_ERROR",
        }
    }
}

impl std::fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistenceError::NotFound { resource } => {
                write!(f, "Persistence resource not found: {}", resource)
            }
            PersistenceError::InvalidArgument { message } => {
                write!(f, "Invalid persistence argument: {}", message)
            }
            PersistenceError::SerializeFailed { reason } => {
                write!(f, "Persistence serialize failed: {}", reason)
            }
            PersistenceError::DeserializeFailed { reason } => {
                write!(f, "Persistence deserialize failed: {}", reason)
            }
            PersistenceError::VersionMismatch { expected, actual } => {
                write!(
                    f,
                    "Persistence version mismatch: expected {}, actual {}",
                    expected, actual
                )
            }
            PersistenceError::Unsupported { message } => {
                write!(f, "Persistence operation unsupported: {}", message)
            }
            PersistenceError::Io { reason } => {
                write!(f, "Persistence IO error: {}", reason)
            }
        }
    }
}

impl std::error::Error for PersistenceError {}

/// 校验持久化模式版本
pub fn validate_schema_version(actual: &str) -> Result<(), PersistenceError> {
    if actual == PERSISTENCE_SCHEMA_VERSION {
        return Ok(());
    }
    Err(PersistenceError::VersionMismatch {
        expected: PERSISTENCE_SCHEMA_VERSION.to_string(),
        actual: actual.to_string(),
    })
}

/// 图快照
///
/// 图的完整序列化视图，用于跨平台持久化和恢复。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// 模式版本
    pub schema_version: String,
    /// 创建时间戳（Unix 秒）
    pub created_at: u64,
    /// 节点列表
    pub nodes: Vec<Node>,
    /// 边列表
    pub edges: Vec<Edge>,
}

/// 缓存文档
///
/// 从远程或本地加载的原始文档内容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDocument {
    /// 内容哈希
    pub content_hash: String,
    /// 获取时间戳（Unix 秒）
    pub fetched_at: u64,
    /// 原始内容
    pub content: String,
    /// 是否包含敏感内容；敏感内容不进入持久化缓存
    #[serde(default)]
    pub sensitive: bool,
}

/// 图元数据
///
/// 描述图快照的元信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphMeta {
    /// 模式版本
    pub schema_version: String,
    /// 更新时间戳（Unix 秒）
    pub updated_at: u64,
    /// 项目引用
    pub project_ref: String,
    /// 图引用
    pub graph_ref: String,
}
