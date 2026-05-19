use anyhow::{Context, Result};
use std::path::Path;
use std::time::SystemTime;

/// 存储层文件元数据。
///
/// 该结构只保留扫描和增量判断需要的稳定字段，避免上层直接依赖
/// `std::fs::Metadata` 这种本地文件系统专用类型。
#[derive(Debug, Clone)]
pub struct StorageFileMetadata {
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// 元数据内容读取抽象。
///
/// M99 的远程会话和 WASM 方向都需要把“从哪里读内容”和“如何解析内容”拆开。
/// 当前先提供本地文件系统实现，并让 parser/scanner 真实消费这个接口。
pub trait StorageProvider {
    /// 读取原始字节。
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>>;

    /// 读取文件元数据。
    fn metadata(&self, path: &Path) -> Result<StorageFileMetadata>;

    /// 读取 UTF-8 文本。
    fn read_to_string(&self, path: &Path) -> Result<String> {
        let bytes = self.read_bytes(path)?;
        String::from_utf8(bytes)
            .with_context(|| format!("Failed to decode UTF-8 from: {}", path.display()))
    }
}

/// 本地文件系统存储实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalStorageProvider;

impl StorageProvider for LocalStorageProvider {
    fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).with_context(|| format!("Failed to read file: {}", path.display()))
    }

    fn metadata(&self, path: &Path) -> Result<StorageFileMetadata> {
        let metadata = std::fs::metadata(path)
            .with_context(|| format!("Failed to read metadata: {}", path.display()))?;
        Ok(StorageFileMetadata {
            modified: metadata.modified().ok(),
            size: metadata.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct MemoryStorageProvider {
        path: PathBuf,
        bytes: Vec<u8>,
    }

    impl StorageProvider for MemoryStorageProvider {
        fn read_bytes(&self, path: &Path) -> Result<Vec<u8>> {
            if path == self.path {
                Ok(self.bytes.clone())
            } else {
                anyhow::bail!("missing memory file: {}", path.display())
            }
        }

        fn metadata(&self, path: &Path) -> Result<StorageFileMetadata> {
            if path == self.path {
                Ok(StorageFileMetadata {
                    modified: Some(SystemTime::UNIX_EPOCH),
                    size: self.bytes.len() as u64,
                })
            } else {
                anyhow::bail!("missing memory file metadata: {}", path.display())
            }
        }
    }

    #[test]
    fn test_storage_provider_read_to_string_uses_provider_bytes() {
        let storage = MemoryStorageProvider {
            path: PathBuf::from("memory.spg"),
            bytes: br#"{"canvas":{}}"#.to_vec(),
        };

        let text = storage
            .read_to_string(Path::new("memory.spg"))
            .expect("memory file should decode");
        assert_eq!(text, r#"{"canvas":{}}"#);
    }
}
