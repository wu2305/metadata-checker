//! redb 持久化提供者实现
//!
//! M40.8：复用现有 GraphDB 的 redb 数据库，不新建第二套数据库。
//! 新增 graph_snapshots、document_cache、graph_meta 三个表。
//! file_states 委托给 GraphDB 的 IndexStateStore 能力。

use crate::graph::FileState;
use crate::persistence::GraphPersistenceProvider;
use crate::persistence::types::{
    CachedDocument, GraphMeta, GraphSnapshot, PersistenceError, validate_schema_version,
};
use redb::{Database, ReadableDatabase, TableDefinition};
use std::path::Path;

const GRAPH_SNAPSHOTS_TABLE: TableDefinition<(&str, &str), Vec<u8>> =
    TableDefinition::new("persistence_graph_snapshots");
const DOCUMENT_CACHE_TABLE: TableDefinition<(&str, &str), Vec<u8>> =
    TableDefinition::new("persistence_document_cache");
const GRAPH_META_TABLE: TableDefinition<(&str, &str), Vec<u8>> =
    TableDefinition::new("persistence_graph_meta");
const FILE_STATES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("file_states");

/// redb 持久化提供者
///
/// 复用现有 GraphDB 的数据库文件，新增独立的持久化表。
/// 不破坏现有 nodes/edges/file_states/meta 表结构。
pub struct RedbPersistenceProvider {
    db_path: String,
}

impl RedbPersistenceProvider {
    /// 创建新的 redb 持久化提供者
    ///
    /// db_path 是现有 GraphDB 的数据库文件路径。
    pub fn new(db_path: &Path) -> anyhow::Result<Self> {
        // 确保数据库文件存在并初始化表
        let db = Database::create(db_path).map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        {
            let _ =
                write_txn
                    .open_table(GRAPH_SNAPSHOTS_TABLE)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            let _ =
                write_txn
                    .open_table(DOCUMENT_CACHE_TABLE)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            let _ = write_txn
                .open_table(GRAPH_META_TABLE)
                .map_err(|e| PersistenceError::Io {
                    reason: format!("{}", e),
                })?;
        }
        write_txn.commit().map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;

        Ok(Self {
            db_path: db_path.to_string_lossy().to_string(),
        })
    }

    fn with_db<F, T>(&self, f: F) -> Result<T, PersistenceError>
    where
        F: FnOnce(Database) -> Result<T, PersistenceError>,
    {
        let db = Database::create(&self.db_path).map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        f(db)
    }
}

impl GraphPersistenceProvider for RedbPersistenceProvider {
    type Error = PersistenceError;

    fn load_graph_snapshot(
        &self,
        project_ref: &str,
        graph_ref: &str,
    ) -> Result<GraphSnapshot, Self::Error> {
        self.with_db(|db| {
            let read_txn = db.begin_read().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            let table =
                read_txn
                    .open_table(GRAPH_SNAPSHOTS_TABLE)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            let value = table
                .get((project_ref, graph_ref))
                .map_err(|e| PersistenceError::Io {
                    reason: format!("{}", e),
                })?;
            match value {
                Some(v) => {
                    let bytes = v.value();
                    let snapshot: GraphSnapshot = serde_json::from_slice(bytes.as_slice())
                        .map_err(|e| PersistenceError::DeserializeFailed {
                            reason: format!("graph snapshot: {}", e),
                        })?;
                    validate_schema_version(&snapshot.schema_version)?;
                    Ok(snapshot)
                }
                None => Err(PersistenceError::NotFound {
                    resource: format!("graph snapshot: {}/{}", project_ref, graph_ref),
                }),
            }
        })
    }

    fn save_graph_snapshot(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        snapshot: &GraphSnapshot,
    ) -> Result<(), Self::Error> {
        validate_schema_version(&snapshot.schema_version)?;
        self.with_db(|db| {
            let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            {
                let mut table = write_txn.open_table(GRAPH_SNAPSHOTS_TABLE).map_err(|e| {
                    PersistenceError::Io {
                        reason: format!("{}", e),
                    }
                })?;
                let bytes = serde_json::to_vec(snapshot).map_err(|e| {
                    PersistenceError::SerializeFailed {
                        reason: format!("graph snapshot: {}", e),
                    }
                })?;
                table.insert((project_ref, graph_ref), bytes).map_err(|e| {
                    PersistenceError::Io {
                        reason: format!("{}", e),
                    }
                })?;
            }
            write_txn.commit().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })
        })
    }

    fn load_document_cache(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<CachedDocument, Self::Error> {
        self.with_db(|db| {
            let read_txn = db.begin_read().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            let table =
                read_txn
                    .open_table(DOCUMENT_CACHE_TABLE)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            let value =
                table
                    .get((project_ref, source_path))
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            match value {
                Some(v) => {
                    let bytes = v.value();
                    serde_json::from_slice(bytes.as_slice()).map_err(|e| {
                        PersistenceError::DeserializeFailed {
                            reason: format!("document cache: {}", e),
                        }
                    })
                }
                None => Err(PersistenceError::NotFound {
                    resource: format!("document cache: {}/{}", project_ref, source_path),
                }),
            }
        })
    }

    fn save_document_cache(
        &mut self,
        project_ref: &str,
        source_path: &str,
        document: &CachedDocument,
    ) -> Result<(), Self::Error> {
        self.with_db(|db| {
            let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            {
                let mut table = write_txn.open_table(DOCUMENT_CACHE_TABLE).map_err(|e| {
                    PersistenceError::Io {
                        reason: format!("{}", e),
                    }
                })?;
                if document.sensitive {
                    table
                        .remove((project_ref, source_path))
                        .map_err(|e| PersistenceError::Io {
                            reason: format!("{}", e),
                        })?;
                } else {
                    let bytes = serde_json::to_vec(document).map_err(|e| {
                        PersistenceError::SerializeFailed {
                            reason: format!("document cache: {}", e),
                        }
                    })?;
                    table
                        .insert((project_ref, source_path), bytes)
                        .map_err(|e| PersistenceError::Io {
                            reason: format!("{}", e),
                        })?;
                }
            }
            write_txn.commit().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })
        })
    }

    fn load_file_state(
        &self,
        project_ref: &str,
        source_path: &str,
    ) -> Result<FileState, Self::Error> {
        // 委托给 GraphDB 的 file_states 表
        let db = Database::create(&self.db_path).map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        let read_txn = db.begin_read().map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        let table = read_txn
            .open_table(FILE_STATES_TABLE)
            .map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
        let value = table.get(source_path).map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        match value {
            Some(v) => {
                let bytes = v.value();
                serde_json::from_slice(bytes.as_slice()).map_err(|e| {
                    PersistenceError::DeserializeFailed {
                        reason: format!("file state: {}", e),
                    }
                })
            }
            None => Err(PersistenceError::NotFound {
                resource: format!("file state: {}/{}", project_ref, source_path),
            }),
        }
    }

    fn save_file_state(
        &mut self,
        _project_ref: &str,
        source_path: &str,
        state: &FileState,
    ) -> Result<(), Self::Error> {
        self.with_db(|db| {
            let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            {
                let mut table =
                    write_txn
                        .open_table(FILE_STATES_TABLE)
                        .map_err(|e| PersistenceError::Io {
                            reason: format!("{}", e),
                        })?;
                let bytes =
                    serde_json::to_vec(state).map_err(|e| PersistenceError::SerializeFailed {
                        reason: format!("file state: {}", e),
                    })?;
                table
                    .insert(source_path, bytes)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            }
            write_txn.commit().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })
        })
    }

    fn load_graph_meta(
        &self,
        project_ref: &str,
        graph_ref: &str,
    ) -> Result<GraphMeta, Self::Error> {
        self.with_db(|db| {
            let read_txn = db.begin_read().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            let table =
                read_txn
                    .open_table(GRAPH_META_TABLE)
                    .map_err(|e| PersistenceError::Io {
                        reason: format!("{}", e),
                    })?;
            let value = table
                .get((project_ref, graph_ref))
                .map_err(|e| PersistenceError::Io {
                    reason: format!("{}", e),
                })?;
            match value {
                Some(v) => {
                    let bytes = v.value();
                    let meta: GraphMeta =
                        serde_json::from_slice(bytes.as_slice()).map_err(|e| {
                            PersistenceError::DeserializeFailed {
                                reason: format!("graph meta: {}", e),
                            }
                        })?;
                    validate_schema_version(&meta.schema_version)?;
                    Ok(meta)
                }
                None => Err(PersistenceError::NotFound {
                    resource: format!("graph meta: {}/{}", project_ref, graph_ref),
                }),
            }
        })
    }

    fn save_graph_meta(
        &mut self,
        project_ref: &str,
        graph_ref: &str,
        meta: &GraphMeta,
    ) -> Result<(), Self::Error> {
        validate_schema_version(&meta.schema_version)?;
        self.with_db(|db| {
            let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            {
                let mut table =
                    write_txn
                        .open_table(GRAPH_META_TABLE)
                        .map_err(|e| PersistenceError::Io {
                            reason: format!("{}", e),
                        })?;
                let bytes =
                    serde_json::to_vec(meta).map_err(|e| PersistenceError::SerializeFailed {
                        reason: format!("graph meta: {}", e),
                    })?;
                table.insert((project_ref, graph_ref), bytes).map_err(|e| {
                    PersistenceError::Io {
                        reason: format!("{}", e),
                    }
                })?;
            }
            write_txn.commit().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })
        })
    }
}
