use anyhow::Context;
use std::path::Path;

/// 将文件恢复为原始内容。
pub fn restore_file(path: &Path, original: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, original).with_context(|| format!("restore {}", path.display()))
}
