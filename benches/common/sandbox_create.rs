use anyhow::{Context, Result, ensure};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static WORKSPACE_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// 真实项目 benchmark 使用的临时工作区。
pub struct BenchWorkspace {
    pub root: PathBuf,
    pub project_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Drop for BenchWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 创建只包含 `.spg` / `.tbl` 的真实项目 sandbox。
pub fn create_workspace(label: &str, source_project_dir: &Path) -> Result<BenchWorkspace> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = WORKSPACE_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!("metadata-checker-bench-{label}-{nanos}-{seq}"));
    let project_dir = root.join("project");
    let db_path = root.join("metadata-checker.graphdb");
    copy_metadata_tree(source_project_dir, &project_dir)
        .with_context(|| format!("copy real project metadata into {}", project_dir.display()))?;
    Ok(BenchWorkspace {
        root,
        project_dir,
        db_path,
    })
}

/// 创建已完成初始图构建的 sandbox。
pub fn create_indexed_workspace(label: &str, source_project_dir: &Path) -> Result<BenchWorkspace> {
    let workspace = create_workspace(label, source_project_dir)?;
    let report = ProjectIndexer::scan(&workspace.project_dir, &workspace.db_path)
        .map_err(|error| anyhow::anyhow!("initial scan for {label}: {error}"))?;
    ensure!(
        report.dirty > 0 || report.indexed > 0,
        "initial scan should index real project metadata"
    );
    Ok(workspace)
}

fn copy_metadata_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("read source directory {}", source.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if should_skip_dir(name) {
            continue;
        }
        let next_destination = destination.join(entry.file_name());
        if path.is_dir() {
            copy_metadata_tree(&path, &next_destination)?;
        } else if is_metadata_file(&path) {
            if let Some(parent) = next_destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &next_destination).with_context(|| {
                format!(
                    "copy metadata file {} -> {}",
                    path.display(),
                    next_destination.display()
                )
            })?;
        }
    }
    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | ".metadata-checker.graphdb"
    )
}

fn is_metadata_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension == "spg" || extension == "tbl")
}
