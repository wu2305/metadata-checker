use anyhow::{Context, Result, bail};
use metadata_checker::perf_report::build_core_profile_report;
use std::path::PathBuf;

const DEFAULT_OUTPUT: &str = "target/m51-profile/core-profile.json";

fn main() -> Result<()> {
    let args = Args::parse(std::env::args().skip(1))?;
    let report = build_core_profile_report(&args.project_dir, args.sample_count)?;
    if let Some(parent) = args.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let content = serde_json::to_vec_pretty(&report).context("serialize core profile report")?;
    std::fs::write(&args.output, content)
        .with_context(|| format!("write core profile report {}", args.output.display()))?;
    println!("{}", args.output.display());
    Ok(())
}

struct Args {
    project_dir: PathBuf,
    output: PathBuf,
    sample_count: usize,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut project_dir: Option<PathBuf> = None;
        let mut output = PathBuf::from(DEFAULT_OUTPUT);
        let mut sample_count = 1usize;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--project-dir" => {
                    let value = args.next().context("--project-dir requires a value")?;
                    project_dir = Some(PathBuf::from(value));
                }
                "--output" => {
                    let value = args.next().context("--output requires a value")?;
                    output = PathBuf::from(value);
                }
                "--sample-count" => {
                    let value = args.next().context("--sample-count requires a value")?;
                    sample_count = value.parse().context("parse --sample-count")?;
                    if sample_count == 0 {
                        bail!("--sample-count must be greater than 0");
                    }
                }
                "--help" | "-h" => {
                    println!("{}", usage());
                    std::process::exit(0);
                }
                other => bail!("unknown argument {other}\n{}", usage()),
            }
        }

        Ok(Self {
            project_dir: project_dir.context("--project-dir is required")?,
            output,
            sample_count,
        })
    }
}

fn usage() -> &'static str {
    "Usage: m51_core_profile_report --project-dir PATH [--output PATH] [--sample-count N]"
}
