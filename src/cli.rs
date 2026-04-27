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
}

impl Cli {
    pub fn is_human(&self) -> bool {
        self.human || self.interactive
    }

    pub fn is_interactive(&self) -> bool {
        self.interactive
    }
}
