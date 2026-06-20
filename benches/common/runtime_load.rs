use super::sandbox_create::BenchWorkspace;
use anyhow::{Context, Result};
use metadata_checker::runtime::GraphRuntime;

/// 加载 warm runtime，供 CRUD 闭环 benchmark 复用。
pub fn load_warm_runtime(workspace: &BenchWorkspace) -> Result<GraphRuntime> {
    GraphRuntime::load_with_project_dir(&workspace.db_path, Some(&workspace.project_dir))
        .context("load warm runtime for CRUD benchmark")
}
