use anyhow::{Context, Result};
use std::path::Path;
use std::time::SystemTime;

// ============================================================================
// M36 DocumentProvider：内容读取抽象
// ============================================================================

/// 文档内容读取抽象。
///
/// M36 的边界：只负责 `read_bytes` / `read_to_string` / `metadata`，
/// 不增加 `write_bytes`；写入 session/cache 放到 M40。
/// 目录遍历（discover files）放到 M39 `ProjectIndexer`。
///
/// 长期意义上的 StorageProvider（redb / IndexedDB / memory index）
/// 由 `GraphStore` / `IndexStore` 占位，M36 不替换现有 redb。
pub trait DocumentProvider {
    /// 读取原始字节。
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>>;

    /// 读取文件元数据。
    fn metadata(&self, path: &Path) -> Result<DocumentFileMetadata>;

    /// 读取 UTF-8 文本。
    fn read_to_string(&self, path: &Path) -> Result<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes)
            .with_context(|| format!("Failed to decode UTF-8 from: {}", path.display()))
    }
}

/// 文档层文件元数据。
///
/// 只保留扫描和增量判断需要的稳定字段，避免上层直接依赖
/// `std::fs::Metadata` 这种本地文件系统专用类型。
#[derive(Debug, Clone)]
pub struct DocumentFileMetadata {
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// 本地文件系统文档读取实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalDocumentProvider;

impl DocumentProvider for LocalDocumentProvider {
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).with_context(|| format!("Failed to read file: {}", path.display()))
    }

    fn metadata(&self, path: &Path) -> Result<DocumentFileMetadata> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to read metadata: {}", path.display()))?;
        Ok(DocumentFileMetadata {
            modified: metadata.modified().ok(),
            size: metadata.len(),
        })
    }
}

// ============================================================================
// 向后兼容别名（M36 保留）
// ============================================================================

/// 兼容别名：旧代码中的 `StorageProvider` 等同于 `DocumentProvider`。
///
/// M36 重命名后保留此别名，避免一次性改动所有调用方。
/// 新代码应优先使用 `DocumentProvider`。
pub trait StorageProvider: DocumentProvider {}

impl<T: DocumentProvider> StorageProvider for T {}

/// 兼容别名：旧代码中的 `LocalStorageProvider` 等同于 `LocalDocumentProvider`。
pub use LocalDocumentProvider as LocalStorageProvider;

/// 兼容别名：旧代码中的 `StorageFileMetadata` 等同于 `DocumentFileMetadata`。
pub type StorageFileMetadata = DocumentFileMetadata;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct MemoryDocumentProvider {
        path: PathBuf,
        bytes: Vec<u8>,
    }

    impl DocumentProvider for MemoryDocumentProvider {
        fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
            if path == self.path {
                Ok(self.bytes.clone())
            } else {
                anyhow::bail!("missing memory file: {}", path.display())
            }
        }

        fn metadata(&self, path: &Path) -> Result<DocumentFileMetadata> {
            if path == self.path {
                Ok(DocumentFileMetadata {
                    modified: Some(SystemTime::UNIX_EPOCH),
                    size: self.bytes.len() as u64,
                })
            } else {
                anyhow::bail!("missing memory file metadata: {}", path.display())
            }
        }
    }

    #[test]
    fn test_document_provider_read_to_string_uses_provider_bytes() {
        let storage = MemoryDocumentProvider {
            path: PathBuf::from("memory.spg"),
            bytes: br#"{"canvas":{}}"#.to_vec(),
        };

        let text = storage
            .read_to_string(Path::new("memory.spg"))
            .expect("memory file should decode");
        assert_eq!(text, r#"{"canvas":{}}"#);
    }

    #[test]
    fn test_local_document_provider_is_storage_provider_alias() {
        fn takes_storage(
            _: &dyn StorageProvider,
            _: &dyn DocumentProvider,
            _: &dyn StorageProvider,
            _: &dyn DocumentProvider,
            _: &dyn StorageProvider,
            _: &dyn DocumentProvider,
            _: &dyn StorageProvider,
            _: &dyn DocumentProvider,
        ) {
        }
        let local = LocalDocumentProvider;
        takes_storage(
            &local, &local, &local, &local, &local, &local, &local, &local,
        );
    }
}
