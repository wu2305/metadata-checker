#[cfg(feature = "cli-local")]
use crate::graph_redb::GraphDB;
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
pub fn scan_project(project_dir: &Path, db_path: &Path) -> Result<ScanReport> {
    scan_project_with_report(project_dir, db_path)
}

#[cfg(feature = "cli-local")]
pub fn scan_project_with_report(project_dir: &Path, db_path: &Path) -> Result<ScanReport> {
    scan_project_with_report_internal(project_dir, db_path, None)
}

#[cfg(feature = "cli-local")]
pub fn scan_project_with_report_for_project(
    project_dir: &Path,
    db_path: &Path,
    project_binding: &crate::ownership::ProjectBinding,
) -> Result<ScanReport> {
    scan_project_with_report_internal(project_dir, db_path, Some(project_binding))
}

#[cfg(feature = "cli-local")]
fn scan_project_with_report_internal(
    project_dir: &Path,
    db_path: &Path,
    project_binding: Option<&crate::ownership::ProjectBinding>,
) -> Result<ScanReport> {
    let index_with_diags = match project_binding {
        Some(binding) => indexer::ProjectIndexer::scan_with_diagnostics_for_project(
            project_dir,
            db_path,
            binding,
        )?,
        None => indexer::ProjectIndexer::scan_with_diagnostics(project_dir, db_path)?,
    };
    let index_report = index_with_diags.report;
    let mut diagnostics = index_with_diags.diagnostics;
    // hydrate 诊断只来自 redb 的 v2 shadow 层；grafeo 直写无该层，
    // 跳过二次打开（对 grafeo 也省掉一次全量 lookup 重建）。
    if !crate::graph_store::is_grafeo_db_path(db_path) {
        let graph = match project_binding {
            Some(binding) => GraphDB::open_readonly_with_ownership(db_path, binding)?,
            None => GraphDB::open(db_path)?,
        };
        diagnostics.extend(graph.hydrate_diagnostics().to_diagnostics());
    }
    Ok(ScanReport {
        indexed: index_report.indexed,
        unchanged: index_report.unchanged,
        dirty: index_report.dirty,
        deleted: index_report.deleted,
        node_count: index_with_diags.node_count,
        edge_count: index_with_diags.edge_count,
        diagnostics,
    })
}

#[cfg(feature = "cli-local")]
#[derive(Debug, Clone)]
pub struct IndexReportWithDiagnostics {
    pub report: crate::graph_store::IndexReport,
    pub diagnostics: Vec<crate::output::Diagnostic>,
    /// 提交后图内项目节点数（后端 store 口径；grafeo 侧不含 IndexState meta 节点）
    pub node_count: usize,
    /// 提交后图内边数
    pub edge_count: usize,
}

#[cfg(feature = "cli-local")]
pub mod indexer;
mod spg;
mod tbl;
/// 递归收集目录下所有 .spg 和 .tbl 文件
mod utils;

#[cfg(any(test, feature = "cli-local"))]
pub use spg::scan_raw_diagnostics;
pub use spg::{
    PageIdentityMode, process_spg_file_from_value, process_spg_file_from_value_with_identity,
};
pub use tbl::process_tbl_file_from_string;
pub use utils::{add_edge_with_meta, add_identified_node, add_node, resolve_reference_path};
