//! Session manifest schema
//!
//! Manifest 只记录远程文件索引和本地 graphdb 位置，不记录 token、cookie 或密码。

use serde::{Deserialize, Serialize};

/// session manifest schema 版本
pub const SESSION_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// 远程 session 中的单个元数据文件
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSessionFile {
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
    pub etag: Option<String>,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
    pub hash: Option<String>,
    pub deleted: bool,
}

/// 远程项目 session manifest
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionManifest {
    pub session_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub remote_server: String,
    pub project_ref: String,
    pub project_name: String,
    pub source_origin: String,
    pub files: Vec<RemoteSessionFile>,
    pub graph_db_path: String,
    pub schema_version: u32,
}

impl SessionManifest {
    /// 创建空 manifest。
    pub fn new(
        session_id: impl Into<String>,
        remote_server: impl Into<String>,
        project_ref: impl Into<String>,
        project_name: impl Into<String>,
        source_origin: impl Into<String>,
        graph_db_path: impl Into<String>,
        now_millis: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            created_at: now_millis,
            updated_at: now_millis,
            remote_server: remote_server.into(),
            project_ref: project_ref.into(),
            project_name: project_name.into(),
            source_origin: source_origin.into(),
            files: Vec::new(),
            graph_db_path: graph_db_path.into(),
            schema_version: SESSION_MANIFEST_SCHEMA_VERSION,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_manifest_serializes_without_secret_fields() {
        let manifest = SessionManifest::new(
            "s1",
            "https://autocrm-test.xiaoshouyi.com",
            "analyzer",
            "analyzer",
            "remote",
            "graph.redb",
            100,
        );
        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("\"schema_version\":1"));
        assert!(!json.contains("token"));
        assert!(!json.contains("cookie"));
        assert!(!json.contains("password"));
    }
}
