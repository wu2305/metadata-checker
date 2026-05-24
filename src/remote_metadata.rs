//! 远程元数据读取统一 contract
//!
//! M40.6：定义 RemoteMetadataProvider trait、请求/响应类型、错误枚举。
//! 不同运行环境（WASM fetch、Page JS rc、Browser Extension、CLI）共用同一语义 contract。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 校验 source_path 是否为项目内逻辑路径。
///
/// 拒绝以 `/` 或 `\` 开头的绝对路径、包含 `..` 的逃逸路径、
/// URL  scheme（http://、https://、file://）、Windows 盘符。
pub fn is_logical_source_path(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return false;
    }
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return false;
    }
    if cfg!(windows) && path.len() >= 2 && path.as_bytes()[1] == b':' {
        return false;
    }
    // 拒绝包含 .. 的路径片段
    if path.split(['/', '\\']).any(|segment| segment == "..") {
        return false;
    }
    true
}

/// 远程文件引用
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileRef {
    pub remote_ref: Option<String>,
    pub project_ref: String,
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
}

impl RemoteFileRef {
    /// 创建 RemoteFileRef，强制校验 source_path。
    ///
    /// 若 source_path 不是项目内逻辑路径，返回 Err。
    pub fn try_new(
        project_ref: impl Into<String>,
        source_path: impl Into<String>,
        file_id: Option<String>,
    ) -> Result<Self, String> {
        let path = source_path.into();
        if !is_logical_source_path(&path) {
            return Err(format!(
                "source_path '{}' is not a project-internal logical path",
                path
            ));
        }
        Ok(Self {
            remote_ref: None,
            project_ref: project_ref.into(),
            source_path: path,
            file_id,
            revision: None,
        })
    }
}

/// 远程文件元信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileInfo {
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
    pub content_type: MetadataContentType,
    pub updated_at: Option<String>,
}

/// 远程文件内容
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileContent {
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
    pub content_type: MetadataContentType,
    pub raw_text: String,
}

/// 远程项目信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteProjectInfo {
    pub project_ref: String,
    pub project_name: String,
    pub source_origin: String,
    pub updated_at: Option<String>,
}

/// 远程元数据文件条目
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMetafileEntry {
    pub source_path: String,
    pub file_id: Option<String>,
    pub revision: Option<String>,
    pub content_type: MetadataContentType,
    pub updated_at: Option<String>,
    pub etag: Option<String>,
    pub size: Option<u64>,
}

/// 元数据内容类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetadataContentType {
    SuperPage,
    Table,
    Unknown,
}

impl MetadataContentType {
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "spg" => MetadataContentType::SuperPage,
            "tbl" => MetadataContentType::Table,
            _ => MetadataContentType::Unknown,
        }
    }
}

/// 远程元数据错误码
///
/// 序列化使用大写 SCREAMING_SNAKE_CASE，与 JS provider / roadmap 对齐。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RemoteMetadataErrorCode {
    RemoteAuthRequired,
    RemoteSessionExpired,
    RemoteFetchUnauthorized,
    RemoteFetchForbidden,
    RemoteFetchNotFound,
    RemoteFetchCorsBlocked,
    RemoteFetchFailed,
    RemoteResponseInvalid,
    PageRcUnavailable,
    ClientFetchProxyTimeout,
    ClientFetchProxyFailed,
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
const WASM_FETCH_FILE_INFO_ENDPOINT: &str = "/api/meta/services/getFileInfo";
#[cfg(test)]
const WASM_FETCH_CREDENTIALS_MODE: &str = "include";

#[cfg(test)]
fn remote_fetch_credentials_mode() -> &'static str {
    WASM_FETCH_CREDENTIALS_MODE
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
pub(crate) fn map_http_status_code(status: u16) -> RemoteMetadataErrorCode {
    match status {
        401 => RemoteMetadataErrorCode::RemoteFetchUnauthorized,
        403 => RemoteMetadataErrorCode::RemoteFetchForbidden,
        404 => RemoteMetadataErrorCode::RemoteFetchNotFound,
        _ => RemoteMetadataErrorCode::RemoteFetchFailed,
    }
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
fn percent_encode_query(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
fn percent_encode_path_preserving_slashes(path: &str) -> String {
    path.split('/')
        .map(percent_encode_query)
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
fn build_remote_file_path(file_ref: &RemoteFileRef) -> String {
    let project_ref = file_ref.project_ref.trim_matches('/');
    if project_ref.is_empty() {
        file_ref.source_path.clone()
    } else {
        format!(
            "{}/{}",
            project_ref,
            file_ref.source_path.trim_start_matches('/')
        )
    }
}

#[cfg(any(test, feature = "cli-local", feature = "browser-wasm"))]
pub(crate) fn build_remote_content_url(
    base_url: &str,
    file_ref: &RemoteFileRef,
) -> Result<String, RemoteMetadataError> {
    if !is_logical_source_path(&file_ref.source_path) {
        return Err(RemoteMetadataError::new(
            RemoteMetadataErrorCode::RemoteFetchFailed,
            format!(
                "source_path '{}' is not a project-internal logical path",
                file_ref.source_path
            ),
        ));
    }

    let remote_path = build_remote_file_path(file_ref);
    let encoded_path = percent_encode_path_preserving_slashes(&remote_path);

    Ok(format!(
        "{}{}/{}?downloadContent=true",
        base_url.trim_end_matches('/'),
        WASM_FETCH_FILE_INFO_ENDPOINT,
        encoded_path
    ))
}

#[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
fn extract_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

#[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
fn extract_nested_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    fn from_object_or_file(value: &serde_json::Value, key: &str) -> Option<String> {
        extract_string_field(value, key)
            .or_else(|| value.get("file").and_then(|v| extract_string_field(v, key)))
    }

    extract_string_field(value, key)
        .or_else(|| value.get("data").and_then(|v| from_object_or_file(v, key)))
        .or_else(|| {
            value
                .get("result")
                .and_then(|v| from_object_or_file(v, key))
        })
        .or_else(|| value.get("file").and_then(|v| extract_string_field(v, key)))
}

#[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
pub(crate) fn extract_remote_content_payload(
    response_text: &str,
    file_ref: &RemoteFileRef,
) -> Result<RemoteFileContent, RemoteMetadataError> {
    let parsed = serde_json::from_str::<serde_json::Value>(response_text).ok();
    let raw_text = parsed
        .as_ref()
        .and_then(|value| {
            extract_nested_string_field(value, "content")
                .or_else(|| extract_nested_string_field(value, "raw_text"))
                .or_else(|| extract_nested_string_field(value, "rawText"))
        })
        .unwrap_or_else(|| response_text.to_string());

    if raw_text.is_empty() {
        return Err(RemoteMetadataError::new(
            RemoteMetadataErrorCode::RemoteResponseInvalid,
            "remote metadata response is empty",
        ));
    }

    let file_id = parsed
        .as_ref()
        .and_then(|value| {
            extract_nested_string_field(value, "id")
                .or_else(|| extract_nested_string_field(value, "file_id"))
                .or_else(|| extract_nested_string_field(value, "fileId"))
        })
        .or_else(|| file_ref.file_id.clone());
    let revision = parsed
        .as_ref()
        .and_then(|value| extract_nested_string_field(value, "revision"))
        .or_else(|| file_ref.revision.clone());

    Ok(RemoteFileContent {
        source_path: file_ref.source_path.clone(),
        file_id,
        revision,
        content_type: MetadataContentType::from_extension(
            file_ref.source_path.rsplit('.').next().unwrap_or(""),
        ),
        raw_text,
    })
}

/// 远程元数据错误
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteMetadataError {
    pub code: RemoteMetadataErrorCode,
    pub message: String,
}

impl RemoteMetadataError {
    pub fn new(code: RemoteMetadataErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 远程元数据 provider trait — 同步 contract
///
/// 浏览器 fetch、CLI HTTP 等异步边界通过 `AsyncRemoteMetadataProvider` wrapper 统一。
/// 同步 trait 保留给 TestProvider、内存缓存等无需异步的场景。
pub trait RemoteMetadataProvider {
    fn get_file_info(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError>;
    fn get_file_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError>;
    fn get_related_files(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError>;
}

/// 异步 remote metadata provider contract
///
/// WASM fetch、Browser Extension、CLI HTTP 等实现此 trait。
/// 当前使用返回 `Result` 的 async fn（Rust 2024 Edition 支持）。
#[allow(async_fn_in_trait)]
pub trait AsyncRemoteMetadataProvider {
    async fn get_file_info(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError>;
    async fn get_file_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError>;
    async fn get_related_files(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError>;
}

/// 兼容类型：保留 `WasmFetchMetadataProvider` 名称，但不再使用浏览器原生 fetch。
///
/// 实际实现已统一迁移到 `ReqwestRemoteMetadataProvider`，此类型仅委托调用。
#[cfg(feature = "browser-wasm")]
pub struct WasmFetchMetadataProvider {
    base_url: String,
    inner: crate::remote_metadata_provider::ReqwestRemoteMetadataProvider,
}

#[cfg(feature = "browser-wasm")]
impl WasmFetchMetadataProvider {
    /// 创建兼容类型实例。创建失败时返回错误，不再 panic。
    pub fn new(base_url: impl Into<String>) -> Result<Self, RemoteMetadataError> {
        let base_url = base_url.into();
        let inner = crate::remote_metadata_provider::ReqwestRemoteMetadataProvider::new(&base_url)?;
        Ok(Self { base_url, inner })
    }

    /// 与 `new` 等价的兼容构造接口。
    pub fn try_new(base_url: impl Into<String>) -> Result<Self, RemoteMetadataError> {
        Self::new(base_url)
    }

    /// 保留兼容签名：构造 fetch 参数对象并设置 credentials=include。
    pub fn make_request_init(&self) -> web_sys::RequestInit {
        let init = web_sys::RequestInit::new();
        init.set_method("GET");
        init.set_credentials(web_sys::RequestCredentials::Include);
        init
    }

    /// 构造远程内容读取 URL。
    pub fn build_content_url(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<String, RemoteMetadataError> {
        build_remote_content_url(&self.base_url, file_ref)
    }

    /// 将 HTTP status 映射为 RemoteMetadataErrorCode。
    pub fn map_http_status(status: u16) -> RemoteMetadataErrorCode {
        map_http_status_code(status)
    }

    pub fn with_client(
        base_url: impl Into<String>,
        client: reqwest::Client,
    ) -> Result<Self, RemoteMetadataError> {
        let base_url = base_url.into();
        let inner = crate::remote_metadata_provider::ReqwestRemoteMetadataProvider::with_client(
            base_url.clone(),
            client,
        );
        Ok(Self { base_url, inner })
    }

    /// 保留兼容字段访问能力。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

#[cfg(feature = "browser-wasm")]
impl AsyncRemoteMetadataProvider for WasmFetchMetadataProvider {
    async fn get_file_info(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError> {
        AsyncRemoteMetadataProvider::get_file_info(&self.inner, file_ref).await
    }

    async fn get_file_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError> {
        AsyncRemoteMetadataProvider::get_file_content(&self.inner, file_ref).await
    }

    async fn get_related_files(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError> {
        AsyncRemoteMetadataProvider::get_related_files(&self.inner, _file_ref).await
    }
}

/// 测试用 fixture provider
pub struct TestMetadataProvider {
    fixtures: HashMap<String, RemoteFileContent>,
}

impl TestMetadataProvider {
    pub fn new(fixtures: HashMap<String, RemoteFileContent>) -> Self {
        Self { fixtures }
    }

    pub fn empty() -> Self {
        Self {
            fixtures: HashMap::new(),
        }
    }

    pub fn insert(&mut self, content: RemoteFileContent) {
        self.fixtures.insert(content.source_path.clone(), content);
    }
}

impl RemoteMetadataProvider for TestMetadataProvider {
    fn get_file_info(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError> {
        let content = self.fixtures.get(&file_ref.source_path).ok_or_else(|| {
            RemoteMetadataError::new(
                RemoteMetadataErrorCode::RemoteFetchNotFound,
                format!("file not found: {}", file_ref.source_path),
            )
        })?;

        Ok(RemoteFileInfo {
            source_path: content.source_path.clone(),
            file_id: content.file_id.clone(),
            revision: content.revision.clone(),
            content_type: content.content_type.clone(),
            updated_at: None,
        })
    }

    fn get_file_content(
        &self,
        file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError> {
        self.fixtures
            .get(&file_ref.source_path)
            .cloned()
            .ok_or_else(|| {
                RemoteMetadataError::new(
                    RemoteMetadataErrorCode::RemoteFetchNotFound,
                    format!("file not found: {}", file_ref.source_path),
                )
            })
    }

    fn get_related_files(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fixture(path: &str, raw: &str) -> RemoteFileContent {
        RemoteFileContent {
            source_path: path.to_string(),
            file_id: Some(format!("id-{}", path.replace('/', "-"))),
            revision: Some("1".to_string()),
            content_type: MetadataContentType::from_extension(
                path.rsplit('.').next().unwrap_or(""),
            ),
            raw_text: raw.to_string(),
        }
    }

    #[test]
    fn test_is_logical_source_path_accepts_relative() {
        assert!(is_logical_source_path("app/page.spg"));
        assert!(is_logical_source_path("data/tables/bind.tbl"));
    }

    #[test]
    fn test_is_logical_source_path_rejects_absolute() {
        assert!(!is_logical_source_path("/app/page.spg"));
        assert!(!is_logical_source_path("\\app\\page.spg"));
    }

    #[test]
    fn test_is_logical_source_path_rejects_url() {
        assert!(!is_logical_source_path("https://example.com/app/page.spg"));
        assert!(!is_logical_source_path("http://localhost/app/page.spg"));
        assert!(!is_logical_source_path("file:///tmp/page.spg"));
    }

    #[test]
    fn test_is_logical_source_path_rejects_parent_escape() {
        assert!(!is_logical_source_path("../secret.spg"));
        assert!(!is_logical_source_path("app/../../../secret.spg"));
    }

    #[test]
    fn test_remote_file_ref_try_new_valid() {
        let r = RemoteFileRef::try_new("Test", "app/page.spg", Some("f1".to_string()));
        assert!(r.is_ok());
        let r = r.unwrap();
        assert_eq!(r.source_path, "app/page.spg");
        assert_eq!(r.file_id, Some("f1".to_string()));
    }

    #[test]
    fn test_remote_file_ref_try_new_rejects_absolute() {
        let r = RemoteFileRef::try_new("Test", "/app/page.spg", None);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("not a project-internal"));
    }

    #[test]
    fn test_remote_file_ref_try_new_rejects_url() {
        let r = RemoteFileRef::try_new("Test", "https://example.com/page.spg", None);
        assert!(r.is_err());
    }

    #[test]
    fn test_remote_file_ref_try_new_rejects_parent_escape() {
        let r = RemoteFileRef::try_new("Test", "app/../../page.spg", None);
        assert!(r.is_err());
    }

    #[test]
    fn test_metadata_content_type_from_extension() {
        assert_eq!(
            MetadataContentType::from_extension("spg"),
            MetadataContentType::SuperPage
        );
        assert_eq!(
            MetadataContentType::from_extension("SPG"),
            MetadataContentType::SuperPage
        );
        assert_eq!(
            MetadataContentType::from_extension("tbl"),
            MetadataContentType::Table
        );
        assert_eq!(
            MetadataContentType::from_extension("txt"),
            MetadataContentType::Unknown
        );
    }

    #[test]
    fn test_error_code_serialization_screaming_snake_case() {
        let err = RemoteMetadataError::new(RemoteMetadataErrorCode::RemoteFetchNotFound, "test");
        let json = serde_json::to_string(&err).unwrap();
        assert!(
            json.contains("REMOTE_FETCH_NOT_FOUND"),
            "error code should be SCREAMING_SNAKE_CASE: {}",
            json
        );
    }

    #[test]
    fn test_remote_fetch_content_url_builder_encodes_identity_fields() {
        let file_ref = RemoteFileRef {
            remote_ref: Some("https://bi.example.test/root".to_string()),
            project_ref: "销售 项目".to_string(),
            source_path: "app/Sales Page.app/Page.spg".to_string(),
            file_id: Some("file/123".to_string()),
            revision: Some("rev:7".to_string()),
        };

        let url = build_remote_content_url("https://host.example/base/", &file_ref).unwrap();

        assert_eq!(
            url,
            "https://host.example/base/api/meta/services/getFileInfo/%E9%94%80%E5%94%AE%20%E9%A1%B9%E7%9B%AE/app/Sales%20Page.app/Page.spg?downloadContent=true"
        );
    }

    #[test]
    fn test_remote_fetch_content_url_builder_rejects_invalid_source_path() {
        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "proj".to_string(),
            source_path: "https://evil.example/Page.spg".to_string(),
            file_id: None,
            revision: None,
        };

        let err = build_remote_content_url("https://host.example", &file_ref).unwrap_err();

        assert_eq!(err.code, RemoteMetadataErrorCode::RemoteFetchFailed);
        assert!(err.message.contains("source_path"));
    }

    #[test]
    fn test_wasm_fetch_credentials_contract_is_include() {
        assert_eq!(remote_fetch_credentials_mode(), "include");
    }

    #[test]
    fn test_remote_fetch_http_status_mapping() {
        assert_eq!(
            map_http_status_code(401),
            RemoteMetadataErrorCode::RemoteFetchUnauthorized
        );
        assert_eq!(
            map_http_status_code(403),
            RemoteMetadataErrorCode::RemoteFetchForbidden
        );
        assert_eq!(
            map_http_status_code(404),
            RemoteMetadataErrorCode::RemoteFetchNotFound
        );
        assert_eq!(
            map_http_status_code(500),
            RemoteMetadataErrorCode::RemoteFetchFailed
        );
    }

    #[test]
    fn test_error_code_serializes_uppercase_exactly() {
        let code = serde_json::to_string(&RemoteMetadataErrorCode::RemoteResponseInvalid).unwrap();
        assert_eq!(code, "\"REMOTE_RESPONSE_INVALID\"");
    }

    #[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
    #[test]
    fn test_remote_fetch_payload_extracts_wrapped_content_response() {
        let file_ref =
            RemoteFileRef::try_new("analyzer", "app/Test.app/Page.spg", Some("fallback".into()))
                .unwrap();
        let response_text = serde_json::json!({
            "data": {
                "file": {
                    "fileId": "fid-from-remote",
                    "revision": "rev-2",
                    "content": "{\"canvas\":{\"components\":[]}}"
                }
            }
        })
        .to_string();

        let content = extract_remote_content_payload(&response_text, &file_ref).unwrap();

        assert_eq!(content.source_path, "app/Test.app/Page.spg");
        assert_eq!(content.file_id, Some("fid-from-remote".to_string()));
        assert_eq!(content.revision, Some("rev-2".to_string()));
        assert_eq!(content.content_type, MetadataContentType::SuperPage);
        assert_eq!(content.raw_text, "{\"canvas\":{\"components\":[]}}");
    }

    #[cfg(any(feature = "cli-local", feature = "browser-wasm"))]
    #[test]
    fn test_remote_fetch_payload_preserves_plain_text_response() {
        let file_ref =
            RemoteFileRef::try_new("analyzer", "data/Table.tbl", Some("fallback".into())).unwrap();

        let content = extract_remote_content_payload("{\"fields\":[]}", &file_ref).unwrap();

        assert_eq!(content.file_id, Some("fallback".to_string()));
        assert_eq!(content.revision, None);
        assert_eq!(content.content_type, MetadataContentType::Table);
        assert_eq!(content.raw_text, "{\"fields\":[]}");
    }

    #[cfg(all(feature = "browser-wasm", target_arch = "wasm32"))]
    #[test]
    fn test_wasm_request_init_sets_credentials_include() {
        let provider = WasmFetchMetadataProvider::new("https://host.example").unwrap();
        let init = provider.make_request_init();
        let value = wasm_bindgen::JsValue::from(init);
        let credentials = js_sys::Reflect::get(&value, &"credentials".into())
            .unwrap()
            .as_string()
            .unwrap();

        assert_eq!(credentials, remote_fetch_credentials_mode());
    }

    #[test]
    fn test_provider_returns_fixture_content() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/Test.app/Page.spg", "{\"components\":[]}");
        provider.insert(fixture.clone());

        let file_ref = RemoteFileRef::try_new(
            "Test",
            "app/Test.app/Page.spg",
            Some("id-app-Test.app-Page.spg".to_string()),
        )
        .unwrap();

        let result = provider.get_file_content(&file_ref).unwrap();
        assert_eq!(result.raw_text, "{\"components\":[]}");
        assert_eq!(result.content_type, MetadataContentType::SuperPage);
        assert_eq!(result.file_id, Some("id-app-Test.app-Page.spg".to_string()));
    }

    #[test]
    fn test_provider_returns_fixture_info() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/Test.app/Page.spg", "raw");
        provider.insert(fixture);

        let file_ref =
            RemoteFileRef::try_new("Test", "app/Test.app/Page.spg", Some("f1".to_string()))
                .unwrap();

        let info = provider.get_file_info(&file_ref).unwrap();
        assert_eq!(info.source_path, "app/Test.app/Page.spg");
        assert_eq!(info.revision, Some("1".to_string()));
        assert_eq!(info.content_type, MetadataContentType::SuperPage);
        // file_id 从 fixture 内容推导，不是从 file_ref
        assert_eq!(info.file_id, Some("id-app-Test.app-Page.spg".to_string()));
    }

    #[test]
    fn test_provider_not_found_returns_error() {
        let provider = TestMetadataProvider::empty();
        let file_ref =
            RemoteFileRef::try_new("Test", "app/Missing.app/Page.spg", Some("f1".to_string()))
                .unwrap();

        let err = provider.get_file_content(&file_ref).unwrap_err();
        assert_eq!(err.code, RemoteMetadataErrorCode::RemoteFetchNotFound);
        assert!(err.message.contains("Missing"));
    }

    #[test]
    fn test_provider_related_files_empty() {
        let provider = TestMetadataProvider::empty();
        let file_ref =
            RemoteFileRef::try_new("Test", "app/Test.app/Page.spg", Some("f1".to_string()))
                .unwrap();

        let related = provider.get_related_files(&file_ref).unwrap();
        assert!(related.is_empty());
    }

    #[test]
    fn test_source_path_is_logical_path() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/售后.app/Page.spg", "raw");
        provider.insert(fixture);

        let file_ref = RemoteFileRef::try_new(
            "xiaoshouyi",
            "app/售后.app/Page.spg",
            Some("f1".to_string()),
        )
        .unwrap();

        let result = provider.get_file_content(&file_ref).unwrap();
        assert!(!result.source_path.starts_with('/'));
        assert!(!result.source_path.contains("http"));
        assert!(!result.source_path.contains("xiaoshouyi"));
    }
}
