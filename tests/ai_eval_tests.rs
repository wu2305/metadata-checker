use std::path::PathBuf;

/// M9-D AI 问答评测集结构校验测试。
/// 验证 ai_eval_cases.json 的 schema 完整性、case 数量、字段非空、命令白名单等。
/// ============================================================
/// 加载 ai_eval_cases.json
fn load_ai_eval_cases() -> Vec<serde_json::Value> {
    let path = PathBuf::from("tests/fixtures/corpus/ai_eval/ai_eval_cases.json");
    let content = std::fs::read_to_string(&path).expect("ai_eval_cases.json must exist");
    let val: serde_json::Value =
        serde_json::from_str(&content).expect("ai_eval_cases.json must be valid JSON");
    val["cases"]
        .as_array()
        .expect("cases must be array")
        .to_vec()
}

/// ============================================================
/// 结构校验：顶层字段
#[test]
fn test_ai_eval_schema_version() {
    let path = PathBuf::from("tests/fixtures/corpus/ai_eval/ai_eval_cases.json");
    let content = std::fs::read_to_string(&path).expect("ai_eval_cases.json must exist");
    let val: serde_json::Value =
        serde_json::from_str(&content).expect("ai_eval_cases.json must be valid JSON");

    assert!(
        val.get("schema_version").is_some(),
        "顶层必须有 schema_version"
    );
    assert!(val.get("scope").is_some(), "顶层必须有 scope");
    assert!(val.get("eval_count").is_some(), "顶层必须有 eval_count");
    assert!(val.get("cases").is_some(), "顶层必须有 cases");
}

/// ============================================================
/// 数量校验：case ≥ 8
#[test]
fn test_ai_eval_case_count() {
    let cases = load_ai_eval_cases();
    assert!(
        cases.len() >= 8,
        "M9-D 要求至少 8 个 eval case，实际 {} 个",
        cases.len()
    );
}

/// ============================================================
/// 每个 case 必须包含完整字段
#[test]
fn test_ai_eval_each_case_fields() {
    let required_fields = [
        "case_id",
        "question",
        "project_dir",
        "required_commands",
        "expected_facts",
        "forbidden_claims",
        "evidence_requirements",
        "answer_rubric",
        "uncertainty_policy",
        "minimal_command_plan",
    ];
    let cases = load_ai_eval_cases();
    for case in &cases {
        for field in &required_fields {
            assert!(
                case.get(field).is_some(),
                "case {} 缺少字段 {}",
                case["case_id"].as_str().unwrap_or("?"),
                field
            );
        }
    }
}

/// ============================================================
/// expected_facts 和 forbidden_claims 必须非空
#[test]
fn test_ai_eval_expected_and_forbidden_non_empty() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let expected = case["expected_facts"]
            .as_array()
            .expect("expected_facts 必须是数组");
        let forbidden = case["forbidden_claims"]
            .as_array()
            .expect("forbidden_claims 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");

        assert!(
            !expected.is_empty(),
            "case {} 的 expected_facts 不能为空",
            case_id
        );
        assert!(
            !forbidden.is_empty(),
            "case {} 的 forbidden_claims 不能为空",
            case_id
        );
    }
}

/// ============================================================
/// 每个 case 至少有一个 evidence requirement
#[test]
fn test_ai_eval_evidence_requirements_non_empty() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let ev = case["evidence_requirements"]
            .as_array()
            .expect("evidence_requirements 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");
        assert!(
            !ev.is_empty(),
            "case {} 至少需要一个 evidence_requirement",
            case_id
        );
    }
}

/// ============================================================
/// 命令白名单校验：required_commands 必须是合法 CLI 格式
#[test]
fn test_ai_eval_command_whitelist() {
    let _allowed_prefixes = ["--project-dir"];
    let required_subcommands = [
        "--query-page-logic",
        "--explain",
        "--context",
        "--query-dataflow",
        "--query-model",
    ];
    let cases = load_ai_eval_cases();
    for case in &cases {
        let cmds = case["required_commands"]
            .as_array()
            .expect("required_commands 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");

        assert!(
            !cmds.is_empty(),
            "case {} 至少需要一个 required_command",
            case_id
        );

        for cmd in cmds {
            let cmd_str = cmd.as_str().expect("required_commands 每项必须是字符串");
            assert!(
                cmd_str.starts_with("--project-dir "),
                "case {} 的命令 '{}' 必须以 '--project-dir ' 开头",
                case_id,
                cmd_str
            );

            let has_subcommand = required_subcommands.iter().any(|sub| cmd_str.contains(sub));
            assert!(
                has_subcommand,
                "case {} 的命令 '{}' 必须包含合法子命令",
                case_id, cmd_str
            );
        }
    }
}

/// ============================================================
/// minimal_command_plan 长度校验：最多 3 步
#[test]
fn test_ai_eval_minimal_command_plan_length() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let plan = case["minimal_command_plan"]
            .as_array()
            .expect("minimal_command_plan 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");
        assert!(
            plan.len() <= 3,
            "case {} 的 minimal_command_plan 不能超过 3 步，实际 {} 步",
            case_id,
            plan.len()
        );
    }
}

/// ============================================================
/// answer_rubric 结构校验
#[test]
fn test_ai_eval_answer_rubric_structure() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let rubric = case["answer_rubric"]
            .as_object()
            .expect("answer_rubric 必须是对象");
        let case_id = case["case_id"].as_str().unwrap_or("?");

        assert!(
            rubric.contains_key("must_mention"),
            "case {} 的 answer_rubric 必须有 must_mention",
            case_id
        );
        assert!(
            rubric.contains_key("should_mention"),
            "case {} 的 answer_rubric 必须有 should_mention",
            case_id
        );
        assert!(
            rubric.contains_key("must_not_mention"),
            "case {} 的 answer_rubric 必须有 must_not_mention",
            case_id
        );
    }
}

/// ============================================================
/// uncertainty_policy 非空
#[test]
fn test_ai_eval_uncertainty_policy_non_empty() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let policy = case["uncertainty_policy"].as_str().unwrap_or("");
        let case_id = case["case_id"].as_str().unwrap_or("?");
        assert!(
            !policy.is_empty(),
            "case {} 的 uncertainty_policy 不能为空",
            case_id
        );
    }
}

/// ============================================================
/// eval_count 与 cases 长度一致
#[test]
fn test_ai_eval_count_consistent() {
    let path = PathBuf::from("tests/fixtures/corpus/ai_eval/ai_eval_cases.json");
    let content = std::fs::read_to_string(&path).expect("ai_eval_cases.json must exist");
    let val: serde_json::Value =
        serde_json::from_str(&content).expect("ai_eval_cases.json must be valid JSON");

    let declared = val["eval_count"].as_u64().unwrap_or(0) as usize;
    let actual = val["cases"].as_array().map(|a| a.len()).unwrap_or(0);
    assert_eq!(
        declared, actual,
        "eval_count {} 与 cases 实际长度 {} 不一致",
        declared, actual
    );
}
