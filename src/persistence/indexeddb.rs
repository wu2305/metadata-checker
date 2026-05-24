use crate::graph::FileState;
use crate::persistence::GraphPersistenceProvider;
use crate::persistence::types::{CachedDocument, GraphMeta, GraphSnapshot, PersistenceError};

/// IndexedDB 持久化提供者（stub）
///
/// M40.8 首轮不引入 rexie，所有方法返回 Unsupported 错误。
/// browser-wasm 场景下默认使用 MemoryPersistenceProvider 作为回退。
#[derive(Debug, Clone)]
pub struct IndexedDbPersistenceProvider;

impl IndexedDbPersistenceProvider {
    /// 创建新的 IndexedDB 持久化提供者 stub
    pub fn new() -> Self {
        Self
    }
}

impl Default for IndexedDbPersistenceProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphPersistenceProvider for IndexedDbPersistenceProvider {
    type Error = PersistenceError;

    fn load_graph_snapshot(
        &self,
        _project_ref: &str,
        _graph_ref: &str,
    ) -> Result<GraphSnapshot, Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn save_graph_snapshot(
        &mut self,
        _project_ref: &str,
        _graph_ref: &str,
        _snapshot: &GraphSnapshot,
    ) -> Result<(), Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn load_document_cache(
        &self,
        _project_ref: &str,
        _source_path: &str,
    ) -> Result<CachedDocument, Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn save_document_cache(
        &mut self,
        _project_ref: &str,
        _source_path: &str,
        _document: &CachedDocument,
    ) -> Result<(), Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn load_file_state(
        &self,
        _project_ref: &str,
        _source_path: &str,
    ) -> Result<FileState, Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn save_file_state(
        &mut self,
        _project_ref: &str,
        _source_path: &str,
        _state: &FileState,
    ) -> Result<(), Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn load_graph_meta(
        &self,
        _project_ref: &str,
        _graph_ref: &str,
    ) -> Result<GraphMeta, Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }

    fn save_graph_meta(
        &mut self,
        _project_ref: &str,
        _graph_ref: &str,
        _meta: &GraphMeta,
    ) -> Result<(), Self::Error> {
        Err(PersistenceError::Unsupported {
            message: "IndexedDB persistence not yet implemented".to_string(),
        })
    }
}
