//! 远程元数据读取统一 contract
//!
//! M40.6：定义 RemoteMetadataProvider trait、请求/响应类型、错误枚举。
//! 不同运行环境（WASM fetch、Page JS rc、Browser Extension、CLI）共用同一语义 contract。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 远程文件引用
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileRef {
    pub remote_ref: Option<String>,
    pub project_ref: String,
    pub source_path: String,
    pub file_id: String,
    pub revision: Option<String>,
}

/// 远程文件元信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileInfo {
    pub source_path: String,
    pub file_id: String,
    pub revision: String,
    pub content_type: MetadataContentType,
    pub updated_at: Option<String>,
}

/// 远程文件内容
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteFileContent {
    pub source_path: String,
    pub file_id: String,
    pub revision: String,
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
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

/// 远程元数据 provider trait
///
/// 当前保持同步接口。若后续需要 async，可在各环境特化 wrapper 中引入 async。
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
            file_id: format!("id-{}", path.replace('/', "-")),
            revision: "1".to_string(),
            content_type: MetadataContentType::from_extension(
                path.rsplit('.').next().unwrap_or(""),
            ),
            raw_text: raw.to_string(),
        }
    }

    #[test]
    fn test_metadata_content_type_from_extension() {
        assert_eq!(MetadataContentType::from_extension("spg"), MetadataContentType::SuperPage);
        assert_eq!(MetadataContentType::from_extension("SPG"), MetadataContentType::SuperPage);
        assert_eq!(MetadataContentType::from_extension("tbl"), MetadataContentType::Table);
        assert_eq!(MetadataContentType::from_extension("txt"), MetadataContentType::Unknown);
    }

    #[test]
    fn test_provider_returns_fixture_content() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/Test.app/Page.spg", "{\"components\":[]}");
        provider.insert(fixture.clone());

        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "Test".to_string(),
            source_path: "app/Test.app/Page.spg".to_string(),
            file_id: "id-app-Test.app-Page.spg".to_string(),
            revision: None,
        };

        let result = provider.get_file_content(&file_ref).unwrap();
        assert_eq!(result.raw_text, "{\"components\":[]}");
        assert_eq!(result.content_type, MetadataContentType::SuperPage);
    }

    #[test]
    fn test_provider_returns_fixture_info() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/Test.app/Page.spg", "raw");
        provider.insert(fixture);

        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "Test".to_string(),
            source_path: "app/Test.app/Page.spg".to_string(),
            file_id: "f1".to_string(),
            revision: None,
        };

        let info = provider.get_file_info(&file_ref).unwrap();
        assert_eq!(info.source_path, "app/Test.app/Page.spg");
        assert_eq!(info.revision, "1");
        assert_eq!(info.content_type, MetadataContentType::SuperPage);
    }

    #[test]
    fn test_provider_not_found_returns_error() {
        let provider = TestMetadataProvider::empty();
        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "Test".to_string(),
            source_path: "app/Missing.app/Page.spg".to_string(),
            file_id: "f1".to_string(),
            revision: None,
        };

        let err = provider.get_file_content(&file_ref).unwrap_err();
        assert_eq!(err.code, RemoteMetadataErrorCode::RemoteFetchNotFound);
        assert!(err.message.contains("Missing"));
    }

    #[test]
    fn test_provider_related_files_empty() {
        let provider = TestMetadataProvider::empty();
        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "Test".to_string(),
            source_path: "app/Test.app/Page.spg".to_string(),
            file_id: "f1".to_string(),
            revision: None,
        };

        let related = provider.get_related_files(&file_ref).unwrap();
        assert!(related.is_empty());
    }

    #[test]
    fn test_source_path_is_logical_path() {
        let mut provider = TestMetadataProvider::empty();
        let fixture = make_fixture("app/售后.app/Page.spg", "raw");
        provider.insert(fixture);

        let file_ref = RemoteFileRef {
            remote_ref: None,
            project_ref: "xiaoshouyi".to_string(),
            source_path: "app/售后.app/Page.spg".to_string(),
            file_id: "f1".to_string(),
            revision: None,
        };

        let result = provider.get_file_content(&file_ref).unwrap();
        assert!(!result.source_path.starts_with('/'));
        assert!(!result.source_path.contains("http"));
        assert!(!result.source_path.contains("xiaoshouyi"));
    }
}
