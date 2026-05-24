//! redb 持久化提供者实现
//!
//! M40.8：复用现有 GraphDB 的 redb 数据库，不新建第二套数据库。
//! 图快照从现有 nodes/edges 映射，新增 document_cache、graph_meta 两个表。
//! file_states 继续复用 GraphDB 的 IndexStateStore 表。

use crate::graph::FileState;
use crate::graph_redb::{GraphDB, acquire_graph_db_lock};
use crate::persistence::GraphPersistenceProvider;
use crate::persistence::types::{
    CachedDocument, GraphMeta, GraphSnapshot, PERSISTENCE_SCHEMA_VERSION, PersistenceError,
    validate_schema_version,
};
use redb::{Database, ReadableDatabase, TableDefinition};
use std::path::Path;

const DOCUMENT_CACHE_TABLE: TableDefinition<(&str, &str), Vec<u8>> =
    TableDefinition::new("persistence_document_cache");
const GRAPH_META_TABLE: TableDefinition<(&str, &str), Vec<u8>> =
    TableDefinition::new("persistence_graph_meta");
const FILE_STATES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("file_states");

/// redb 持久化提供者
///
/// 复用现有 GraphDB 的数据库文件，只为非图结构数据新增独立持久化表。
/// 不破坏现有 nodes/edges/file_states/meta 表结构。
pub struct RedbPersistenceProvider {
    db_path: String,
}

impl RedbPersistenceProvider {
    /// 创建新的 redb 持久化提供者
    ///
    /// db_path 是现有 GraphDB 的数据库文件路径。
    pub fn new(db_path: &Path) -> anyhow::Result<Self> {
        let provider = Self {
            db_path: db_path.to_string_lossy().to_string(),
        };
        provider.with_db(|db| {
            let write_txn = db.begin_write().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            {
                let _ = write_txn.open_table(DOCUMENT_CACHE_TABLE).map_err(|e| {
                    PersistenceError::Io {
                        reason: format!("{}", e),
                    }
                })?;
                let _ =
                    write_txn
                        .open_table(GRAPH_META_TABLE)
                        .map_err(|e| PersistenceError::Io {
                            reason: format!("{}", e),
                        })?;
            }
            write_txn.commit().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })
        })?;

        Ok(provider)
    }

    fn with_db<F, T>(&self, f: F) -> Result<T, PersistenceError>
    where
        F: FnOnce(Database) -> Result<T, PersistenceError>,
    {
        let db_path = Path::new(&self.db_path);
        let _lock = acquire_graph_db_lock(db_path).map_err(|e| PersistenceError::Io {
            reason: format!("{}", e),
        })?;
        let db = Database::create(db_path).map_err(|e| PersistenceError::Io {
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
        let graph_db =
            GraphDB::open(Path::new(&self.db_path)).map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
        let nodes = graph_db.graph.node_weights().cloned().collect::<Vec<_>>();
        let edges = graph_db
            .graph
            .edge_references()
            .map(|edge_ref| edge_ref.weight().clone())
            .collect::<Vec<_>>();
        if nodes.is_empty() && edges.is_empty() {
            return Err(PersistenceError::NotFound {
                resource: format!("graph snapshot: {}/{}", project_ref, graph_ref),
            });
        }
        Ok(GraphSnapshot {
            schema_version: PERSISTENCE_SCHEMA_VERSION.to_string(),
            created_at: 0,
            nodes,
            edges,
        })
    }

    fn save_graph_snapshot(
        &mut self,
        _project_ref: &str,
        _graph_ref: &str,
        _snapshot: &GraphSnapshot,
    ) -> Result<(), Self::Error> {
        Err(PersistenceError::Unsupported {
            message:
                "redb graph snapshot save is unsupported; write through GraphDB nodes/edges instead"
                    .to_string(),
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
        self.with_db(|db| {
            let read_txn = db.begin_read().map_err(|e| PersistenceError::Io {
                reason: format!("{}", e),
            })?;
            let table =
                read_txn
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
        })
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
