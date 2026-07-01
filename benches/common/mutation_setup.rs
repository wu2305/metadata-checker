use crate::restore_file::restore_file;
use crate::sandbox_create::BenchWorkspace;
use anyhow::{Context, Result};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::Path;

/// 将元数据文件恢复为基线内容并执行一次 scan，供 mutation benchmark setup 复用。
pub fn restore_metadata_baseline(
    workspace: &BenchWorkspace,
    path: &Path,
    original: &[u8],
) -> Result<()> {
    restore_file(path, original)?;
    ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
        .context("restore metadata baseline scan")?;
    Ok(())
}
