use metadata_checker::cli;
use metadata_checker::context;
use metadata_checker::dependency::DependencyGraph;
use metadata_checker::explain;
use metadata_checker::graph::GraphDB;
use metadata_checker::output;
use metadata_checker::parser;
use metadata_checker::priority;
use metadata_checker::query;
use metadata_checker::scanner;
use metadata_checker::tbl_single;

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};

/// CLI 入口
///
/// 命令行参数解析后，根据子命令执行：
/// - parse：解析单个 .spg 文件
/// - build-graph：扫描项目目录并构建/更新图数据库
/// - query-model / query-page / query-cross / query-dataflow：图查询
///
/// human 模式支持交互式组件查询（--human 不带 --query 时进入交互）。
fn main() -> Result<()> {
    let args = cli::Cli::parse();
    metadata_checker::graph::set_graph_lock_timeout_ms(args.graph_lock_timeout_ms);

    // Validate budget early for all paths
    if args.budget != "compact" && args.budget != "normal" && args.budget != "full" {
        anyhow::bail!(
            "Invalid budget '{}'. Expected: compact | normal | full",
            args.budget
        );
    }

    // Cross-file graph analysis mode
    if let Some(ref project_dir) = args.project_dir {
        let db_path = args
            .resolve_graph_db_path()
            .unwrap_or_else(|| graph_db_path(project_dir));

        if args.check_graph {
            let status = GraphDB::check_graph_db(&db_path);
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }

        if args.build_graph {
            scanner::scan_project(project_dir, &db_path)?;
            println!("Graph database built at {:?}", db_path);
            return Ok(());
        }

        let graph = match GraphDB::open_or_diagnostic(&db_path) {
            Ok(g) => g,
            Err(out) => {
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
        };

        if let Some(ref model_id) = args.query_model {
            let model_node_id = format!("model:{}", model_id);
            query::query_model(&graph, &model_node_id, args.is_human(), &args.budget)?;
            return Ok(());
        }

        if let Some(ref page_id) = args.query_page {
            query::query_page(&graph, page_id, args.is_human())?;
            return Ok(());
        }

        if let Some(ref pages) = args.query_cross {
            if pages.len() >= 2 {
                query::query_cross(&graph, &pages[0], &pages[1], args.is_human())?;
            }
            return Ok(());
        }

        if let Some(ref dataflow_id) = args.query_dataflow {
            let model_node_id = format!("model:{}", dataflow_id);
            query::query_dataflow(&graph, &model_node_id, args.is_human())?;
            return Ok(());
        }

        if let Some(ref page_logic_id) = args.query_page_logic {
            query::query_page_logic(
                &graph,
                page_logic_id,
                args.project_dir.as_deref(),
                args.is_human(),
                &args.budget,
            )?;
            return Ok(());
        }

        if let Some(ref explain_id) = args.explain {
            explain::explain_node_graph(&graph, explain_id, args.is_human())?;
            return Ok(());
        }

        if let Some(ref context_id) = args.context {
            context::context_node_graph(
                &graph,
                context_id,
                args.depth,
                &args.budget,
                args.is_human(),
            )?;
            return Ok(());
        }

        println!(
            "No query specified. Use --query-model, --query-page, --query-cross, --query-dataflow, --query-page-logic, --explain, or --context."
        );
        return Ok(());
    }

    // --check-graph without --project-dir requires explicit --graph-db-path
    if args.check_graph {
        if let Some(ref db_path) = args.graph_db_path {
            let status = GraphDB::check_graph_db(db_path);
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }
        anyhow::bail!("--check-graph requires either --project-dir or --graph-db-path");
    }

    // Validate args
    if args.input.is_none() && args.project_dir.is_none() {
        eprintln!("Error: either <FILE> or --project-dir must be specified.");
        std::process::exit(1);
    }

    let meta = if let Some(ref input) = args.input {
        parser::parse_file(input)?
    } else {
        // Placeholder when only project-dir is used
        parser::parse_file(std::path::Path::new(""))? // Will not reach here due to early returns below
    };

    // --explain mode
    if let Some(ref explain_id) = args.explain {
        if let Some(spg) = &meta.superpage {
            explain::explain_component_spg(spg, explain_id, args.is_human())?;
        } else if let Some(tbl) = &meta.tbl {
            let out = tbl_single::build_tbl_output(tbl, &args.budget);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        return Ok(());
    }

    // --query mode: print component query and exit
    if let Some(ref target_id) = args.query {
        if let Some(spg) = &meta.superpage {
            let graph = DependencyGraph::new(spg);
            if args.is_human() {
                output::print_component_query_human(spg, &graph, target_id, args.priority)?;
            } else {
                output::print_component_query_json(spg, &graph, target_id, args.priority)?;
            }
        } else if let Some(tbl) = &meta.tbl {
            let out = tbl_single::build_tbl_output(tbl, &args.budget);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        return Ok(());
    }

    // --human without --query: enter interactive mode
    if args.is_human() {
        if let Some(spg) = &meta.superpage {
            run_interactive(spg, args.priority)?;
        } else if let Some(tbl) = &meta.tbl {
            tbl_single::print_tbl_human(tbl)?;
        }
        return Ok(());
    }

    // Default non-human: compact summary, --detail for full report
    let priority_analyses = if args.priority {
        meta.superpage.as_ref().map(priority::analyze_priority)
    } else {
        None
    };

    let priority_slice = priority_analyses.as_deref();

    if let Some(tbl) = &meta.tbl {
        let out = tbl_single::build_tbl_output(tbl, &args.budget);
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if args.detail || args.budget == "full" {
        output::print_non_human_to(&meta, priority_slice, &mut io::stdout())?;
    } else if args.budget == "normal" {
        output::print_non_human_to(&meta, priority_slice, &mut io::stdout())?;
    } else {
        // compact: summary-only brief output
        output::print_summary(&meta, priority_slice)?;
    }

    Ok(())
}

/// 根据项目路径生成隔离的图数据库路径
fn graph_db_path(project_dir: &std::path::Path) -> std::path::PathBuf {
    project_dir.join(".metadata-checker.graphdb")
}

fn run_interactive(
    spg: &metadata_checker::superpage::SuperPageMetadata,
    show_priority: bool,
) -> Result<()> {
    let graph = DependencyGraph::new(spg);

    println!("=== SuperPage Interactive Mode ===");
    println!(
        "Version: {} | Theme: {} | Components: {} | Expressions: {}",
        spg.version.as_deref().unwrap_or("N/A"),
        spg.theme.as_deref().unwrap_or("N/A"),
        spg.components.len(),
        spg.expressions.len()
    );

    let model_names: Vec<&str> = spg.sources.iter().map(|s| s.id.as_str()).collect();
    if !model_names.is_empty() {
        println!("Models: {}", model_names.join(", "));
    }

    let param_names: Vec<&str> = spg.params.iter().map(|p| p.id.as_str()).collect();
    if !param_names.is_empty() {
        println!("Params: {}", param_names.join(", "));
    }

    let expr_comp_ids: Vec<&str> = {
        let set: std::collections::HashSet<&str> = spg
            .expressions
            .iter()
            .map(|e| e.component_id.as_str())
            .collect();
        set.into_iter().collect()
    };
    if !expr_comp_ids.is_empty() {
        println!("Components with expressions: {}", expr_comp_ids.join(", "));
    }

    println!("\nCommands:");
    println!("  <component-id>  Query a component");
    println!("  all             Print full human-readable report");
    println!("  q / quit / exit Exit interactive mode");
    println!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        write!(stdout, "> ")?;
        stdout.flush()?;

        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let cmd = line.trim();

        match cmd {
            "q" | "quit" | "exit" => {
                println!("Goodbye!");
                break;
            }
            "" => continue,
            "all" => {
                let meta = metadata_checker::parser::PageMetadata {
                    input_path: None,
                    page_id: None,
                    page_name: None,
                    version: spg.version.clone(),
                    components: vec![],
                    data_bindings: vec![],
                    settings: Default::default(),
                    raw: spg.raw.clone(),
                    superpage: Some(spg.clone()),
                    tbl: None,
                };
                output::print_human(&meta)?;
                if show_priority {
                    let analyses = metadata_checker::priority::analyze_priority(spg);
                    let report = metadata_checker::priority::format_priority_human(&analyses);
                    println!(
                        "
{}",
                        report
                    );
                }
            }
            target_id => {
                let found = spg.components.iter().any(|c| c.id == target_id);
                if !found {
                    println!("Component '{}' not found.", target_id);
                    continue;
                }
                if let Err(e) =
                    output::print_component_query_human(spg, &graph, target_id, show_priority)
                {
                    println!("Error: {}", e);
                }
            }
        }
    }

    Ok(())
}
