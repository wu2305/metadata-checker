use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "metadata-checker",
    about = "Parse page metadata from a low-code platform JSON file",
    long_about = "metadata-checker reads a JSON file containing low-code platform page metadata, validates and extracts structured information, and prints results in either human-readable or machine-friendly (non-human) format. It is intended for integration with AI/LLM toolchains.",
    version
)]
pub struct Cli {
    #[arg(value_name = "FILE", help = "Path to the page metadata JSON file")]
    pub input: Option<PathBuf>,

    #[arg(long, group = "output_mode", help = "Print human-readable output with headers and indentation. When used alone, enters interactive mode.")]
    pub human: bool,

    #[arg(long, group = "output_mode", help = "Print compact machine-friendly JSON output (default)")]
    pub non_human: bool,

    #[arg(long, value_name = "ID", help = "Query detailed info for a specific component ID")]
    pub query: Option<String>,

    #[arg(long, help = "Additionally print priority analysis of defaultValue vs exp vs calcCondition")]
    pub priority: bool,

    #[arg(long, value_name = "DIR", help = "Project directory to scan for cross-file analysis")]
    pub project_dir: Option<PathBuf>,

    #[arg(long, help = "Build/update graph database from project directory")]
    pub build_graph: bool,

    #[arg(long, value_name = "MODEL", help = "Query relationships for a specific model")]
    pub query_model: Option<String>,

    #[arg(long, value_name = "PAGE", help = "Query relationships for a specific page")]
    pub query_page: Option<String>,

    #[arg(long, value_name = "PAGES", num_args = 2, help = "Query cross-file relations between two pages")]
    pub query_cross: Option<Vec<String>>,
}

impl Cli {
    pub fn is_human(&self) -> bool {
        self.human
    }
}
