use clap::Parser;
use std::path::PathBuf;

/// 命令行参数定义
///
/// 使用 clap 派生宏定义所有 CLI 参数和子命令。

#[derive(Parser, Debug)]
#[command(
    name = "metadata-checker",
    about = "Parse page metadata from a low-code platform JSON file",
    long_about = "metadata-checker reads a JSON file containing low-code platform page metadata, validates and extracts structured information, and prints results in either human-readable or machine-friendly (non-human) format. It is intended for integration with AI/LLM toolchains.",
    version
)]
/// 根命令参数
pub struct Cli {
    #[arg(value_name = "FILE", help = "Path to the page metadata JSON file")]
    pub input: Option<PathBuf>,

    #[arg(
        long,
        group = "output_mode",
        help = "Enter interactive REPL for expert exploration"
    )]
    pub human: bool,

    #[arg(long, help = "Alias for --human, enter interactive REPL")]
    pub interactive: bool,

    #[arg(
        long,
        group = "output_mode",
        help = "Print machine-friendly single JSON output (default)"
    )]
    pub non_human: bool,

    #[arg(
        long,
        value_name = "ID",
        help = "Query detailed info for a specific component ID"
    )]
    pub query: Option<String>,

    #[arg(
        long,
        help = "Append priority analysis (merged into JSON for non-human, text for human)"
    )]
    pub priority: bool,

    #[arg(
        long,
        help = "Output full raw structure instead of compact summary (non-human mode)"
    )]
    pub detail: bool,

    #[arg(
        long,
        value_name = "DIR",
        help = "Project directory to scan for cross-file analysis"
    )]
    pub project_dir: Option<PathBuf>,

    #[arg(
        long,
        value_name = "PATH",
        help = "Custom graph database path (default: <project-dir>/.metadata-checker.graphdb)"
    )]
    pub graph_db_path: Option<PathBuf>,

    #[arg(
        long,
        value_name = "MS",
        default_value = "10000",
        help = "Graph database lock acquisition timeout in milliseconds (default: 10000)"
    )]
    pub graph_lock_timeout_ms: u64,

    #[arg(long, help = "Check graph database status and output JSON report")]
    pub check_graph: bool,

    #[arg(long, help = "Build/update graph database from project directory")]
    pub build_graph: bool,

    #[arg(
        long,
        value_name = "MODEL",
        help = "Query relationships for a specific model (requires --project-dir)"
    )]
    pub query_model: Option<String>,

    #[arg(
        long,
        value_name = "PAGE",
        help = "Query relationships for a specific page (requires --project-dir)"
    )]
    pub query_page: Option<String>,

    #[arg(
        long,
        value_name = "PAGES",
        num_args = 2,
        help = "Query cross-file relations between two pages (requires --project-dir)"
    )]
    pub query_cross: Option<Vec<String>>,

    #[arg(
        long,
        value_name = "MODEL",
        help = "Expand and query a DataFlow model's internal subgraph (requires --project-dir)"
    )]
    pub query_dataflow: Option<String>,

    #[arg(
        long,
        value_name = "ID",
        help = "Explain a component, action, model, field, page, or dataflow by ID"
    )]
    pub explain: Option<String>,

    #[arg(
        long,
        value_name = "ID",
        help = "Output minimal closure context around a target node (requires --project-dir)"
    )]
    pub context: Option<String>,

    #[arg(
        long,
        value_name = "N",
        default_value = "1",
        help = "Context depth for --context (default: 1)"
    )]
    pub depth: usize,

    #[arg(
        long,
        value_name = "BUDGET",
        default_value = "normal",
        help = "Output budget: compact | normal | full (default: normal)"
    )]
    pub budget: String,

    #[arg(
        long,
        value_name = "PAGE",
        help = "Query page-level logic summary (requires --project-dir)"
    )]
    pub query_page_logic: Option<String>,
}

impl Cli {
    pub fn is_human(&self) -> bool {
        self.human || self.interactive
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }

    /// 解析图数据库路径
    pub fn resolve_graph_db_path(&self) -> Option<PathBuf> {
        if let Some(ref explicit) = self.graph_db_path {
            return Some(explicit.clone());
        }
        self.project_dir
            .as_ref()
            .map(|dir| dir.join(".metadata-checker.graphdb"))
    }
}
