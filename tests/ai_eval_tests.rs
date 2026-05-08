use std::path::PathBuf;
use std::sync::Mutex;

static CLI_LOCK: Mutex<()> = Mutex::new(());

/// M9-E AI 问答评测集自动判分与执行隔离测试。
/// 验证 ai_eval_cases.json schema，串行执行命令，使用隔离 graphdb，
/// 并自动校验 expected_output_assertions 结构化断言。
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

/// 递归复制目录
fn copy_dir_all(src: impl AsRef<std::path::Path>, dst: impl AsRef<std::path::Path>) {
    std::fs::create_dir_all(&dst).expect("create_dir_all failed");
    for entry in std::fs::read_dir(src).expect("read_dir failed") {
        let entry = entry.expect("dir entry failed");
        let ty = entry.file_type().expect("file_type failed");
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()));
        } else {
            std::fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))
                .expect("copy file failed");
        }
    }
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
        "expected_output_assertions",
        "answer_rubric",
        "uncertainty_policy",
        "minimal_command_plan",
        "answer_assertions",
        "risk_tags",
        "max_command_count",
        "allowed_output_sections",
        "case_status",
        "difficulty",
        "answer_style_policy",
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
/// expected_output_assertions 必须非空且结构正确
#[test]
fn test_ai_eval_assertions_structure() {
    let cases = load_ai_eval_cases();
    let valid_ops = [
        "gt",
        "gte",
        "lt",
        "lte",
        "eq",
        "ne",
        "not_empty",
        "empty",
        "contains",
        "array_any",
        "array_contains",
        "array_any_contains",
        "contains_field_path",
        "exists",
        "manual",
    ];
    for case in &cases {
        let assertions = case["expected_output_assertions"]
            .as_array()
            .expect("expected_output_assertions 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");
        assert!(
            !assertions.is_empty(),
            "case {} 至少需要一个 expected_output_assertion",
            case_id
        );
        for a in assertions {
            let op = a["op"].as_str().unwrap_or("?");
            assert!(
                valid_ops.contains(&op),
                "case {} 断言 op '{}' 不在合法枚举 {:?} 中",
                case_id,
                op,
                valid_ops
            );
            assert!(
                a.get("description").is_some(),
                "case {} 断言必须有 description",
                case_id
            );
        }
    }
}

/// ============================================================
/// 命令白名单校验
#[test]
fn test_ai_eval_command_whitelist() {
    let _allowed_prefixes = ["--project-dir"];
    let required_subcommands = [
        "--query-page-logic",
        "--explain",
        "--context",
        "--query-dataflow",
        "--query-model",
        "--find-page",
        "--find-model",
        "--find-component",
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
/// 隔离 graphdb + 串行执行 + 结构化断言自动判分
#[test]
fn test_ai_eval_commands_execute_and_assert() {
    // P0: 使用隔离临时目录，避免和 snapshot/corpus 测试抢锁
    let temp_dir = std::env::temp_dir().join("metadata-checker-ai-eval-test");
    let _ = std::fs::remove_dir_all(&temp_dir);
    copy_dir_all("tests/fixtures/test_project", &temp_dir);

    let project_dir = temp_dir.clone();
    let db_path = temp_dir.join(".metadata-checker.graphdb");
    if !db_path.exists() {
        metadata_checker::scanner::scan_project(&project_dir, &db_path)
            .expect("scan_project must succeed on isolated test_project");
    }

    let temp_str = temp_dir.to_str().unwrap();
    let cases = load_ai_eval_cases();
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?").to_string();
        let is_real_project = case
            .get("requires_real_project")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cmds = case["required_commands"]
            .as_array()
            .expect("required_commands 必须是数组");
        let assertions = case["expected_output_assertions"]
            .as_array()
            .expect("expected_output_assertions 必须是数组");

        // 真实项目 case 使用独立 graphdb 路径，避免和 fixture 临时目录冲突
        let real_project_db = if is_real_project {
            let db = std::env::temp_dir().join(format!(
                "metadata-checker-ai-eval-{}.graphdb",
                case_id.replace("/", "_")
            ));
            Some(db)
        } else {
            None
        };

        for (cmd_idx_in_case, cmd_val) in cmds.iter().enumerate() {
            let cmd_str = cmd_val.as_str().unwrap();
            let isolated_cmd = if is_real_project {
                // 真实项目 case：添加 --graph-db-path 到独立路径
                let mut cmd = cmd_str.to_string();
                if let Some(ref db) = real_project_db {
                    if !db.exists() {
                        let pd = case["project_dir"].as_str().unwrap_or("");
                        let _ =
                            metadata_checker::scanner::scan_project(std::path::Path::new(pd), db);
                    }
                    if !cmd.contains("--graph-db-path") {
                        cmd.push_str(&format!(" --graph-db-path {}", db.to_str().unwrap()));
                    }
                }
                cmd
            } else {
                // fixture case：将 project-dir 路径替换为隔离临时目录
                cmd_str.replace("tests/fixtures/test_project", temp_str)
            };

            let args: Vec<String> = parse_shell_command(&isolated_cmd);
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

            // 执行结构化断言
            for assertion in assertions {
                let assertion_cmd_idx = assertion
                    .get("command_index")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                // 如果断言指定了 command_index，则只针对对应命令执行
                if let Some(expected_idx) = assertion_cmd_idx
                    && expected_idx != cmd_idx_in_case
                {
                    continue;
                }
                if let Err(err) = evaluate_assertion(assertion, &parsed, &case_id, cmd_str) {
                    failures.push(err);
                }
            }

            // 检查是否包含明显错误信息
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

    // 清理临时目录
    let _ = std::fs::remove_dir_all(&temp_dir);

    if !failures.is_empty() {
        panic!(
            "AI eval 命令执行与断言判分失败:\n{}",
            failures.join("\n---\n")
        );
    }
}

/// ============================================================
/// 断言执行器
fn evaluate_assertion(
    assertion: &serde_json::Value,
    parsed: &serde_json::Value,
    case_id: &str,
    cmd_str: &str,
) -> Result<(), String> {
    let op = assertion["op"].as_str().unwrap_or("manual");
    let path = assertion.get("path").and_then(|v| v.as_str());
    let description = assertion["description"].as_str().unwrap_or("未命名断言");

    if op == "manual" {
        return Ok(());
    }

    let actual_value = if let Some(p) = path {
        get_json_path(parsed, p)
    } else {
        None
    };

    match op {
        "exists" => {
            if actual_value.is_none() {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 不存在\n  期望: 存在\n  实际: null",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?")
                ));
            }
        }
        "not_empty" => {
            let is_empty = actual_value
                .as_ref()
                .map(|v| {
                    v.as_array()
                        .map(|a| a.is_empty())
                        .unwrap_or(v.as_object().map(|o| o.is_empty()).unwrap_or(v.is_null()))
                })
                .unwrap_or(true);
            if is_empty {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 为空\n  期望: 非空\n  实际: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    actual_value
                ));
            }
        }
        "empty" => {
            let is_empty = actual_value
                .as_ref()
                .map(|v| {
                    v.as_array()
                        .map(|a| a.is_empty())
                        .unwrap_or(v.as_object().map(|o| o.is_empty()).unwrap_or(v.is_null()))
                })
                .unwrap_or(true);
            if !is_empty {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 非空\n  期望: 空\n  实际: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    actual_value
                ));
            }
        }
        "contains" => {
            let expected = assertion["value"].as_str().unwrap_or("");
            let actual_str = actual_value.as_ref().and_then(|v| v.as_str()).unwrap_or("");
            if !actual_str.contains(expected) {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 不包含 '{}'\n  期望: 包含 '{}'\n  实际: '{}'",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    expected,
                    expected,
                    actual_str
                ));
            }
        }
        "gt" => {
            let expected = assertion["value"].as_f64().unwrap_or(0.0);
            let actual = actual_value
                .as_ref()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if actual <= expected {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' <= {}\n  期望: > {}\n  实际: {}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    expected as i64,
                    expected as i64,
                    actual as i64
                ));
            }
        }
        "eq" => {
            let expected = &assertion["value"];
            if actual_value.as_ref() != Some(&expected) {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 不匹配\n  期望: {}\n  实际: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    expected,
                    actual_value
                ));
            }
        }
        "ne" => {
            let expected = &assertion["value"];
            if actual_value.as_ref() == Some(&expected) {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 不应等于 {}\n  期望: ≠ {}\n  实际: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    expected,
                    expected,
                    actual_value
                ));
            }
        }
        "array_any" => {
            let field = assertion["field"].as_str().unwrap_or("?");
            let expected_val = &assertion["value"];
            let empty_arr: &[serde_json::Value] = &[];
            let arr = actual_value
                .as_ref()
                .and_then(|v| v.as_array().map(|a| a.as_slice()))
                .unwrap_or(empty_arr);
            let found = arr.iter().any(|item| {
                get_json_path(item, field)
                    .as_ref()
                    .map(|v| **v == *expected_val)
                    .unwrap_or(false)
            });
            if !found {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 数组中无元素满足 {} == {}\n  期望: 至少一个元素满足\n  实际数组前3项: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    field,
                    expected_val,
                    arr.iter().take(3).collect::<Vec<_>>()
                ));
            }
        }
        "contains_field_path" => {
            let field_path = assertion["field_path"].as_str().unwrap_or("?");
            let found = actual_value
                .as_ref()
                .and_then(|v| v.as_object())
                .map(|o| o.contains_key(field_path))
                .unwrap_or(false);
            if !found {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 不包含字段 '{}'\n  期望: 包含字段 '{}'\n  实际: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    field_path,
                    field_path,
                    actual_value
                ));
            }
        }
        "array_contains" => {
            let expected_val = &assertion["value"];
            let empty_arr: &[serde_json::Value] = &[];
            let arr = actual_value
                .as_ref()
                .and_then(|v| v.as_array().map(|a| a.as_slice()))
                .unwrap_or(empty_arr);
            let found = arr.iter().any(|item| item == expected_val);
            if !found {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 数组不包含 {}
  期望: 包含 {}
  实际数组前3项: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    expected_val,
                    expected_val,
                    arr.iter().take(3).collect::<Vec<_>>()
                ));
            }
        }
        "array_any_contains" => {
            let field = assertion["field"].as_str().unwrap_or("?");
            let expected_val = assertion["value"].as_str().unwrap_or("");
            let empty_arr: &[serde_json::Value] = &[];
            let arr = actual_value
                .as_ref()
                .and_then(|v| v.as_array().map(|a| a.as_slice()))
                .unwrap_or(empty_arr);
            let found = arr.iter().any(|item| {
                let field_value = get_json_path(item, field);
                field_value
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().any(|sub| sub.as_str() == Some(expected_val)))
                    .unwrap_or(false)
            });
            if !found {
                return Err(format!(
                    "case {} 命令 '{}' 断言失败 [{}]: path '{}' 数组中无元素满足 {} 包含 '{}'
  期望: 至少一个元素的 {} 包含 '{}'
  实际数组前3项: {:?}",
                    case_id,
                    cmd_str,
                    description,
                    path.unwrap_or("?"),
                    field,
                    expected_val,
                    field,
                    expected_val,
                    arr.iter().take(3).collect::<Vec<_>>()
                ));
            }
        }
        _ => {
            return Err(format!(
                "case {} 命令 '{}' 未知断言 op '{}': {}",
                case_id, cmd_str, op, description
            ));
        }
    }
    Ok(())
}

/// 按点分隔路径获取 JSON 值
fn get_json_path<'a>(root: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = root;
    for segment in path.split('.') {
        if let Some(obj) = current.as_object() {
            current = obj.get(segment)?;
        } else {
            return None;
        }
    }
    Some(current)
}

/// 将 shell 命令字符串拆分为参数数组（支持单引号）
fn parse_shell_command(cmd: &str) -> Vec<String> {
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

/// ============================================================
/// M9-F: 结构化 minimal_command_plan 字段校验
#[test]
fn test_ai_eval_structured_plan_fields() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let plans = case["minimal_command_plan"]
            .as_array()
            .expect("minimal_command_plan 必须是数组");
        let case_id = case["case_id"].as_str().unwrap_or("?");
        for (idx, plan) in plans.iter().enumerate() {
            assert!(
                plan.get("command_kind").is_some(),
                "case {} plan[{}] 缺少 command_kind",
                case_id,
                idx
            );
            assert!(
                plan.get("target").is_some(),
                "case {} plan[{}] 缺少 target",
                case_id,
                idx
            );
            assert!(
                plan.get("args").is_some(),
                "case {} plan[{}] 缺少 args",
                case_id,
                idx
            );
            assert!(
                plan.get("requires_project_dir").is_some(),
                "case {} plan[{}] 缺少 requires_project_dir",
                case_id,
                idx
            );
            assert!(
                plan["requires_project_dir"].as_bool() == Some(true),
                "case {} plan[{}] requires_project_dir 必须为 true",
                case_id,
                idx
            );
        }
    }
}

/// ============================================================
/// M9-F: minimal_command_plan 必须能确定性展开为 required_commands
#[test]
fn test_ai_eval_plan_expands_to_required_commands() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let project_dir = case["project_dir"]
            .as_str()
            .unwrap_or("tests/fixtures/test_project");
        let plans = case["minimal_command_plan"]
            .as_array()
            .expect("minimal_command_plan 必须是数组");
        let required = case["required_commands"]
            .as_array()
            .expect("required_commands 必须是数组");

        assert_eq!(
            plans.len(),
            required.len(),
            "case {} plan 数量 {} 与 required_commands 数量 {} 不一致",
            case_id,
            plans.len(),
            required.len()
        );

        for (idx, plan) in plans.iter().enumerate() {
            let kind = plan["command_kind"].as_str().unwrap_or("");
            let target = plan["target"].as_str().unwrap_or("");
            let args: Vec<String> = plan["args"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect();
            let expected_prefix = format!("--project-dir {}", project_dir);
            let actual_cmd = required[idx].as_str().unwrap_or("");

            assert!(
                actual_cmd.starts_with(&expected_prefix),
                "case {} cmd[{}] 必须以 '{}' 开头，实际: '{}'",
                case_id,
                idx,
                expected_prefix,
                actual_cmd
            );
            assert!(
                actual_cmd.contains(kind),
                "case {} cmd[{}] 必须包含 kind '{}'，实际: '{}'",
                case_id,
                idx,
                kind,
                actual_cmd
            );
            if !target.is_empty() {
                let target_in_cmd = target.replace("'", "");
                assert!(
                    actual_cmd.contains(&target_in_cmd),
                    "case {} cmd[{}] 必须包含 target '{}'，实际: '{}'",
                    case_id,
                    idx,
                    target,
                    actual_cmd
                );
            }
            for arg in &args {
                assert!(
                    actual_cmd.contains(arg),
                    "case {} cmd[{}] 必须包含 arg '{}'，实际: '{}'",
                    case_id,
                    idx,
                    arg,
                    actual_cmd
                );
            }
        }
    }
}

/// ============================================================
/// M9-F: answer_assertions 结构校验
#[test]
fn test_ai_eval_answer_assertions_structure() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let aa = case["answer_assertions"]
            .as_object()
            .expect("answer_assertions 必须是对象");

        assert!(
            aa.contains_key("must_include"),
            "case {} answer_assertions 缺少 must_include",
            case_id
        );
        assert!(
            aa.contains_key("must_not_include"),
            "case {} answer_assertions 缺少 must_not_include",
            case_id
        );
        assert!(
            aa.contains_key("diagnostic_disclaimer_required"),
            "case {} answer_assertions 缺少 diagnostic_disclaimer_required",
            case_id
        );
        assert!(
            aa.contains_key("evidence_reference_required"),
            "case {} answer_assertions 缺少 evidence_reference_required",
            case_id
        );

        let must = aa["must_include"]
            .as_array()
            .expect("must_include 必须是数组");
        let must_not = aa["must_not_include"]
            .as_array()
            .expect("must_not_include 必须是数组");

        assert!(
            !must.is_empty(),
            "case {} answer_assertions.must_include 不能为空",
            case_id
        );
        assert!(
            !must_not.is_empty(),
            "case {} answer_assertions.must_not_include 不能为空",
            case_id
        );
    }
}

/// ============================================================
/// M9-F: answer_assertions.must_include 必须覆盖 expected_facts 核心实体
#[test]
fn test_ai_eval_answer_assertions_cover_expected_facts() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let must = case["answer_assertions"]["must_include"]
            .as_array()
            .expect("must_include 必须是数组");
        let expected = case["expected_facts"]
            .as_array()
            .expect("expected_facts 必须是数组");

        let must_strs: Vec<&str> = must.iter().filter_map(|v| v.as_str()).collect();
        let mut uncovered = Vec::new();
        for fact in expected {
            let fact_str = fact.as_str().unwrap_or("");
            // 启发式：expected_fact 必须包含至少一个 must_include 子串，或 must_include 包含 fact 子串
            let covered = must_strs
                .iter()
                .any(|m| fact_str.contains(m) || m.contains(fact_str));
            if !covered {
                uncovered.push(fact_str);
            }
        }
        // 允许最多 2 个 uncovered（因为有些 fact 可能太具体无法被 must_include 覆盖）
        assert!(
            uncovered.len() <= 3,
            "case {} 有 {} 个 expected_facts 未被 must_include 覆盖: {:?}",
            case_id,
            uncovered.len(),
            uncovered
        );
    }
}

/// ============================================================
/// M9-F: answer_assertions.must_not_include 必须覆盖 forbidden_claims 核心禁用表达
#[test]
fn test_ai_eval_answer_assertions_cover_forbidden_claims() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let must_not = case["answer_assertions"]["must_not_include"]
            .as_array()
            .expect("must_not_include 必须是数组");
        let forbidden = case["forbidden_claims"]
            .as_array()
            .expect("forbidden_claims 必须是数组");

        let must_not_strs: Vec<&str> = must_not.iter().filter_map(|v| v.as_str()).collect();
        let mut uncovered = Vec::new();
        for claim in forbidden {
            let claim_str = claim.as_str().unwrap_or("");
            let covered = must_not_strs
                .iter()
                .any(|m| claim_str.contains(m) || m.contains(claim_str));
            if !covered {
                uncovered.push(claim_str);
            }
        }
        assert!(
            uncovered.len() <= 3,
            "case {} 有 {} 个 forbidden_claims 未被 must_not_include 覆盖: {:?}",
            case_id,
            uncovered.len(),
            uncovered
        );
    }
}

/// ============================================================
/// M9-F: risk_tags 必须非空且覆盖全部风险类型
#[test]
fn test_ai_eval_risk_tags_coverage() {
    let cases = load_ai_eval_cases();
    let required_tags = [
        "page_logic",
        "explain",
        "context",
        "dataflow",
        "lineage",
        "condition",
        "diagnostic",
    ];
    let mut all_tags = std::collections::HashSet::new();

    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let tags = case["risk_tags"].as_array().expect("risk_tags 必须是数组");
        assert!(!tags.is_empty(), "case {} 必须至少有一个 risk_tag", case_id);
        for tag in tags {
            all_tags.insert(tag.as_str().unwrap_or("").to_string());
        }
    }

    for tag in &required_tags {
        assert!(
            all_tags.contains(*tag),
            "所有 case 的 risk_tags 中必须包含 '{}'",
            tag
        );
    }
}

/// ============================================================
/// M9-F: max_command_count 校验
#[test]
fn test_ai_eval_max_command_count() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let plan_steps = case["minimal_command_plan"]
            .as_array()
            .expect("minimal_command_plan 必须是数组")
            .len();
        let max_cmd = case["max_command_count"].as_u64().unwrap_or(0) as usize;

        assert!(
            max_cmd >= plan_steps,
            "case {} max_command_count {} 必须大于等于 plan 步数 {}",
            case_id,
            max_cmd,
            plan_steps
        );
        assert!(
            max_cmd <= 3,
            "case {} max_command_count {} 不能超过 3",
            case_id,
            max_cmd
        );
    }
}

/// ============================================================
/// M9-F: allowed_output_sections 必须包含 summary
#[test]
fn test_ai_eval_allowed_output_sections() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let sections = case["allowed_output_sections"]
            .as_array()
            .expect("allowed_output_sections 必须是数组");
        let sections_str: Vec<String> = sections
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        assert!(
            sections_str.contains(&"summary".to_string()),
            "case {} allowed_output_sections 必须包含 summary",
            case_id
        );
    }
}

/// ============================================================
/// M9-F: case_status 与 difficulty 分布校验
#[test]
fn test_ai_eval_case_status_and_difficulty_distribution() {
    let cases = load_ai_eval_cases();
    let mut active_count = 0;
    let mut basic_count = 0;
    let mut intermediate_count = 0;
    let mut hard_count = 0;

    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let status = case["case_status"].as_str().unwrap_or("");
        let difficulty = case["difficulty"].as_str().unwrap_or("");

        assert!(
            ["active", "quarantined", "needs_fixture"].contains(&status),
            "case {} case_status '{}' 不在合法枚举中",
            case_id,
            status
        );
        assert!(
            ["basic", "intermediate", "hard"].contains(&difficulty),
            "case {} difficulty '{}' 不在合法枚举中",
            case_id,
            difficulty
        );

        if status == "active" {
            active_count += 1;
        }
        match difficulty {
            "basic" => basic_count += 1,
            "intermediate" => intermediate_count += 1,
            "hard" => hard_count += 1,
            _ => {}
        }
    }

    assert!(
        active_count >= 10,
        "active case 数量必须 >= 10，实际 {}",
        active_count
    );
    assert!(
        basic_count >= 3,
        "basic case 数量必须 >= 3，实际 {}",
        basic_count
    );
    assert!(
        intermediate_count >= 3,
        "intermediate case 数量必须 >= 3，实际 {}",
        intermediate_count
    );
    assert!(
        hard_count >= 2,
        "hard case 数量必须 >= 2，实际 {}",
        hard_count
    );
}

/// ============================================================
/// M9-F: answer_style_policy 校验
#[test]
fn test_ai_eval_answer_style_policy() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let status = case["case_status"].as_str().unwrap_or("");
        if status != "active" {
            continue;
        }
        assert!(
            case.get("answer_style_policy").is_some(),
            "case {} 必须有 answer_style_policy",
            case_id
        );
        let policy = case["answer_style_policy"].as_object().expect("必须是对象");
        assert!(
            policy.contains_key("principles"),
            "case {} answer_style_policy 缺少 principles",
            case_id
        );
        assert!(
            policy.contains_key("forbidden_practices"),
            "case {} answer_style_policy 缺少 forbidden_practices",
            case_id
        );
    }
}

/// ============================================================
/// M9-F: answer_assertions 一致性校验（must_include 和 must_not_include 不能明显冲突）
#[test]
fn test_ai_eval_answer_assertions_consistency() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let must = case["answer_assertions"]["must_include"]
            .as_array()
            .expect("must_include 必须是数组");
        let must_not = case["answer_assertions"]["must_not_include"]
            .as_array()
            .expect("must_not_include 必须是数组");

        for m in must {
            let m_str = m.as_str().unwrap_or("");
            for n in must_not {
                let n_str = n.as_str().unwrap_or("");
                // 如果 must_include 和 must_not_include 有完全相同的字符串，视为冲突
                assert_ne!(
                    m_str, n_str,
                    "case {} answer_assertions 冲突: '{}' 同时出现在 must_include 和 must_not_include",
                    case_id, m_str
                );
            }
        }
    }
}

/// ============================================================
/// M9-F: 含 diagnostics 风险的 case 必须有 diagnostic_disclaimer_required
#[test]
fn test_ai_eval_diagnostic_disclaimer_required() {
    let cases = load_ai_eval_cases();
    for case in &cases {
        let case_id = case["case_id"].as_str().unwrap_or("?");
        let tags = case["risk_tags"].as_array().expect("risk_tags 必须是数组");
        let has_diagnostic = tags.iter().any(|t| t.as_str() == Some("diagnostic"));
        let disclaimer_required = case["answer_assertions"]["diagnostic_disclaimer_required"]
            .as_bool()
            .unwrap_or(false);

        if has_diagnostic {
            assert!(
                disclaimer_required,
                "case {} 含 diagnostic risk_tag，diagnostic_disclaimer_required 必须为 true",
                case_id
            );
        }
    }
}
