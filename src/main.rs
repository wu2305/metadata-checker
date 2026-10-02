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

use anyhow::{Context, Result};
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
        || args.find.is_some()
        || args.relations.is_some()
        || args.find_page.is_some()
        || args.find_model.is_some()
        || args.find_component.is_some()
        || args.resolve_model.is_some()
        || args.advise_query.is_some()
        || args.gql.is_some()
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
    use metadata_checker::route::{self, BareTargetResolution, PrefixedTargetResolution};
    use metadata_checker::tool_contract::ToolCommand;

    let mut diagnostics: Vec<serde_json::Value> = Vec::new();
    let mut target = raw_target.trim().to_string();

    // 只写了前缀、没写名字（`--relations page:`）是一次「这个项目里有哪些页面」的探查，
    // 不是一个写错的 target。问题本身就是模糊的（用户不知道页面叫什么，元数据也塞不进
    // 上下文），模型第一步做发现式探查是对的做法；此前工具用 TARGET_NOT_FOUND 回应它，
    // 白白烧掉一次命令机会，模型接着就开始编路径。列出来才是这条命令的正确答案。
    if surface != route::Surface::Find {
        if let Some(prefix) = route::TARGET_PREFIXES
            .iter()
            .find(|prefix| target == **prefix)
        {
            return enumerate_prefix_targets(runtime, surface, prefix);
        }
    }

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
                diagnostics.push(surface_diagnostic("RESOLVED_TARGET", diagnostic));
                target = resolved;
            }
            // 歧义和查不到都不猜：把候选如实交回去，模型下一轮可以直接用规范 target。
            // 这比返回一个 INVALID_TARGET 让它自己编路径要短一轮，也不会编出
            // `comp:app/未知页面.spg|button1` 这种东西。
            BareTargetResolution::Ambiguous { candidates } => {
                return answer_ambiguous_target(
                    runtime,
                    args,
                    surface,
                    &target,
                    &candidates,
                    diagnostics,
                );
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

    // 跨页协议 `page:A,page:B` 的每一侧独立归一。此前整串含逗号就跳过归一，
    // `page:actions_test,page:page_relations` 这种两侧都没写全的 target 被原样发给
    // QueryCross：拿不存在的节点 id 查出 0 条路径，再以 full confidence 交回
    // 「两页没有关系」。查不到/有歧义必须如实说，不能变成确认的空结果。
    if surface == route::Surface::Relations && target.contains(',') {
        let sides: Vec<String> = target
            .split(',')
            .map(|side| side.trim().to_string())
            .collect();
        let mut resolved_sides: Vec<String> = Vec::with_capacity(sides.len());
        for (index, side) in sides.iter().enumerate() {
            // 缺 page: 前缀的一侧交给下游 query_cross 协议校验报 INVALID_TARGET，
            // 这里只补「前缀对、路径没写全」的归一。
            if !route::has_type_prefix(side) {
                resolved_sides.push(side.clone());
                continue;
            }
            match normalize_target_against_graph(runtime, side)? {
                PrefixedTargetResolution::Exact => resolved_sides.push(side.clone()),
                PrefixedTargetResolution::Resolved {
                    target: resolved,
                    diagnostic,
                } => {
                    diagnostics.push(surface_diagnostic("RESOLVED_TARGET", diagnostic));
                    resolved_sides.push(resolved);
                }
                PrefixedTargetResolution::Ambiguous { candidates } => {
                    return Ok(serde_json::json!({
                        "ok": false,
                        "error": {
                            "code": "AMBIGUOUS_TARGET",
                            "message": format!(
                                "'{side}' 匹配到多个节点。从 next_queries 里挑一条直接执行，不要重复这一条命令。"
                            ),
                        },
                        "candidate_targets": candidates,
                        "next_queries": cross_retry_commands(
                            surface, &sides, index, &candidates, &resolved_sides
                        ),
                        "diagnostics": diagnostics,
                    }));
                }
                PrefixedTargetResolution::NotFound { candidates } => {
                    return Ok(serde_json::json!({
                        "ok": false,
                        "error": {
                            "code": "TARGET_NOT_FOUND",
                            "message": format!("找不到 '{side}'，改用 next_queries 里的命令重试，或先用 --find 定位"),
                        },
                        "candidate_targets": candidates,
                        "next_queries": cross_retry_commands(
                            surface, &sides, index, &candidates, &resolved_sides
                        ),
                        "diagnostics": diagnostics,
                    }));
                }
            }
        }
        target = resolved_sides.join(",");
    }

    // 带前缀但没写全的 target 走另一条归一。裸名归一只在完全没有前缀时触发，而新表面
    // 恰恰教会了模型总是写前缀——M58.3 评测里 117 次 trial 只有 1 次走到了裸名那条路，
    // 剩下的拒绝里 83% 是 `page:actions_test` 这种前缀对、路径没写全的写法。
    let mut near_miss: Vec<String> = Vec::new();
    if surface != route::Surface::Find && route::has_type_prefix(&target) && !target.contains(',') {
        match normalize_target_against_graph(runtime, &target)? {
            PrefixedTargetResolution::Exact => {}
            PrefixedTargetResolution::Resolved {
                target: resolved,
                diagnostic,
            } => {
                diagnostics.push(surface_diagnostic("RESOLVED_TARGET", diagnostic));
                target = resolved;
            }
            PrefixedTargetResolution::Ambiguous { candidates } => {
                return answer_ambiguous_target(
                    runtime,
                    args,
                    surface,
                    &target,
                    &candidates,
                    diagnostics,
                );
            }
            // 归一失败不等于 target 无效：`field:` / `model:` 这类 target 未必是图节点，
            // 照样能被下游命令正确回答。这里只把近似候选留到真的失败时再用。
            PrefixedTargetResolution::NotFound { candidates } => near_miss = candidates,
        }
    }

    execute_resolved_target(
        runtime,
        args,
        surface,
        &target,
        &near_miss,
        diagnostics,
        &args.budget,
    )
}

/// 在一个已经归一好的 target 上执行路由计划并合并输出。
///
/// 从 [`run_surface`] 里拆出来，是为了让「一个 target 匹配到多个节点」时能对每个候选
/// 各跑一遍——见 [`answer_ambiguous_target`]。
fn execute_resolved_target(
    runtime: &mut metadata_checker::runtime::GraphRuntime,
    args: &cli::Cli,
    surface: metadata_checker::route::Surface,
    target: &str,
    near_miss: &[String],
    mut diagnostics: Vec<serde_json::Value>,
    budget: &str,
) -> Result<serde_json::Value> {
    use metadata_checker::route;

    let target = target.to_string();
    let plan = route::route(surface, &target, args.depth)?;
    let mut merged: Option<serde_json::Value> = None;

    for call in &plan.calls {
        // 主调用已经把 target 判成「定位不到 / 有歧义」时，**共享同一套旧 target
        // 解析**的补充调用问的是同一个 target，必然得到同一个定位结果。继续发只会
        // 重复整图解析（裸 target 每次都要 `iter_nodes`）再产出一份同样的壳子。
        //
        // 这里必须按命令白名单判断，不能假设「所有补充调用都同意」：
        // `--relations model:X` 的补充调用是 `QueryDataflow`，它一度直接精确
        // `get_node`，于是主调用报 `AMBIGUOUS_TARGET` 时它却静默命中物理模型——
        // 跳过它会把那份（有分歧的）结果一起吞掉。白名单只列已接线同一套解析的
        // 命令；未接线的命令照常执行，分歧会如实暴露而不是被静默吃掉。
        if call.merge_key.is_some()
            && shares_legacy_target_resolution(call.command)
            && merged.as_ref().is_some_and(|base| {
                has_diagnostic_code(base, "TARGET_NOT_FOUND")
                    || has_diagnostic_code(base, "AMBIGUOUS_TARGET")
            })
        {
            continue;
        }

        let outcome = run_cli_runtime_tool(
            runtime,
            cli::CliToolInput {
                command: call.command,
                target: Some(call.target.clone()),
                budget: budget.to_string(),
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
            // 主调用失败但我们手上有近似候选时，把候选交回去。底层的候选是按名字模糊
            // 打分出来的（`same prefix (component)` 这类），完全忽略文件段，模型拿到也
            // 用不上；结构性近似项才是下一轮能直接用的东西。
            (Err(error), None) if !near_miss.is_empty() => {
                return Ok(serde_json::json!({
                    "ok": false,
                    "error": {
                        "code": "TARGET_NOT_FOUND",
                        "message": format!(
                            "找不到 '{target}'：{error}。改用 next_queries 里的命令重试，不要重复这一条。"
                        ),
                    },
                    "candidate_targets": near_miss,
                    "next_queries": retry_commands(surface, near_miss),
                    "diagnostics": diagnostics,
                }));
            }
            (Err(error), None) => return Err(error),
            // 补充块不适用于当前节点类型是正常的，记一条 diagnostic 就够了；
            // 让它拖垮整条命令等于换个方式重造「选错动词就一无所获」。
            (Err(error), Some(key)) => {
                diagnostics.push(surface_diagnostic(
                    "SUPPLEMENT_UNAVAILABLE",
                    format!("补充块 '{key}' 取不到：{error}"),
                ));
            }
        }
    }

    let mut result = merged.unwrap_or(serde_json::Value::Null);
    if !diagnostics.is_empty() {
        append_diagnostics(&mut result, &diagnostics);
    }
    // 底层命令找不到目标时未必返回 Err——它也可能返回一个 ok 的壳子，只在 diagnostics
    // 里记一条 TARGET_NOT_FOUND。那种输出对模型等于什么都没说，而我们手上正好有结构性
    // 近似候选。
    if !near_miss.is_empty() && has_diagnostic_code(&result, "TARGET_NOT_FOUND") {
        if let Some(object) = result.as_object_mut() {
            object.insert(
                "candidate_targets".to_string(),
                serde_json::json!(near_miss),
            );
        }
    }
    augment_field_lineage_queries(&mut result);
    enforce_budget_size(&mut result, budget);
    dedupe_diagnostics(&mut result);
    attach_confidence(&mut result);
    Ok(result)
}

/// 把 DataFlow 子图里已解析出的字段名拼成可执行的字段血缘命令。
///
/// `--relations model:df_a` 回答的是模型级关系，而「这个字段是从哪一路传过来的」只有
/// `--explain field:X.y` 能答。字段名就在合并进来的 `details.dataflow_subgraph.field_traces`
/// 里，但那是补充块——生成 next_queries 的 model.rs 看不到它，只能给一个 `<字段名>` 占位符。
/// 合并发生在这一层，就在这一层把占位符换成真实字段。
fn augment_field_lineage_queries(result: &mut serde_json::Value) {
    let Some(target) = result
        .get("query_target")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
    else {
        return;
    };
    let Some(model_name) = target.strip_prefix("model:").map(ToString::to_string) else {
        return;
    };

    let mut fields: Vec<String> = Vec::new();
    if let Some(traces) = result
        .get("details")
        .and_then(|details| details.get("dataflow_subgraph"))
        .and_then(|subgraph| subgraph.get("field_traces"))
        .and_then(serde_json::Value::as_array)
    {
        for trace in traces {
            let Some(field) = trace.get("field").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !field.is_empty() && !fields.iter().any(|known| known == field) {
                fields.push(field.to_string());
            }
        }
    }
    if fields.is_empty() {
        return;
    }

    let Some(queries) = result
        .get_mut("next_queries")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    // 占位符版本已经没有用了：手上有真实字段名，就别再让模型去猜一个。
    queries.retain(|query| {
        !query
            .as_str()
            .is_some_and(|text| text.contains("field:") && text.contains("<字段名>"))
    });
    // 排在最前面：模型几乎总是照抄第一条，而字段级来源正是这次调用没能回答的那部分。
    for (offset, field) in fields.iter().take(3).enumerate() {
        let command = metadata_checker::output::schema::format_next_query(
            "--explain {} for field-level lineage (chain across DataFlows)",
            &format!("field:{model_name}.{field}"),
        );
        if !queries.iter().any(|q| q.as_str() == Some(command.as_str())) {
            queries.insert(offset.min(queries.len()), serde_json::json!(command));
        }
    }
}

/// 打印三动词表面的结果。
///
/// 给模型读的那份不缩进：缩进和换行占了输出的三到四成字节，一个事实都不携带，
/// 而模型正是按字节被淹的。人要读的那份（`--human` / `--interactive`）照旧缩进。
fn print_surface_result(args: &cli::Cli, result: &serde_json::Value) -> Result<()> {
    if args.is_human() {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("{}", serde_json::to_string(result)?);
    }
    Ok(())
}

/// 各预算档下整条输出的字节上限。full 不设限。
fn budget_byte_cap(budget: &str) -> Option<usize> {
    match budget {
        "compact" => Some(8_000),
        "normal" => Some(40_000),
        _ => None,
    }
}

fn json_byte_len(value: &serde_json::Value) -> usize {
    serde_json::to_string(value)
        .map(|text| text.len())
        .unwrap_or(0)
}

/// 让 `--budget compact` 真的是 compact——但一个事实都不能少。
///
/// 各命令自己按「条目数」截断，表面层还会把补充块整个折进 `details`：
/// `--explain comp:...|input1 --budget compact` 实测 23948 字节，光 condition_facts 就有
/// 10114。评测里这条用例 6 次有 4 次挂在「模型响应不是合法 JSON」——模型不是答错，是被淹了。
///
/// 第一版按大小砍 `details` 里最大的那一块，测试立刻抓到了它在干什么：
/// `blocking_conditions` 被整块换成省略说明，`action_flows` 里唯一一条
/// `category=unknown` 的记录被砍掉——正是那两条用例要问的东西。按大小挑要删什么，
/// 删掉的必然是信息量最大的条目，因为它们最长。
///
/// 第二版改成「递归删掉值为空的键」，快照测试又抓到了另一个问题：`details.reads` 是
/// `[]` 时整个键消失，消费方看到的不再是「读取 0 个」而是「没有这个字段」。M58 存在的
/// 理由就是不让模型在这种地方猜，用一种歧义换另一种不划算，而且它只省下 5% 字节。
///
/// 最后留下的只有一件事：按名字截断确定的外围块（[`PERIPHERAL_DETAIL_BLOCKS`]），
/// 并如实记一条 OUTPUT_TRUNCATED。真正的字节大头另有其人——给模型的那份 JSON 不再缩进
/// （见 [`print_surface_result`]），那是纯粹的字节，不含任何事实。
///
/// 收到底还超预算就到此为止：宁可输出偏长，也不拿答案换字节。
fn enforce_budget_size(result: &mut serde_json::Value, budget: &str) {
    let Some(cap) = budget_byte_cap(budget) else {
        return;
    };
    let before = json_byte_len(result);
    if before <= cap {
        return;
    }
    // 外围块逐档收紧，直到进预算或收无可收。收到 1 条就停：留一条样例比留 0 条更能
    // 让模型知道这里有东西、以及它长什么样。
    let mut totals: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let floor = if budget == "compact" { 1 } else { 2 };
    for keep in [8usize, 5, 3, 2, 1] {
        if json_byte_len(result) <= cap || keep < floor {
            break;
        }
        let Some(map) = result
            .get_mut("details")
            .and_then(serde_json::Value::as_object_mut)
        else {
            break;
        };
        for key in PERIPHERAL_DETAIL_BLOCKS {
            let Some(items) = map.get_mut(*key).and_then(serde_json::Value::as_array_mut) else {
                continue;
            };
            if items.len() > keep {
                totals.entry(key).or_insert(items.len());
                items.truncate(keep);
            }
        }
    }
    let truncated: Vec<String> = totals
        .iter()
        .map(|(key, total)| {
            let kept = result
                .get("details")
                .and_then(|details| details.get(*key))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            format!("{key}（{total} -> {kept}）")
        })
        .collect();

    if !truncated.is_empty() {
        let note = surface_diagnostic(
            "OUTPUT_TRUNCATED",
            format!(
                "--budget {budget} 放不下全部外围上下文，已截断：{}。这些是围绕目标的周边节点，不是问题本身的事实；条数见 summary 里对应的 *_count，需要全集请提高 --budget。",
                truncated.join("、")
            ),
        );
        append_diagnostics(result, &[note]);
    }
}

/// 预算不够时可以按条数截断的 details 块。
///
/// 只列「围绕目标的周边信息」：它们的完整条数在 summary 的 `*_count` 里另有记录，
/// 截断不会让任何一句结论变得无法作答。`--relations page:page_relations` 实测 103874
/// 字节的 details 里，光 related_context 就占 75166——问「跳转传了哪些参数」时，
/// 这 31 条周边节点一条都用不上，却挤掉了模型读到 navigation 的机会。
///
/// 反过来，condition_facts / action_flows / navigation / blocking_conditions 这些
/// 永远不进这张表：它们就是被问的那个事实本身。
const PERIPHERAL_DETAIL_BLOCKS: &[&str] = &["related_context", "primary_paths"];

/// 把「这次输出的诊断对结论意味着什么」写进 summary。
///
/// 诊断散在 `diagnostics` 和 `details.risk_diagnostics` 两处，compact 预算下后者还会被
/// 截断；模型要自己扫两个数组、认出哪些 code 会削弱结论，实际上没人做得到这一步。
fn attach_confidence(result: &mut serde_json::Value) {
    let mut codes: Vec<String> = Vec::new();
    for path in [
        &["diagnostics"][..],
        &["details", "risk_diagnostics"][..],
        &["details", "risk_diagnostics", "items"][..],
    ] {
        let mut cursor = Some(&*result);
        for key in path {
            cursor = cursor.and_then(|value| value.get(key));
        }
        let Some(entries) = cursor.and_then(serde_json::Value::as_array) else {
            continue;
        };
        codes.extend(
            entries
                .iter()
                .filter_map(|entry| entry.get("code").and_then(serde_json::Value::as_str))
                .map(ToString::to_string),
        );
    }
    let Some(summary) = result.get_mut("summary").and_then(|s| s.as_object_mut()) else {
        return;
    };
    summary.insert(
        "confidence".to_string(),
        metadata_checker::output::answer_effect::confidence_value(codes.iter().map(String::as_str)),
    );
}

/// 该命令是否与 `build_query_model_output` 共用同一套旧 target 解析。
///
/// 只有这些命令才能在「主调用已报定位失败」时被安全跳过——它们对同一个 target 必然
/// 得到同一个定位结论。未接线的命令（或将来新增的）不在表内，照常执行：宁可多跑一次
/// 也不静默吞掉一个可能给出不同答案的补充块。
///
/// 新增接线 resolver 的命令时应同步加入本表（见
/// `docs/knowledge/topic-cli-stdio-to-query.md` 1.8 的入口清单）。
fn shares_legacy_target_resolution(command: metadata_checker::tool_contract::ToolCommand) -> bool {
    use metadata_checker::tool_contract::ToolCommand;
    matches!(
        command,
        ToolCommand::Explain
            | ToolCommand::ExplainCondition
            | ToolCommand::Context
            | ToolCommand::QueryModel
            | ToolCommand::QueryDataflow
    )
}

/// 把候选渲染成可以直接照抄执行的命令。
///
/// 只把候选 id 列成数组是不够的：M58.3 评测里模型拿到两个候选之后，重发了一模一样的
/// 那条歧义命令，把仅有的两次命令机会用光。候选要以「下一条命令」的形态出现。
fn retry_commands(surface: metadata_checker::route::Surface, candidates: &[String]) -> Vec<String> {
    candidates
        .iter()
        .take(5)
        .map(|candidate| format!("{} '{candidate}'", surface.as_str()))
        .collect()
}

/// 为跨页查询构造重试命令：把失败侧换成真实候选，其余侧保留已归一的结果。
///
/// 通用的 [`retry_commands`] 只会把候选拼成单页命令，模型照抄之后问题就从
/// 「两页的关系」悄悄换成了「一页的逻辑」——重试命令必须保住原来的问句。
fn cross_retry_commands(
    surface: metadata_checker::route::Surface,
    sides: &[String],
    failed_index: usize,
    candidates: &[String],
    resolved_sides: &[String],
) -> Vec<String> {
    candidates
        .iter()
        .take(5)
        .map(|candidate| {
            let mut parts = sides.to_vec();
            // resolved_sides 与 sides 的前 failed_index 项一一对应（每侧循环只 push 一次）。
            for (resolved_index, resolved) in resolved_sides.iter().enumerate() {
                parts[resolved_index] = resolved.clone();
            }
            parts[failed_index] = candidate.clone();
            format!("{} '{}'", surface.as_str(), parts.join(","))
        })
        .collect()
}

/// 枚举输出最多列出多少个节点。
const MAX_ENUMERATED_TARGETS: usize = 40;

/// 把「只写了前缀」当成一次枚举查询来回答：列出该类型下的全部节点。
///
/// 返回的是一条正常答案（`ok: true`），不是错误：模型问「有哪些页面」，工具知道答案。
fn enumerate_prefix_targets(
    runtime: &metadata_checker::runtime::GraphRuntime,
    surface: metadata_checker::route::Surface,
    prefix: &str,
) -> Result<serde_json::Value> {
    // 读图失败必须上抛：吞成空列表会把「图读不出来」伪装成「没有这类节点」。
    let enumerated: Vec<String> =
        metadata_checker::graph_store::GraphReadStore::iter_nodes(&runtime.graph)
            .context("枚举图节点失败")?
            .map(|node| node.id)
            .collect();
    let mut ids: Vec<&str> = enumerated
        .iter()
        .map(String::as_str)
        .filter(|id| id.starts_with(prefix))
        .collect();
    ids.sort_unstable();
    let total = ids.len();
    let shown: Vec<&str> = ids.into_iter().take(MAX_ENUMERATED_TARGETS).collect();

    let mut diagnostics = Vec::new();
    if total == 0 {
        diagnostics.push(surface_diagnostic(
            "NO_MATCHES_FOUND",
            format!("图里没有 '{prefix}' 类型的节点。"),
        ));
    }
    if total > shown.len() {
        diagnostics.push(surface_diagnostic(
            "OUTPUT_TRUNCATED",
            format!("共 {total} 个，只列出前 {}。", shown.len()),
        ));
    }

    let what_is_it = if total == 0 {
        format!("图里没有 '{prefix}' 类型的节点。")
    } else {
        format!(
            "'{prefix}' 类型的节点共 {total} 个，全名如下；挑一个完整 id 作为 target 再问一次。"
        )
    };

    let mut result = serde_json::json!({
        "schema_version": "1.0",
        "kind": "QueryAdvice",
        "query_target": prefix,
        "summary": {
            "what_is_it": what_is_it,
            "resolved_count": total,
            "shown_count": shown.len(),
            "targets": shown,
        },
        "details": { "targets": shown },
        "evidence": [],
        "diagnostics": diagnostics,
        "next_queries": retry_commands(
            surface,
            &shown.iter().map(|id| (*id).to_string()).collect::<Vec<_>>(),
        ),
    });
    attach_confidence(&mut result);
    Ok(result)
}

/// 从一份答案里摘出直接相邻的节点 id。
///
/// 全答模式下 details 常常放不进预算（两三份完整 details 拼起来就超了），于是
/// `summary.answers[*].summary` 成了模型唯一读得到的东西——而它只有计数，没有名字。
/// 评测里 `context_button1_neighbors` 12 次有 10 次少了 `action1`：那个 id 明明在
/// evidence 里，但模型不去翻 evidence。「周围有哪些依赖」问的就是这些名字，
/// 它们必须出现在答案里，而不是只出现在证据里。
fn key_related_nodes(answer: &serde_json::Value) -> Vec<String> {
    let mut nodes: Vec<String> = Vec::new();
    let Some(details) = answer.get("details").and_then(serde_json::Value::as_object) else {
        return nodes;
    };
    for key in [
        "triggers",
        "affects",
        "writes",
        "reads",
        "triggered_by",
        "navigation",
        "lineage",
    ] {
        let Some(items) = details.get(key).and_then(serde_json::Value::as_array) else {
            continue;
        };
        for item in items.iter().take(6) {
            for field in ["node_id", "id", "to", "from"] {
                if let Some(id) = item.get(field).and_then(serde_json::Value::as_str)
                    && !id.is_empty()
                    && !nodes.iter().any(|known| known == id)
                {
                    nodes.push(id.to_string());
                    break;
                }
            }
        }
    }
    nodes.truncate(12);
    nodes
}

/// 候选少到可以全答的上限。
///
/// 超过这个数就只能交回候选：把十几个节点的答案拼在一条输出里，模型读到的是噪音。
const MAX_ANSWERED_CANDIDATES: usize = 3;

/// 候选很少时，对每个候选分别作答，而不是交回一个错误让模型再猜一轮。
///
/// M58.3 评测里 `--explain comp:button1` 恰好匹配两个节点（actions_test 与 page_relations
/// 各有一个 button1），问题本身（「button1 周围还有哪些依赖」）没给任何可用来消歧的信息。
/// 此前工具返回 AMBIGUOUS_TARGET，模型把仅有的两次命令机会花在重发同一条命令上，
/// 那条用例 6 次 trial 全挂。
///
/// 不做排序：任何「挑一个最像的」规则在这里都是随意的断点，只不过恰好在这个 fixture 上
/// 选对。两个节点都是合法答案，就都答出来，由 answers[*].target 标明各是哪一个。
fn answer_ambiguous_target(
    runtime: &mut metadata_checker::runtime::GraphRuntime,
    args: &cli::Cli,
    surface: metadata_checker::route::Surface,
    target: &str,
    candidates: &[String],
    diagnostics: Vec<serde_json::Value>,
) -> Result<serde_json::Value> {
    if candidates.len() > MAX_ANSWERED_CANDIDATES {
        return Ok(ambiguous_target_error(
            surface,
            target,
            candidates,
            diagnostics,
        ));
    }

    // 同一个预算现在要装下 N 份答案，所以每份都降一档。预算是「这次输出能有多大」的
    // 承诺，不是「每个节点能有多大」——按原档跑两遍就等于悄悄把输出翻倍。
    let per_candidate_budget = if candidates.len() > 1 {
        match args.budget.as_str() {
            "full" => "normal",
            _ => "compact",
        }
    } else {
        args.budget.as_str()
    };

    let mut answers: Vec<(String, serde_json::Value)> = Vec::new();
    for candidate in candidates {
        // 单个候选答不出来不该拖垮整条命令：其余候选的答案照样是模型要的事实。
        if let Ok(value) = execute_resolved_target(
            runtime,
            args,
            surface,
            candidate,
            &[],
            Vec::new(),
            per_candidate_budget,
        ) {
            answers.push((candidate.clone(), value));
        }
    }
    if answers.is_empty() {
        return Ok(ambiguous_target_error(
            surface,
            target,
            candidates,
            diagnostics,
        ));
    }

    Ok(combine_candidate_answers(
        surface,
        target,
        candidates,
        answers,
        diagnostics,
        &args.budget,
    ))
}

/// 把逐个候选的答案拼成一条输出。
///
/// 形状对每个候选是对称的：没有哪一个被放在「主答案」的位置上，因为没有依据这么排。
fn combine_candidate_answers(
    surface: metadata_checker::route::Surface,
    target: &str,
    candidates: &[String],
    answers: Vec<(String, serde_json::Value)>,
    mut diagnostics: Vec<serde_json::Value>,
    budget: &str,
) -> serde_json::Value {
    let answered: Vec<&str> = answers.iter().map(|(id, _)| id.as_str()).collect();
    let schema_version = answers[0]
        .1
        .get("schema_version")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("1.0"));
    let kind = answers[0]
        .1
        .get("kind")
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let mut evidence: Vec<serde_json::Value> = Vec::new();
    let mut next_queries: Vec<serde_json::Value> = Vec::new();
    let summary_answers: Vec<serde_json::Value> = answers
        .iter()
        .map(|(id, value)| {
            serde_json::json!({
                "target": id,
                "summary": value.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                "key_nodes": key_related_nodes(value),
            })
        })
        .collect();
    let detail_answers: Vec<serde_json::Value> = answers
        .iter()
        .map(|(id, value)| {
            serde_json::json!({
                "target": id,
                "details": value.get("details").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();
    for (_, value) in &answers {
        if let Some(items) = value.get("evidence").and_then(serde_json::Value::as_array) {
            evidence.extend(items.iter().cloned());
        }
        if let Some(items) = value
            .get("diagnostics")
            .and_then(serde_json::Value::as_array)
        {
            diagnostics.extend(items.iter().cloned());
        }
        if let Some(items) = value
            .get("next_queries")
            .and_then(serde_json::Value::as_array)
        {
            next_queries.extend(items.iter().cloned());
        }
    }

    diagnostics.push(surface_diagnostic(
        "AMBIGUOUS_TARGET_ANSWERED",
        format!(
            "'{target}' 匹配到 {} 个节点，已对每个节点分别作答；答案在 summary.answers / details.answers 里按 target 分组，回答时要说明结论属于哪一个节点。",
            answered.len()
        ),
    ));

    // 两三份完整 details 拼起来会超出预算。这里不能像单目标那样按大小逐块砍——
    // details.answers 是一个每候选一项的数组，砍掉一半就等于悄悄替模型挑了一个候选，
    // 而「不替它挑」正是全答模式存在的理由。要么整块留，要么整块省掉。
    let summary_bytes = json_byte_len(&serde_json::json!(summary_answers));
    let details_bytes = json_byte_len(&serde_json::json!(detail_answers));
    let cap = budget_byte_cap(budget).unwrap_or(usize::MAX);
    let details = if summary_bytes.saturating_add(details_bytes) > cap {
        diagnostics.push(surface_diagnostic(
            "OUTPUT_TRUNCATED",
            format!(
                "多候选合并输出放不进 --budget {budget}，details 已整体省略；summary.answers 里每个候选的结论都是完整的，需要细节请用 next_queries 里的单目标命令。"
            ),
        ));
        serde_json::json!({
            "answers_omitted": "多候选答案的 details 已省略；summary.answers 里有每个候选的结论，需要细节请用 next_queries 里的单目标命令。",
        })
    } else {
        serde_json::json!({ "answers": detail_answers })
    };

    let mut result = serde_json::json!({
        "schema_version": schema_version,
        "kind": kind,
        "query_target": target,
        "summary": {
            "what_is_it": format!(
                "'{target}' 匹配到 {} 个节点，以下对每个节点分别作答。",
                answered.len()
            ),
            "resolved_count": answered.len(),
            "answered_targets": answered,
            "answers": summary_answers,
        },
        "details": details,
        "evidence": evidence,
        "diagnostics": diagnostics,
        "next_queries": if next_queries.is_empty() {
            serde_json::json!(retry_commands(surface, candidates))
        } else {
            serde_json::json!(next_queries)
        },
    });
    dedupe_diagnostics(&mut result);
    attach_confidence(&mut result);
    result
}

/// 同一条诊断在一次输出里只说一遍。
///
/// 归一层和底层命令会各记一条 TARGET_NOT_FOUND，多候选合并还会把相同的诊断带进来
/// 好几份。重复的诊断不增加任何事实，只是把 diagnostics 撑长，让真正重要的那条更难被读到。
fn dedupe_diagnostics(result: &mut serde_json::Value) {
    let Some(entries) = result
        .get_mut("diagnostics")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    entries.retain(|entry| {
        let key = |field: &str| {
            entry
                .get(field)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        seen.insert((key("code"), key("message")))
    });
}

/// 歧义 target 的统一错误形态：候选 + 可直接执行的下一条命令 + 别重发原命令。
fn ambiguous_target_error(
    surface: metadata_checker::route::Surface,
    target: &str,
    candidates: &[String],
    diagnostics: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "error": {
            "code": "AMBIGUOUS_TARGET",
            "message": format!(
                "'{target}' 匹配到多个节点。从 next_queries 里挑一条直接执行，不要重复这一条命令。"
            ),
        },
        "candidate_targets": candidates,
        "next_queries": retry_commands(surface, candidates),
        "diagnostics": diagnostics,
    })
}

/// 输出的 diagnostics 里是否有指定 code。
fn has_diagnostic_code(result: &serde_json::Value, code: &str) -> bool {
    result
        .get("diagnostics")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|entries| {
            entries.iter().any(|entry| {
                entry
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value == code)
            })
        })
}

/// 拿真实节点 id 集合归一一个带前缀的 target。
///
/// 先试 O(1) 的精确命中，命中就完全不扫图——绝大多数调用走这条路，归一不该给正确
/// 写法的 target 加成本。
fn normalize_target_against_graph(
    runtime: &metadata_checker::runtime::GraphRuntime,
    target: &str,
) -> Result<metadata_checker::route::PrefixedTargetResolution> {
    use metadata_checker::graph_store::GraphReadStore;
    // O(1) 精确命中先返回——绝大多数调用在这里就结束，不该给正确写法的
    // target 加扫图成本。读错（坏数据）上抛，不当作未命中去做模糊归一。
    if runtime
        .graph
        .get_node(target)
        .with_context(|| format!("读取节点 {target} 失败"))?
        .is_some()
    {
        return Ok(metadata_checker::route::PrefixedTargetResolution::Exact);
    }
    let node_ids: Vec<String> = runtime
        .graph
        .iter_nodes()
        .context("枚举图节点失败")?
        .map(|node| node.id)
        .collect();
    Ok(metadata_checker::route::normalize_prefixed_target(
        target,
        node_ids.iter().map(String::as_str),
    ))
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
///
/// M58.3 复核返修（D1）：统一走六字段信封构造路径（`envelope_diagnostic` →
/// `Diagnostic` 序列化：`code/severity/count/sample_location/answer_impact/
/// first_seen_phase` + message/suggestion），不再手写旧形态 JSON（location 键、
/// 无 count/answer_impact/first_seen_phase）。severity 由 `severity_for(code)`
/// 权威派生，保持本层原有 info 意图（RESOLVED_TARGET 等已在表中登记为 Info）。
fn surface_diagnostic(code: &str, message: String) -> serde_json::Value {
    match serde_json::to_value(metadata_checker::diagnostics::envelope_diagnostic(
        code,
        1,
        metadata_checker::output::Location::default(),
        message,
    )) {
        Ok(value) => value,
        // 信封必填字段由 envelope_diagnostic 全量构造，正常不会失败；
        // 真失败也要 fail-visible，留最小 JSON 占位（与 runtime 合并路径同策略）。
        Err(error) => serde_json::json!({
            "severity": "warning",
            "code": metadata_checker::diagnostics::CODE_DIAGNOSTIC_SERIALIZE_FAILED,
            "message": format!("surface diagnostic {code} failed to serialize: {error}"),
        }),
    }
}

fn append_diagnostics(result: &mut serde_json::Value, diagnostics: &[serde_json::Value]) {
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

    // M59-3 C4：只读 GQL。失败输出结构化 `{ok:false,error:{code,message}}`，
    // 与 session 命令同信封，调用方（LLM）按 code 决定下一步；成功输出结果表。
    if let Some(ref query) = args.gql {
        match runtime.graph.query_gql_read_only(query, args.gql_max_rows) {
            Ok(table) => {
                if args.is_human() {
                    println!("{}", table.to_tsv());
                } else {
                    println!("{}", serde_json::to_string(&table.to_json())?);
                }
            }
            Err(error) => {
                println!(
                    "{}",
                    serde_json::to_string(&serde_json::json!({
                        "ok": false,
                        "error": {"code": error.code, "message": error.message},
                    }))?
                );
            }
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
        print_surface_result(args, &result)?;
        return Ok(true);
    }

    if let Some(ref explain_id) = args.explain {
        let result = run_surface(
            runtime,
            args,
            metadata_checker::route::Surface::Explain,
            explain_id,
        )?;
        print_surface_result(args, &result)?;
        return Ok(true);
    }

    if let Some(ref relations_target) = args.relations {
        let result = run_surface(
            runtime,
            args,
            metadata_checker::route::Surface::Relations,
            relations_target,
        )?;
        print_surface_result(args, &result)?;
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

    // 图 Schema 契约：不需要项目目录或图库，最先处理。
    if args.graph_schema {
        if args.human {
            print!("{}", metadata_checker::graph_schema::to_markdown());
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&metadata_checker::graph_schema::to_json())?
            );
        }
        return Ok(());
    }

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
        let project_binding = match metadata_checker::ownership::ProjectBinding::new(
            context.manifest.project_ref.clone(),
        ) {
            Ok(binding) => binding,
            Err(err) => {
                return print_session_error(
                    "SESSION_QUERY_GRAPH_LOAD_FAILED",
                    format!("invalid session project binding: {err}"),
                );
            }
        };
        let runtime = match metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
            &graph_db_path,
            Some(&mirror_dir),
            metadata_checker::runtime::RuntimeMode::LongLived,
            &project_binding,
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
                    let project_binding = match metadata_checker::ownership::ProjectBinding::new(
                        report.project_ref.clone(),
                    ) {
                        Ok(binding) => binding,
                        Err(err) => {
                            return print_session_error(
                                "SESSION_QUERY_GRAPH_LOAD_FAILED",
                                format!("invalid session project binding: {err}"),
                            );
                        }
                    };
                    let mut runtime =
                        match metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
                            &graph_db_path,
                            Some(&mirror_dir),
                            metadata_checker::runtime::RuntimeMode::OneShot,
                            &project_binding,
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

        let explicit_binding = match args.project_ref.as_ref() {
            Some(p) => Some(metadata_checker::ownership::ProjectBinding::new(p.clone())?),
            None => None,
        };
        metadata_checker::stdio_server::run_stdio_server(
            &db_path,
            project_dir,
            explicit_binding.as_ref(),
            diff_refresh_context,
        )?;
        return Ok(());
    }

    let project_binding = args
        .project_ref
        .as_ref()
        .map(|p| metadata_checker::ownership::ProjectBinding::new(p.clone()))
        .transpose()?;

    // graphdb-only runtime lifecycle commands do not need a project directory.
    if args.project_dir.is_none() && (args.status || args.reload_graph || args.check_reload) {
        let db_path = args.graph_db_path.as_ref().ok_or_else(|| {
            anyhow::anyhow!("--status/--reload-graph/--check-reload requires --graph-db-path when --project-dir is absent")
        })?;
        let mut runtime = match &project_binding {
            Some(binding) => {
                metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
                    db_path,
                    None::<&std::path::Path>,
                    metadata_checker::runtime::RuntimeMode::OneShot,
                    binding,
                )?
            }
            None => metadata_checker::runtime::GraphRuntime::load(db_path)?,
        };
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
        let mut runtime = match &project_binding {
            Some(binding) => {
                metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
                    db_path,
                    None::<&std::path::Path>,
                    metadata_checker::runtime::RuntimeMode::OneShot,
                    binding,
                )?
            }
            None => metadata_checker::runtime::GraphRuntime::load(db_path)?,
        };
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
            let status = GraphDB::check_graph_db_for_project(&db_path, project_binding.as_ref());
            println!("{}", serde_json::to_string_pretty(&status)?);
            return Ok(());
        }

        if args.build_graph {
            let report = match &project_binding {
                Some(binding) => {
                    scanner::scan_project_with_report_for_project(project_dir, &db_path, binding)?
                }
                None => scanner::scan_project_with_report(project_dir, &db_path)?,
            };
            let payload = serde_json::to_string(&report)
                .map_err(|err| anyhow::anyhow!("failed to serialize ScanReport: {err}"))?;
            if args.is_human() {
                // human 模式保留可读统计行；有诊断时额外输出结构化 JSON 信封
                println!(
                    "Indexed {} files | Unchanged: {} | Dirty: {} | Deleted: {} | Nodes: {} | Edges: {}",
                    report.indexed,
                    report.unchanged,
                    report.dirty,
                    report.deleted,
                    report.node_count,
                    report.edge_count
                );
                if report.diagnostics.is_empty() {
                    println!("Graph database built at {:?}", db_path);
                } else {
                    println!("{}", payload);
                }
            } else {
                // 默认（non-human）输出恰好一个 JSON 文档，与 cli.rs 声明的默认模式一致；
                // 统计行的全部字段在 ScanReport 中均有对应字段，无信息损失。
                println!("{}", payload);
            }
            return Ok(());
        }

        let mut runtime = match &project_binding {
            Some(binding) => match metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
                &db_path,
                Some(project_dir),
                metadata_checker::runtime::RuntimeMode::OneShot,
                binding,
            ) {
                Ok(r) => r,
                Err(_e) => {
                    let out = metadata_checker::graph::GraphDB::check_graph_db_for_project(
                        &db_path,
                        Some(binding),
                    );
                    println!("{}", serde_json::to_string_pretty(&out)?);
                    return Ok(());
                }
            },
            None => match metadata_checker::runtime::GraphRuntime::load_with_project_dir(
                &db_path,
                Some(project_dir),
            ) {
                Ok(r) => r,
                Err(_e) => {
                    let out = metadata_checker::graph::GraphDB::check_graph_db(&db_path);
                    println!("{}", serde_json::to_string_pretty(&out)?);
                    return Ok(());
                }
            },
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
            let status = GraphDB::check_graph_db_for_project(db_path, project_binding.as_ref());
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
