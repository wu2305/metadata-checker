//! M41.8-B: 基于 reqwest 的 native HTTP RemoteSessionProvider 实现
//!
//! 对接真实 BI 元数据接口，支持 LZString+Base64 解压与常见响应包装。使用
//! reqwest::blocking::Client 以匹配同步 RemoteSessionProvider trait。

use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use base64::Engine;
use lz_str::{decompress_from_base64, decompress_from_encoded_uri_component};
use regex::Regex;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::remote_metadata::{
    MetadataContentType, RemoteFileContent, RemoteFileInfo, RemoteFileRef,
};
use crate::session::auth::{
    AuthContext, AuthSessionErrorCode, AuthenticatedSession, SessionBootstrapper,
};
use crate::session::remote_provider::{
    RemoteChangeSet, RemoteMetafileEntry, RemoteProjectInfo, RemoteSessionProvider,
};

/// 统一脱敏占位符。
const SANITIZED_VALUE: &str = "***";

/// 会话相关错误信息的统一脱敏入口。
pub fn sanitize_session_error_message(message: &str) -> String {
    let mut sanitized = message.to_string();

    let set_cookie_re = Regex::new(r"(?i)(set-cookie)\s*[:=]\s*[^\r\n]+")
        .expect("valid regex for set-cookie redaction");
    sanitized = set_cookie_re
        .replace_all(&sanitized, format!("$1: {SANITIZED_VALUE}"))
        .to_string();

    let sensitive_keys = [
        "password",
        "pass",
        "token",
        "cookie",
        "authorization",
        "cipherpassport",
    ]
    .join("|");
    let key_value_re = Regex::new(&format!(
        r#"(?i)({})(\s*[:=]\s*)("[^"]*"|'[^']*'|`[^`]*`|[^\s,;\)\]]+)"#,
        sensitive_keys
    ))
    .expect("valid regex for sensitive key-value redaction");
    sanitized = key_value_re
        .replace_all(&sanitized, |captures: &regex::Captures| {
            let key = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            let sep = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
            format!("{key}{sep}{SANITIZED_VALUE}")
        })
        .to_string();

    sanitized
}

/// BI 项目信息。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiMetaProject {
    #[serde(default)]
    project_name: Option<String>,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// BI 文件信息结构。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiFileInfo {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    #[serde(alias = "fileId")]
    file_id_alias: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    parent_dir: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_string")]
    revision: Option<String>,
    #[serde(default)]
    is_folder: Option<bool>,
    #[serde(default)]
    modify_time: Option<u64>,
    #[serde(flatten)]
    _extra: HashMap<String, serde_json::Value>,
}

/// `/api/me/whoami` 返回结构。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiWhoami {
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    anonymous: Option<bool>,
}

impl BiFileInfo {
    fn resolved_id(&self, fallback: Option<&str>) -> Option<String> {
        if let Some(id) = &self.id {
            return Some(id.clone());
        }
        if let Some(id) = &self.file_id_alias {
            return Some(id.clone());
        }
        fallback.map(std::string::ToString::to_string)
    }

    fn resolved_path<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.path
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(fallback)
    }

    fn resolve_extension<'a>(&'a self, fallback: &'a str) -> &'a str {
        self.name
            .as_deref()
            .or_else(|| self.path.as_deref())
            .map(|path| path.rsplit('.').next().unwrap_or(fallback))
            .unwrap_or(fallback)
    }

    /// 还原 BI 文件的资源路径，兼容仅返回 parentDir + name 的真实接口。
    fn resource_path(&self) -> Option<String> {
        if let Some(path) = self.path.as_deref().filter(|path| !path.trim().is_empty()) {
            return Some(path.trim().to_string());
        }

        let name = self.name.as_deref()?.trim();
        if name.is_empty() {
            return None;
        }

        let parent = self
            .parent_dir
            .as_deref()
            .filter(|parent| !parent.trim().is_empty())?;
        Some(format!("{}/{}", parent.trim_end_matches('/'), name))
    }
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
            .cookie_store(true)
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

    /// 使用 AuthContext 构造 provider。
    ///
    /// 若 AuthContext 为 CookieJar，会在 Client 中注入对应 cookie。
    pub fn with_auth_context(
        base_url: impl Into<String>,
        auth: &crate::session::auth::AuthContext,
    ) -> Result<Self> {
        let mut builder = Client::builder();

        if let crate::session::auth::AuthContext::CookieJar { jar_ref } = auth {
            if let Ok(jar) = std::fs::read_to_string(jar_ref) {
                let mut headers = reqwest::header::HeaderMap::new();
                for line in jar.lines() {
                    if let Some((name, value)) = line.split_once('=') {
                        headers.insert(
                            reqwest::header::COOKIE,
                            format!("{}={}", name.trim(), value.trim()).parse().unwrap(),
                        );
                    }
                }
                builder = builder.default_headers(headers);
            }
        }

        let client = builder
            .build()
            .context("failed to build reqwest blocking client with auth")?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    /// 使用用户名密码登录。
    ///
    /// 成功后 cookie jar 会自动保存 session，后续请求复用。
    /// 失败返回稳定错误，不泄漏 password。
    pub fn login(&self, username: &str, password: &str, user_directory: &str) -> Result<()> {
        let login_args = serde_json::json!({
            "user": username,
            "password": password,
            "remember": false,
            "userDirectory": user_directory,
        });
        let cipher_passport =
            base64::engine::general_purpose::STANDARD.encode(serde_json::to_string(&login_args)?);
        let body = serde_json::json!({ "cipherPassport": cipher_passport });

        let url = self.url("/api/auth/signin");
        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .with_context(|| "login HTTP request failed")?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(anyhow!("login failed: 401 Unauthorized"));
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(anyhow!("login failed: 403 Forbidden"));
        }
        if !status.is_success() {
            return Err(anyhow!("login failed: HTTP {}", status.as_u16()));
        }

        let text = response
            .text()
            .with_context(|| "failed to read login response")?;
        if text.trim().is_empty() {
            return Err(anyhow!("login failed: response body is empty"));
        }
        let preview = sanitize_session_error_message(&text.chars().take(100).collect::<String>());
        let json: Value = serde_json::from_str(&text)
            .with_context(|| format!("login response is not valid JSON: {preview}"))?;

        if json.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let msg = json
                .get("message")
                .or_else(|| json.get("error")?.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("login failed");
            return Err(anyhow!(
                "login failed: {}",
                sanitize_session_error_message(msg)
            ));
        }

        Ok(())
    }

    fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{}/{}", base, path)
    }

    /// 发送 GET 请求并获取文本。
    fn get_text(&self, path: &str) -> Result<String> {
        let url = self.url(path);
        let safe_url = sanitize_session_error_message(&url);
        let response = self.client.get(&url).send().map_err(|err| {
            anyhow!(
                "HTTP request failed: {}: {}",
                safe_url,
                sanitize_session_error_message(&err.to_string())
            )
        })?;

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

        let text = response.text().map_err(|err| {
            anyhow!(
                "failed to read response body from {}: {}",
                safe_url,
                sanitize_session_error_message(&err.to_string())
            )
        })?;
        if text.trim().is_empty() {
            return Err(anyhow!("remote session response is empty"));
        }

        Ok(text)
    }

    /// 处理 BI 结构化接口的 JSON 或 LZString+Base64 响应。
    fn decode_structured_payload(&self, raw: &str, context: &str) -> Result<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(anyhow!(
                "failed to decode {context}: response body is empty"
            ));
        }

        if looks_like_json_document(trimmed) {
            return Ok(raw.to_string());
        }

        if let Ok(Value::String(inner)) = serde_json::from_str::<Value>(trimmed) {
            if looks_like_json_document(inner.trim()) {
                return Ok(inner);
            }
            return decompress_lzstring_base64(&inner, context);
        }

        decompress_lzstring_base64(trimmed, context)
    }

    /// 处理 BI 文件内容响应：压缩内容会解压，普通文本保持原样。
    fn decode_content_payload(&self, raw: &str) -> Result<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("remote session getFileContent response is empty"));
        }

        if looks_like_json_document(trimmed) {
            return Ok(raw.to_string());
        }

        if let Ok(Value::String(inner)) = serde_json::from_str::<Value>(trimmed) {
            if looks_like_json_document(inner.trim()) {
                return Ok(inner);
            }
            return Ok(decompress_lzstring_base64(&inner, "file content").unwrap_or(inner));
        }

        Ok(decompress_lzstring_base64(trimmed, "file content").unwrap_or_else(|_| raw.to_string()))
    }

    /// 解析 JSON（返回通用 Value，便于兼容通用包装层）。
    fn parse_json(value: &str, context: &str) -> Result<Value> {
        serde_json::from_str(value).with_context(|| format!("failed to parse {} JSON", context))
    }

    /// 通过一次性 access token 请求 whoami，建立当前 client 内的 session。
    pub fn bootstrap_with_access_token(&self, access_token: &str) -> Result<AuthenticatedSession> {
        if access_token.trim().is_empty() {
            return Err(anyhow!(
                "{}: access token is empty",
                AuthSessionErrorCode::AccessTokenUnavailable.as_str()
            ));
        }

        let path = format!(
            "/api/me/whoami?access_token={}",
            percent_encode(access_token.trim())
        );
        let raw = self.get_text(&path).with_context(|| {
            format!(
                "{}: whoami bootstrap request failed",
                AuthSessionErrorCode::SessionBootstrapFailed.as_str()
            )
        })?;
        let json_text = self
            .decode_structured_payload(&raw, "whoami")
            .with_context(|| {
                format!(
                    "{}: failed to decode whoami response",
                    AuthSessionErrorCode::SessionBootstrapFailed.as_str()
                )
            })?;
        let whoami: BiWhoami = serde_json::from_str(&json_text).with_context(|| {
            format!(
                "{}: failed to parse whoami response",
                AuthSessionErrorCode::SessionBootstrapFailed.as_str()
            )
        })?;

        if whoami.anonymous.unwrap_or(false) {
            return Err(anyhow!(
                "{}: whoami returned anonymous user",
                AuthSessionErrorCode::SessionBootstrapAnonymous.as_str()
            ));
        }

        let user_id = whoami.user_id.filter(|value| !value.trim().is_empty());
        let Some(user_id) = user_id else {
            return Err(anyhow!(
                "{}: whoami response missing userId",
                AuthSessionErrorCode::SessionBootstrapFailed.as_str()
            ));
        };

        Ok(AuthenticatedSession {
            user_id,
            user_name: whoami.user_name,
            auth_context: AuthContext::RuntimeSession {
                session_ref: "reqwest-memory-cookie-jar".to_string(),
            },
        })
    }

    /// 提取普通字段，兼容 data/file/result 一级包装。
    fn find_wrapped_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        if let Some(v) = value.get(key) {
            return Some(v);
        }

        const WRAPPERS: [&str; 3] = ["data", "file", "result"];
        for wrapper in WRAPPERS {
            if let Some(nested) = value.get(wrapper) {
                if let Some(v) = nested.get(key) {
                    return Some(v);
                }
                if let Some(nested_file) = nested.get("file") {
                    if let Some(v) = nested_file.get(key) {
                        return Some(v);
                    }
                }
            }
        }

        None
    }

    /// 先直接取数组字段，再兼容 data/file/result 包装后的 children/files/数组。
    fn find_wrapped_array<'a>(value: &'a Value, key: &str) -> Option<&'a [Value]> {
        if let Some(items) = value.get(key).and_then(Value::as_array) {
            return Some(items.as_slice());
        }
        if let Some(items) = value.get("children").and_then(Value::as_array) {
            return Some(items);
        }
        if let Some(items) = value.get("files").and_then(Value::as_array) {
            return Some(items);
        }

        const WRAPPERS: [&str; 3] = ["data", "file", "result"];
        for wrapper in WRAPPERS {
            let nested = match value.get(wrapper) {
                Some(v) => v,
                None => continue,
            };

            if let Some(items) = nested.get(key).and_then(Value::as_array) {
                return Some(items);
            }
            if let Some(items) = nested.as_array() {
                return Some(items.as_slice());
            }
            if let Some(items) = nested.get("children").and_then(Value::as_array) {
                return Some(items);
            }
            if let Some(items) = nested.get("files").and_then(Value::as_array) {
                return Some(items);
            }
            if let Some(nested_file) = nested.get("file") {
                if let Some(items) = nested_file.as_array() {
                    return Some(items);
                }
                if let Some(items) = nested_file.get("children").and_then(Value::as_array) {
                    return Some(items);
                }
                if let Some(items) = nested_file.get("files").and_then(Value::as_array) {
                    return Some(items);
                }
            }
            if let Some(nested_result) = nested.get("result") {
                if let Some(items) = nested_result.as_array() {
                    return Some(items);
                }
                if let Some(items) = nested_result.get("children").and_then(Value::as_array) {
                    return Some(items);
                }
                if let Some(items) = nested_result.get("files").and_then(Value::as_array) {
                    return Some(items);
                }
            }
        }

        value.as_array().map(|items| items.as_slice())
    }

    /// 提取对象，兼容 data/file/result 包装层。
    fn find_wrapped_object<'a>(value: &'a Value) -> Option<&'a Value> {
        if value.is_object() {
            const WRAPPERS: [&str; 3] = ["data", "file", "result"];
            for wrapper in WRAPPERS {
                if let Some(payload) = value.get(wrapper) {
                    if let Some(file) = payload.get("file") {
                        if file.is_object() {
                            return Some(file);
                        }
                        if let Some(result) = file.get("result") {
                            if result.is_object() {
                                return Some(result);
                            }
                        }
                    }
                    if let Some(result) = payload.get("result") {
                        if result.is_object() {
                            return Some(result);
                        }
                    }
                    if payload.is_object() {
                        return Some(payload);
                    }
                }
            }

            if let Some(file) = value.get("file") {
                if file.is_object() {
                    return Some(file);
                }
                if let Some(result) = file.get("result") {
                    if result.is_object() {
                        return Some(result);
                    }
                }
            }

            if let Some(result) = value.get("result") {
                if result.is_object() {
                    return Some(result);
                }
            }
        }

        if value.is_object() { Some(value) } else { None }
    }

    /// 读取某个项目或模块路径下的全部后代文件。
    fn list_metafiles_from_descendant_path(
        &self,
        project_ref: &str,
        descendant_path: &str,
    ) -> Result<Vec<RemoteMetafileEntry>> {
        let path = format!(
            "/api/meta/services/getFileDescendant/{}",
            percent_encode_path(descendant_path, true)
        );
        let raw = self.get_text(&path)?;
        let json_text = self.decode_structured_payload(&raw, "file descendants")?;
        let parsed = Self::parse_json(&json_text, "file descendants")?;
        let file_list = Self::find_wrapped_array(&parsed, "children")
            .context("failed to parse file descendants: expected array")?;

        self.parse_metafile_entries(project_ref, file_list)
    }

    /// 读取项目根节点下的模块列表，用于真实 BI 的分模块拉取流程。
    fn list_project_children(&self, project_ref: &str) -> Result<Vec<BiFileInfo>> {
        let path = format!(
            "/api/meta/services/getFileChildren/{}",
            percent_encode_path(project_ref, false)
        );
        let raw = self.get_text(&path)?;
        let json_text = self.decode_structured_payload(&raw, "project children")?;
        let parsed = Self::parse_json(&json_text, "project children")?;
        let children = Self::find_wrapped_array(&parsed, "children")
            .context("failed to parse project children: expected array")?;

        children
            .iter()
            .map(|child| {
                serde_json::from_value(child.clone()).with_context(|| {
                    format!("failed to parse project child entry for project {project_ref}")
                })
            })
            .collect()
    }

    /// 将 BI 文件数组转换成 session 同步可消费的远程文件条目。
    fn parse_metafile_entries(
        &self,
        project_ref: &str,
        file_list: &[Value],
    ) -> Result<Vec<RemoteMetafileEntry>> {
        let mut entries = Vec::new();
        for file_value in file_list {
            let info: BiFileInfo =
                serde_json::from_value(file_value.clone()).with_context(|| {
                    format!("failed to parse file descendant entry for project {project_ref}")
                })?;
            if let Some(entry) = Self::entry_from_file_info(project_ref, info) {
                entries.push(entry);
            }
        }

        Ok(entries)
    }

    /// 将单个 BI 文件信息转换成远程文件条目，目录节点会被跳过。
    fn entry_from_file_info(project_ref: &str, info: BiFileInfo) -> Option<RemoteMetafileEntry> {
        if info.is_folder.unwrap_or(false) {
            return None;
        }

        let raw_path = info.resource_path()?;
        let source_path = normalize_source_path(&raw_path, project_ref);
        if source_path.is_empty() {
            return None;
        }

        Some(RemoteMetafileEntry {
            project_ref: project_ref.to_string(),
            source_path,
            file_id: info.resolved_id(None),
            revision: info.revision,
            etag: None,
            mtime: info.modify_time,
            size: None,
            deleted: false,
        })
    }
}

impl SessionBootstrapper for ReqwestRemoteSessionProvider {
    fn bootstrap_with_access_token(&self, access_token: &str) -> Result<AuthenticatedSession> {
        Self::bootstrap_with_access_token(self, access_token)
    }
}

impl RemoteSessionProvider for ReqwestRemoteSessionProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProjectInfo>> {
        let raw = self.get_text("/api/me/getPermissionInfo")?;
        let json_text = self.decode_structured_payload(&raw, "permission info")?;
        let parsed = Self::parse_json(&json_text, "permission info")?;

        let meta_projects_value = Self::find_wrapped_field(&parsed, "metaProjects")
            .context("failed to parse permission info: missing metaProjects field")?;
        let meta_projects: Vec<BiMetaProject> = serde_json::from_value(meta_projects_value.clone())
            .context("failed to parse permission info metaProjects")?;

        let mut projects = Vec::with_capacity(meta_projects.len());
        for meta_project in meta_projects {
            let project_ref = meta_project
                .project_name
                .as_ref()
                .cloned()
                .or_else(|| meta_project.name.clone())
                .or_else(|| meta_project.id.clone())
                .ok_or_else(|| anyhow!("permission project missing project identifier"))?;
            let project_name = meta_project
                .desc
                .or_else(|| meta_project.project_name.clone())
                .unwrap_or_else(|| project_ref.clone());

            projects.push(RemoteProjectInfo {
                project_ref: project_ref.clone(),
                project_name,
                source_origin: meta_project
                    .path
                    .unwrap_or_else(|| format!("/{project_ref}")),
            });
        }

        Ok(projects)
    }

    fn list_metafiles(&self, project_ref: &str) -> Result<Vec<RemoteMetafileEntry>> {
        match self.list_metafiles_from_descendant_path(project_ref, project_ref) {
            Ok(entries) => Ok(entries),
            Err(project_descendant_error) => {
                let children = self.list_project_children(project_ref).with_context(|| {
                    format!(
                        "failed to list project children after project descendant request failed: {}",
                        project_descendant_error
                    )
                })?;

                let mut entries = Vec::new();
                for child in children {
                    if child.is_folder.unwrap_or(false) {
                        let Some(child_path) = child.resource_path() else {
                            continue;
                        };
                        entries.extend(self.list_metafiles_from_descendant_path(
                            project_ref,
                            child_path.trim_start_matches('/'),
                        )?);
                    } else if let Some(entry) = Self::entry_from_file_info(project_ref, child) {
                        entries.push(entry);
                    }
                }

                Ok(entries)
            }
        }
    }

    fn fetch_metafile_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo> {
        let file_id = file_ref
            .file_id
            .as_deref()
            .context("file_ref must have file_id for remote fetch")?;
        let path = format!(
            "/api/meta/services/getFileInfo/{}",
            percent_encode_path(file_id, false)
        );
        let raw = self.get_text(&path)?;
        let json_text = self.decode_structured_payload(&raw, "file info")?;
        let parsed = Self::parse_json(&json_text, "file info")?;
        let info_value = Self::find_wrapped_object(&parsed)
            .context("failed to parse file info payload: expected object")?;

        let info: BiFileInfo = serde_json::from_value(info_value.clone())
            .with_context(|| format!("failed to parse file info for {file_id}"))?;

        let file_id = info
            .resolved_id(file_ref.file_id.as_deref())
            .ok_or_else(|| anyhow!("file info missing id"))?;
        let source_path = normalize_source_path(
            info.resolved_path(&file_ref.source_path),
            &file_ref.project_ref,
        );
        let extension_fallback = file_ref.source_path.rsplit('.').next().unwrap_or("");
        let revision = info.revision.clone();
        let content_type =
            MetadataContentType::from_extension(info.resolve_extension(extension_fallback));

        Ok(RemoteFileInfo {
            source_path: if source_path.is_empty() {
                file_ref.source_path.clone()
            } else {
                source_path
            },
            file_id: Some(file_id),
            revision,
            content_type,
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
            percent_encode_path(file_id, false)
        );
        let raw = self.get_text(&path)?;
        let raw_text = self.decode_content_payload(&raw)?;

        Ok(RemoteFileContent {
            source_path: file_ref.source_path.clone(),
            file_id: file_ref.file_id.clone(),
            revision: file_ref.revision.clone(),
            content_type: MetadataContentType::from_extension(
                file_ref.source_path.rsplit('.').next().unwrap_or(""),
            ),
            raw_text,
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
            .filter(
                |entry| match (&entry.revision, since_revision.parse::<u64>()) {
                    (Some(revision), Ok(since)) => revision
                        .parse::<u64>()
                        .map(|version| version > since)
                        .unwrap_or(false),
                    _ => true,
                },
            )
            .collect();

        let to_revision = changed_files
            .iter()
            .filter_map(|entry| entry.revision.as_deref())
            .filter_map(|revision| revision.parse::<u64>().ok())
            .max()
            .map(|version| version.to_string());

        Ok(RemoteChangeSet {
            project_ref: project_ref.to_string(),
            project_name: project_ref.to_string(),
            since_revision: since_revision.to_string(),
            to_revision,
            changed_files,
        })
    }
}

fn decompress_lzstring_base64(input: &str, context: &str) -> Result<String> {
    let trimmed = input.trim();
    let decoded = decompress_from_base64(trimmed)
        .filter(|decoded| !decoded.is_empty())
        .or_else(|| {
            decompress_from_encoded_uri_component(trimmed).filter(|decoded| !decoded.is_empty())
        })
        .ok_or_else(|| anyhow!("failed to decode {context}: LZString decompression failed"))?;

    String::from_utf16(&decoded).map_err(|_| {
        anyhow!("failed to decode {context}: decompressed payload is not valid UTF-16")
    })
}

fn looks_like_json_document(input: &str) -> bool {
    input
        .chars()
        .next()
        .is_some_and(|first| matches!(first, '{' | '['))
}

/// 将 BI 绝对路径转换为项目内逻辑路径。
fn normalize_source_path(path: &str, project_ref: &str) -> String {
    let prefix = format!("/{project_ref}/");
    if path.starts_with(&prefix) {
        path[prefix.len()..].to_string()
    } else {
        path.trim_start_matches('/').to_string()
    }
}

fn percent_encode_path(path: &str, preserve_slash: bool) -> String {
    if preserve_slash {
        path.split('/')
            .map(percent_encode)
            .collect::<Vec<_>>()
            .join("/")
    } else {
        percent_encode(path)
    }
}

fn percent_encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// 将 BI 可能返回的字符串/数字版本号统一反序列化为字符串。
fn deserialize_optional_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }))
}
