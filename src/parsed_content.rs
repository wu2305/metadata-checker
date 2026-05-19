use crate::source_id::SourceId;
use anyhow::{Context, Result};
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// 元数据内容原始形态。
///
/// M36 的边界：只负责持有原始内容，不做类型判断、不建图、不查询、不输出。
#[derive(Debug, Clone)]
pub enum MetadataContent {
    /// 原始字节。
    Bytes(Arc<[u8]>),
    /// UTF-8 文本。
    Text(Arc<str>),
    /// 已解析 JSON。
    Json(Arc<Value>),
    /// 预留压缩格式，M36 不实现完整压缩流解析。
    #[allow(dead_code)]
    CompressedBytes {
        encoding: String,
        bytes: Arc<[u8]>,
    },
}

/// 解析后的内容容器，负责反序列化缓存。
///
/// 核心保证：`ParsedContent::json()` 连续调用必须复用同一个 `Arc<Value>`。
/// 后续 parser/scanner 入口优先从此获取 `serde_json::Value`。
#[derive(Debug)]
pub struct ParsedContent {
    /// 来源身份。
    pub source: SourceId,
    /// 原始内容。
    pub content: MetadataContent,
    /// 内容哈希，用于增量判断。
    pub content_hash: Option<String>,
    /// 懒解析缓存，Mutex + Option 保证线程安全且只反序列化一次。
    parsed_json: Mutex<Option<Arc<Value>>>,
}

impl ParsedContent {
    /// 从内存文本构造。
    pub fn from_text(source: SourceId, text: impl Into<String>) -> Self {
        Self {
            source,
            content: MetadataContent::Text(Arc::from(text.into().into_boxed_str())),
            content_hash: None,
            parsed_json: Mutex::new(None),
        }
    }

    /// 从内存字节构造。
    pub fn from_bytes(source: SourceId, bytes: Vec<u8>) -> Self {
        Self {
            source,
            content: MetadataContent::Bytes(Arc::from(bytes.into_boxed_slice())),
            content_hash: None,
            parsed_json: Mutex::new(None),
        }
    }

    /// 从已解析 JSON 构造。
    pub fn from_json(source: SourceId, value: Value) -> Self {
        Self {
            source,
            content: MetadataContent::Json(Arc::new(value)),
            content_hash: None,
            parsed_json: Mutex::new(None),
        }
    }

    /// 获取 JSON Value，首次调用时反序列化并缓存。
    ///
    /// 连续调用保证复用同一个 `Arc<Value>`。
    pub fn json(&self) -> Result<Arc<Value>> {
        let mut guard = self.parsed_json.lock().unwrap();
        if let Some(ref cached) = *guard {
            return Ok(Arc::clone(cached));
        }

        let value = match &self.content {
            MetadataContent::Json(v) => Arc::clone(v),
            MetadataContent::Text(t) => {
                Arc::new(
                    serde_json::from_str::<Value>(t)
                        .with_context(|| format!("Failed to parse JSON text from: {}", self.source.source_path))?,
                )
            }
            MetadataContent::Bytes(b) => {
                Arc::new(
                    serde_json::from_slice::<Value>(b)
                        .with_context(|| format!("Failed to parse JSON bytes from: {}", self.source.source_path))?,
                )
            }
            MetadataContent::CompressedBytes { .. } => {
                anyhow::bail!("CompressedBytes not supported in M36");
            }
        };

        *guard = Some(Arc::clone(&value));
        Ok(value)
    }

    /// 返回 content_hash，若未计算则返回 None。
    pub fn content_hash(&self) -> Option<&str> {
        self.content_hash.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_id::{ProjectRef, SourceId};

    fn test_source() -> SourceId {
        SourceId::from_memory(ProjectRef::new("test"), "app/page.spg")
    }

    #[test]
    fn test_parsed_content_from_text_json_reuse() {
        let source = test_source();
        let parsed = ParsedContent::from_text(source, r#"{"canvas": {"components": []}}"#.to_string());

        let v1 = parsed.json().expect("first json parse should succeed");
        let v2 = parsed.json().expect("second json parse should reuse");

        // 必须复用同一个 Arc
        assert!(Arc::ptr_eq(&v1, &v2), "json() must reuse the same Arc<Value>");
        assert_eq!(v1.get("canvas").and_then(|c| c.get("components")).is_some(), true);
    }

    #[test]
    fn test_parsed_content_from_json_reuse() {
        let source = test_source();
        let value = serde_json::json!({"version": "1.0"});
        let parsed = ParsedContent::from_json(source, value);

        let v1 = parsed.json().expect("json should be available");
        let v2 = parsed.json().expect("json should be reused");

        assert!(Arc::ptr_eq(&v1, &v2), "from_json must reuse the same Arc<Value>");
    }

    #[test]
    fn test_parsed_content_invalid_json_returns_error() {
        let source = test_source();
        let parsed = ParsedContent::from_text(source, "not valid json".to_string());

        let result = parsed.json();
        assert!(result.is_err(), "invalid JSON should return error");
    }

    #[test]
    fn test_parsed_content_from_bytes_json() {
        let source = test_source();
        let bytes = r#"{"canvas": {}}"#.as_bytes().to_vec();
        let parsed = ParsedContent::from_bytes(source, bytes);

        let v1 = parsed.json().unwrap();
        let v2 = parsed.json().unwrap();
        assert!(Arc::ptr_eq(&v1, &v2), "from_bytes must reuse Arc<Value>");
    }
}
