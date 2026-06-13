//! M41 native 侧远程 session provider contract
//!
//! 该模块定义原生（非浏览器）环境可复用的批量同步接口：
//! 先列项目，再列元数据文件，最后按文件逐个获取 info/content。
//! 所有接口返回 `anyhow::Result`，不直接依赖 HTTP，便于单测和离线验证。

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef, VisibleManifestEntry,
};

/// 远程项目基础信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteProjectInfo {
    /// 项目唯一标识。
    pub project_ref: String,
    /// 项目展示名。
    pub project_name: String,
    /// 项目源信息（如远端空间或应用源）。
    pub source_origin: String,
}

/// 远程元文件条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMetafileEntry {
    /// 所属项目标识。
    pub project_ref: String,
    /// 相对路径（只含项目内路径）。
    pub source_path: String,
    /// 远端文件 ID。
    pub file_id: Option<String>,
    /// 远端版本号。
    pub revision: Option<String>,
    /// 缓存协商标记。
    pub etag: Option<String>,
    /// 修改时间戳（毫秒）。
    pub mtime: Option<u64>,
    /// 文件大小（字节）。
    pub size: Option<u64>,
    /// 删除标记（用于删档同步）。
    pub deleted: bool,
}

impl RemoteMetafileEntry {
    /// 由文件路径推断内容类型。
    pub fn content_type(&self) -> MetadataContentType {
        MetadataContentType::from_extension(self.source_path.rsplit('.').next().unwrap_or(""))
    }

    /// 从可见清单条目生成会话文件条目，用于上游 contract 复用。
    ///
    /// 可见清单中目录项会被过滤掉，这里保留 `deleted=false`，供
    /// 会话侧增量同步流程按需写入。
    pub fn from_visible_manifest_entry(project_ref: &str, entry: &VisibleManifestEntry) -> Self {
        Self {
            project_ref: project_ref.to_string(),
            source_path: entry.source_path.clone(),
            file_id: entry.id.clone(),
            revision: entry.revision.clone(),
            etag: None,
            mtime: entry.modify_time,
            size: None,
            deleted: false,
        }
    }
}

/// 变化集返回结构。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteChangeSet {
    /// 目标项目。
    pub project_ref: String,
    /// 当前项目名。
    pub project_name: String,
    /// 查询起始版本。
    pub since_revision: String,
    /// 返回版本上界（便于保存游标）。
    pub to_revision: Option<String>,
    /// 本次同步到的元文件。
    pub changed_files: Vec<RemoteMetafileEntry>,
}

/// native 远程 session provider 抽象。
pub trait RemoteSessionProvider {
    /// 列出可同步项目。
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>>;
    /// 列出某项目的远程元文件列表。
    fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>>;
    /// 获取某远程元文件的元信息。
    fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo>;
    /// 获取某远程元文件内容。
    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent>;
    /// 获取某项目自上次版本以来的变化。
    fn fetch_changed_since(
        &self,
        project_ref: &str,
        since_revision: &str,
    ) -> Result<RemoteChangeSet>;
}

#[derive(Clone)]
struct ProjectState {
    project_name: String,
    source_origin: String,
    files: BTreeMap<String, RemoteMetafileEntry>,
    contents: BTreeMap<String, RemoteFileContent>,
}

fn revision_to_u64(revision: &str, context: &str) -> Result<u64> {
    revision
        .parse::<u64>()
        .with_context(|| format!("invalid revision '{revision}' for {context}"))
}

fn is_newer(current: &Option<String>, since_revision: &str) -> Result<bool> {
    let current = current.as_deref().context("file revision is empty")?;
    let current = revision_to_u64(current, "file revision")?;
    let since = revision_to_u64(since_revision, "since_revision")?;
    Ok(current > since)
}

/// 纯内存 provider，适配测试和离线验证。
#[derive(Clone)]
pub struct InMemoryRemoteSessionProvider {
    projects: BTreeMap<String, ProjectState>,
}

impl InMemoryRemoteSessionProvider {
    /// 创建空 provider。
    pub fn new() -> Self {
        Self {
            projects: BTreeMap::new(),
        }
    }

    /// 注册一个项目。
    pub fn register_project(&mut self, project: RemoteProjectInfo) -> Result<()> {
        if self.projects.contains_key(&project.project_ref) {
            return Err(anyhow!("project '{}' already exists", project.project_ref));
        }
        self.projects.insert(
            project.project_ref.clone(),
            ProjectState {
                project_name: project.project_name,
                source_origin: project.source_origin,
                files: BTreeMap::new(),
                contents: BTreeMap::new(),
            },
        );
        Ok(())
    }

    /// 添加一个元文件及其内容。
    pub fn add_metafile(
        &mut self,
        entry: RemoteMetafileEntry,
        content: RemoteFileContent,
    ) -> Result<()> {
        if entry.source_path != content.source_path {
            return Err(anyhow!(
                "source_path mismatch: entry '{}' != content '{}'",
                entry.source_path,
                content.source_path
            ));
        }

        let project = self
            .projects
            .get_mut(&entry.project_ref)
            .ok_or_else(|| anyhow!("project '{}' not found", entry.project_ref))?;
        project
            .files
            .insert(entry.source_path.clone(), entry.clone());
        project
            .contents
            .insert(content.source_path.clone(), content);
        Ok(())
    }

    fn file_entry(&self, file_ref: &RemoteFileRef) -> Result<&RemoteMetafileEntry> {
        let project = self
            .projects
            .get(&file_ref.project_ref)
            .with_context(|| format!("project '{}' not found", file_ref.project_ref))?;
        let entry = project
            .files
            .get(&file_ref.source_path)
            .context("metafile not found")?;
        Ok(entry)
    }

    fn fetch_changed(
        project: &ProjectState,
        since_revision: &str,
    ) -> Result<Vec<RemoteMetafileEntry>> {
        let mut changed = Vec::new();
        for entry in project.files.values() {
            if is_newer(&entry.revision, since_revision)? {
                changed.push(entry.clone());
            }
        }
        changed.sort_by(|a, b| a.source_path.cmp(&b.source_path));
        Ok(changed)
    }
}

impl fmt::Debug for InMemoryRemoteSessionProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let project_count = self.projects.len();
        let file_count: usize = self.projects.values().map(|state| state.files.len()).sum();
        f.debug_struct("InMemoryRemoteSessionProvider")
            .field("project_count", &project_count)
            .field("file_count", &file_count)
            .finish()
    }
}

impl RemoteSessionProvider for InMemoryRemoteSessionProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        Ok(self
            .projects
            .iter()
            .map(|(project_ref, state)| RemoteProjectInfo {
                project_ref: project_ref.clone(),
                project_name: state.project_name.clone(),
                source_origin: state.source_origin.clone(),
            })
            .collect())
    }

    fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        let project = self
            .projects
            .get(project_ref)
            .with_context(|| format!("project '{}' not found", project_ref))?;
        Ok(project.files.values().cloned().collect())
    }

    fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        let entry = self.file_entry(file_ref)?;
        Ok(RemoteFileInfo {
            source_path: entry.source_path.clone(),
            file_id: entry.file_id.clone(),
            revision: entry.revision.clone(),
            content_type: entry.content_type(),
            updated_at: entry.mtime.map(|mtime| mtime.to_string()),
        })
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        let _ = self.file_entry(file_ref)?;
        let project = self
            .projects
            .get(&file_ref.project_ref)
            .with_context(|| format!("project '{}' not found", file_ref.project_ref))?;
        project
            .contents
            .get(&file_ref.source_path)
            .cloned()
            .context("metafile content not found")
    }

    fn fetch_changed_since(
        &self,
        project_ref: &str,
        since_revision: &str,
    ) -> Result<RemoteChangeSet> {
        let project = self
            .projects
            .get(project_ref)
            .with_context(|| format!("project '{}' not found", project_ref))?;
        let changed_files = Self::fetch_changed(project, since_revision)?;
        let to_revision = changed_files
            .iter()
            .filter_map(|entry| {
                entry
                    .revision
                    .as_deref()
                    .and_then(|rev| rev.parse::<u64>().ok())
            })
            .max()
            .map(|revision| revision.to_string());
        Ok(RemoteChangeSet {
            project_ref: project_ref.to_string(),
            project_name: project.project_name.clone(),
            since_revision: since_revision.to_string(),
            to_revision,
            changed_files,
        })
    }
}

/// 测试用 provider 别名，语义与 InMemory 保持一致。
#[derive(Clone)]
pub struct TestRemoteSessionProvider {
    inner: InMemoryRemoteSessionProvider,
}

impl TestRemoteSessionProvider {
    /// 创建空测试 provider。
    pub fn new() -> Self {
        Self {
            inner: InMemoryRemoteSessionProvider::new(),
        }
    }

    /// 注册项目，继承内存 provider 能力。
    pub fn register_project(&mut self, project: RemoteProjectInfo) -> Result<()> {
        self.inner.register_project(project)
    }

    /// 写入一条元文件及其内容。
    pub fn add_metafile(
        &mut self,
        entry: RemoteMetafileEntry,
        content: RemoteFileContent,
    ) -> Result<()> {
        self.inner.add_metafile(entry, content)
    }
}

impl fmt::Debug for TestRemoteSessionProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestRemoteSessionProvider")
            .field("inner", &self.inner)
            .finish()
    }
}

impl RemoteSessionProvider for TestRemoteSessionProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        self.inner.list_projects()
    }

    fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        self.inner.list_metafiles(project_ref)
    }

    fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        self.inner.fetch_metafile_info(file_ref)
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        self.inner.fetch_metafile_content(file_ref)
    }

    fn fetch_changed_since(
        &self,
        project_ref: &str,
        since_revision: &str,
    ) -> Result<RemoteChangeSet> {
        self.inner.fetch_changed_since(project_ref, since_revision)
    }
}
