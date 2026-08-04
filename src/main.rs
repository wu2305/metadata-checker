use metadata_checker::cli;
use metadata_checker::dependency::DependencyGraph;
use metadata_checker::explain;
use metadata_checker::graph::GraphDB;
use metadata_checker::output;
use metadata_checker::parser;
use metadata_checker::priority;
use metadata_checker::scanner;
use metadata_checker::session::reqwest_provider::{
    is_session_auth_error, sanitize_session_error_message,
};
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

fn session_refresh_filter_from_cli(
    args: &cli::Cli,
) -> Option<metadata_checker::session::SessionRefreshFilter> {
    let module = args.remote_module.clone();
    let source_path = args.remote_source.clone();
    let file_id = args.remote_file.clone();
    if module.is_none() && source_path.is_none() && file_id.is_none() {
        return None;
    }
    Some(metadata_checker::session::SessionRefreshFilter {
        module,
        source_path,
        file_id,
        current_source_path: None,
    })
}

fn has_session_query_request(args: &cli::Cli) -> bool {
    args.query_model.is_some()
        || args.query_page.is_some()
        || args.query_cross.is_some()
        || args.query_dataflow.is_some()
        || args.query_page_logic.is_some()
        || args.explain.is_some()
        || args.explain_condition.is_some()
        || args.context.is_some()
        || args.find_page.is_some()
        || args.find_model.is_some()
        || args.find_component.is_some()
        || args.resolve_model.is_some()
        || args.advise_query.is_some()
}

/// 执行一次三动词表面调用：裸名归一 -> 路由展开 -> 逐条执行 -> 合并输出。
///
/// 合并保留主调用的 `summary` / `details` / `evidence` / `diagnostics` 顶层形状，补充调用
/// 的 `details` 折进 `details.<merge_key>`。模型被教的输出契约不变，只是一次能拿到的事实
/// 更全——这正是让 `--explain` 与 `--explain-condition` 的区分不可观测的手段。
fn run_surface(
    runtime: &mut metadata_checker::runtime::GraphRuntime,
    args: &cli::Cli,
    surface: metadata_checker::route::Surface,
    raw_target: &str,
) -> Result<serde_json::Value> {
    use metadata_checker::route::{self, BareTargetResolution};
    use metadata_checker::tool_contract::ToolCommand;

    let mut diagnostics: Vec<String> = Vec::new();
    let mut target = raw_target.trim().to_string();

    // `--find` 本身就是定位动词，不需要先定位。
    if surface != route::Surface::Find && !route::has_type_prefix(&target) {
        let found = run_cli_runtime_tool(
            runtime,
            cli::CliToolInput {
                command: ToolCommand::Find,
                target: Some(target.clone()),
                budget: args.budget.clone(),
                human: false,
                intent: None,
                page_scope: None,
                depth: None,
                check_reload: false,
            },
        )?;
        match route::resolve_bare_target(&target, &found) {
            BareTargetResolution::Resolved {
                target: resolved,
                diagnostic,
            } => {
                diagnostics.push(diagnostic);
                target = resolved;
            }
            // 歧义和查不到都不猜：把候选如实交回去，模型下一轮可以直接用规范 target。
            // 这比返回一个 INVALID_TARGET 让它自己编路径要短一轮，也不会编出
            // `comp:app/未知页面.spg|button1` 这种东西。
            BareTargetResolution::Ambiguous { candidates } => {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": {
                        "code": "AMBIGUOUS_TARGET",
                        "message": format!("'{target}' 匹配到多个节点，请用其中一个规范 target 重试"),
                    },
                    "candidate_targets": candidates,
                    "diagnostics": diagnostics,
                }));
            }
            BareTargetResolution::NotFound => {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": {
                        "code": "TARGET_NOT_FOUND",
                        "message": format!("找不到 '{target}'，先用 --find 定位"),
                    },
                    "candidate_targets": found
                        .get("details")
                        .and_then(|details| details.get("matches"))
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                    "diagnostics": diagnostics,
                }));
            }
        }
    }

    let plan = route::route(surface, &target, args.depth)?;
    let mut merged: Option<serde_json::Value> = None;

    for call in &plan.calls {
        let outcome = run_cli_runtime_tool(
            runtime,
            cli::CliToolInput {
                command: call.command,
                target: Some(call.target.clone()),
                budget: args.budget.clone(),
                human: args.is_human(),
                // 表面层不再要求模型选 intent，但写了就必须透传：`--intent writer` 不只是
                // 收窄，auto 遍历根本不产出 writer_facts。丢掉它等于悄悄换掉了用户要的答案。
                intent: Some(args.intent.clone()),
                page_scope: None,
                depth: args.depth,
                check_reload: false,
            },
        );

        match (outcome, call.merge_key) {
            (Ok(value), None) => merged = Some(value),
            (Ok(value), Some(key)) => {
                if let Some(base) = merged.as_mut() {
                    merge_supplement(base, key, value);
                }
            }
            (Err(error), None) => return Err(error),
            // 补充块不适用于当前节点类型是正常的，记一条 diagnostic 就够了；
            // 让它拖垮整条命令等于换个方式重造「选错动词就一无所获」。
            (Err(error), Some(key)) => {
                diagnostics.push(format!("SUPPLEMENT_UNAVAILABLE: {key}: {error}"));
            }
        }
    }

    let mut result = merged.unwrap_or(serde_json::Value::Null);
    if !diagnostics.is_empty() {
        append_diagnostics(&mut result, &diagnostics);
    }
    Ok(result)
}

/// 把补充调用的 details 折进主输出的 `details.<key>`，并合并它的 evidence。
fn merge_supplement(base: &mut serde_json::Value, key: &str, supplement: serde_json::Value) {
    let Some(object) = base.as_object_mut() else {
        return;
    };
    if let Some(details) = supplement.get("details").cloned() {
        let slot = object
            .entry("details")
            .or_insert_with(|| serde_json::json!({}));
        if let Some(target) = slot.as_object_mut() {
            target.insert(key.to_string(), details);
        }
    }
    // 补充块的 summary 计数要能被看到，否则合并反而让模型少拿到事实：
    // `--relations model:X` 的 DataFlow 输入输出计数就在补充调用的 summary 里。
    // 主调用永远优先，只补它没有的键——summary 的语义由主命令定义，不能被覆盖。
    if let (Some(extra), Some(base_summary)) = (
        supplement
            .get("summary")
            .and_then(serde_json::Value::as_object),
        object.get_mut("summary").and_then(|s| s.as_object_mut()),
    ) {
        for (field, value) in extra {
            base_summary
                .entry(field.clone())
                .or_insert_with(|| value.clone());
        }
    }
    for field in ["evidence", "diagnostics"] {
        let Some(extra) = supplement.get(field).and_then(serde_json::Value::as_array) else {
            continue;
        };
        let slot = object.entry(field).or_insert_with(|| serde_json::json!([]));
        if let Some(existing) = slot.as_array_mut() {
            existing.extend(extra.iter().cloned());
        }
    }
}

/// 把路由层自己产生的诊断追加到输出的 `diagnostics` 数组。
fn append_diagnostics(result: &mut serde_json::Value, diagnostics: &[String]) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    object
        .entry("diagnostics")
        .or_insert_with(|| serde_json::json!([]));
    if let Some(existing) = object.get_mut("diagnostics").and_then(|d| d.as_array_mut()) {
        existing.extend(diagnostics.iter().map(|note| serde_json::json!(note)));
    }
}

fn run_query_commands(
    args: &cli::Cli,
    runtime: &mut metadata_checker::runtime::GraphRuntime,
    project_dir: Option<&std::path::Path>,
) -> Result<bool> {
    if args.status {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if args.reload_graph {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if args.check_reload {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref model_id) = args.query_model {
        let model_node_id = if model_id.starts_with("model:") {
            model_id.clone()
        } else {
            format!("model:{}", model_id)
        };
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref page_id) = args.query_page {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref pages) = args.query_cross {
        if pages.len() >= 2 {
            let target = format!("{}, {}", pages[0], pages[1]);
            let result = run_cli_runtime_tool(
                runtime,
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
        return Ok(true);
    }

    if let Some(ref dataflow_id) = args.query_dataflow {
        let model_node_id = if dataflow_id.starts_with("model:") {
            dataflow_id.clone()
        } else {
            format!("model:{}", dataflow_id)
        };
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref page_logic_id) = args.query_page_logic {
        if args.is_human() {
            // M38：human 模式暂时保留旧路径，待统一 human 渲染器后再迁到 runtime
            metadata_checker::query::query_page_logic(
                &runtime.graph,
                page_logic_id,
                project_dir,
                true,
                &args.budget,
            )?;
        } else {
            let result = run_cli_runtime_tool(
                runtime,
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
        return Ok(true);
    }

    if let Some(ref keyword) = args.find {
        let result = run_surface(
            runtime,
            args,
            metadata_checker::route::Surface::Find,
            keyword,
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(true);
    }

    if let Some(ref explain_id) = args.explain {
        let result = run_surface(
            runtime,
            args,
            metadata_checker::route::Surface::Explain,
            explain_id,
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(true);
    }

    if let Some(ref relations_target) = args.relations {
        let result = run_surface(
            runtime,
            args,
            metadata_checker::route::Surface::Relations,
            relations_target,
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(true);
    }

    if let Some(ref explain_target) = args.explain_condition {
        let intent = explain::TraversalIntent::parse(&args.intent)?;
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref context_id) = args.context {
        let result = run_cli_runtime_tool(
            runtime,
            cli::CliToolInput {
                command: metadata_checker::tool_contract::ToolCommand::Context,
                target: Some(context_id.clone()),
                budget: args.budget.clone(),
                human: args.is_human(),
                intent: None,
                page_scope: None,
                depth: Some(args.depth.unwrap_or(1)),
                check_reload: false,
            },
        )?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(true);
    }

    if let Some(ref keyword) = args.find_page {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref keyword) = args.find_model {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    if let Some(ref keyword) = args.find_component {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
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
            runtime,
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
        return Ok(true);
    }

    if let Some(ref advise_target) = args.advise_query {
        let result = run_cli_runtime_tool(
            runtime,
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
        return Ok(true);
    }

    Ok(false)
}

/// 输出 session 命令的结构化错误。
fn print_session_error(code: &str, message: impl Into<String>) -> Result<()> {
    let safe_message = sanitize_session_error_message(&message.into());
    println!(
        "{}",
        serde_json::to_string(&serde_json::json!({
            "ok": false,
            "error": {
                "code": code,
                "message": safe_message,
            },
        }))?
    );
    Ok(())
}

/// 格式化错误链，供 session 命令输出可诊断但已脱敏的错误。
fn format_error_chain(err: &anyhow::Error) -> String {
    let mut parts = Vec::new();
    for cause in err.chain() {
        let message = cause.to_string();
        if parts.last().is_some_and(|last| last == &message) {
            continue;
        }
        parts.push(message);
    }
    sanitize_session_error_message(&parts.join(": "))
}

/// 读取 session manifest，失败时转换为稳定 JSON envelope。
fn read_session_manifest_or_print_error(
    session_manager: &metadata_checker::session::SessionManager,
    session_id: &str,
) -> Result<Option<metadata_checker::session::SessionManifest>> {
    match session_manager.read_manifest(session_id) {
        Ok(manifest) => Ok(Some(manifest)),
        Err(err) => {
            let code = if err.to_string().contains("invalid session_id") {
                "SESSION_INVALID_ID"
            } else {
                "SESSION_NOT_FOUND"
            };
            print_session_error(code, err.to_string())?;
            Ok(None)
        }
    }
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
    let _telemetry_guard =
        metadata_checker::telemetry::init(metadata_checker::telemetry::TelemetryConfig {
            mode: match args.trace {
                cli::TraceMode::Off => metadata_checker::telemetry::TelemetryMode::Off,
                cli::TraceMode::Json => metadata_checker::telemetry::TelemetryMode::Json,
                cli::TraceMode::Otlp => metadata_checker::telemetry::TelemetryMode::Otlp,
            },
            otlp_endpoint: args.otlp_endpoint.clone(),
            otlp_metrics_endpoint: args.otlp_metrics_endpoint.clone(),
        })?;

    // Session management commands (M41.12)
    let session_root = args.session_dir.clone().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        std::path::PathBuf::from(home).join(".metadata-checker/sessions")
    });
    let session_manager = metadata_checker::session::SessionManager::new(&session_root);

    if args.session_list {
        let sessions = session_manager.list_sessions()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "sessions": sessions,
                "session_dir": session_root.to_string_lossy(),
            }))?
        );
        return Ok(());
    }

    if let Some(ref id) = args.session_show {
        let Some(manifest) = read_session_manifest_or_print_error(&session_manager, id)? else {
            return Ok(());
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "manifest": manifest,
            }))?
        );
        return Ok(());
    }

    if let Some(ref id) = args.session_status {
        let Some(manifest) = read_session_manifest_or_print_error(&session_manager, id)? else {
            return Ok(());
        };
        let mirror_root =
            metadata_checker::session::sync::project_mirror_root(&session_manager.session_dir(id));
        let file_count = std::fs::read_dir(&mirror_root)
            .map(|entries| entries.filter(|e| e.is_ok()).count())
            .unwrap_or(0);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "session_id": id,
                "project_ref": manifest.project_ref,
                "project_name": manifest.project_name,
                "file_count": file_count,
                "graph_db_path": manifest.graph_db_path,
                "updated_at": manifest.updated_at,
            }))?
        );
        return Ok(());
    }

    if let Some(ref id) = args.session_delete {
        let session_dir = session_manager.session_dir(id);
        if session_dir.exists() {
            std::fs::remove_dir_all(&session_dir)?;
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "deleted": id,
            }))?
        );
        return Ok(());
    }

    // M55: one-shot 差量刷新（remote_server/project_ref/graph_db_path 均取自 manifest）
    if let Some(ref session_id) = args.session_diff_refresh {
        let Some(_manifest) = read_session_manifest_or_print_error(&session_manager, session_id)?
        else {
            return Ok(());
        };
        let (username, password) = match (
            args.remote_username.as_deref(),
            args.remote_password.as_deref(),
        ) {
            (Some(u), Some(p)) => (u, p),
            _ => {
                return print_session_error(
                    "SESSION_AUTH_REQUIRED",
                    "--session-diff-refresh requires --remote-username and --remote-password",
                );
            }
        };
        let context = match metadata_checker::session::DiffRefreshRuntimeContext::bind_bi_session(
            metadata_checker::session::SessionManager::new(&session_root),
            session_id,
            username,
            password,
        ) {
            Ok(context) => context,
            Err(err) => {
                return print_session_error(
                    "SESSION_AUTH_REQUIRED",
                    format!("diff refresh bind failed: {}", format_error_chain(&err)),
                );
            }
        };

        // 以 LongLived 模式加载（prepare_replacement 需要 read model）；
        // 但本次为 CLI one-shot，强制使用同步持久化，先 persist 后 install。
        let graph_db_path = std::path::PathBuf::from(&context.manifest.graph_db_path);
        let session_dir = session_manager.session_dir(session_id);
        let mirror_dir = metadata_checker::session::sync::project_mirror_root(&session_dir);
        let runtime = match metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode(
            &graph_db_path,
            Some(&mirror_dir),
            metadata_checker::runtime::RuntimeMode::LongLived,
        ) {
            Ok(runtime) => runtime,
            Err(err) => {
                return print_session_error(
                    "DIFF_REFRESH_FAILED",
                    format!("failed to load session graph: {}", format_error_chain(&err)),
                );
            }
        };

        let mut orchestrator = metadata_checker::diff_refresh::DiffRefreshOrchestrator::new(
            metadata_checker::session::SessionManager::new(&session_root),
            session_dir,
            context.manifest,
            context.source,
            context.provider,
            runtime,
        );
        orchestrator.set_one_shot_mode(true);
        match orchestrator.refresh_once() {
            Ok(report) => {
                let mut value = serde_json::to_value(&report)?;
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("ok".to_string(), serde_json::json!(true));
                    obj.insert("session_id".to_string(), serde_json::json!(session_id));
                }
                println!("{}", serde_json::to_string(&value)?);
            }
            Err(err) => {
                let code = if is_session_auth_error(&err) {
                    "SESSION_AUTH_REQUIRED"
                } else {
                    "DIFF_REFRESH_FAILED"
                };
                return print_session_error(
                    code,
                    format!("diff refresh failed: {}", format_error_chain(&err)),
                );
            }
        }
        return Ok(());
    }

    if let Some(ref session_id) = args.session_refresh {
        if let Some(source) = args
            .remote_source
            .as_deref()
            .filter(|value| value.trim().is_empty())
        {
            return print_session_error(
                "SESSION_INVALID_REMOTE_SOURCE",
                format!("--remote-source cannot be empty: {source}"),
            );
        }
        if let Some(module) = args
            .remote_module
            .as_deref()
            .filter(|value| value.trim().is_empty())
        {
            return print_session_error(
                "SESSION_INVALID_REMOTE_MODULE",
                format!("--remote-module cannot be empty: {module}"),
            );
        }
        if let Some(file) = args
            .remote_file
            .as_deref()
            .filter(|value| value.trim().is_empty())
        {
            return print_session_error(
                "SESSION_INVALID_REMOTE_FILE",
                format!("--remote-file cannot be empty: {file}"),
            );
        }

        let remote_server = match args.remote_server.as_deref() {
            Some(s) => s,
            None => {
                return print_session_error(
                    "SESSION_MISSING_REMOTE_SERVER",
                    "--session-refresh/--remote-index requires --remote-server/--base-url",
                );
            }
        };
        let project_ref = match args.remote_project.as_deref() {
            Some(s) => s,
            None => {
                return print_session_error(
                    "SESSION_MISSING_REMOTE_PROJECT",
                    "--session-refresh/--remote-index requires --remote-project/--project",
                );
            }
        };

        // 先校验本地参数（sync_mode），再创建 provider / 登录
        let sync_mode = match args.session_sync_mode.as_deref() {
            Some("full") => metadata_checker::session::SessionSyncMode::Full,
            Some("partial") | None => metadata_checker::session::SessionSyncMode::Partial,
            Some(mode) => {
                return print_session_error(
                    "SESSION_INVALID_SYNC_MODE",
                    format!("invalid sync mode: {}", mode),
                );
            }
        };

        if has_session_query_request(&args) {
            if let Err(err) = tool_contract::validate_budget(&args.budget) {
                return print_session_error(
                    "SESSION_INVALID_BUDGET",
                    format!("invalid budget: {err}"),
                );
            }
        }

        let provider =
            match metadata_checker::session::ReqwestRemoteSessionProvider::new(remote_server) {
                Ok(p) => p,
                Err(e) => {
                    return print_session_error(
                        "SESSION_PROVIDER_CREATE_FAILED",
                        format!("failed to create remote session provider: {}", e),
                    );
                }
            };

        // 登录认证
        let username = args.remote_username.as_deref();
        let password = args.remote_password.as_deref();
        match (username, password) {
            (Some(u), Some(p)) => {
                if let Err(e) = provider.login(u, p, "sys") {
                    return print_session_error(
                        "SESSION_AUTH_REQUIRED",
                        format!("remote login failed: {}", format_error_chain(&e)),
                    );
                }
            }
            (Some(_), None) => {
                return print_session_error(
                    "SESSION_AUTH_REQUIRED",
                    "--session-refresh requires both --remote-username and --remote-password",
                );
            }
            (None, Some(_)) => {
                return print_session_error(
                    "SESSION_AUTH_REQUIRED",
                    "--session-refresh requires both --remote-username and --remote-password",
                );
            }
            (None, None) => {
                return print_session_error(
                    "SESSION_AUTH_REQUIRED",
                    "--session-refresh requires --remote-username and --remote-password for remote BI login",
                );
            }
        }

        let options = metadata_checker::session::SessionRefreshOptions {
            session_id: session_id.clone(),
            remote_server: remote_server.to_string(),
            project_ref: project_ref.to_string(),
            project_name: project_ref.to_string(),
            sync_mode,
            create_if_missing: true,
            filter: session_refresh_filter_from_cli(&args),
            graph_db_path: args.graph_db_path.clone(),
        };

        let report = metadata_checker::session::refresh_session_from_remote(
            &provider,
            &session_manager,
            options,
        );

        match report {
            Ok(report) => {
                if has_session_query_request(&args) {
                    let mirror_dir = session_manager
                        .session_dir(&report.session_id)
                        .join("project");
                    let graph_db_path = args
                        .graph_db_path
                        .clone()
                        .unwrap_or_else(|| std::path::PathBuf::from(&report.graph_db_path));
                    let mut runtime =
                        match metadata_checker::runtime::GraphRuntime::load_with_project_dir(
                            &graph_db_path,
                            Some(&mirror_dir),
                        ) {
                            Ok(runtime) => runtime,
                            Err(err) => {
                                return print_session_error(
                                    "SESSION_QUERY_GRAPH_LOAD_FAILED",
                                    format!("failed to load graph for session query: {err}"),
                                );
                            }
                        };

                    if let Err(err) = run_query_commands(&args, &mut runtime, Some(&mirror_dir)) {
                        return print_session_error(
                            "SESSION_QUERY_FAILED",
                            format!("session query failed: {}", format_error_chain(&err)),
                        );
                    };
                    return Ok(());
                }
                let files = serde_json::to_value(&report.files)?;

                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "ok": true,
                        "session_id": report.session_id,
                        "project_ref": report.project_ref,
                        "session_dir": report.session_dir,
                        "graph_db_path": report.graph_db_path,
                        "sync": {
                            "written": report.sync.written,
                            "skipped": report.sync.skipped,
                            "deleted": report.sync.deleted,
                        },
                        "files": files,
                        "diagnostics": report.diagnostics
                            .iter()
                            .map(|diagnostic| serde_json::json!({
                                "code": diagnostic.code,
                                "message": sanitize_session_error_message(&diagnostic.message),
                            }))
                            .collect::<Vec<_>>(),
                        "index": {
                            "indexed": report.index.indexed,
                            "unchanged": report.index.unchanged,
                            "dirty": report.index.dirty,
                            "deleted": report.index.deleted,
                        },
                    }))?
                );
            }
            Err(err) => {
                print_session_error(
                    "SESSION_REFRESH_FAILED",
                    format!("session refresh failed: {}", format_error_chain(&err)),
                )?;
            }
        }
        return Ok(());
    }

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

        // M55: 可选绑定 session diff refresh context（不写入 manifest）
        let diff_refresh_context = match args.runtime_session_id.as_deref() {
            Some(session_id) => {
                let Some(manifest) =
                    read_session_manifest_or_print_error(&session_manager, session_id)?
                else {
                    return Ok(());
                };
                let manifest_db_path = std::path::PathBuf::from(&manifest.graph_db_path);
                if manifest_db_path != db_path {
                    return print_session_error(
                        "DIFF_REFRESH_GRAPH_PATH_MISMATCH",
                        format!(
                            "manifest graph_db_path '{}' does not match runtime graph db path '{}'",
                            manifest.graph_db_path,
                            db_path.display()
                        ),
                    );
                }
                let (username, password) = match (
                    args.remote_username.as_deref(),
                    args.remote_password.as_deref(),
                ) {
                    (Some(u), Some(p)) => (u, p),
                    _ => {
                        return print_session_error(
                            "SESSION_AUTH_REQUIRED",
                            "--runtime-session-id requires --remote-username and --remote-password",
                        );
                    }
                };
                match metadata_checker::session::DiffRefreshRuntimeContext::bind_bi_session(
                    metadata_checker::session::SessionManager::new(&session_root),
                    session_id,
                    username,
                    password,
                ) {
                    Ok(context) => Some(context),
                    Err(err) => {
                        return print_session_error(
                            "SESSION_AUTH_REQUIRED",
                            format!("diff refresh bind failed: {}", format_error_chain(&err)),
                        );
                    }
                }
            }
            None => None,
        };

        metadata_checker::stdio_server::run_stdio_server(
            &db_path,
            project_dir,
            diff_refresh_context,
        )?;
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

    if args.project_dir.is_none()
        && !(args.input.is_some() && args.explain.is_some())
        && has_session_query_request(&args)
    {
        let db_path = args.graph_db_path.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "graph queries require --project-dir, --graph-db-path, or --remote-index"
            )
        })?;
        let mut runtime = metadata_checker::runtime::GraphRuntime::load(db_path)?;
        if run_query_commands(&args, &mut runtime, None)? {
            return Ok(());
        }
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
        if run_query_commands(&args, &mut runtime, Some(project_dir))? {
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
