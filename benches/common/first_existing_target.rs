use std::path::{Path, PathBuf};

/// 在候选相对路径中返回第一个存在的文件。
pub fn first_existing_target(project_dir: &Path, candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(|relative| project_dir.join(relative))
        .find(|path| path.exists())
}
