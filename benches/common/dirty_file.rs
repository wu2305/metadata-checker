use anyhow::Context;
use std::path::Path;

/// 在原始文件末尾追加确定性空白，制造可重复 dirty 变体。
pub fn write_dirty_variant(path: &Path, original: &[u8], iteration: usize) -> anyhow::Result<()> {
    let mut content = original.to_vec();
    content.push(b'\n');
    content.extend(std::iter::repeat(b' ').take((iteration % 31) + 1));
    std::fs::write(path, content).with_context(|| format!("write dirty variant {}", path.display()))
}
