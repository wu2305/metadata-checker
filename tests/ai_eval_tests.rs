// use metadata_checker::output::AiOutput;
use std::path::PathBuf;
use std::sync::Mutex;

static CLI_LOCK: Mutex<()> = Mutex::new(());

/// M9-D AI 问答评测集结构校验与命令执行测试。
/// 验证 ai_eval_cases.json 的 schema 完整性，并确保 required_commands 真实可执行。
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

/// 运行 CLI 命令并返回 stdout
fn run_cli(args: &[&str]) -> String {
    let _guard = CLI_LOCK.lock().unwrap();
    let bin = std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker");
    let cmd_output = std::process::Command::new(&bin)
        .args(args)
        .output()
        .expect("Failed to run metadata-checker binary");
    String::from_utf8(cmd_output.stdout).expect("Invalid UTF-8")
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

/// ============================================================
/// 真实执行 required_commands，验证输出为稳定 AiOutput 契约
#[test]
fn test_ai_eval_commands_execute_and_validate() {
    // P0: 确保图数据库已构建
    let project_dir = PathBuf::from("tests/fixtures/test_project");
    let db_path = PathBuf::from("tests/fixtures/test_project/.metadata-checker.graphdb");
    if !db_path.exists() {
        metadata_checker::scanner::scan_project(&project_dir, &db_path)
            .expect("scan_project must succeed on test_project");
    }

    let cases = load_ai_eval_cases();
    let mut failures = Vec::new();

    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?").to_string();
        let cmds = case["required_commands"]
            .as_array()
            .expect("required_commands 必须是数组");

        for cmd_val in cmds {
            let cmd_str = cmd_val.as_str().unwrap();
            // 解析 shell 命令字符串为参数数组
            let args: Vec<String> = parse_shell_command(cmd_str);
            let output = run_cli(&args.iter().map(|s| s.as_str()).collect::<Vec<_>>());
            let output_trimmed = output.trim();

            // 验证输出是合法 JSON
            let parsed: serde_json::Value = match serde_json::from_str(output_trimmed) {
                Ok(v) => v,
                Err(e) => {
                    failures.push(format!(
                        "case {} 命令 '{}' 输出不是合法 JSON: {}\n原始输出前200字: {}",
                        case_id,
                        cmd_str,
                        e,
                        &output_trimmed[..output_trimmed.len().min(200)]
                    ));
                    continue;
                }
            };

            // 验证顶层 AiOutput 结构
            if let Err(err) = validate_ai_output_contract(&parsed, &case_id, cmd_str) {
                failures.push(err);
                continue;
            }

            // 验证关键字段非空：summary、details、evidence、diagnostics、next_queries
            for field in [
                "summary",
                "details",
                "evidence",
                "diagnostics",
                "next_queries",
            ] {
                if parsed.get(field).is_none() {
                    failures.push(format!(
                        "case {} 命令 '{}' 缺少顶层字段 {}",
                        case_id, cmd_str, field
                    ));
                }
            }

            // 验证 kind 非空且为合法枚举
            let kind = parsed
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("MISSING");
            let valid_kinds = [
                "SuperPage",
                "PageQuery",
                "ModelQuery",
                "CrossPageQuery",
                "DataFlowQuery",
                "ComponentQuery",
                "PriorityQuery",
                "Explain",
                "Context",
                "PageLogic",
            ];
            if !valid_kinds.contains(&kind) {
                failures.push(format!(
                    "case {} 命令 '{}' kind '{}' 不在合法枚举中",
                    case_id, cmd_str, kind
                ));
            }

            // 验证 summary 非空
            if let Some(summary) = parsed.get("summary")
                && summary.as_object().map(|o| o.is_empty()).unwrap_or(true)
            {
                failures.push(format!(
                    "case {} 命令 '{}' summary 为空对象",
                    case_id, cmd_str
                ));
            }

            // 检查是否包含明显错误信息（Node not found / Page not found / Error:）
            if output_trimmed.contains("Node not found")
                || output_trimmed.contains("Page not found")
                || output_trimmed.contains("Error:")
            {
                failures.push(format!(
                    "case {} 命令 '{}' 输出包含错误: {}",
                    case_id,
                    cmd_str,
                    &output_trimmed[..output_trimmed.len().min(200)]
                ));
            }
        }
    }

    if !failures.is_empty() {
        panic!("AI eval 命令执行验证失败:\n{}", failures.join("\n---\n"));
    }
}

/// 将 shell 命令字符串拆分为参数数组（简单实现，支持单引号）
fn parse_shell_command(cmd: &str) -> Vec<String> {
    // 使用 shlex 风格解析：按空格拆分，但保留单引号包裹的内容
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\u{0027}' {
            in_quote = !in_quote;
            i += 1;
            continue;
        }
        if c.is_whitespace() && !in_quote {
            if !current.is_empty() {
                args.push(current.clone());
                current.clear();
            }
            i += 1;
            continue;
        }
        current.push(c);
        i += 1;
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

/// 验证 AiOutput 顶层契约
fn validate_ai_output_contract(
    parsed: &serde_json::Value,
    case_id: &str,
    cmd_str: &str,
) -> Result<(), String> {
    let required_fields = [
        "schema_version",
        "kind",
        "query_target",
        "summary",
        "details",
        "evidence",
        "diagnostics",
        "next_queries",
    ];
    for field in &required_fields {
        if parsed.get(field).is_none() {
            return Err(format!(
                "case {} 命令 '{}' 缺少 AiOutput 顶层字段 {}",
                case_id, cmd_str, field
            ));
        }
    }
    Ok(())
}
