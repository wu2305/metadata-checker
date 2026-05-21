use metadata_checker::cli;
use metadata_checker::dependency::DependencyGraph;
use metadata_checker::explain;
use metadata_checker::graph::GraphDB;
use metadata_checker::output;
use metadata_checker::parser;
use metadata_checker::priority;
use metadata_checker::scanner;
use metadata_checker::tool_contract::{self, InvocationAdapter};

use anyhow::Result;
use clap::Parser;
use std::io::{self, Write};

/// 通过 CLI adapter 执行 runtime 工具调用。
fn run_cli_runtime_tool(
    runtime: &mut metadata_checker::runtime::GraphRuntime,
    input: cli::CliToolInput,
) -> Result<serde_json::Value> {
    let adapter = cli::CliAdapter;
    let invocation = adapter.parse_input(input)?;
    let req = metadata_checker::runtime::RuntimeQueryRequest {
        command: invocation.command,
        target: invocation.target.unwrap_or_default(),
        budget: invocation.budget.unwrap_or_else(|| "normal".to_string()),
        human: invocation.human,
        intent: invocation.intent,
        page_scope: invocation.page_scope,
        depth: invocation.depth,
        check_reload: invocation.check_reload,
    };
    Ok(runtime.query(req)?.result)
}

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

    // M38: 统一参数校验
    if let Err(err) = tool_contract::validate_budget(&args.budget) {
        anyhow::bail!("{}", err);
    }

    // Stdio server mode (M24)
    if args.serve_stdio {
        let db_path = match args.resolve_graph_db_path() {
            Some(p) => p,
            None => anyhow::bail!("--serve-stdio requires --graph-db-path or --project-dir"),
        };
        let project_dir = args.project_dir.as_deref();
        metadata_checker::stdio_server::run_stdio_server(&db_path, project_dir)?;
        return Ok(());
    }

    // graphdb-only runtime lifecycle commands do not need a project directory.
    if args.project_dir.is_none() && (args.status || args.reload_graph || args.check_reload) {
        let db_path = args.graph_db_path.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--status/--reload-graph/--check-reload requires --graph-db-path when --project-dir is absent")
        })?;
        let mut runtime = metadata_checker::runtime::GraphRuntime::load(db_path)?;
        let command = if args.status {
            metadata_checker::tool_contract::ToolCommand::Status
        } else if args.reload_graph {
            metadata_checker::tool_contract::ToolCommand::ReloadGraph
        } else {
            metadata_checker::tool_contract::ToolCommand::CheckReload
        };
        let result = run_cli_runtime_tool(
            &mut runtime,
            cli::CliToolInput {
                command,
                target: None,
                budget: args.budget.clone(),
                human: false,
                intent: None,
                page_scope: None,
                depth: None,
                check_reload: false,
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
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

        let mut runtime = match metadata_checker::runtime::GraphRuntime::load_with_project_dir(
            &db_path,
            Some(project_dir),
        ) {
            Ok(r) => r,
            Err(_e) => {
                let out = metadata_checker::graph::GraphDB::check_graph_db(&db_path);
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
        };
        let _graph = &runtime.graph;

        if args.status {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::Status,
                    target: None,
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if args.reload_graph {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::ReloadGraph,
                    target: None,
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if args.check_reload {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::CheckReload,
                    target: None,
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref model_id) = args.query_model {
            let model_node_id = if model_id.starts_with("model:") {
                model_id.clone()
            } else {
                format!("model:{}", model_id)
            };
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::QueryModel,
                    target: Some(model_node_id),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref page_id) = args.query_page {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::QueryPage,
                    target: Some(page_id.clone()),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref pages) = args.query_cross {
            if pages.len() >= 2 {
                let target = format!("{}, {}", pages[0], pages[1]);
                let result = run_cli_runtime_tool(
                    &mut runtime,
                    cli::CliToolInput {
                        command: metadata_checker::tool_contract::ToolCommand::QueryCross,
                        target: Some(target),
                        budget: args.budget.clone(),
                        human: args.is_human(),
                        intent: None,
                        page_scope: None,
                        depth: None,
                        check_reload: false,
                    },
                )?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            return Ok(());
        }

        if let Some(ref dataflow_id) = args.query_dataflow {
            let model_node_id = if dataflow_id.starts_with("model:") {
                dataflow_id.clone()
            } else {
                format!("model:{}", dataflow_id)
            };
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::QueryDataflow,
                    target: Some(model_node_id),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref page_logic_id) = args.query_page_logic {
            if args.is_human() {
                // M38：human 模式暂时保留旧路径，待统一 human 渲染器后再迁到 runtime
                metadata_checker::query::query_page_logic(
                    &runtime.graph,
                    page_logic_id,
                    args.project_dir.as_deref(),
                    true,
                    &args.budget,
                )?;
            } else {
                let result = run_cli_runtime_tool(
                    &mut runtime,
                    cli::CliToolInput {
                        command: metadata_checker::tool_contract::ToolCommand::QueryPageLogic,
                        target: Some(page_logic_id.clone()),
                        budget: args.budget.clone(),
                        human: false,
                        intent: None,
                        page_scope: None,
                        depth: None,
                        check_reload: false,
                    },
                )?;
                println!("{}", serde_json::to_string_pretty(&result)?);
            }
            return Ok(());
        }

        if let Some(ref explain_id) = args.explain {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::Explain,
                    target: Some(explain_id.clone()),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref explain_target) = args.explain_condition {
            let intent = explain::TraversalIntent::parse(&args.intent)?;
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::ExplainCondition,
                    target: Some(explain_target.clone()),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: Some(intent.as_str().to_string()),
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref context_id) = args.context {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::Context,
                    target: Some(context_id.clone()),
                    budget: args.budget.clone(),
                    human: args.is_human(),
                    intent: None,
                    page_scope: None,
                    depth: Some(args.depth),
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref keyword) = args.find_page {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::FindPage,
                    target: Some(keyword.clone()),
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref keyword) = args.find_model {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::FindModel,
                    target: Some(keyword.clone()),
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref keyword) = args.find_component {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::FindComponent,
                    target: Some(keyword.clone()),
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: None,
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref resolve_args) = args.resolve_model {
            let (page_id, local_model_id) = match resolve_args.len() {
                2 => (resolve_args[0].clone(), resolve_args[1].clone()),
                1 if args.resolve_model_page.is_some() => (
                    args.resolve_model_page.clone().unwrap(),
                    resolve_args[0].clone(),
                ),
                _ => {
                    anyhow::bail!(
                        "--resolve-model requires either 'PAGE_ID MODEL_ID' or --resolve-model-page"
                    );
                }
            };
            let target = format!("{}|{}", page_id, local_model_id);
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::QueryPageLogic,
                    target: Some(target),
                    budget: args.budget.clone(),
                    human: false,
                    intent: None,
                    page_scope: Some(page_id),
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        if let Some(ref advise_target) = args.advise_query {
            let result = run_cli_runtime_tool(
                &mut runtime,
                cli::CliToolInput {
                    command: metadata_checker::tool_contract::ToolCommand::AdviseQuery,
                    target: Some(advise_target.clone()),
                    budget: args.budget.clone(),
                    human: false,
                    intent: Some(
                        args.question_kind
                            .clone()
                            .unwrap_or_else(|| "auto".to_string()),
                    ),
                    page_scope: args.advise_query_page.clone(),
                    depth: None,
                    check_reload: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }

        println!(
            "No query specified. Use --query-model, --query-page, --query-cross, --query-dataflow, --query-page-logic, --explain, --context, --find-page, --find-model, --find-component, --resolve-model, or --advise-query."
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
            let out = output::tbl::build_tbl_output(tbl, &args.budget);
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
            let out = output::tbl::build_tbl_output(tbl, &args.budget);
            println!("{}", serde_json::to_string_pretty(&out)?);
        }
        return Ok(());
    }

    // --human without --query: enter interactive mode
    if args.is_human() {
        if let Some(spg) = &meta.superpage {
            run_interactive(spg, args.priority)?;
        } else if let Some(tbl) = &meta.tbl {
            output::tbl::print_tbl_human(tbl)?;
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
        let out = output::tbl::build_tbl_output(tbl, &args.budget);
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
