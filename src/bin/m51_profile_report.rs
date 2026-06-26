use anyhow::{Context, Result, bail, ensure};
use metadata_checker::graph::GraphDB;
use metadata_checker::perf_report::{PageLogicProfileScenario, build_page_logic_profile_report};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_OUTPUT: &str = "target/m51-profile/page-logic-profile.json";

fn main() -> Result<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let db_path = options
        .graph_db_path
        .unwrap_or_else(|| temp_graph_db_path("m51-profile-report"));

    let report = ProjectIndexer::scan(&options.project_dir, &db_path)
        .with_context(|| format!("scan project {}", options.project_dir.display()))?;
    ensure!(
        report.dirty > 0 || report.indexed > 0 || report.unchanged > 0,
        "project scan did not observe metadata files"
    );

    let graph = GraphDB::open(&db_path).with_context(|| format!("open {}", db_path.display()))?;
    let profile_report = build_page_logic_profile_report(
        &graph,
        Some(&options.project_dir),
        &options.scenarios,
        options.sample_count,
    )?;

    if let Some(parent) = options.output.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    let content = serde_json::to_vec_pretty(&profile_report).context("serialize profile report")?;
    std::fs::write(&options.output, content)
        .with_context(|| format!("write {}", options.output.display()))?;
    println!("{}", options.output.display());
    Ok(())
}

struct Options {
    project_dir: PathBuf,
    graph_db_path: Option<PathBuf>,
    output: PathBuf,
    sample_count: usize,
    scenarios: Vec<PageLogicProfileScenario>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut project_dir = None;
        let mut graph_db_path = None;
        let mut output = PathBuf::from(DEFAULT_OUTPUT);
        let mut sample_count = 3_usize;
        let mut scenarios = Vec::new();
        let mut args = args.into_iter();

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--project-dir" => project_dir = Some(PathBuf::from(next_value(&mut args, &arg)?)),
                "--graph-db-path" => {
                    graph_db_path = Some(PathBuf::from(next_value(&mut args, &arg)?));
                }
                "--output" => output = PathBuf::from(next_value(&mut args, &arg)?),
                "--sample-count" => {
                    sample_count = next_value(&mut args, &arg)?
                        .parse()
                        .context("--sample-count must be an integer")?;
                }
                "--scenario" => {
                    scenarios.push(parse_scenario(&next_value(&mut args, &arg)?)?);
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                _ => bail!("unknown argument: {arg}"),
            }
        }

        let project_dir = project_dir.context("--project-dir is required")?;
        ensure!(sample_count > 0, "--sample-count must be greater than 0");
        if scenarios.is_empty() {
            scenarios = default_scenarios();
        }

        Ok(Self {
            project_dir,
            graph_db_path,
            output,
            sample_count,
            scenarios,
        })
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .with_context(|| format!("missing value for {flag}"))
}

fn parse_scenario(value: &str) -> Result<PageLogicProfileScenario> {
    let parts: Vec<&str> = value.split('|').collect();
    ensure!(
        parts.len() == 3,
        "--scenario must use name|page_id|budget format"
    );
    Ok(PageLogicProfileScenario::new(parts[0], parts[1], parts[2]))
}

fn default_scenarios() -> Vec<PageLogicProfileScenario> {
    vec![
        PageLogicProfileScenario::new(
            "query_page_logic_contract_full",
            "page:app/销售.app/销售/合同协议.spg",
            "full",
        ),
        PageLogicProfileScenario::new(
            "query_page_logic_contract_normal",
            "page:app/销售.app/销售/合同协议.spg",
            "normal",
        ),
        PageLogicProfileScenario::new(
            "query_page_logic_member_registered_compact",
            "page:app/售后.app/绑定车辆/会员已注册.spg",
            "compact",
        ),
    ]
}

fn temp_graph_db_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-{label}-{}-{nanos}.graphdb",
        std::process::id()
    ))
}

fn print_help() {
    println!(
        "Usage: m51_profile_report --project-dir PATH [options]\n\
\n\
Options:\n\
  --project-dir PATH              Real project metadata directory\n\
  --graph-db-path PATH            Optional graphdb path; defaults to a temp graphdb\n\
  --output PATH                   Output JSON path; defaults to {DEFAULT_OUTPUT}\n\
  --sample-count N                Samples per scenario; defaults to 3\n\
  --scenario name|page_id|budget  Scenario; may be repeated\n"
    );
}
