#[cfg(feature = "cli-local")]
use anyhow::Result;
#[cfg(feature = "cli-local")]
use std::path::Path;

/// 项目目录扫描模块
///
/// 递归扫描项目目录下的所有 .spg 和 .tbl 文件：
/// - .spg：解析组件、表达式、动作，构建 Page/Component/Model/Field 节点
/// - .tbl：解析 App 类型（可写）和 DataFlow 类型（只读加工流程）
///
/// 支持增量更新：对比文件 mtime/size/hash，只重新处理变更文件。
/// Scan a project directory and build/update the graph database.
#[cfg(feature = "cli-local")]
pub fn scan_project(project_dir: &Path, db_path: &Path) -> Result<()> {
    let report = indexer::ProjectIndexer::scan(project_dir, db_path)?;
    eprintln!(
        "Indexed {} files | Unchanged: {} | Dirty: {} | Deleted: {}",
        report.indexed, report.unchanged, report.dirty, report.deleted
    );
    if report.dirty == 0 && report.deleted == 0 {
        eprintln!("No graph changes detected; skip persistence");
    }
    Ok(())
}

#[cfg(feature = "cli-local")]
pub mod indexer;
mod spg;
mod tbl;
/// 递归收集目录下所有 .spg 和 .tbl 文件
mod utils;

pub use spg::process_spg_file_from_value;
pub use tbl::process_tbl_file_from_string;
pub use utils::{add_edge_with_meta, add_node, resolve_reference_path};
