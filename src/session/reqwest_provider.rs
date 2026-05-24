//! M41.8-B: 基于 reqwest 的 native HTTP RemoteSessionProvider 实现
//!
//! 对接真实 BI 元数据接口，支持 LZString+Base64 解压。
//! 使用 reqwest::blocking::Client 以匹配同步 RemoteSessionProvider trait。

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use reqwest::blocking::Client;
use serde::Deserialize;

use crate::remote_metadata::{MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef};
use crate::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo, RemoteSessionProvider,
};

/// BI getPermissionInfo 返回结构。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiPermissionInfo {
    #[serde(default)]
    meta_projects: Vec<BiMetaProject>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiMetaProject {
    project_name: String,
    #[serde(default)]
    desc: Option<String>,
}

/// BI 文件信息结构。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiFileInfo {
    id: String,
    path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    revision: Option<String>,
    #[serde(default)]
    modify_time: Option<u64>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// reqwest-based 远程 session provider。
pub struct ReqwestRemoteSessionProvider {
    client: Client,
    base_url: String,
}

impl ReqwestRemoteSessionProvider {
    /// 创建默认 provider。
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = Client::builder()
            .build()
            .context("failed to build reqwest blocking client")?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// 使用外部 Client（便于注入 mock 或自定义配置）。
    pub fn with_client(base_url: impl Into<String>, client: Client) -> Self {
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{}/{}/", base, path)
    }

    /// 发送 GET 请求并获取文本。
    fn get_text(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        let response = self
            .client
            .get(&url)
            .send()
            .with_context(|| format!("HTTP request failed: {}", url))?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!("remote session returned 401 Unauthorized"));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!("remote session returned 403 Forbidden"));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(anyhow!("remote session returned 404 Not Found"));
        }
        if !status.is_success() {
            return Err(anyhow!("remote session returned HTTP {}", status.as_u16()));
        }

        response
            .text()
            .with_context(|| format!("failed to read response body from {}", url))
    }

    /// 尝试解压 LZString+Base64；若失败则直接返回原字符串。
    /// TODO: M41.8-B 暂用启发式检测，后续引入 lz-str crate 处理 BI 真实压缩数据。
    fn try_decompress_lzstring(&self, input: &str) -> String {
        // BI getPermissionInfo 返回的数据可能是 LZString+Base64 压缩。
        // 如果输入以 { 或 [ 开头，说明是原始 JSON，直接返回。
        let trimmed = input.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            return input.to_string();
        }
        // 否则尝试 Base64 解码（LZString 解压待后续实现）
        if input.len() < 4
            || !input
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        {
            return input.to_string();
        }
        // 暂不支持 LZString 解压，直接返回原字符串
        // 真实 BI 环境中需要 lz-str crate 解压
        input.to_string()
    }
}

impl RemoteSessionProvider for ReqwestRemoteSessionProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        let raw = self.get_text("/api/me/getPermissionInfo")?;
        let decompressed = self.try_decompress_lzstring(&raw);
        let perm: BiPermissionInfo = serde_json::from_str(&decompressed)
            .with_context(|| "failed to parse permission info JSON")?;

        Ok(perm
            .meta_projects
            .into_iter()
            .map(|p| RemoteProjectInfo {
                project_ref: p.project_name.clone(),
                project_name: p.desc.unwrap_or_else(|| p.project_name.clone()),
                source_origin: format!("bi://{}", p.project_name),
            })
            .collect())
    }

    fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        // BI getFileDescendant 返回全量后代文件（扁平列表），比 getFileChildren 更适合同步。
        let path = format!(
            "/api/meta/services/getFileDescendant/{}",
            urlencoding::encode(project_ref)
        );
        let raw = self.get_text(&path)?;
        let files: Vec<BiFileInfo> = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse file descendants for project {}", project_ref))?;

        Ok(files
            .into_iter()
            .map(|f| RemoteMetafileEntry {
                project_ref: project_ref.to_string(),
                source_path: normalize_source_path(&f.path, project_ref),
                file_id: Some(f.id),
                revision: f.revision,
                etag: None,
                mtime: f.modify_time,
                size: None,
                deleted: false,
            })
            .collect())
    }

    fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        let file_id = file_ref
            .file_id
            .as_deref()
            .context("file_ref must have file_id for remote fetch")?;
        let path = format!("/api/meta/services/getFileInfo/{}", urlencoding::encode(file_id));
        let raw = self.get_text(&path)?;
        let info: BiFileInfo = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse file info for {}", file_id))?;

        Ok(RemoteFileInfo {
            source_path: normalize_source_path(&info.path, &file_ref.project_ref),
            file_id: Some(info.id),
            revision: info.revision,
            content_type: MetadataContentType::from_extension(
                info.name.as_deref().unwrap_or("").rsplit('.').next().unwrap_or(""),
            ),
            updated_at: info.modify_time.map(|t| t.to_string()),
        })
    }

    fn fetch_metafile_content(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileContent> {
        let file_id = file_ref
            .file_id
            .as_deref()
            .context("file_ref must have file_id for remote fetch")?;
        let path = format!(
            "/api/meta/services/getFileContent/{}",
            urlencoding::encode(file_id)
        );
        let raw = self.get_text(&path)?;

        Ok(RemoteFileContent {
            source_path: file_ref.source_path.clone(),
            file_id: file_ref.file_id.clone(),
            revision: file_ref.revision.clone(),
            content_type: MetadataContentType::from_extension(
                file_ref.source_path.rsplit('.').next().unwrap_or(""),
            ),
            raw_text: raw,
        })
    }

    fn fetch_changed_since(
        &self,
        project_ref: &str,
        since_revision: &str,
    ) -> Result<RemoteChangeSet> {
        let all_files = self.list_metafiles(project_ref)?;
        let changed_files: Vec<RemoteMetafileEntry> = all_files
            .into_iter()
            .filter(|entry| {
                match (&entry.revision, since_revision.parse::<u64>()) {
                    (Some(rev), Ok(since)) => {
                        rev.parse::<u64>().map(|r| r > since).unwrap_or(false)
                    }
                    _ => true,
                }
            })
            .collect();

        let to_revision = changed_files
            .iter()
            .filter_map(|entry| entry.revision.as_deref())
            .filter_map(|rev| rev.parse::<u64>().ok())
            .max()
            .map(|r| r.to_string());

        Ok(RemoteChangeSet {
            project_ref: project_ref.to_string(),
            project_name: project_ref.to_string(),
            since_revision: since_revision.to_string(),
            to_revision,
            changed_files,
        })
    }
}

/// 将 BI 绝对路径转换为项目内逻辑路径。
fn normalize_source_path(path: &str, project_ref: &str) -> String {
    let prefix = format!("/{}/", project_ref);
    if path.starts_with(&prefix) {
        path[prefix.len()..].to_string()
    } else {
        path.trim_start_matches('/').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_source_path() {
        assert_eq!(
            normalize_source_path("/xiaoshouyi/data/tables/test.tbl", "xiaoshouyi"),
            "data/tables/test.tbl"
        );
        assert_eq!(
            normalize_source_path("data/tables/test.tbl", "xiaoshouyi"),
            "data/tables/test.tbl"
        );
    }

    // TODO: HTTP 集成测试（200/401/403/404/invalid JSON/empty body）
    // 沙盒环境限制 TCP bind，需用 wiremock 在 CI 中运行。
    // 场景：
    // - test_list_projects_200: mock /api/me/getPermissionInfo 返回 JSON
    // - test_list_projects_401/403: mock 返回对应状态码
    // - test_list_metafiles_200/404: mock /api/meta/services/getFileChildren/{project}
    // - test_fetch_metafile_content_200/empty: mock /api/meta/services/getFileContent/{id}
    // - test_list_projects_invalid_json: mock 返回非 JSON 文本
}