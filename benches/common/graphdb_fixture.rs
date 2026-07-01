use anyhow::Context;
use std::path::Path;

/// 复制 graphdb 并破坏 header，供损坏路径 benchmark 使用。
pub fn corrupt_graphdb_header(source: &Path, destination: &Path) -> anyhow::Result<()> {
    let mut bytes =
        std::fs::read(source).with_context(|| format!("read graphdb {}", source.display()))?;
    for byte in bytes.iter_mut().take(64) {
        *byte = 0;
    }
    std::fs::write(destination, bytes)
        .with_context(|| format!("write corrupt graphdb {}", destination.display()))
}
