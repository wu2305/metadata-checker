use anyhow::Result;
#[cfg(feature = "cli-local")]
use std::path::Path;
#[cfg(feature = "cli-local")]
use crate::graph_redb::GraphDB;

/// 项目目录扫描模块
///
/// 递归扫描项目目录下的所有 .spg 和 .tbl 文件：
/// - .spg：解析组件、表达式、动作，构建 Page/Component/Model/Field 节点
/// - .tbl：解析 App 类型（可写）和 DataFlow 类型（只读加工流程）
///
/// 支持增量更新：对比文件 mtime/size/hash，只重新处理变更文件。
/// Scan a project directory and build/update the graph database.
/// 构建报告（含结构化诊断 envelope）
#[cfg(feature = "cli-local")]
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScanReport {
    pub indexed: usize,
    pub unchanged: usize,
    pub dirty: usize,
    pub deleted: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub diagnostics: Vec<crate::output::Diagnostic>,
}

#[cfg(feature = "cli-local")]
pub fn scan_project(project_dir: &Path, db_path: &Path) -> Result<()> {
    let report = scan_project_with_report(project_dir, db_path)?;
    if !report.diagnostics.is_empty() {
        println!("{}", serde_json::to_string(&report).unwrap_or_default());
    }
    Ok(())
}

#[cfg(feature = "cli-local")]
pub fn scan_project_with_report(project_dir: &Path, db_path: &Path) -> Result<ScanReport> {
    let index_report = indexer::ProjectIndexer::scan(project_dir, db_path)?;
    let graph = GraphDB::open(db_path)?;
    let diagnostics = graph.hydrate_diagnostics().to_diagnostics();
    Ok(ScanReport {
        indexed: index_report.indexed,
        unchanged: index_report.unchanged,
        dirty: index_report.dirty,
        deleted: index_report.deleted,
        node_count: graph.graph.node_count(),
        edge_count: graph.graph.edge_count(),
        diagnostics,
    })
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
