use crate::graph::FileState;
use crate::persistence::GraphPersistenceProvider;
use crate::persistence::types::{
    CachedDocument, GraphMeta, GraphSnapshot, PersistenceError, validate_schema_version,
};
use std::collections::HashMap;

/// 内存持久化提供者
///
/// 使用 HashMap 存储所有数据，用于测试、harness 和 browser-wasm 回退。
/// 线程安全由调用方保证（单线程场景使用 RefCell，多线程场景使用 Mutex）。
#[derive(Debug, Clone)]
pub struct MemoryPersistenceProvider {
    graph_snapshots: HashMap<(String, String), GraphSnapshot>,
    document_cache: HashMap<(String, String), CachedDocument>,
    file_states: HashMap<(String, String), FileState>,
    graph_meta: HashMap<(String, String), GraphMeta>,
}

impl MemoryPersistenceProvider {
    /// 创建新的内存持久化提供者
    pub fn new() -> Self {
        Self {
            graph_snapshots: HashMap::new(),
            document_cache: HashMap::new(),
            file_states: HashMap::new(),
            graph_meta: HashMap::new(),
        }
    }
}

impl Default for MemoryPersistenceProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphPersistenceProvider for MemoryPersistenceProvider {
    type Error = PersistenceError;

    fn load_graph_snapshot(
        &self,
        project_ref: &str,
        graph_ref: &str,
    ) -> Result<GraphSnapshot, Self::Error> {
        let key = (project_ref.to_string(), graph_ref.to_string());
        let snapshot =
            self.graph_snapshots
                .get(&key)
                .cloned()
                .ok_or_else(|| PersistenceError::NotFound {
                    resource: format!("graph snapshot: {}/{}", project_ref, graph_ref),
                })?;
        validate_schema_version(&snapshot.schema_version)?;
        Ok(snapshot)
    }

    fn save_graph_snapshot(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        snapshot: &GraphSnapshot,
    ) -> Result<(), Self::Error> {
        validate_schema_version(&snapshot.schema_version)?;
        let key = (project_ref.to_string(), graph_ref.to_string());
        self.graph_snapshots.insert(key, snapshot.clone());
        Ok(())
    }

    fn load_document_cache(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<CachedDocument, Self::Error> {
        let key = (project_ref.to_string(), source_path.to_string());
        self.document_cache
            .get(&key)
            .cloned()
            .ok_or_else(|| PersistenceError::NotFound {
                resource: format!("document cache: {}/{}", project_ref, source_path),
            })
    }

    fn save_document_cache(
        &mut self,
        project_ref: &str,
        source_path: &str,
        document: &CachedDocument,
    ) -> Result<(), Self::Error> {
        let key = (project_ref.to_string(), source_path.to_string());
        if document.sensitive {
            self.document_cache.remove(&key);
            return Ok(());
        }
        self.document_cache.insert(key, document.clone());
        Ok(())
    }

    fn load_file_state(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<FileState, Self::Error> {
        let key = (project_ref.to_string(), source_path.to_string());
        self.file_states
            .get(&key)
            .cloned()
            .ok_or_else(|| PersistenceError::NotFound {
                resource: format!("file state: {}/{}", project_ref, source_path),
            })
    }

    fn save_file_state(
        &mut self,
        project_ref: &str,
        source_path: &str,
        state: &FileState,
    ) -> Result<(), Self::Error> {
        let key = (project_ref.to_string(), source_path.to_string());
        self.file_states.insert(key, state.clone());
        Ok(())
    }

    fn load_graph_meta(
        &self,
        project_ref: &str,
        graph_ref: &str,
    ) -> Result<GraphMeta, Self::Error> {
        let key = (project_ref.to_string(), graph_ref.to_string());
        let meta =
            self.graph_meta
                .get(&key)
                .cloned()
                .ok_or_else(|| PersistenceError::NotFound {
                    resource: format!("graph meta: {}/{}", project_ref, graph_ref),
                })?;
        validate_schema_version(&meta.schema_version)?;
        Ok(meta)
    }

    fn save_graph_meta(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        meta: &GraphMeta,
    ) -> Result<(), Self::Error> {
        validate_schema_version(&meta.schema_version)?;
        let key = (project_ref.to_string(), graph_ref.to_string());
        self.graph_meta.insert(key, meta.clone());
        Ok(())
    }
}
