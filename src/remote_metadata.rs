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
    fn get_file_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo, RemoteMetadataError>;
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

/// WASM 浏览器原生 fetch provider 骨架
///
/// M40.6 首轮实现最小 contract：构造 RequestInit.credentials = include，
/// 映射 HTTP/CORS/network 错误。实际 fetch 调用需 wasm-bindgen 暴露。
#[cfg(feature = "browser-wasm")]
pub struct WasmFetchMetadataProvider {
    pub base_url: String,
}

#[cfg(feature = "browser-wasm")]
impl WasmFetchMetadataProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    /// 构造 fetch RequestInit，设置 credentials = include
    pub fn make_request_init(&self) -> wasm_bindgen::JsValue {
        let init = js_sys::Object::new();
        js_sys::Reflect::set(
            &init,
            &"credentials".into(),
            &"include".into(),
        )
        .unwrap();
        init.into()
    }

    /// 将 HTTP status 映射为 RemoteMetadataErrorCode
    pub fn map_http_status(status: u16) -> RemoteMetadataErrorCode {
        match status {
            401 => RemoteMetadataErrorCode::RemoteFetchUnauthorized,
            403 => RemoteMetadataErrorCode::RemoteFetchForbidden,
            404 => RemoteMetadataErrorCode::RemoteFetchNotFound,
            _ => RemoteMetadataErrorCode::RemoteFetchFailed,
        }
    }
}

#[cfg(feature = "browser-wasm")]
impl AsyncRemoteMetadataProvider for WasmFetchMetadataProvider {
    async fn get_file_info(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileInfo, RemoteMetadataError> {
        // M40.6 首轮保留骨架，实际 fetch 实现后续里程碑补齐
        Err(RemoteMetadataError::new(
            RemoteMetadataErrorCode::RemoteFetchFailed,
            "WasmFetchMetadataProvider not yet fully implemented",
        ))
    }

    async fn get_file_content(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<RemoteFileContent, RemoteMetadataError> {
        Err(RemoteMetadataError::new(
            RemoteMetadataErrorCode::RemoteFetchFailed,
            "WasmFetchMetadataProvider not yet fully implemented",
        ))
    }

    async fn get_related_files(
        &self,
        _file_ref: &RemoteFileRef,
    ) -> Result<Vec<RemoteFileRef>, RemoteMetadataError> {
        Ok(Vec::new())
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
        self.fixtures
            .insert(content.source_path.clone(), content);
    }
}

impl RemoteMetadataProvider for TestMetadataProvider {
    fn get_file_info(&self, file_ref: &RemoteFileRef) -> Result<RemoteFileInfo, RemoteMetadataError> {
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
        assert_eq!(MetadataContentType::from_extension("spg"), MetadataContentType::SuperPage);
        assert_eq!(MetadataContentType::from_extension("SPG"), MetadataContentType::SuperPage);
        assert_eq!(MetadataContentType::from_extension("tbl"), MetadataContentType::Table);
        assert_eq!(MetadataContentType::from_extension("txt"), MetadataContentType::Unknown);
    }

    #[test]
    fn test_error_code_serialization_screaming_snake_case() {
        let err = RemoteMetadataError::new(RemoteMetadataErrorCode::RemoteFetchNotFound, "test");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("REMOTE_FETCH_NOT_FOUND"), "error code should be SCREAMING_SNAKE_CASE: {}", json);
    }

    #[test]
    fn test_provider_returns_fixture_content() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/Test.app/Page.spg", "{\"components\":[]}");
        provider.insert(fixture.clone());

        let file_ref = RemoteFileRef::try_new("Test", "app/Test.app/Page.spg", Some("id-app-Test.app-Page.spg".to_string())).unwrap();

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

        let file_ref = RemoteFileRef::try_new("Test", "app/Test.app/Page.spg", Some("f1".to_string())).unwrap();

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
        let file_ref = RemoteFileRef::try_new("Test", "app/Missing.app/Page.spg", Some("f1".to_string())).unwrap();

        let err = provider.get_file_content(&file_ref).unwrap_err();
        assert_eq!(err.code, RemoteMetadataErrorCode::RemoteFetchNotFound);
        assert!(err.message.contains("Missing"));
    }

    #[test]
    fn test_provider_related_files_empty() {
        let provider = TestMetadataProvider::empty();
        let file_ref = RemoteFileRef::try_new("Test", "app/Test.app/Page.spg", Some("f1".to_string())).unwrap();

        let related = provider.get_related_files(&file_ref).unwrap();
        assert!(related.is_empty());
    }

    #[test]
    fn test_source_path_is_logical_path() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/售后.app/Page.spg", "raw");
        provider.insert(fixture);

        let file_ref = RemoteFileRef::try_new("xiaoshouyi", "app/售后.app/Page.spg", Some("f1".to_string())).unwrap();

        let result = provider.get_file_content(&file_ref).unwrap();
        assert!(!result.source_path.starts_with('/'));
        assert!(!result.source_path.contains("http"));
        assert!(!result.source_path.contains("xiaoshouyi"));
    }
}
