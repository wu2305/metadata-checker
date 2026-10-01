//! Session manifest schema
//!
//! Manifest 只记录远程文件索引和本地 graphdb 位置，不记录 token、cookie 或密码。

use serde::{Deserialize, Serialize};

/// session manifest schema 版本
pub const SESSION_MANIFEST_SCHEMA_VERSION: u32 = 1;

/// 远程 session 中的单个元数据文件
///
/// # 两个状态的边界（M59-2 B）
///
/// `revision` / `hash` 只描述**镜像已获取**到哪个远端版本，不描述**图已成功
/// 索引**到哪个版本。解析失败的文件（坏 TBL）内容已经落进镜像并被记下
/// revision，但从未进入候选图；此时若只用 revision 判断「无需重投」，该事件
/// 就被永久消费，重试只能等未来某个新事件偶然触发。
///
/// `indexed_hash` 区分这两件事：它只在文件**成功解析并入图**后才更新。
/// `[`RemoteSessionFile::needs_index_retry`]` 是判断是否重投的权威入口。
/// 旧 manifest 没有这个字段，`#[serde(default)]` 让它反序列化为 `None`——
/// 首次升级判定为需重投：下一轮 bootstrap 把已拉取的活动文件全量重投一次
/// （宁多重投一轮，不静默丢失败文件），prepare 后由调用方推进
/// `indexed_hash`，一轮内收敛，之后不再重投。已有 checkpoint 的升级不
/// 触发额外重投，失败文件仍由既有水位重投机制覆盖。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSessionFile {
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
    pub etag: Option<String>,
    pub mtime: Option<u64>,
    pub size: Option<u64>,
    /// 镜像中当前内容的 hash（**已获取**，不代表已成功入图）。
    pub hash: Option<String>,
    /// 最近一次**成功解析并入图**的内容 hash。
    ///
    /// `None` 且 `hash` 为 `Some` ⇒ 内容已拉取但从未成功入图（解析失败或
    /// 旧 manifest 升级），必须重投重试。
    #[serde(default)]
    pub indexed_hash: Option<String>,
    pub deleted: bool,
}

impl RemoteSessionFile {
    /// 该文件是否需要重新投递重试（镜像已获取但图未成功索引到同一内容）。
    ///
    /// 删除墓碑不参与重试——它已不在远端，重投没有意义。
    pub fn needs_index_retry(&self) -> bool {
        if self.deleted {
            return false;
        }
        match (&self.hash, &self.indexed_hash) {
            // 从未成功入图：内容已拉取 ⇒ 必须重投。
            (Some(_), None) => true,
            // 镜像内容与最近一次成功入图的内容不一致 ⇒ 尚未消化，必须重投。
            (Some(mirror), Some(indexed)) => mirror != indexed,
            // 没有内容可比对（缺 hash）⇒ 无从判断脏，不重投。
            (None, _) => false,
        }
    }
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
