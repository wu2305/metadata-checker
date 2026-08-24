#![cfg(feature = "cli-local")]

//! M58.3 三动词命令表面测试。
//!
//! 每条断言都对应 M58 六轮干净评测里一类具体的被拒命令。115 条拒绝中约八成不是模型
//! 选错动词，而是命令表面无法从问题里判定；这里锁住让它变得可判定的那些行为，防止
//! 后续改动把已经收敛掉的歧义重新放回去。

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 由 Cargo 注入的二进制路径。
///
/// 不能写死 `target/debug/metadata-checker`：远端开发环境和 CI 用的是自定义
/// `CARGO_TARGET_DIR`（分别是 `target/cnb/workspace` 和 `target/cnb/coverage`），
/// 写死路径的测试在本地全绿、一上远端就全部 NotFound。
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metadata-checker"))
}

/// 每个用例独占一份 graphdb，避免并行测试互相抢锁。
fn workspace(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("m58-3-surface-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create workspace");
    let db = dir.join("case.graphdb");
    let status = Command::new(bin())
        .args([
            "--project-dir",
            "tests/fixtures/test_project",
            "--graph-db-path",
            db.to_str().unwrap(),
            "--build-graph",
        ])
        .output()
        .expect("build graph");
    assert!(status.status.success(), "graphdb 构建失败");
    db
}

fn surface(db: &Path, args: &[&str]) -> Value {
    let mut argv = vec![
        "--non-human".to_string(),
        "--project-dir".to_string(),
        "tests/fixtures/test_project".to_string(),
        "--graph-db-path".to_string(),
        db.to_str().unwrap().to_string(),
        "--budget".to_string(),
        "compact".to_string(),
    ];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let output = Command::new(bin()).args(&argv).output().expect("run cli");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "输出不是 JSON: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn detail_keys(value: &Value) -> Vec<String> {
    value
        .get("details")
        .and_then(Value::as_object)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// `--explain` 一次调用必须同时给出节点语义和条件成因。
///
/// M58 里 `condition_action_behavior` 与 `fixture_condition_button2_action_gate` 是同一个
/// 按钮上互为镜像的两个 case：模型把各自的答案发给了对方，34 条拒绝（占全部拒绝 30%）
/// 全部来自这一次二选一。两路都给之后，这个选择不再存在。
#[test]
fn test_explain_returns_semantics_and_conditions_in_one_call() {
    let db = workspace("explain-merged");
    let output = surface(&db, &["--explain", "comp:app/actions_test.spg|button2"]);

    let keys = detail_keys(&output);
    assert!(
        keys.iter().any(|key| key == "condition_facts"),
        "--explain 必须带上条件成因，实际 details 键：{keys:?}"
    );
    assert!(
        keys.iter().any(|key| key == "triggers" || key == "writes"),
        "--explain 必须保留节点语义，实际 details 键：{keys:?}"
    );
}

/// 互为镜像的两个 target 必须返回同一组事实块。
///
/// 只要 `comp:` 与 `action:` 两种写法拿到的东西不同，模型就仍然要在提问阶段盲选一种，
/// 也就仍然会有一半的时候选错。
#[test]
fn test_component_and_action_targets_expose_the_same_fact_blocks() {
    let db = workspace("mirror-targets");
    let by_component = surface(&db, &["--explain", "comp:app/actions_test.spg|button2"]);
    let by_action = surface(
        &db,
        &["--explain", "action:app/actions_test.spg|button2|action1"],
    );

    for output in [&by_component, &by_action] {
        assert!(
            detail_keys(output)
                .iter()
                .any(|key| key == "condition_facts"),
            "两种写法都必须带条件成因"
        );
    }
}

/// 裸名唯一命中时由 Rust 归一，并留下可审计的 diagnostic。
///
/// 定位是确定性的图查询，不该由模型编路径。
#[test]
fn test_bare_target_is_resolved_by_the_tool() {
    let db = workspace("bare-resolve");
    // input_chain_a 在 fixture 里只存在于 actions_test.spg；`input1` 这类通用 id 反而
    // 跨 4 个页面重名，正是下面那条歧义用例覆盖的情况。
    let output = surface(&db, &["--explain", "input_chain_a"]);

    assert_ne!(
        output.get("ok").and_then(Value::as_bool),
        Some(false),
        "唯一命中的裸名不应报错：{output}"
    );
    let diagnostics = output
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        has_diagnostic(&diagnostics, "RESOLVED_TARGET"),
        "归一必须留痕，实际 diagnostics：{diagnostics:?}"
    );
}

/// diagnostics 必须是结构化对象，不能混入裸字符串。
///
/// `AiOutput.diagnostics` 是 `Vec<Diagnostic>`；表面层曾往里塞 `format!` 出来的字符串，
/// 按结构解析这个数组的调用方会直接崩在这里。
#[test]
fn test_surface_diagnostics_are_structured_objects() {
    let db = workspace("diagnostic-shape");
    let output = surface(&db, &["--explain", "input_chain_a"]);

    let diagnostics = output
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!diagnostics.is_empty(), "{output}");
    for entry in &diagnostics {
        let object = entry
            .as_object()
            .unwrap_or_else(|| panic!("diagnostics 元素必须是对象：{entry}"));
        for field in ["code", "severity", "message"] {
            assert!(object.contains_key(field), "缺少 {field}：{entry}");
        }
        // 信封诊断为 sample_location；路由层 surface_diagnostic 仍是旧形态 location
        assert!(
            object.contains_key("sample_location") || object.contains_key("location"),
            "缺少 sample_location/location：{entry}"
        );
    }
}

/// 前缀写对、文件路径没写全的 target 由 Rust 补全。
///
/// M58.3 评测里 109 条拒绝有 83% 是这一类：模型学会了写前缀，于是只处理裸名的归一
/// 再也没被触发过（117 次 trial 里只生效 1 次）。
#[test]
fn test_partially_qualified_target_is_completed() {
    let db = workspace("partial-target");
    for target in [
        "page:actions_test",
        "page:actions_test.spg",
        "page:app/actions_test",
    ] {
        let output = surface(&db, &["--relations", target]);
        assert_ne!(
            output.get("ok").and_then(Value::as_bool),
            Some(false),
            "'{target}' 应当被补全：{output}"
        );
        let keys = detail_keys(&output);
        assert!(keys.iter().any(|key| key == "entrypoints"), "{keys:?}");
    }
}

/// 组件 target 的文件段同样可以只写文件名。
#[test]
fn test_partially_qualified_component_target_is_completed() {
    let db = workspace("partial-component");
    let output = surface(&db, &["--explain", "comp:actions_test|button2"]);

    assert_ne!(
        output.get("ok").and_then(Value::as_bool),
        Some(false),
        "{output}"
    );
    let diagnostics = output
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        has_diagnostic(&diagnostics, "RESOLVED_TARGET"),
        "补全必须留痕：{diagnostics:?}"
    );
}

/// 模型编出来的文件路径不该让命令一无所获。
///
/// 评测里 `comp:app/售后.app/首页.spg|button1` 这类捏造路径出现 19 次，底层返回的候选
/// 是 0 条。只要 `|` 后面的节点身份还在，要么唯一定位、要么交回真实候选。
#[test]
fn test_hallucinated_path_yields_resolution_or_real_candidates() {
    let db = workspace("hallucinated-path");
    let output = surface(&db, &["--explain", "comp:app/no_such_page.spg|button1"]);

    // button1 在 fixture 里跨两个页面重名：编出来的路径被丢掉，两个真实节点都要答。
    let answered = answered_targets(&output);
    assert!(answered.len() >= 2, "必须对每个真实候选作答：{output}");
    assert!(
        answered
            .iter()
            .all(|candidate| candidate.starts_with("comp:") && candidate.contains("button1")),
        "答案必须挂在可直接使用的规范 target 上：{answered:?}"
    );
}

/// 全答模式下被回答的那些 target。
fn answered_targets(output: &Value) -> Vec<String> {
    output["summary"]["answered_targets"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 写全的合法 target 不能被归一改写。
#[test]
fn test_exact_target_is_not_rewritten() {
    let db = workspace("exact-untouched");
    let output = surface(&db, &["--explain", "comp:app/actions_test.spg|button2"]);

    let diagnostics = output
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !has_diagnostic(&diagnostics, "RESOLVED_TARGET"),
        "精确 target 不该触发归一：{diagnostics:?}"
    );
}

fn has_diagnostic(diagnostics: &[Value], code: &str) -> bool {
    diagnostics.iter().any(|entry| {
        entry
            .get("code")
            .and_then(Value::as_str)
            .is_some_and(|value| value == code)
    })
}

/// 裸名有歧义、候选又很少时，对每个候选分别作答，而不是交回一个错误。
///
/// M58 里模型对「button1 周围还有什么」的应对是 `comp:app/未知页面.spg|button1`（3 次）
/// 和把 bootstrap 占位符原样抄成 `comp:app/<relative-file>.spg|button1`（2 次）。M58.3 改成
/// 交回真实候选之后，模型转而把仅有的两次命令机会花在重发同一条歧义命令上——那条用例
/// 6 次 trial 全挂。问题本身（「button1 周围还有哪些依赖」）没给任何可用来消歧的信息，
/// 两个节点都是合法答案，所以两个都答。
#[test]
fn test_ambiguous_bare_target_answers_every_candidate() {
    let db = workspace("bare-ambiguous");
    // button1 同时存在于 actions_test.spg 和 page_relations.spg。
    let output = surface(&db, &["--explain", "button1"]);

    assert!(
        output.get("error").is_none(),
        "候选少到能全答时不该返回错误：{output}"
    );
    let candidates = answered_targets(&output);
    assert!(candidates.len() >= 2, "必须逐个作答：{candidates:?}");
    // 每个候选都要带自己的结论，不能只报一句「匹配到多个」。
    let answers = output["summary"]["answers"].as_array().expect("answers");
    assert_eq!(answers.len(), candidates.len(), "{output}");
    assert!(
        answers
            .iter()
            .all(|answer| answer["summary"]["what_is_it"].is_string()),
        "每个候选都要有自己的 summary：{output}"
    );
    // 谁是谁必须说清楚，否则两份答案混在一起比不答更糟。
    let diagnostics = output["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        has_diagnostic(&diagnostics, "AMBIGUOUS_TARGET_ANSWERED"),
        "{diagnostics:?}"
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.starts_with("comp:")),
        "候选必须是可直接使用的规范 target：{candidates:?}"
    );
}

/// `--find` 不带类型过滤：问「这个名字在哪」的人不知道它是什么类型。
#[test]
fn test_find_is_type_agnostic() {
    let db = workspace("find-agnostic");
    let output = surface(&db, &["--find", "button1"]);

    let types: Vec<String> = output
        .get("details")
        .and_then(|details| details.get("matches"))
        .and_then(Value::as_array)
        .map(|matches| {
            matches
                .iter()
                .filter_map(|entry| entry.get("node_type").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    assert!(types.iter().any(|kind| kind == "Component"), "{types:?}");
    assert!(
        types.iter().any(|kind| kind == "Action"),
        "一次搜索要跨类型返回，否则模型还得先猜类型：{types:?}"
    );
}

/// `--relations` 按前缀分流，页面拿到页面逻辑。
#[test]
fn test_relations_on_page_returns_page_logic() {
    let db = workspace("relations-page");
    let output = surface(&db, &["--relations", "page:app/actions_test.spg"]);

    let keys = detail_keys(&output);
    assert!(keys.iter().any(|key| key == "entrypoints"), "{keys:?}");
    assert!(keys.iter().any(|key| key == "action_flows"), "{keys:?}");
}

/// 走错动词时必须直接说出该走哪个，而不是只回一个 INVALID_TARGET。
///
/// 拒绝本身不贵，贵的是拒绝之后模型还得再猜一轮。
#[test]
fn test_wrong_verb_error_names_the_right_verb() {
    // 不能省掉 --graph-db-path：默认路径指向 fixture 目录里的 .metadata-checker.graphdb，
    // 那是个 gitignore 掉的构建产物，在干净检出上不存在，命令会先报 GRAPH_DB_NOT_FOUND
    // 而根本走不到路由。
    let db = workspace("wrong-verb");
    let output = Command::new(bin())
        .args([
            "--non-human",
            "--project-dir",
            "tests/fixtures/test_project",
            "--graph-db-path",
            db.to_str().unwrap(),
            "--relations",
            "comp:app/actions_test.spg|button2",
        ])
        .output()
        .expect("run cli");
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(message.contains("--explain"), "{message}");
}

/// 补充块取不到只记 diagnostic，不能让整条命令失败。
///
/// 否则等于换个地方重造「选错动词就一无所获」。
#[test]
fn test_unavailable_supplement_degrades_to_diagnostic() {
    let db = workspace("supplement-degrade");
    // 普通模型没有 DataFlow 子图，--relations 的补充调用必然取不到。
    let output = surface(&db, &["--relations", "model:model1"]);

    assert_ne!(
        output.get("ok").and_then(Value::as_bool),
        Some(false),
        "补充块缺失不应让主查询失败：{output}"
    );
    assert!(output.get("summary").is_some(), "主输出必须保留：{output}");
}

/// 收敛掉的旧动词保持可用，外部调用方不受影响。
#[test]
fn test_legacy_verbs_still_work() {
    let db = workspace("legacy-aliases");
    for args in [
        vec!["--explain-condition", "comp:app/actions_test.spg|input1"],
        vec!["--query-page-logic", "page:app/actions_test.spg"],
        vec!["--find-component", "button1"],
        vec!["--context", "comp:app/actions_test.spg|button1"],
    ] {
        let output = surface(&db, &args);
        assert!(
            output.get("summary").is_some(),
            "旧动词 {args:?} 必须继续可用：{output}"
        );
    }
}

/// 只读页要在输出里把「只读」说出来，而不是留下三个 0 让模型自己推。
#[test]
fn test_a_readonly_page_states_its_absences_in_words() {
    let db = workspace("readonly-words");
    let output = surface(&db, &["--relations", "page:app/dataflow_embedded.spg"]);
    let summary = &output["summary"];
    let conclusion = summary["conclusion"].as_str().expect("conclusion");
    assert!(conclusion.contains("只读"), "{conclusion}");
    assert!(conclusion.contains("无写入"), "{conclusion}");

    let absent: Vec<&str> = summary["absent"]
        .as_array()
        .expect("absent")
        .iter()
        .map(|item| item["what"].as_str().unwrap())
        .collect();
    assert!(absent.contains(&"write_targets"), "{absent:?}");
    assert!(absent.contains(&"entrypoints"), "{absent:?}");
}

/// 语义不确定的诊断要给出「对结论意味着什么」，否则模型只能忽略它。
#[test]
fn test_an_uncertain_diagnostic_lowers_stated_confidence() {
    let db = workspace("confidence");
    let output = surface(&db, &["--relations", "page:app/actions_test.spg"]);
    let confidence = &output["summary"]["confidence"];
    assert_eq!(confidence["level"], "reduced");
    let reasons = confidence["reasons"].as_array().expect("reasons");
    let unknown_action = reasons
        .iter()
        .find(|reason| reason["code"] == "UNKNOWN_ACTION_TYPE")
        .expect("UNKNOWN_ACTION_TYPE 的影响必须被说出来");
    assert!(
        unknown_action["effect"]
            .as_str()
            .unwrap()
            .contains("保守回答")
    );
    assert!(
        output["diagnostics"]
            .as_array()
            .expect("diagnostics")
            .iter()
            .any(|entry| entry["code"] == "UNKNOWN_ACTION_TYPE"
                && entry["answer_effect"].is_string())
    );
}

/// 确定的「没有」不该把一个本来能确定的答案降级。
#[test]
fn test_a_confirmed_absence_keeps_confidence_full() {
    let db = workspace("absence-confidence");
    let output = surface(&db, &["--relations", "page:app/dataflow_embedded.spg"]);
    assert_eq!(output["summary"]["confidence"]["level"], "full");
}

/// 带前缀的歧义 target 同样逐个作答，并留下能继续深挖单个节点的命令。
///
/// 交回候选让模型自己再问一轮曾经是这里的行为。它没用：模型会把仅剩的命令机会用来
/// 重发同一条命令。能答就答，next_queries 留给「想单独看某一个」的下一步。
#[test]
fn test_ambiguous_prefixed_target_answers_every_candidate() {
    let db = workspace("ambiguous-next");
    let output = surface(&db, &["--explain", "comp:button1"]);
    assert!(output.get("error").is_none(), "{output}");
    assert_eq!(answered_targets(&output).len(), 2, "{output}");
    let next: Vec<&str> = output["next_queries"]
        .as_array()
        .expect("next_queries")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(!next.is_empty());
    // 每个被回答的节点都要留下可以单独深挖它的下一条命令。
    for target in answered_targets(&output) {
        assert!(
            next.iter().any(|command| command.contains(&target)),
            "{target} 没有对应的 next_query：{next:?}"
        );
    }
}

/// 不歧义的 target 不该被全答模式改变形状。
///
/// 全答会把 summary 换成 `answers` 数组。绝大多数调用是明确的单目标，它们读到的
/// 必须还是原来那个 summary，否则等于为了救一条用例把其余全部改坏。
#[test]
fn test_unambiguous_target_keeps_single_answer_shape() {
    let db = workspace("unambiguous-shape");
    let output = surface(&db, &["--explain", "comp:app/actions_test.spg|button1"]);
    assert!(output.get("error").is_none(), "{output}");
    assert!(output["summary"]["answers"].is_null(), "{output}");
    assert!(output["summary"]["what_is_it"].is_string(), "{output}");
}

/// 跳转要把目标页面说出来，且不能把参数节点当成页面。
#[test]
fn test_navigation_names_the_target_page() {
    let db = workspace("nav-statement");
    let output = surface(&db, &["--relations", "page:app/page_relations.spg"]);
    let summary = &output["summary"];
    assert_eq!(summary["page_jump_count"], 1, "{output}");
    assert_eq!(summary["page_embed_count"], 1, "{output}");
    let statement = output["summary"]["navigation_statement"]
        .as_str()
        .expect("navigation_statement");
    assert!(statement.contains("目标页面为："), "{statement}");
    assert!(statement.contains("page:"), "{statement}");
    assert!(!statement.contains("目标页面为：param"), "{statement}");
    assert!(
        !statement.contains("dataflow_embedded"),
        "嵌入页不能被描述为用户跳转目标：{statement}"
    );
    let conclusion = summary["conclusion"].as_str().expect("conclusion");
    assert!(conclusion.contains("1 个跳转"), "{conclusion}");
    assert!(!conclusion.contains("2 个跳转"), "{conclusion}");
}

/// 跳转和传参必须出现在 conclusion 本身。
///
/// 导航页的 conclusion 以「无写入目标、用户无法通过该页面更改数据」收尾。模型读完这句
/// 就去回答「跳转传了哪些参数」，答出来的是「无参数」——而 PassesParam 边就在同一次输出
/// 的 details 里。conclusion 是最被信任的那一句，页面最主要的行为必须写在里面。
#[test]
fn test_page_conclusion_states_parameter_passing() {
    let db = workspace("conclusion-params");
    let output = surface(&db, &["--relations", "page:app/page_relations.spg"]);
    let conclusion = output["summary"]["conclusion"]
        .as_str()
        .expect("conclusion");
    assert!(conclusion.contains("参数"), "{conclusion}");
    assert!(conclusion.contains("目标详情"), "{conclusion}");
}

/// 只写前缀是一次「有哪些页面」的探查，不是写错的 target。
///
/// 问题天生模糊（用户说不清、元数据也塞不进上下文），模型第一步做发现式探查是对的。
/// 此前工具用 TARGET_NOT_FOUND 回应，白白烧掉一次命令机会，模型接着就开始编路径。
#[test]
fn test_bare_prefix_enumerates_instead_of_erroring() {
    let db = workspace("prefix-enumeration");
    let output = surface(&db, &["--relations", "page:"]);
    assert!(output.get("error").is_none(), "{output}");
    let targets = output["summary"]["targets"]
        .as_array()
        .expect("targets")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(targets.len() >= 2, "{targets:?}");
    assert!(
        targets.iter().all(|id| id.starts_with("page:")),
        "{targets:?}"
    );
    // 列出来还不够：下一条命令要能直接照抄。
    let next = output["next_queries"].as_array().expect("next_queries");
    assert!(
        next.iter()
            .filter_map(Value::as_str)
            .all(|command| command.starts_with("--relations ")),
        "{next:?}"
    );
}

/// 模型级关系要给出通往字段血缘的入口。
///
/// `--relations model:X` 答的是模型级关系；「这个字段是从哪一路传过来的」只有
/// `--explain field:X.y` 能答。字段名就在合并进来的 DataFlow 子图里，模型不该自己猜。
#[test]
fn test_model_relations_offer_field_lineage_commands() {
    let db = workspace("field-lineage-next");
    let output = surface(&db, &["--relations", "model:df_b"]);
    let next: Vec<&str> = output["next_queries"]
        .as_array()
        .expect("next_queries")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        next.iter().any(|command| command.contains("field:df_b.")),
        "{next:?}"
    );
    // 占位符是「你自己去猜一个字段名」，有真实字段名时不该再出现。
    assert!(
        !next.iter().any(|command| command.contains("<字段名>")),
        "{next:?}"
    );
}

/// 跨页 target 的每一侧独立归一：逗号只是分隔符，不是豁免归一的理由。
///
/// 此前整串含逗号就跳过归一，`page:actions_test,page:page_relations` 被原样发给
/// QueryCross，拿不存在的节点 id 查出 0 条路径再以 full confidence 交回「没有关系」。
#[test]
fn test_relations_cross_normalizes_each_side_independently() {
    let db = workspace("cross-normalize-sides");
    let output = surface(
        &db,
        &["--relations", "page:actions_test,page:page_relations"],
    );
    assert!(
        output.get("ok").and_then(Value::as_bool) != Some(false),
        "归一后应正常执行: {}",
        output
    );
    assert_eq!(
        output["query_target"].as_str().expect("query_target"),
        "page:app/actions_test.spg <-> page:app/page_relations.spg"
    );
    let resolved = output["diagnostics"]
        .as_array()
        .expect("diagnostics")
        .iter()
        .filter(|entry| entry["code"].as_str() == Some("RESOLVED_TARGET"))
        .count();
    assert_eq!(resolved, 2, "两侧都应归一: {}", output["diagnostics"]);
}

/// 归一不出的一侧必须如实报 TARGET_NOT_FOUND，不能变成确认的空结果。
#[test]
fn test_relations_cross_reports_unresolvable_side() {
    let db = workspace("cross-unresolvable-side");
    let output = surface(&db, &["--relations", "page:actions_test,page:不存在的页面"]);
    assert_eq!(output["ok"].as_bool(), Some(false));
    assert_eq!(output["error"]["code"].as_str(), Some("TARGET_NOT_FOUND"));
    assert!(
        output["error"]["message"]
            .as_str()
            .expect("message")
            .contains("page:不存在的页面"),
        "{}",
        output["error"]
    );
}

/// graphdb-only 模式不需要 --project-dir 就能跑新表面的两个动词。
fn surface_graphdb_only(db: &Path, args: &[&str]) -> Value {
    let mut argv = vec![
        "--non-human".to_string(),
        "--graph-db-path".to_string(),
        db.to_str().unwrap().to_string(),
        "--budget".to_string(),
        "compact".to_string(),
    ];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    let output = Command::new(bin()).args(&argv).output().expect("run cli");
    let stdout = String::from_utf8(output.stdout).expect("utf-8");
    serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "输出不是 JSON: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// `--find` 此前不在 graphdb-only 门控名单里，带库查询被错当成缺参数拒绝。
#[test]
fn test_graphdb_only_find_works_without_project_dir() {
    let db = workspace("graphdb-only-find");
    let output = surface_graphdb_only(&db, &["--find", "actions_test"]);
    let matches = output["details"]["matches"].as_array().expect("matches");
    assert!(
        matches
            .iter()
            .any(|entry| entry["id"].as_str() == Some("page:app/actions_test.spg")),
        "{matches:?}"
    );
}

/// `--relations` 同上门控遗漏。
#[test]
fn test_graphdb_only_relations_works_without_project_dir() {
    let db = workspace("graphdb-only-relations");
    let output = surface_graphdb_only(&db, &["--relations", "page:app/actions_test.spg"]);
    assert_eq!(
        output["query_target"].as_str(),
        Some("page:app/actions_test.spg"),
        "{output}"
    );
}
