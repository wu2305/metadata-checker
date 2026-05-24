//! Session 目录管理
//!
//! 负责创建、读取和列出 session manifest；远程同步和 CLI 命令在后续任务中接入。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use super::manifest::SessionManifest;

const MANIFEST_FILE_NAME: &str = "session.json";

/// 本地 session 管理器
#[derive(Debug, Clone)]
pub struct SessionManager {
    root: PathBuf,
}

impl SessionManager {
    /// 创建 session 管理器。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// session 根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 计算 session 目录。
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.root.join(session_id)
    }

    /// 计算 manifest 路径。
    pub fn manifest_path(&self, session_id: &str) -> PathBuf {
        self.session_dir(session_id).join(MANIFEST_FILE_NAME)
    }

    /// 创建空 session manifest 并写入磁盘。
    pub fn create_session(
        &self,
        session_id: &str,
        remote_server: &str,
        project_ref: &str,
        project_name: &str,
        source_origin: &str,
    ) -> Result<SessionManifest> {
        validate_session_id(session_id)?;
        let session_dir = self.session_dir(session_id);
        fs::create_dir_all(&session_dir)
            .with_context(|| format!("failed to create session dir {}", session_dir.display()))?;
        let graph_db_path = session_dir.join("graph.redb").to_string_lossy().to_string();
        let manifest = SessionManifest::new(
            session_id,
            remote_server,
            project_ref,
            project_name,
            source_origin,
            graph_db_path,
            now_millis(),
        );
        self.write_manifest(&manifest)?;
        Ok(manifest)
    }

    /// 写入 manifest。
    pub fn write_manifest(&self, manifest: &SessionManifest) -> Result<()> {
        validate_session_id(&manifest.session_id)?;
        let path = self.manifest_path(&manifest.session_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create manifest dir {}", parent.display()))?;
        }
        let json = serde_json::to_vec_pretty(manifest)?;
        fs::write(&path, json)
            .with_context(|| format!("failed to write session manifest {}", path.display()))?;
        Ok(())
    }

    /// 读取 manifest。
    pub fn read_manifest(&self, session_id: &str) -> Result<SessionManifest> {
        validate_session_id(session_id)?;
        let path = self.manifest_path(session_id);
        let bytes = fs::read(&path)
            .with_context(|| format!("failed to read session manifest {}", path.display()))?;
        let manifest = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse session manifest {}", path.display()))?;
        Ok(manifest)
    }

    /// 列出已有 session id。
    pub fn list_sessions(&self) -> Result<Vec<String>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)
            .with_context(|| format!("failed to read sessions root {}", self.root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let id = entry.file_name().to_string_lossy().to_string();
            if self.manifest_path(&id).exists() {
                sessions.push(id);
            }
        }
        sessions.sort();
        Ok(sessions)
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || session_id.contains('/')
        || session_id.contains('\\')
        || session_id == "."
        || session_id == ".."
    {
        bail!("invalid session_id: {}", session_id);
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "metadata-checker-session-test-{}-{}",
            name,
            now_millis()
        ))
    }

    #[test]
    fn session_manager_creates_reads_and_lists_manifest() {
        let root = test_root("basic");
        let manager = SessionManager::new(&root);
        let created = manager
            .create_session(
                "session-1",
                "https://autocrm-test.xiaoshouyi.com",
                "analyzer",
                "analyzer",
                "remote",
            )
            .unwrap();

        assert_eq!(created.session_id, "session-1");
        assert!(manager.manifest_path("session-1").exists());
        let read = manager.read_manifest("session-1").unwrap();
        assert_eq!(read.project_name, "analyzer");
        assert_eq!(manager.list_sessions().unwrap(), vec!["session-1"]);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn session_manager_rejects_path_escape_session_id() {
        let manager = SessionManager::new(test_root("reject"));
        let err = manager
            .create_session("../bad", "https://host", "p", "p", "remote")
            .unwrap_err();
        assert!(err.to_string().contains("invalid session_id"));
    }
}
