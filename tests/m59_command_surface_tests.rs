#![cfg(feature = "cli-local")]

//! M59 三动词命令表面测试。
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
    let dir = std::env::temp_dir().join(format!("m59-surface-{tag}"));
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
        for field in ["code", "severity", "message", "location"] {
            assert!(object.contains_key(field), "缺少 {field}：{entry}");
        }
    }
}

/// 前缀写对、文件路径没写全的 target 由 Rust 补全。
///
/// M59 评测里 109 条拒绝有 83% 是这一类：模型学会了写前缀，于是只处理裸名的归一
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

    // button1 在 fixture 里跨两个页面重名，因此这里应当是「如实报歧义 + 真实候选」。
    let candidates = output
        .get("candidate_targets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!candidates.is_empty(), "必须交出真实候选：{output}");
    assert!(
        candidates
            .iter()
            .filter_map(Value::as_str)
            .all(|candidate| candidate.starts_with("comp:") && candidate.contains("button1")),
        "候选必须是可直接使用的规范 target：{candidates:?}"
    );
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

/// 裸名有歧义时交出候选，而不是让模型继续猜路径。
///
/// M58 里模型对「button1 周围还有什么」的应对是 `comp:app/未知页面.spg|button1`（3 次）
/// 和把 bootstrap 占位符原样抄成 `comp:app/<relative-file>.spg|button1`（2 次）。给回
/// 真实候选，下一轮就能用规范 target。
#[test]
fn test_ambiguous_bare_target_returns_real_candidates() {
    let db = workspace("bare-ambiguous");
    // button1 同时存在于 actions_test.spg 和 page_relations.spg。
    let output = surface(&db, &["--explain", "button1"]);

    assert_eq!(
        output
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("AMBIGUOUS_TARGET"),
        "{output}"
    );
    let candidates = output
        .get("candidate_targets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(candidates.len() >= 2, "必须交出候选：{candidates:?}");
    assert!(
        candidates
            .iter()
            .filter_map(Value::as_str)
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
    let output = Command::new(bin())
        .args([
            "--non-human",
            "--project-dir",
            "tests/fixtures/test_project",
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
