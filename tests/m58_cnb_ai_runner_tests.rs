#![cfg(feature = "cli-local")]

#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use m58_ai_eval::{
    AgentTurn, CaseReport, ChatMessage, ChatRequest, CnbChatAdapter, CommandPolicy, CommandRequest,
    CommandTrace, ExecutedCommand, FakeModelAdapter, JudgeResult, ModelAdapter, RunReport,
    RunnerConfig, fixture_llm_cases, judge_answer, load_eval_cases, parse_agent_turn,
    redact_secret, run_case, run_fixture_llm_cases, validate_fixture_llm_case_count,
    write_run_report,
};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

/// 验证 M58 首批 fixture LLM case 仍保持 active 且首个 case 顺序稳定。
#[test]
fn test_m58_fixture_llm_has_active_cases() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);

    assert!(selected.len() >= 3);
    assert!(selected.iter().all(|case| case.case_status == "active"));
    assert_eq!(selected[0].case_id, "page_purpose_actions_test");
    assert!(!selected[0].question.is_empty());
    assert_eq!(selected[0].difficulty, "basic");
    assert!(selected[0].value.is_object());
}

/// 验证 live baseline 锁定 13 个 active fixture LLM case，避免评测集静默缩水。
#[test]
fn test_m58_live_runner_requires_exact_fixture_llm_case_count() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);
    assert_eq!(selected.len(), 13);

    let mut reduced = cases.clone();
    reduced.retain(|case| case.case_id != "condition_action_behavior");
    let error = validate_fixture_llm_case_count(&reduced, 13)
        .unwrap_err()
        .to_string();
    assert!(error.contains("fixture_llm case count"));
    assert!(error.contains("expected 13"));
}

/// 验证 fixture LLM 集合覆盖多个 Skill 理解任务族，而不是只测一轮页面问答。
#[test]
fn test_m58_fixture_llm_covers_skill_comprehension_families() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);
    let task_families = selected
        .iter()
        .map(|case| case.task_family.as_str())
        .collect::<BTreeSet<_>>();

    assert!(selected.len() >= 10);
    for task_family in [
        "page_logic",
        "dataflow",
        "navigation",
        "diagnostic",
        "condition",
    ] {
        assert!(
            task_families.contains(task_family),
            "缺少任务族 {task_family}"
        );
    }
    assert!(
        selected
            .iter()
            .any(|case| case.evaluation_dimensions.error_injection)
    );
    assert!(
        selected
            .iter()
            .any(|case| case.evaluation_dimensions.output_truncation)
    );
    assert!(
        selected
            .iter()
            .any(|case| case.evaluation_dimensions.dependency_depth >= 2)
    );
}

/// 验证 CNB live 入口明确启用多 trial，而本地报告文档暴露稳定性指标。
#[test]
fn test_m58_live_trial_contract_is_explicit() {
    let pipeline = std::fs::read_to_string(".cnb.yml").unwrap();
    assert!(pipeline.contains("M58_AI_EVAL_TRIALS: \"3\""));
    assert!(pipeline.contains("test \"${M58_AI_EVAL_TRIALS:-0}\" -gt 1"));

    let template = std::fs::read_to_string("docs/reference/ai-eval-run-template.md").unwrap();
    for field in [
        "trial_pass_rate",
        "case_stable_pass_rate",
        "cases_with_flaky_trials",
        "M58_AI_EVAL_TRIALS",
    ] {
        assert!(template.contains(field), "报告模板缺少 {field}");
    }
}

/// CI 报告阶段必须整份输出 run.md。
///
/// 之前这一阶段用 `grep` 逐字段捞 run.json：grep 只匹配键名所在行，failure_classes /
/// command_trace_stats 这类聚合对象的体永远打不出来，`grep -A N` 又把命令轨迹从中间截断。
/// 结果是一次 CI 评测跑完，日志里没有足够信息判断模型为什么失败。
#[test]
fn test_m58_ci_report_stage_emits_full_markdown_report() {
    let pipeline = std::fs::read_to_string(".cnb.yml").unwrap();
    assert!(pipeline.contains("cat target/m58-ai-eval/run.md"));
    assert!(
        !pipeline.contains("target/m58-ai-eval/run.json\n        echo"),
        "报告阶段不应再用 grep 逐字段抽取 run.json"
    );
}

/// AI 评测集执行测试必须真的在 CI 里跑。
///
/// 它曾经在 main 与分支两条流水线里都被 `--skip` 掉，于是评测集回归在 CI 上永远是绿的。
/// 缺真实项目语料现在由测试内的存在性门禁显式跳过，不再需要整条测试消失。
#[test]
fn test_m58_ai_eval_execution_test_is_not_skipped_in_ci() {
    let pipeline = std::fs::read_to_string(".cnb.yml").unwrap();
    assert!(
        !pipeline.contains("--skip test_ai_eval_commands_execute_and_assert"),
        "AI 评测执行测试不应在 CI 中被跳过"
    );
}

/// 验证 fixture case 可以声明供报告和难度分析使用的任务维度。
#[test]
fn test_m58_loader_exposes_evaluation_dimensions() {
    let temp_path = std::env::temp_dir().join(format!(
        "m58-evaluation-dimensions-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let content = r#"{
        "schema_version":"1.3",
        "cases":[{
            "case_id":"dimension_case",
            "question":"q",
            "tier":"fixture_llm",
            "case_status":"active",
            "difficulty":"hard",
            "risk_tags":["condition"],
            "evaluation_dimensions":{
                "task_family":"condition",
                "target_resolution":"search",
                "distractor_count":2,
                "dependency_depth":3,
                "stateful":true,
                "error_injection":false,
                "output_truncation":true
            }
        }]
    }"#;
    std::fs::write(&temp_path, content).unwrap();

    let cases = load_eval_cases(&temp_path).unwrap();
    assert_eq!(cases[0].task_family, "condition");
    assert_eq!(cases[0].difficulty, "hard");
    assert_eq!(cases[0].evaluation_dimensions.target_resolution, "search");
    assert_eq!(cases[0].evaluation_dimensions.distractor_count, 2);
    assert_eq!(cases[0].evaluation_dimensions.dependency_depth, 3);
    assert_eq!(cases[0].evaluation_dimensions.stateful, true);
    assert_eq!(cases[0].evaluation_dimensions.output_truncation, true);

    std::fs::remove_file(temp_path).unwrap();
}

/// 验证 runner 不接受零次 trial，避免生成看似成功的空报告。
#[test]
fn test_m58_runner_rejects_zero_trial_count() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);
    let output_dir = unique_test_output_dir("m58-zero-trials");
    let mut config = fixture_runner_config(output_dir.clone());
    config.trial_count = 0;
    let mut adapter = FakeModelAdapter::from_responses(Vec::new());

    let error = run_fixture_llm_cases(&selected, &mut adapter, &config)
        .unwrap_err()
        .to_string();
    assert!(error.contains("trial_count"));
    assert_eq!(adapter.requests().len(), 0);

    if output_dir.exists() {
        std::fs::remove_dir_all(output_dir).unwrap();
    }
}

/// 验证旧 schema 缺失 tier 时仍按真实项目规则回退。
#[test]
fn test_m58_loader_defaults_missing_tier() {
    let temp_path = std::env::temp_dir().join("m58-missing-tier-case.json");
    let content = r#"{
        "schema_version": "1.0",
        "cases": [
            {"case_id":"fixture_case","question":"q","case_status":"active"},
            {"case_id":"real_case","question":"q","requires_real_project":true}
        ]
    }"#;
    std::fs::write(&temp_path, content).unwrap();

    let cases = load_eval_cases(&temp_path).unwrap();
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].tier, m58_ai_eval::EvalTier::FixtureStructural);
    assert_eq!(cases[1].tier, m58_ai_eval::EvalTier::RealManual);

    std::fs::remove_file(temp_path).unwrap();
}

/// 验证模型只能返回严格的 command/final JSON 协议。
#[test]
fn test_m58_agent_turn_protocol_accepts_command_and_final() {
    let command = parse_agent_turn(
        r#"{"kind":"command","command_kind":"--relations","target":"page:app/actions_test.spg","args":[],"budget":null}"#,
    )
    .unwrap();
    assert_eq!(
        command,
        AgentTurn::Command(CommandRequest {
            command_kind: "--relations".to_string(),
            target: "page:app/actions_test.spg".to_string(),
            args: Vec::new(),
            budget: None,
        })
    );

    let final_turn = parse_agent_turn(r#"{"kind":"final","answer":"页面有按钮入口。"}"#).unwrap();
    assert_eq!(final_turn, AgentTurn::Final("页面有按钮入口。".to_string()));
}

/// 验证 JSON 协议拒绝未知字段、非法 JSON 和 shell 注入载荷。
#[test]
fn test_m58_agent_turn_protocol_rejects_unsafe_payloads() {
    assert!(parse_agent_turn(
        r#"{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg","args":[],"budget":null,"project_dir":"/tmp/override"}"#,
    )
    .is_err());
    assert!(parse_agent_turn("{not-json").is_err());
    assert!(parse_agent_turn(
        r#"{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg","args":["--depth","2; touch /tmp/pwned"],"budget":null}"#,
    )
    .is_err());
    assert!(parse_agent_turn(
        r#"{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg && whoami","args":[],"budget":null}"#,
    )
    .is_err());
}

/// 验证白名单命令固定项目、graphdb 和二进制路径，且不能重复消费步骤。
#[test]
fn test_m58_command_policy_binds_paths_and_steps() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "page_purpose_actions_test")
        .unwrap();
    let project_dir = PathBuf::from("/private/tmp/m58-fixed-project");
    let graph_db_path = PathBuf::from("/private/tmp/m58-fixed.graphdb");
    let binary_path = PathBuf::from("/private/tmp/metadata-checker");
    let policy = CommandPolicy::from_case(
        case,
        project_dir.clone(),
        graph_db_path.clone(),
        binary_path.clone(),
    )
    .unwrap();
    let request = CommandRequest {
        command_kind: "--relations".to_string(),
        target: "page:app/actions_test.spg".to_string(),
        args: Vec::new(),
        budget: Some("compact".to_string()),
    };

    let validated = policy.validate(&request, &[], 0).unwrap();
    assert_eq!(validated.step_index(), Some(0));
    assert_eq!(validated.binary_path(), binary_path.as_path());
    assert_eq!(
        validated.argv(),
        vec![
            OsString::from("--non-human"),
            OsString::from("--project-dir"),
            project_dir.into_os_string(),
            OsString::from("--graph-db-path"),
            graph_db_path.into_os_string(),
            OsString::from("--relations"),
            OsString::from("page:app/actions_test.spg"),
            OsString::from("--budget"),
            OsString::from("compact"),
        ]
    );
    // 原样重发：那一步已被消费，且拿到的是同一份输出，仍然拒绝。
    let used = |steps: &[usize]| -> Vec<ExecutedCommand> {
        steps
            .iter()
            .map(|index| ExecutedCommand::for_test(*index, &request))
            .collect()
    };
    assert!(policy.validate(&request, &used(&[0]), 0).is_err());
    assert!(policy.validate(&request, &used(&[0, 1]), 0).is_err());
    assert!(
        policy
            .validate(
                &CommandRequest {
                    target: "page:app/other.spg".to_string(),
                    ..request
                },
                &[],
                0,
            )
            .is_err()
    );
}

/// 验证 plan 未声明预算时，模型显式 compact 仍使用默认预算语义。
#[test]
fn test_m58_command_policy_defaults_missing_budget_to_compact() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "readonly_page_check")
        .unwrap();
    assert_eq!(
        case.value["minimal_command_plan"][0]["budget"],
        serde_json::Value::Null
    );
    let policy = CommandPolicy::from_case(
        case,
        PathBuf::from("/private/tmp/m58-fixed-project"),
        PathBuf::from("/private/tmp/m58-fixed.graphdb"),
        PathBuf::from("/private/tmp/metadata-checker"),
    )
    .unwrap();

    let validated = policy
        .validate(
            &CommandRequest {
                command_kind: "--relations".to_string(),
                target: "page:app/dataflow_embedded.spg".to_string(),
                args: Vec::new(),
                budget: Some("compact".to_string()),
            },
            &[],
            0,
        )
        .unwrap();
    assert_eq!(validated.argv().last(), Some(&OsString::from("compact")));

    let default_validated = policy
        .validate(
            &CommandRequest {
                command_kind: "--relations".to_string(),
                target: "page:app/dataflow_embedded.spg".to_string(),
                args: Vec::new(),
                budget: None,
            },
            &[],
            0,
        )
        .unwrap();
    assert_eq!(
        default_validated.argv().last(),
        Some(&OsString::from("compact"))
    );
}

/// 验证 fake adapter 按顺序返回响应，并记录每轮对话 history。
#[test]
fn test_m58_fake_adapter_records_history_and_fails_when_empty() {
    let mut adapter = FakeModelAdapter::from_responses(vec!["first".to_string()]);
    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "question".to_string(),
        }],
        model: "ignored-by-fake".to_string(),
        stream: false,
        reasoning_effort: None,
    };

    assert_eq!(adapter.complete(&request).unwrap(), "first");
    assert!(adapter.complete(&request).is_err());
    assert_eq!(adapter.requests().len(), 2);
    assert_eq!(adapter.requests()[0], request);
    assert_eq!(adapter.requests()[1], request);
}

/// 验证 CNB adapter 的 URL、Authorization 和 JSON body contract。
#[test]
fn test_m58_cnb_adapter_sends_redacted_safe_request() {
    let token = "m58-test-secret-token";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"kind\\\":\\\"final\\\",\\\"answer\\\":\\\"\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"\\\"}\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "configured-model".to_string(),
    )
    .unwrap();
    let debug = format!("{adapter:?}");
    assert!(!debug.contains(token));

    let request = ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "question".to_string(),
        }],
        model: "caller-override-model".to_string(),
        stream: true,
        reasoning_effort: None,
    };
    assert_eq!(
        adapter.complete(&request).unwrap(),
        r#"{"kind":"final","answer":"ok"}"#
    );

    let raw_request = server.join().unwrap();
    let raw_request_lower = raw_request.to_ascii_lowercase();
    assert!(raw_request.starts_with("POST /org/repo/-/ai/chat/completions HTTP/1.1"));
    assert!(raw_request_lower.contains("authorization: bearer m58-test-secret-token"));
    let request_body = raw_request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap();
    assert!(!request_body.contains(token));
    let request_json: serde_json::Value = serde_json::from_str(request_body).unwrap();
    assert_eq!(request_json["model"], "configured-model");
    assert_eq!(request_json["stream"], true);
    assert_eq!(request_json["messages"][0]["role"], "user");
    assert_eq!(request_json["messages"][0]["content"], "question");
    assert!(raw_request_lower.contains("accept: text/event-stream"));
}

/// 验证 CNB SSE 流里的非法 JSON 不会泄漏 token。
#[test]
fn test_m58_cnb_sse_rejects_invalid_json_without_leaking_token() {
    let token = "m58-invalid-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"{\"}}]}\n\n",
        "data: {not-json m58-invalid-secret}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("CNB AI Chat"));
}

/// 验证成功响应必须声明 text/event-stream，避免把普通 JSON 当成 SSE 解析。
#[test]
fn test_m58_cnb_sse_rejects_non_event_stream_content_type() {
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "application/json", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-content-type-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(error.contains("Content-Type"));
    assert!(error.contains("text/event-stream"));
}

/// 缺省不得发送 reasoning_effort，否则历史基线不可比。
///
/// CNB swagger 的 body schema 只声明 messages/model/stream。多发一个字段会改变请求体，
/// 之前所有 run 都是在不带该字段的条件下跑出来的。
#[test]
fn test_m58_chat_request_omits_reasoning_effort_by_default() {
    let request = ChatRequest {
        messages: Vec::new(),
        model: "m".to_string(),
        stream: true,
        reasoning_effort: None,
    };
    let body = serde_json::to_string(&request).unwrap();
    assert!(
        !body.contains("reasoning_effort"),
        "缺省请求体不得包含 reasoning_effort: {body}"
    );

    let with_effort = ChatRequest {
        reasoning_effort: Some("high".to_string()),
        ..request
    };
    let body = serde_json::to_string(&with_effort).unwrap();
    assert!(body.contains("\"reasoning_effort\":\"high\""));
}

/// 统计 reasoning_content 长度，但绝不把推理原文带出解析层。
///
/// 该计数是 `reasoning_effort` 是否真的生效的唯一证据：不带该字段时实测恒为 0。
#[test]
fn test_m58_cnb_adapter_counts_reasoning_chars_without_leaking_them() {
    let secret_reasoning = "让我想想这道题目的解法";
    let response_body = format!(
        "data: {{\"model\":\"model\",\"choices\":[{{\"delta\":{{\"reasoning_content\":\"{secret_reasoning}\"}}}}]}}\n\n\
         data: {{\"model\":\"model\",\"choices\":[{{\"delta\":{{\"content\":\"ok\"}}}}]}}\n\n\
         data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", &response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-reasoning-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();

    let content = adapter.complete(&empty_chat_request()).unwrap();
    server.join().unwrap();
    assert_eq!(content, "ok", "推理内容不得混进 assistant content");
    assert_eq!(
        adapter.reasoning_chars(),
        secret_reasoning.chars().count(),
        "必须按字符数统计推理长度"
    );
}

/// 验证服务端静默换模型时会报错，而不是产出一份标着错误模型名的基线。
///
/// CNB 对未知模型名不报错，而是用默认模型服务请求（实测请求
/// `definitely-not-a-real-model-xyz` 同样返回 deepseek-v4-flash）。M58 的目的就是拿不同
/// 便宜模型做对比，如果不校验，把 M58_CNB_MODEL 换成 gemma4-31b 只会得到一份「标着
/// gemma4、实际由默认模型回答」的报告，整个对比静默失效。
#[test]
fn test_m58_cnb_adapter_rejects_silently_substituted_model() {
    let response_body = concat!(
        "data: {\"model\":\"deepseek-v4-flash\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-model-swap-secret".to_string(),
        "gemma4-31b".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(error.contains("deepseek-v4-flash"), "错误必须点名实际模型");
    assert!(error.contains("gemma4-31b"), "错误必须点名请求的模型");
}

/// 服务端回报的模型与请求一致时不得误报。
#[test]
fn test_m58_cnb_adapter_accepts_matching_served_model() {
    let response_body = concat!(
        "data: {\"model\":\"gemma4-31b\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-model-match-secret".to_string(),
        "gemma4-31b".to_string(),
    )
    .unwrap();

    let content = adapter.complete(&empty_chat_request()).unwrap();
    server.join().unwrap();
    assert_eq!(content, "ok");
}

/// 验证 SSE 中意外的非 data 行不会被静默忽略。
#[test]
fn test_m58_cnb_sse_rejects_unexpected_non_data_line() {
    let response_body = concat!(
        "unexpected: m58-invalid-sse-line\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-sse-line-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(error.contains("unexpected SSE line"));
}

/// 验证 CNB SSE 流只有 [DONE] 时会返回错误。
#[test]
fn test_m58_cnb_sse_rejects_done_only_stream_without_leaking_token() {
    let token = "m58-done-only-secret";
    let response_body = "data: [DONE]\n\n";
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("CNB AI Chat"));
}

/// 验证 CNB SSE 流中空 choices 数组会被拒绝。
#[test]
fn test_m58_cnb_sse_rejects_empty_choices_without_leaking_token() {
    let token = "m58-empty-choices-secret";
    let response_body = concat!("data: {\"choices\":[]}\n\n", "data: [DONE]\n\n",);
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("CNB AI Chat"));
}

/// 验证 CNB SSE chunk 里 delta 缺失时会返回明确错误且不泄漏 token。
#[test]
fn test_m58_cnb_sse_rejects_missing_delta_without_leaking_token() {
    let token = "m58-missing-choices-secret";
    let response_body = concat!("data: {\"choices\":[{}]}\n\n", "data: [DONE]\n\n",);
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("CNB AI Chat SSE chunk 缺少 delta"));
}

/// 思考把输出预算吃光时，报错必须点名 finish_reason=length。
///
/// 这是 reasoning_effort=high 那次 A/B 里 `button_submit_effect` 3/3 失败的形态：
/// 旧代码统一报「SSE 内容为空」，既看不出是被截断还是上游没返回，也看不出思考
/// 已经烧掉了多少预算，只能靠重跑猜。
#[test]
fn test_m58_cnb_sse_reports_length_truncation_when_thinking_exhausts_budget() {
    let token = "m58-length-truncation-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想了很久\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(
        error.contains("finish_reason=length"),
        "必须点名截断原因，实际为: {error}"
    );
    assert!(
        error.contains("thinking 4 字符"),
        "必须带上思考规模，实际为: {error}"
    );
}

/// 有服务端 usage 时，空回答的报错要给出权威 token 数，而不只是客户端字符数。
///
/// 报文形状照抄真实抓包：deepseek 把 usage 挂在最后一个带 choices 的 chunk 上，
/// 且除末个 chunk 外 finish_reason 一律是空串（不是 null）。
#[test]
fn test_m58_cnb_sse_reports_server_usage_on_empty_answer() {
    let token = "m58-usage-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"},\"finish_reason\":\"\"}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"\"},\"finish_reason\":\"length\"}],\
         \"usage\":{\"completion_tokens\":64,\"completion_thinking_tokens\":64}}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("finish_reason=length"), "实际为: {error}");
    assert!(
        error.contains("completion_tokens=64") && error.contains("thinking_tokens=64"),
        "必须带上服务端权威 token 计数，实际为: {error}"
    );
}

/// 末尾只带 usage、不带 choices 的收尾 chunk 不能被当成坏流。
///
/// deepseek 把 usage 挂在最后一个 choices chunk 上，但 M58 要换的模型未必；
/// OpenAI 兼容流普遍会多发一个 choices 为空的 usage chunk。
#[test]
fn test_m58_cnb_sse_accepts_trailing_usage_only_chunk() {
    let token = "m58-usage-only-chunk-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"completion_tokens\":7,\"completion_thinking_tokens\":0}}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let content = adapter.complete(&empty_chat_request()).unwrap();
    server.join().unwrap();
    assert_eq!(content, "ok");
}

/// 模型只思考、不回答时，报错要和「上游什么都没返回」区分开。
#[test]
fn test_m58_cnb_sse_reports_reasoning_only_response() {
    let token = "m58-reasoning-only-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"只想不说\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(
        error.contains("只返回了思考内容") && error.contains("finish_reason=stop"),
        "实际为: {error}"
    );
}

/// 回答完整但被 length 截断，同样要失败，而不是把半截 JSON 交给 judge。
#[test]
fn test_m58_cnb_adapter_rejects_truncated_answer() {
    let token = "m58-truncated-answer-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"kind\\\":\\\"fin\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("回答被输出预算截断"), "实际为: {error}");
}

/// 流完好但模型只思考不作答时，trial 必须判 protocol_error 而不是 runner_error。
///
/// M58 评的是「模型能否借助工具答对」，判据只能是它的**回答**。连接、鉴权、SSE 帧、
/// `[DONE]`、模型身份全部正常，只是模型把 completion 预算花光在思考上——责任在模型。
/// 记成 runner_error 等于用我们这侧的基础设施故障替模型背锅：reasoning_effort 的 A/B
/// 里 15 个 trial 被算在错误的账上，直接影响 pass_rate 的可比性。
#[test]
fn test_m58_runner_classifies_reasoning_only_answer_as_protocol_error() {
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想了很久\"},\"finish_reason\":\"\"}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\
         \"usage\":{\"completion_tokens\":114,\"completion_thinking_tokens\":114}}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-protocol-error-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-reasoning-only-protocol");
    let config = fixture_runner_config(output_dir.clone());

    let report = run_case(&case, &mut adapter, &config).unwrap();
    server.join().unwrap();

    assert_eq!(report.failure_classes, vec!["protocol_error".to_string()]);
    let note = report.judge_notes.join(" ");
    assert!(
        note.contains("model returned no usable answer"),
        "protocol_error 必须写明是模型没给回答，实际为: {note}"
    );
    assert!(
        note.contains("只返回了思考内容"),
        "judge_notes 必须保留确定性归因，实际为: {note}"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 边界另一侧：一个思考字符都没有的空流仍然是我们这侧的传输问题，判 runner_error。
///
/// 没有这条，上面那条改动会把所有空回答一律洗成 protocol_error，真正的上游故障
/// 就被算进模型的账，方向刚好反了。
#[test]
fn test_m58_runner_keeps_runner_error_for_empty_stream_without_thinking() {
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "m58-runner-error-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-empty-stream-runner-error");
    let config = fixture_runner_config(output_dir.clone());

    let report = run_case(&case, &mut adapter, &config).unwrap();
    server.join().unwrap();

    assert_eq!(report.failure_classes, vec!["runner_error".to_string()]);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 内联 `<think>` 段必须被剥掉并计入 reasoning_chars，不得混进 assistant content。
///
/// CNB 上的 deepseek 走 `reasoning_content` 旁路，但 M58 要换的 gemma4 / nemotron
/// 普遍把思考内联在 content 里。不剥的话思考原文会进 history、进报告、进 judge 输入。
/// 起止标记会被 SSE 切在不同 chunk，所以必须在拼接完成后剥。
#[test]
fn test_m58_cnb_sse_strips_inline_think_block_split_across_chunks() {
    let token = "m58-inline-think-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"<thi\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"nk>四个字符\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"</think>ok\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let content = adapter.complete(&empty_chat_request()).unwrap();
    server.join().unwrap();
    assert_eq!(content, "ok", "内联思考不得混进 assistant content");
    assert_eq!(
        adapter.reasoning_chars(),
        "四个字符".chars().count(),
        "内联思考必须计入 reasoning_chars"
    );
}

/// 只有内联思考、没有回答时，剥完是空串，必须报「只返回了思考内容」。
#[test]
fn test_m58_cnb_sse_reports_inline_think_only_response() {
    let token = "m58-inline-think-only-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"<think>还没想完\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let error = adapter
        .complete(&empty_chat_request())
        .unwrap_err()
        .to_string();
    server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("只返回了思考内容"), "实际为: {error}");
}

/// 验证 CNB SSE delta 里的 provider 额外字段不会影响 content 拼接。
#[test]
fn test_m58_cnb_sse_accepts_provider_delta_extra_fields_without_leaking_token() {
    let token = "m58-extra-fields-secret";
    let response_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"chain\",\"function_call\":null,\"refusal\":null,\"tool_calls\":[],\"content\":\"{\\\"kind\\\":\\\"final\\\",\\\"answer\\\":\\\"ok\\\"}\",\"extra_fields\":{\"provider\":\"cnb\"}}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", "text/event-stream", response_body);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();

    let content = adapter.complete(&empty_chat_request()).unwrap();
    server.join().unwrap();
    assert_eq!(content, r#"{"kind":"final","answer":"ok"}"#);
}

/// 验证 CNB HTTP 错误和缺字段响应不会泄漏 token 或伪造成功。
#[test]
fn test_m58_cnb_adapter_redacts_http_error_and_rejects_invalid_response() {
    let token = "m58-error-secret";
    let (endpoint, error_server) = spawn_fake_cnb_server(
        "500 Internal Server Error",
        "application/json",
        r#"{"error":"m58-error-secret"}"#,
    );
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "model".to_string(),
    )
    .unwrap();
    let request = empty_chat_request();
    let error = adapter.complete(&request).unwrap_err().to_string();
    error_server.join().unwrap();
    assert!(!error.contains(token));
    assert!(error.contains("HTTP 500"));
    assert!(error.contains("[REDACTED]"));
    assert_eq!(
        redact_secret("token=m58-error-secret", token),
        "token=[REDACTED]"
    );

    let (endpoint, missing_choice_server) =
        spawn_fake_cnb_server("200 OK", "application/json", r#"{"choices":[]}"#);
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        "another-secret".to_string(),
        "model".to_string(),
    )
    .unwrap();
    assert!(adapter.complete(&request).is_err());
    missing_choice_server.join().unwrap();
}

/// 验证 CNB HTTP/transport 失败在没有 assistant content 时归为 runner_error。
#[test]
fn test_m58_fake_runner_records_runner_error_for_cnb_transport_failure() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-cnb-transport-runner-error");
    let config = fixture_runner_config(output_dir.clone());
    let token = "m58-runner-transport-token";
    let (endpoint, error_server) = spawn_fake_cnb_server(
        "500 Internal Server Error",
        "application/json",
        r#"{"error":"service unavailable"}"#,
    );
    let mut adapter = CnbChatAdapter::new(
        endpoint,
        "org/repo".to_string(),
        token.to_string(),
        "fake-model".to_string(),
    )
    .unwrap();

    let report = run_case(&case, &mut adapter, &config).unwrap();
    error_server.join().unwrap();

    assert_eq!(report.status, "error");
    assert_eq!(report.passed, false);
    assert_eq!(report.failure_classes, vec!["runner_error".to_string()]);
    assert_eq!(report.command_trace, Vec::<CommandTrace>::new());
    // 诊断必须带上 HTTP 状态，否则 CI 日志里区分不了「上游 500」和「模型答错」。
    assert_eq!(report.judge_notes.len(), 1);
    assert!(
        report.judge_notes[0].starts_with("model adapter completion failed: "),
        "实际为: {}",
        report.judge_notes[0]
    );
    assert!(report.judge_notes[0].contains("HTTP 500"));
    // 诊断进了报告，token 仍然不能进。
    assert!(!report.judge_notes[0].contains(token));

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证 adapter 不接受缺失的连接配置。
#[test]
fn test_m58_cnb_adapter_rejects_empty_configuration() {
    let _ = CnbChatAdapter::from_env();
    assert!(
        CnbChatAdapter::new(
            String::new(),
            "org/repo".to_string(),
            "token".to_string(),
            "model".to_string(),
        )
        .is_err()
    );
    assert!(
        CnbChatAdapter::new(
            "http://127.0.0.1".to_string(),
            String::new(),
            "token".to_string(),
            "model".to_string(),
        )
        .is_err()
    );
}

/// 验证 AnswerJudge 归一化中英文标点并通过正常证据与诊断回答。
#[test]
fn test_m58_judge_accepts_evidence_and_conservative_diagnostic() {
    let assertions = serde_json::json!({
        "must_include": ["用户入口", "model1"],
        "must_not_include": ["只读"],
        "diagnostic_disclaimer_required": true,
        "evidence_reference_required": true
    });
    let trace = vec![accepted_trace()];
    let result: JudgeResult = judge_answer(
        "结论：页面有用户入口，写入 model1。依据 summary；诊断可能不完整。",
        &assertions,
        &["UNKNOWN_ACTION_TYPE".to_string()],
        &trace,
    );

    assert_eq!(result.passed, true);
    assert_eq!(result.failure_classes, Vec::<String>::new());
    assert_eq!(result.judge_notes, Vec::<String>::new());
}

/// 验证 AnswerJudge 同时报告遗漏事实、禁用断言、忽略诊断和证据缺失。
#[test]
fn test_m58_judge_reports_deterministic_failure_classes() {
    let assertions = serde_json::json!({
        "must_include": ["用户入口", "model1"],
        "must_not_include": ["只读"],
        "diagnostic_disclaimer_required": true,
        "evidence_reference_required": true
    });
    let result = judge_answer(
        "页面是只读的。",
        &assertions,
        &["UNKNOWN_ACTION_TYPE".to_string()],
        &[accepted_trace()],
    );

    // 证据要求由 command trace 判定：这里存在已接受命令，因此不再因为回答里
    // 没有出现 summary/details 等内部 section 名而失败。
    assert_eq!(result.passed, false);
    assert_eq!(
        result.failure_classes,
        vec![
            "missed_fact".to_string(),
            "hallucination".to_string(),
            "ignored_diagnostic".to_string(),
        ]
    );
    assert_eq!(result.judge_notes.len(), 4);
}

/// 验证 must_include 同义组：命中任意一个等价表述即视为覆盖该事实。
#[test]
fn test_m58_judge_accepts_must_include_synonym_group() {
    let assertions = serde_json::json!({
        "must_include": ["只读", ["无写入", "没有写入", "不写入"]],
        "must_not_include": [],
        "diagnostic_disclaimer_required": false,
        "evidence_reference_required": false
    });

    // 简短回答只用了同义组里的第二个表述，仍应通过。
    let result = judge_answer(
        "该页面只读，没有写入。",
        &assertions,
        &[],
        &[accepted_trace()],
    );
    assert_eq!(result.failure_classes, Vec::<String>::new());
    assert_eq!(result.passed, true);

    // 同义组里一个表述都没命中时才算 missed_fact。
    let miss = judge_answer("该页面只读。", &assertions, &[], &[accepted_trace()]);
    assert_eq!(miss.failure_classes, vec!["missed_fact".to_string()]);
}

/// 验证证据要求检查的是「回答是否基于已执行命令」，而不是回答里的关键词。
#[test]
fn test_m58_judge_reports_ungrounded_answer() {
    let assertions = serde_json::json!({
        "must_include": [],
        "must_not_include": [],
        "diagnostic_disclaimer_required": false,
        "evidence_reference_required": true
    });

    // 没有任何命令：既没有证据，也没有命令轨迹。
    let ungrounded = judge_answer("页面写入 model1。", &assertions, &[], &[]);
    assert_eq!(
        ungrounded.failure_classes,
        vec!["ungrounded_answer".to_string(), "no_command".to_string()]
    );

    // 有已接受命令时，即使回答里没有出现 summary 等英文 section 名也算有证据。
    let grounded = judge_answer("页面写入 model1。", &assertions, &[], &[accepted_trace()]);
    assert_eq!(grounded.failure_classes, Vec::<String>::new());
    assert_eq!(grounded.passed, true);
}

/// 验证命令轨迹会单独标记错误命令和过度读取细节。
#[test]
fn test_m58_judge_reports_command_risk_classes() {
    let assertions = serde_json::json!({
        "must_include": [],
        "must_not_include": [],
        "diagnostic_disclaimer_required": false,
        "evidence_reference_required": false
    });
    let over_read_trace = CommandTrace {
        budget: Some("full".to_string()),
        detail_request: true,
        ..accepted_trace()
    };
    let rejected_trace = CommandTrace {
        accepted: false,
        plan_step_index: None,
        ..accepted_trace()
    };
    let result = judge_answer(
        "summary 已返回。",
        &assertions,
        &[],
        &[over_read_trace, rejected_trace],
    );

    // 被 plan 拒绝的命令单独记为 command_rejected，与「已执行但无对应 plan step」
    // 的 wrong_command 区分开，便于判断是真实路由错误还是仅偏离手写 plan。
    assert_eq!(result.passed, false);
    assert_eq!(
        result.failure_classes,
        vec![
            "command_rejected".to_string(),
            "over_read_details".to_string()
        ]
    );
}

/// 验证 RunReport 聚合字段和 JSON/Markdown 双输出不包含 prompt 或模型原文。
#[test]
fn test_m58_run_report_is_structured_and_redacted() {
    let report = RunReport::from_cases(
        "fake".to_string(),
        "test-model".to_string(),
        Some("build-123".to_string()),
        "2026-07-27T00:00:00Z".to_string(),
        vec![
            CaseReport {
                case_id: "pass_case".to_string(),
                trial_index: 0,
                task_family: "page_logic".to_string(),
                difficulty: "basic".to_string(),
                status: "pass".to_string(),
                passed: true,
                max_command_count: 2,
                failure_classes: Vec::new(),
                judge_notes: Vec::new(),
                command_trace: vec![accepted_trace()],
            },
            CaseReport {
                case_id: "fail_case".to_string(),
                trial_index: 0,
                task_family: "page_logic".to_string(),
                difficulty: "basic".to_string(),
                status: "fail".to_string(),
                passed: false,
                max_command_count: 2,
                failure_classes: vec!["missed_fact".to_string()],
                judge_notes: vec!["short note".to_string()],
                command_trace: vec![CommandTrace {
                    accepted: false,
                    plan_step_index: None,
                    budget_upgrade: true,
                    ..accepted_trace()
                }],
            },
        ],
    );
    assert_eq!(report.pass_rate, 0.5);
    assert_eq!(report.failure_classes.get("missed_fact"), Some(&1));
    assert_eq!(report.command_trace_stats.total_commands, 2);
    assert_eq!(report.command_trace_stats.accepted_commands, 1);
    assert_eq!(report.command_trace_stats.rejected_commands, 1);
    assert_eq!(report.command_trace_stats.cases_with_commands, 2);
    assert_eq!(report.command_trace_stats.average_commands_per_case, 1.0);
    assert_eq!(
        report.command_trace_stats.max_command_count_exceeded_cases,
        0
    );
    assert_eq!(report.command_trace_stats.budget_upgrade_count, 1);

    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_dir = std::env::temp_dir().join(format!("m58-run-report-{run_id}"));
    let json_path = output_dir.join("run.json");
    let markdown_path = output_dir.join("run.md");
    write_run_report(&report, &json_path, &markdown_path).unwrap();

    let json = std::fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["schema_version"], "1.4.0");
    assert_eq!(parsed["cases"].as_array().unwrap().len(), 2);
    assert_eq!(parsed["command_trace_stats"]["accepted_commands"], 1);
    assert_eq!(parsed["trial_pass_rate"], 0.5);
    assert_eq!(parsed["trial_count"], 2);
    assert_eq!(parsed["case_count"], 2);
    assert_eq!(parsed["case_stable_pass_rate"], 0.5);
    assert_eq!(parsed["cases_with_flaky_trials"], 0);
    assert_eq!(json.contains("prompt"), false);
    assert_eq!(json.contains("answer"), false);
    assert_eq!(json.contains("CNB_TOKEN"), false);
    let markdown = std::fs::read_to_string(&markdown_path).unwrap();
    assert_eq!(markdown.contains("pass_case"), true);
    assert_eq!(markdown.contains("missed_fact"), true);
    assert_eq!(markdown.contains("case_stable_pass_rate"), true);
    assert_eq!(markdown.contains("trial_index"), true);
    // 命令轨迹与 judge 诊断必须在报告本体里。之前它们只能靠 CI 日志 grep 反推，
    // 而 grep 拿不到对象体、`-A N` 又会截断，导致失败无法复盘。
    assert_eq!(markdown.contains("## Command trace"), true);
    assert_eq!(markdown.contains("## Judge notes"), true);
    assert_eq!(markdown.contains("short note"), true);
    assert_eq!(markdown.contains("prompt"), false);
    assert_eq!(markdown.contains("answer"), false);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证 RunReport 不会用 trial 平均值掩盖同一 case 的不稳定结果。
#[test]
fn test_m58_run_report_tracks_trial_stability() {
    let report = RunReport::from_cases(
        "fake".to_string(),
        "test-model".to_string(),
        None,
        "2026-07-28T00:00:00Z".to_string(),
        vec![
            CaseReport {
                case_id: "same_case".to_string(),
                trial_index: 0,
                task_family: "page_logic".to_string(),
                difficulty: "basic".to_string(),
                status: "pass".to_string(),
                passed: true,
                max_command_count: 1,
                failure_classes: Vec::new(),
                judge_notes: Vec::new(),
                command_trace: vec![accepted_trace()],
            },
            CaseReport {
                case_id: "same_case".to_string(),
                trial_index: 1,
                task_family: "page_logic".to_string(),
                difficulty: "basic".to_string(),
                status: "error".to_string(),
                passed: false,
                max_command_count: 1,
                failure_classes: vec!["protocol_error".to_string()],
                judge_notes: vec!["protocol failure".to_string()],
                command_trace: Vec::new(),
            },
        ],
    );

    assert_eq!(report.pass_rate, 0.5);
    assert_eq!(report.trial_pass_rate, 0.5);
    assert_eq!(report.trial_count, 2);
    assert_eq!(report.case_count, 1);
    assert_eq!(report.case_stable_pass_rate, 0.0);
    assert_eq!(report.cases_with_flaky_trials, 1);
    assert_eq!(report.command_trace_stats.cases_with_commands, 1);
    assert_eq!(report.command_trace_stats.average_commands_per_case, 1.0);
    assert_eq!(report.command_trace_stats.average_commands_per_trial, 0.5);
    assert_eq!(report.cases[1].trial_index, 1);
    assert_eq!(report.cases[1].task_family, "page_logic");
}

/// 验证被拒绝的命令按 `<task_family> -> <命令>` 聚合成路由混淆矩阵。
///
/// 这是工具接口指标：它回答「哪类问题会被误路由到哪个 CLI 动词」，
/// 是决定合并/改名命令的依据，因此必须与 accepted 命令严格区分开。
#[test]
fn test_m58_run_report_aggregates_command_routing_confusion() {
    let rejected = |kind: &str| CommandTrace {
        command_kind: kind.to_string(),
        target: "page:app/actions_test.spg".to_string(),
        args: Vec::new(),
        budget: Some("compact".to_string()),
        plan_step_index: None,
        route: None,
        accepted: false,
        detail_request: false,
        budget_upgrade: false,
        output_sections: Vec::new(),
    };
    let case = |case_id: &str, family: &str, trace: Vec<CommandTrace>| CaseReport {
        case_id: case_id.to_string(),
        trial_index: 0,
        task_family: family.to_string(),
        difficulty: "basic".to_string(),
        status: "error".to_string(),
        passed: false,
        max_command_count: 1,
        failure_classes: vec!["command_rejected".to_string()],
        judge_notes: Vec::new(),
        command_trace: trace,
    };

    let report = RunReport::from_cases(
        "fake".to_string(),
        "test-model".to_string(),
        None,
        "2026-07-28T00:00:00Z".to_string(),
        vec![
            case("a", "page_logic", vec![rejected("--explain")]),
            case("b", "page_logic", vec![rejected("--explain")]),
            case("c", "page_logic", vec![rejected("--explain-condition")]),
            case("d", "field_lineage", vec![rejected("--explain")]),
            // accepted 命令不得进入混淆矩阵。
            CaseReport {
                case_id: "e".to_string(),
                trial_index: 0,
                task_family: "page_logic".to_string(),
                difficulty: "basic".to_string(),
                status: "pass".to_string(),
                passed: true,
                max_command_count: 1,
                failure_classes: Vec::new(),
                judge_notes: Vec::new(),
                command_trace: vec![accepted_trace()],
            },
        ],
    );

    assert_eq!(report.command_routing_confusion.len(), 3);
    assert_eq!(
        report.command_routing_confusion["page_logic -> --explain"],
        2
    );
    assert_eq!(
        report.command_routing_confusion["page_logic -> --explain-condition"],
        1
    );
    assert_eq!(
        report.command_routing_confusion["field_lineage -> --explain"],
        1
    );
    assert_eq!(
        report
            .command_routing_confusion
            .contains_key("page_logic -> --relations"),
        false
    );
    // 反过来，accepted 命令必须出现在 route usage 里，两张表互补且不重叠。
    assert_eq!(
        report.command_route_usage["page_logic -> --relations (primary)"],
        1
    );
    assert_eq!(report.command_route_usage.len(), 1);
}

/// 验证被接受命令按 `<task_family> -> <命令> (primary|alternate)` 聚合。
///
/// plan 接受等价备选之后，「选中规范动词」和「选了另一条等价路」都算 accepted。
/// 若不分开统计，工具表面是否自解释的信号会被通过率抹平，
/// 这张表就是放宽接受面之后仍能定位可疑动词的依据。
#[test]
fn test_m58_run_report_aggregates_command_route_usage() {
    let accepted = |kind: &str, route: &str| CommandTrace {
        command_kind: kind.to_string(),
        target: "comp:app/actions_test.spg|input1".to_string(),
        args: Vec::new(),
        budget: Some("compact".to_string()),
        plan_step_index: Some(0),
        route: Some(route.to_string()),
        accepted: true,
        detail_request: false,
        budget_upgrade: false,
        output_sections: vec!["summary".to_string()],
    };
    let case = |case_id: &str, family: &str, trace: Vec<CommandTrace>| CaseReport {
        case_id: case_id.to_string(),
        trial_index: 0,
        task_family: family.to_string(),
        difficulty: "basic".to_string(),
        status: "pass".to_string(),
        passed: true,
        max_command_count: 2,
        failure_classes: Vec::new(),
        judge_notes: Vec::new(),
        command_trace: trace,
    };

    let report = RunReport::from_cases(
        "fake".to_string(),
        "test-model".to_string(),
        None,
        "2026-08-02T00:00:00Z".to_string(),
        vec![
            case(
                "a",
                "condition",
                vec![accepted("--explain-condition", "primary")],
            ),
            case(
                "b",
                "condition",
                vec![accepted("--explain-condition", "alternate")],
            ),
            case(
                "c",
                "condition",
                vec![accepted("--explain-condition", "alternate")],
            ),
            case("d", "context", vec![accepted("--explain", "alternate")]),
        ],
    );

    assert_eq!(
        report.command_route_usage["condition -> --explain-condition (primary)"],
        1
    );
    assert_eq!(
        report.command_route_usage["condition -> --explain-condition (alternate)"],
        2
    );
    assert_eq!(
        report.command_route_usage["context -> --explain (alternate)"],
        1
    );
    // 被接受的命令不得同时进入混淆矩阵。
    assert!(report.command_routing_confusion.is_empty());
}

/// 验证 fake runner 真实执行 CLI、多轮回传 stdout，并完成全部 fixture LLM case。
#[test]
fn test_m58_fake_runner_executes_fixture_llm_cases() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);
    let output_dir = unique_test_output_dir("m58-fake-runner");
    let config = fixture_runner_config(output_dir.clone());
    let responses = build_fake_case_responses(&selected);
    let expected_request_count = responses.len();
    let mut adapter = FakeModelAdapter::from_responses(responses);

    let report = run_fixture_llm_cases(&selected, &mut adapter, &config).unwrap();
    assert_eq!(report.cases.len(), selected.len());
    assert_eq!(report.case_count, selected.len());
    assert_eq!(
        report.pass_rate,
        1.0,
        "fake fixture failures: {:?}",
        report
            .cases
            .iter()
            .filter(|case| !case.passed)
            .map(|case| (&case.case_id, &case.status, &case.failure_classes))
            .collect::<Vec<_>>()
    );
    assert_eq!(report.cases.iter().all(|case| case.status == "pass"), true);
    assert_eq!(adapter.requests().len(), expected_request_count);
    assert_eq!(adapter.requests()[0].messages[0].role, "user");
    assert_eq!(
        adapter.requests()[0].messages[0]
            .content
            .contains("SKILL.md"),
        true
    );
    assert_eq!(
        adapter.requests()[0].messages[0]
            .content
            .contains(&selected[0].question),
        true
    );
    for request in adapter
        .requests()
        .iter()
        .filter(|request| request.messages.len() > 1)
    {
        assert_eq!(request.messages.len() % 2, 1);
        assert_eq!(request.messages.last().unwrap().role, "user");
        assert_eq!(request.messages[1].role, "assistant");
        assert_eq!(request.messages[2].role, "user");
        assert_eq!(request.messages[2].content.starts_with('{'), true);
        let _: serde_json::Value = serde_json::from_str(&request.messages[2].content).unwrap();
    }

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证同一 fixture case 的多个 trial 使用独立 history 并各自产生报告行。
#[test]
fn test_m58_fake_runner_executes_multiple_independent_trials() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);
    let output_dir = unique_test_output_dir("m58-multi-trial-runner");
    let mut config = fixture_runner_config(output_dir.clone());
    config.trial_count = 2;
    let one_trial_responses = build_fake_case_responses(&selected[..1]);
    let mut responses = one_trial_responses.clone();
    responses.extend(one_trial_responses);
    let mut adapter = FakeModelAdapter::from_responses(responses);

    let report = run_fixture_llm_cases(&selected[..1], &mut adapter, &config).unwrap();
    assert_eq!(report.cases.len(), 2);
    assert_eq!(report.cases[0].trial_index, 0);
    assert_eq!(report.cases[1].trial_index, 1);
    assert_eq!(report.pass_rate, 1.0);
    assert_eq!(report.case_stable_pass_rate, 1.0);
    assert_eq!(report.cases_with_flaky_trials, 0);
    assert_eq!(adapter.requests().len(), 4);
    assert_eq!(
        adapter.requests()[0].messages[0],
        adapter.requests()[2].messages[0]
    );
    assert_eq!(adapter.requests()[0].messages.len(), 1);
    assert_eq!(adapter.requests()[2].messages.len(), 1);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证 bootstrap 明确约束小模型的 JSON 协议、最小查询顺序与保守结论规则。
#[test]
fn test_m58_bootstrap_prompt_contract_for_small_model() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let first_plan_target = case.value["minimal_command_plan"][0]["target"]
        .as_str()
        .unwrap();
    let output_dir = unique_test_output_dir("m58-bootstrap-contract");
    let config = fixture_runner_config(output_dir.clone());
    let mut adapter = FakeModelAdapter::from_responses(vec![
        serde_json::json!({
            "kind": "final",
            "answer": "证据不足，无法给出结论。"
        })
        .to_string(),
    ]);

    let _report = run_case(&case, &mut adapter, &config).unwrap();
    let bootstrap = &adapter.requests()[0].messages[0].content;
    let bootstrap_prefix = bootstrap.split("\n\nSKILL.md:\n").next().unwrap();

    assert_eq!(adapter.requests()[0].messages[0].role, "user");
    // 校验 bootstrap 使用通用路由与预算语义规则，不泄露 case 计划元数据。
    assert!(bootstrap.contains("raw JSON object"));
    assert!(bootstrap.contains("No Markdown fences"));
    assert!(bootstrap.contains("每轮只输出一行 JSON"));
    assert!(bootstrap.contains("final 对象只能有 kind 和 answer 两个键"));
    // 三动词表面：路由规则必须是一张表，不是六条用分号串起来的散句。
    assert!(bootstrap.contains("只有三个查询动词"));
    assert!(bootstrap.contains("`--find`"));
    assert!(bootstrap.contains("`--explain`"));
    assert!(bootstrap.contains("`--relations`"));
    assert!(bootstrap.contains("动词只决定问什么，target 前缀决定去哪"));
    assert!(bootstrap.contains("同一个组件写成 comp: 还是 action: 返回同一组事实块"));

    // M58 里 SKILL.md 的快速分流表说按钮问题先走 --explain-condition，bootstrap 第 3 条
    // 却说按钮问题走 --explain——同一份 prompt 里两条互相矛盾的路由指令，模型只能靠猜，
    // 34 条拒绝（占全部拒绝 30%）就来自这次二选一。这几条断言锁住矛盾不会被写回来。
    assert!(
        !bootstrap.contains("单组件/按钮/动作 -> --explain"),
        "旧的分号串式路由规则必须移除"
    );
    assert!(
        !bootstrap.contains("writer/value-source/condition -> --explain-condition"),
        "不得再要求模型在 --explain 与 --explain-condition 之间二选一"
    );
    assert!(
        !bootstrap.contains("点击/按钮/组件/动作问题不得使用 --query-page-logic"),
        "禁令式路由规则已由前缀分流取代"
    );

    // 占位符必须消失：M58 里模型两次把 `comp:app/<relative-file>.spg|button1` 原样当成
    // 真实路径发了出来。prompt 里出现的路径样子，模型就会照抄。
    assert!(
        !bootstrap.contains("<relative-file>"),
        "bootstrap 不得包含会被原样抄写的路径占位符"
    );
    assert!(bootstrap.contains("绝对不要自己拼造文件路径"));

    // M59 评测里 109 条拒绝有 83% 是 `page:actions_test` 这种「前缀写对、路径没写全」的
    // 写法。补全已经由 Rust 做掉，prompt 必须把这件事说出来——否则模型仍然会为了凑出
    // 完整路径去编，或者多花一轮 --find。
    assert!(
        bootstrap.contains("target 只写你确实知道的部分"),
        "bootstrap 必须说明 target 可以只写已知部分"
    );
    assert!(
        bootstrap.contains("RESOLVED_TARGET") && bootstrap.contains("candidate_targets"),
        "bootstrap 必须说明补全和歧义两种反馈"
    );

    assert!(bootstrap.contains("budget 只能放在 JSON 顶层字段，不能放进 args"));
    assert!(bootstrap.contains("compact"));
    assert!(bootstrap.contains("compact / normal / full"));
    assert!(!bootstrap.contains("compact path"));
    assert!(bootstrap.contains("next user message as the only evidence"));
    assert!(bootstrap.contains("enough evidence"));
    assert!(bootstrap.contains("final answer 必须包含至少一个 literal section name"));
    assert!(bootstrap.contains("页面整体回答至少说明入口/写入计数和一个 action"));
    assert!(bootstrap.contains("组件或动作类问题的回答至少说明组件、action 和写入目标"));
    assert!(bootstrap.contains("字段回答至少说明字段和写入者或来源"));

    // 回答形状的约束保留，但必须和选命令的规则分开：混在一起正是上面那次矛盾的来源。
    assert!(bootstrap.contains("M58 runner answer-shape override"));
    assert!(bootstrap.contains("只约束最终回答的写法，不改变上面的选命令规则"));
    assert!(bootstrap.contains("页面整体最终回答必须写出用户入口与写入目标的实际情况"));
    // 回答形状跟着工具输出走，不替某一个页面的样子立模板。
    assert!(bootstrap.contains("summary.conclusion 存在时，final answer 必须逐字照抄"));
    assert!(bootstrap.contains("summary.absent 非空时"));
    assert!(bootstrap.contains("summary.confidence.level 不是 full 时"));
    assert!(bootstrap.contains("field final 必须 literal 包含 页面 action、写入、字段"));
    assert!(bootstrap.contains("diagnostics"));
    assert!(bootstrap.contains("truncation"));
    assert!(bootstrap.contains("do not guess"));
    assert!(bootstrap.contains("Do not include hidden context"));
    assert!(bootstrap.contains("Do not include answer keys"));
    assert!(bootstrap.contains("Do not include expected_facts"));
    assert!(bootstrap.contains("Do not include must_include"));
    assert!(
        bootstrap
            .contains("This is only a shape example, not the current case answer/target/plan.")
    );
    assert!(bootstrap.contains(
        r#"{"kind":"command","command_kind":"--explain","target":"<target>","args":[],"budget":"compact"}"#
    ));
    assert!(bootstrap.contains("Case Question:"));
    assert!(bootstrap.contains(r#"{"kind":"final","answer":"..."}"#));
    assert!(bootstrap.contains(&case.question));

    assert!(!bootstrap_prefix.contains("tests/fixtures/corpus/ai_eval/ai_eval_cases.json"));
    assert!(!bootstrap_prefix.contains("docs/reference/m58-gemma4-test-prompt.md"));
    assert!(!bootstrap.contains(first_plan_target));
    assert!(!bootstrap.contains("按钮可以提交数据到 model1"));
    assert!(!bootstrap.contains("按钮可以删除 model2 数据"));
    assert!(!bootstrap.contains("没有任何写入操作"));
    assert!(!bootstrap.contains("Authorization: Bearer"));
    assert!(!bootstrap.contains("m58-test-secret-token"));

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证模型提出不在 plan 中的命令时 runner 不启动 CLI 并生成 command_rejected。
/// 这条路径会立即结束 trial，是 live run 中唯一实际产生拒绝分类的位置，
/// 因此必须与 judge_answer 的分类保持一致，否则 command_rejected 永远不可达。
///
/// target 必须是图里真实存在、但不是本 case 要问的那个节点：寻址不到的 target 现在会
/// 交给工具去回答（见 test_m58_runner_lets_the_tool_answer_an_unaddressable_target），
/// 拒绝只留给「去错了地方」。
#[test]
fn test_m58_runner_blocks_command_outside_plan() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-blocked-command");
    let config = fixture_runner_config(output_dir.clone());
    let response = serde_json::json!({
        "kind": "command",
        "command_kind": "--query-page-logic",
        "target": "page:app/page_relations.spg",
        "args": [],
        "budget": null
    });
    let mut adapter = FakeModelAdapter::from_responses(vec![response.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.status, "error");
    assert_eq!(report.passed, false);
    assert_eq!(report.failure_classes, vec!["command_rejected".to_string()]);
    assert_eq!(report.command_trace.len(), 1);
    assert_eq!(report.command_trace[0].accepted, false);
    assert_eq!(report.command_trace[0].plan_step_index, None);
    assert_eq!(report.command_trace[0].route, None);
    assert_eq!(adapter.requests().len(), 1);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证 plan step 的 alternatives 被接受，且 runner 执行的是模型真正选的那条命令。
///
/// 真实用户问句是模糊的，且 `.spg` 不进模型上下文，模型只能盲选动词；因此语义等价的
/// 写法都必须算路由成功。但接受面放宽后有个必须守住的前提：执行的命令要和 trace 记录
/// 的命令一致，否则报告会声称跑了 primary、实际跑的却是别的命令。
#[test]
fn test_m58_runner_accepts_plan_alternative_and_records_route() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "condition_action_behavior")
        .expect("case 必须存在")
        .clone();
    // 规范写法是 action: target，等价备选是同一个按钮的 comp: 写法。
    let alternative = &case.value["minimal_command_plan"][0]["alternatives"][0];
    assert_eq!(alternative["target"], "comp:app/actions_test.spg|button2");

    let output_dir = unique_test_output_dir("m58-plan-alternative");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": alternative["command_kind"],
        "target": alternative["target"],
        "args": alternative["args"],
        "budget": alternative["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary：button2 的 action 受门禁条件约束。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    let trace = &report.command_trace[0];
    assert!(trace.accepted, "等价备选命令必须被接受");
    assert_eq!(trace.plan_step_index, Some(0));
    assert_eq!(trace.route.as_deref(), Some("alternate"));
    // trace 记录的必须是模型选的写法，而不是 plan 的规范写法。
    assert_eq!(trace.target, "comp:app/actions_test.spg|button2");
    assert!(
        !trace.output_sections.is_empty(),
        "备选命令必须真的执行过 CLI 并回传 section"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// `--intent` 不再是拒绝的理由。
///
/// 这一条与它取代的旧断言方向相反，理由在数据里：SKILL.md 要求模型「按问题选 intent」，
/// plan 却因为自己没写 intent 而拒绝写了 intent 的命令。M58 六轮里 17 条拒绝出自这里，
/// 其中 `--intent display` 回答「为什么不显示」恰恰是最该给分的选择。接口不能一边要求、
/// 一边惩罚；intent 是否用得好由回答质量去衡量，不该冒充路由缺陷。
#[test]
fn test_m58_runner_accepts_narrowing_intent_without_counting_it_as_misroute() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "fixture_condition_model_filter_user_var")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];
    assert!(
        (step["args"].as_array()).is_none_or(|args| args.is_empty()),
        "规范写法不带 intent，正是这条测试要覆盖的差异"
    );

    let output_dir = unique_test_output_dir("m58-intent-narrowing");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": ["--intent", "availability"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    let trace = &report.command_trace[0];
    assert!(trace.accepted, "写了 intent 不该被记成路由失败");
    // 但它必须仍然如实出现在 trace 里，否则过度收窄就变得不可见了。
    assert_eq!(
        trace.args,
        vec!["--intent".to_string(), "availability".to_string()]
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// `--find` 不占 plan 步数。
///
/// M58 里 `context_button1_neighbors` 问的是「button1 周围还有什么」——根本没给页面。
/// 模型 8 次选择先 `--find-component button1` 定位，全部被记成路由失败：它做对了，只是
/// plan 没给它做对的余地。定位不是回答，不该消费回答的预算。
#[test]
fn test_m58_runner_lets_discovery_precede_the_plan_step() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "context_button1_neighbors")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];

    let output_dir = unique_test_output_dir("m58-free-discovery");
    let config = fixture_runner_config(output_dir.clone());
    let find = serde_json::json!({
        "kind": "command",
        "command_kind": "--find",
        "target": "button1",
        "args": [],
        "budget": "compact",
    });
    let planned = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary：button1 的上下游依赖见 details。",
    });
    let mut adapter = FakeModelAdapter::from_responses(vec![
        find.to_string(),
        planned.to_string(),
        final_answer.to_string(),
    ]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 2);

    let discovery = &report.command_trace[0];
    assert!(discovery.accepted, "定位命令必须被接受");
    assert_eq!(
        discovery.plan_step_index, None,
        "定位命令不占 plan 步数，否则后面的正题就没预算了"
    );

    let answer_step = &report.command_trace[1];
    assert!(answer_step.accepted);
    assert_eq!(answer_step.plan_step_index, Some(0));
    assert!(
        !report
            .failure_classes
            .contains(&"command_rejected".to_string()),
        "先定位再回答不该被记成路由失败：{:?}",
        report.failure_classes
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 模型/DataFlow 关系查询的裸名与 `model:` 前缀写法必须视为同一条命令。
///
/// CLI 内部会把两种写法规范化到同一个节点，输出逐字节相同。精确字符串比较会把
/// 「动词选对、实体也选对，只是多写了一个类型前缀」记成路由失败，让 command_rejected
/// 和 command_routing_confusion 谎报工具表面的缺陷。
#[test]
fn test_m58_runner_accepts_dataflow_target_with_model_prefix() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "dataflow_output_source")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];
    assert_eq!(step["command_kind"], "--relations");
    assert_eq!(step["target"], "model:dataflow_output");

    let output_dir = unique_test_output_dir("m58-dataflow-prefix");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": "--relations",
        "target": "dataflow_output",
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary：dataflow_output 由 DataFlow 的输出节点产生。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    let trace = &report.command_trace[0];
    assert!(trace.accepted, "省略 model: 前缀的等价 target 必须被接受");
    assert_eq!(trace.plan_step_index, Some(0));
    assert_eq!(trace.route.as_deref(), Some("primary"));

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 没写全但能被工具确定性补全的 target 必须视为同一条命令。
///
/// M59 评测里 109 条拒绝有 91 条长这样：`page:actions_test` 少了目录和扩展名。工具现在
/// 会把它补成 `page:app/actions_test.spg`，但 plan 是逐字符比较 target 的，命令在进 CLI
/// 之前就被判负——评测测的于是是「有没有原样打出我们写下的字符串」，而不是「工具能不能
/// 被走通」。判据仍然完全来自被测工具本身：只有归一到同一个真实节点才算命中。
#[test]
fn test_m58_runner_accepts_target_the_tool_resolves() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "page_purpose_actions_test")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];
    assert_eq!(step["target"], "page:app/actions_test.spg");

    let output_dir = unique_test_output_dir("m58-partial-target");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": "page:actions_test",
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    let trace = &report.command_trace[0];
    assert!(trace.accepted, "工具能补全的 target 必须被接受");
    assert_eq!(trace.plan_step_index, Some(0));
    // trace 必须记模型真正发的那个写法，否则报告与跑过的 CLI 对不上。
    assert_eq!(trace.target, "page:actions_test");

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 已消费的 step 上换个写法再问一次，只要能拿到不一样的输出，就不是走错路。
///
/// SKILL.md 和 bootstrap 都要求「compact 作为默认第一轮，仅在 diagnostics 或
/// OUTPUT_TRUNCATED 时升级」。M59 评测里 `field_lineage_model1_name` 9 次 trial 全部这样
/// 死：第一条命令完全正确、被接受，模型按指示升到 normal，plan 里那一步已被消费，整条
/// trial 记成 command_rejected。接口不能一边要求升级 budget，一边把升级判成路由失败。
#[test]
fn test_m58_runner_accepts_budget_upgrade_retry() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "field_lineage_model1_name")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];

    let output_dir = unique_test_output_dir("m58-budget-retry");
    let config = fixture_runner_config(output_dir.clone());
    let command = |budget: &str| {
        serde_json::json!({
            "kind": "command",
            "command_kind": step["command_kind"],
            "target": step["target"],
            "args": step["args"],
            "budget": budget,
        })
        .to_string()
    };
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary 和 details：字段 name 的写入来自页面 action。",
    });
    let mut adapter = FakeModelAdapter::from_responses(vec![
        command("compact"),
        command("normal"),
        final_answer.to_string(),
    ]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 2);
    assert!(report.command_trace[0].accepted);
    assert!(
        report.command_trace[1].accepted,
        "按 SKILL.md 指示升级 budget 不该被记成路由失败"
    );
    assert!(
        !report
            .failure_classes
            .contains(&"command_rejected".to_string()),
        "{:?}",
        report.failure_classes
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 同一个 step 的另一种可接受写法必须能接着用。
///
/// `dataflow_output_source` 的 plan 把 `--relations model:dataflow_output` 和
/// `--explain model:dataflow_output` 都写成了这一步的可接受写法。M59 评测里模型先后发了
/// 这两条，7 次 trial 因此判负——两条都是 plan 自己认可的路由，先后发出去不是走错路。
#[test]
fn test_m58_runner_accepts_the_other_variant_of_a_used_step() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "dataflow_output_source")
        .expect("case 必须存在")
        .clone();

    let output_dir = unique_test_output_dir("m58-other-variant");
    let config = fixture_runner_config(output_dir.clone());
    let command = |kind: &str| {
        serde_json::json!({
            "kind": "command",
            "command_kind": kind,
            "target": "model:dataflow_output",
            "args": [],
            "budget": "compact",
        })
        .to_string()
    };
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary：dataflow_output 由 DataFlow 的输出节点产生。",
    });
    let mut adapter = FakeModelAdapter::from_responses(vec![
        command("--relations"),
        command("--explain"),
        final_answer.to_string(),
    ]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 2);
    assert!(report.command_trace[0].accepted);
    assert!(
        report.command_trace[1].accepted,
        "plan 自己认可的另一种写法不该被记成路由失败"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 工具寻址不到的 target 交给工具去说，不在 policy 层直接判负。
///
/// 归一不出真实节点时 CLI 返回 AMBIGUOUS_TARGET / TARGET_NOT_FOUND 加真实候选，那正是
/// 模型下一轮能用的东西。在 policy 层拒绝会让整条 trial 立刻结束，模型永远看不到候选，
/// 于是「工具能不能帮人把目标找对」这件事根本测不到。额度仍受 MAX_DISCOVERY_COMMANDS 限制。
#[test]
fn test_m58_runner_lets_the_tool_answer_an_unaddressable_target() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "page_purpose_actions_test")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];

    let output_dir = unique_test_output_dir("m58-unaddressable");
    let config = fixture_runner_config(output_dir.clone());
    // 图里没有这个页面：工具会返回 TARGET_NOT_FOUND，模型据此改用真实 target。
    let probe = serde_json::json!({
        "kind": "command",
        "command_kind": "--relations",
        "target": "page:app/不存在的页面.spg",
        "args": [],
        "budget": "compact",
    });
    let real = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary。用户入口：button1；按钮：button1；写入目标：model1；action：action1。",
    });
    let mut adapter = FakeModelAdapter::from_responses(vec![
        probe.to_string(),
        real.to_string(),
        final_answer.to_string(),
    ]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 2);
    assert!(
        report.command_trace[0].accepted,
        "寻址不到的 target 应当交给工具回答，而不是直接判负"
    );
    assert_eq!(
        report.command_trace[0].plan_step_index, None,
        "定位命令不占 plan 步数"
    );
    assert!(report.command_trace[1].accepted);
    assert_eq!(report.command_trace[1].plan_step_index, Some(0));

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 原样重发同一条命令仍然拒绝：拿到的是同一份输出，模型在原地打转。
#[test]
fn test_m58_runner_rejects_identical_command_replay() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "field_lineage_model1_name")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];

    let output_dir = unique_test_output_dir("m58-identical-replay");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": "compact",
    })
    .to_string();
    let mut adapter = FakeModelAdapter::from_responses(vec![command.clone(), command]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 2);
    assert!(report.command_trace[0].accepted);
    assert!(
        !report.command_trace[1].accepted,
        "同 budget 原样重发拿到的是同一份输出，仍然是失败"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 类型前缀的放宽只针对 `--query-dataflow`；其它命令的前缀是消歧义所必需的。
#[test]
fn test_m58_runner_still_rejects_stripped_prefix_for_other_commands() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "fixture_condition_input1_visible_disabled")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];
    let stripped = step["target"]
        .as_str()
        .unwrap()
        .split_once(':')
        .expect("target 必须带类型前缀")
        .1
        .to_string();

    let output_dir = unique_test_output_dir("m58-prefix-strict");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": stripped,
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    // 剥掉前缀的 target 归一不到真实节点，于是走定位通道由工具去说，但它绝不能命中
    // plan step——那才是「接受了一条走错路的命令」。
    assert_eq!(
        report.command_trace[0].plan_step_index, None,
        "非 dataflow 命令剥掉类型前缀后不应命中 plan step"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证 primary 命令仍记为 primary，使放宽接受面不会抹平路由质量信号。
#[test]
fn test_m58_runner_records_primary_route_for_canonical_command() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = cases
        .iter()
        .find(|case| case.case_id == "fixture_condition_input1_visible_disabled")
        .expect("case 必须存在")
        .clone();
    let step = &case.value["minimal_command_plan"][0];

    let output_dir = unique_test_output_dir("m58-plan-primary");
    let config = fixture_runner_config(output_dir.clone());
    let command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": step["budget"],
    });
    let final_answer = serde_json::json!({
        "kind": "final",
        "answer": "见 summary：input1 受 visibleCondition 与 disableCondition 控制，\
                   分别依赖 input2 和 input1。",
    });
    let mut adapter =
        FakeModelAdapter::from_responses(vec![command.to_string(), final_answer.to_string()]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.command_trace.len(), 1);
    assert_eq!(report.command_trace[0].route.as_deref(), Some("primary"));

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证首轮 malformed JSON 即记 protocol_error，runner 不会修复后继续执行后续合法命令。
#[test]
fn test_m58_fake_runner_records_protocol_error_for_malformed_response_before_valid_command() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-malformed-protocol");
    let config = fixture_runner_config(output_dir.clone());
    let step = &case.value["minimal_command_plan"][0];
    let malformed = format!(
        "```json\n{}\n```",
        serde_json::json!({
            "kind": "command",
            "command_kind": step["command_kind"],
            "target": step["target"],
            "args": step["args"],
            "budget": step["budget"]
        })
    );
    let valid_command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": step["budget"]
    })
    .to_string();
    let mut adapter = FakeModelAdapter::from_responses(vec![malformed, valid_command]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.status, "error");
    assert_eq!(report.passed, false);
    assert_eq!(report.failure_classes, vec!["protocol_error".to_string()]);
    assert_eq!(report.judge_notes.len(), 1);
    assert!(
        report.judge_notes[0].starts_with("model response violated command/final JSON protocol: ")
    );
    assert!(!report.judge_notes[0].contains("```"));
    assert_eq!(report.command_trace, Vec::<CommandTrace>::new());
    assert_eq!(adapter.requests().len(), 1);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证合法命令后，带 diagnostic disclaimer 的 final 仍可通过现有 judge。
#[test]
fn test_m58_fake_runner_accepts_valid_command_then_final_with_diagnostic_disclaimer() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-valid-command-final");
    let config = fixture_runner_config(output_dir.clone());
    let step = &case.value["minimal_command_plan"][0];
    let mut adapter = FakeModelAdapter::from_responses(vec![
        serde_json::json!({
            "kind": "command",
            "command_kind": step["command_kind"],
            "target": step["target"],
            "args": step["args"],
            "budget": step["budget"]
        })
        .to_string(),
        serde_json::json!({
            "kind": "final",
            "answer": "结论：页面有用户入口、按钮 action 和写入目标。依据 summary 和 details，按钮会写入 model1，也会删除 model2；诊断可能不完整。"
        })
        .to_string(),
    ]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.status, "pass");
    assert_eq!(report.passed, true);
    assert_eq!(report.failure_classes, Vec::<String>::new());
    assert_eq!(report.command_trace.len(), 1);
    assert_eq!(report.command_trace[0].accepted, true);
    assert_eq!(adapter.requests().len(), 2);

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 验证合法 planned command 已执行并记录后，第二轮 fenced 响应会记为 protocol_error 且不重试。
#[test]
fn test_m58_fake_runner_records_protocol_error_after_valid_planned_command() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-post-command-protocol-error");
    let config = fixture_runner_config(output_dir.clone());
    let step = &case.value["minimal_command_plan"][0];
    let valid_command = serde_json::json!({
        "kind": "command",
        "command_kind": step["command_kind"],
        "target": step["target"],
        "args": step["args"],
        "budget": step["budget"]
    })
    .to_string();
    let malformed_followup = format!(
        "```json\n{}\n```",
        serde_json::json!({
            "kind": "final",
            "answer": "这条 fenced final 不应被接受"
        })
    );
    let mut adapter = FakeModelAdapter::from_responses(vec![valid_command, malformed_followup]);

    let report = run_case(&case, &mut adapter, &config).unwrap();
    assert_eq!(report.status, "error");
    assert_eq!(report.passed, false);
    assert_eq!(report.failure_classes, vec!["protocol_error".to_string()]);
    assert_eq!(report.command_trace.len(), 1);
    assert_eq!(report.command_trace[0].accepted, true);
    assert_eq!(report.command_trace[0].plan_step_index, Some(0));
    assert_eq!(
        report.command_trace[0].target,
        step["target"].as_str().unwrap().to_string()
    );
    assert_eq!(adapter.requests().len(), 2);
    assert_eq!(adapter.requests()[1].messages.len(), 3);
    assert_eq!(adapter.requests()[1].messages[1].role, "assistant");
    assert_eq!(adapter.requests()[1].messages[2].role, "user");
    assert_eq!(
        adapter.requests()[1].messages[2].content.starts_with('{'),
        true
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// CNB pipeline 中手动/定时执行的真实 fixture baseline；普通 CI 不运行。
#[test]
#[ignore = "requires CNB_TOKEN, M58_CNB_REPO, M58_CNB_MODEL and release binary"]
fn test_m58_cnb_fixture_llm_baseline() {
    let binary_path = PathBuf::from(
        std::env::var_os("M58_METADATA_CHECKER_BIN")
            .expect("M58_METADATA_CHECKER_BIN must point to release binary"),
    );
    assert_eq!(binary_path.is_file(), true);
    let model_id = std::env::var("M58_CNB_MODEL").expect("M58_CNB_MODEL is required");
    let output_dir = std::env::var_os("M58_AI_EVAL_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/m58-ai-eval"));
    let config = RunnerConfig {
        binary_path,
        skill_path: PathBuf::from("SKILL.md"),
        project_root: PathBuf::from("tests/fixtures/test_project"),
        output_dir: output_dir.clone(),
        provider: "cnb-ai-chat".to_string(),
        model_id,
        cnb_build_id: std::env::var("CNB_BUILD_ID").ok(),
        trial_count: std::env::var("M58_AI_EVAL_TRIALS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3),
    };
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let expected_case_count = 13;
    validate_fixture_llm_case_count(&cases, expected_case_count).unwrap();
    let mut adapter = CnbChatAdapter::from_env().unwrap();
    let reasoning_effort = std::env::var("M58_CNB_REASONING_EFFORT")
        .ok()
        .map(|effort| effort.trim().to_string())
        .filter(|effort| !effort.is_empty());
    let mut report = run_fixture_llm_cases(&cases, &mut adapter, &config).unwrap();
    report.set_reasoning(reasoning_effort.clone(), adapter.reasoning_chars());
    // reasoning_effort 不在 CNB swagger 声明的 body schema 里，是实测出来的透传字段。
    // 上游一旦默默忽略它，这份报告会标着「开了推理」而实际一次都没推理——和模型名被
    // 静默替换是同一类静默失效，必须在写报告前就失败。
    if reasoning_effort.is_some() {
        assert!(
            adapter.reasoning_chars() > 0,
            "请求了 reasoning_effort={:?} 但整个 run 没有收到任何 reasoning_content；\
             该字段可能已不再被上游接受，这份结果不能当作带推理的基线",
            reasoning_effort
        );
    }
    write_run_report(
        &report,
        &output_dir.join("run.json"),
        &output_dir.join("run.md"),
    )
    .unwrap();
    assert_eq!(report.case_count, expected_case_count);
    println!(
        "{}",
        serde_json::json!({
            "provider": report.provider,
            "model_id": report.model_id,
            "case_count": report.cases.len(),
            "pass_rate": report.pass_rate,
            "failure_classes": report.failure_classes,
        })
    );
}

/// 创建使用当前 integration-test binary 的 runner 配置。
fn fixture_runner_config(output_dir: PathBuf) -> RunnerConfig {
    let binary_path = std::env::var_os("CARGO_BIN_EXE_metadata-checker")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/debug/metadata-checker"));
    RunnerConfig {
        binary_path,
        skill_path: PathBuf::from("SKILL.md"),
        project_root: PathBuf::from("tests/fixtures/test_project"),
        output_dir,
        provider: "fake".to_string(),
        model_id: "fake-model".to_string(),
        cnb_build_id: None,
        trial_count: 1,
    }
}

/// 为每个 fixture case 构造 command -> final 的确定性 fake 响应。
fn build_fake_case_responses(cases: &[m58_ai_eval::EvalCase]) -> Vec<String> {
    let mut responses = Vec::new();
    for case in cases {
        for step in case.value["minimal_command_plan"].as_array().unwrap() {
            responses.push(
                serde_json::json!({
                    "kind": "command",
                    "command_kind": step["command_kind"],
                    "target": step["target"],
                    "args": step["args"],
                    "budget": step["budget"]
                })
                .to_string(),
            );
        }
        // must_include 每一项可以是字符串或同义组；fake 回答取第一个表述即可满足。
        let required = case.value["answer_assertions"]["must_include"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| match value {
                serde_json::Value::Array(alternatives) => alternatives
                    .first()
                    .and_then(serde_json::Value::as_str)
                    .expect("同义组至少需要一个字符串表述"),
                other => other.as_str().expect("must_include 项必须是字符串或同义组"),
            })
            .collect::<Vec<_>>()
            .join("、");
        responses.push(
            serde_json::json!({
                "kind": "final",
                "answer": format!("依据 summary：{required}。可能存在诊断不完整。")
            })
            .to_string(),
        );
    }
    responses
}

/// 为每个测试 run 生成不冲突的临时输出目录。
fn unique_test_output_dir(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
}

/// 构造一个已通过 plan、只读取 summary 的命令轨迹。
fn accepted_trace() -> CommandTrace {
    CommandTrace {
        command_kind: "--relations".to_string(),
        target: "page:app/actions_test.spg".to_string(),
        args: Vec::new(),
        budget: Some("compact".to_string()),
        plan_step_index: Some(0),
        route: Some("primary".to_string()),
        accepted: true,
        detail_request: false,
        budget_upgrade: false,
        output_sections: vec!["summary".to_string()],
    }
}

/// 构造最小合法 Chat 请求，供 adapter 异常路径使用。
fn empty_chat_request() -> ChatRequest {
    ChatRequest {
        messages: vec![ChatMessage {
            role: "user".to_string(),
            content: "question".to_string(),
        }],
        model: "model".to_string(),
        stream: false,
        reasoning_effort: None,
    }
}

/// 启动一次性 fake CNB HTTP 服务并返回收到的原始请求。
fn spawn_fake_cnb_server(
    status: &str,
    content_type: &str,
    response_body: &str,
) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_string();
    let content_type = content_type.to_string();
    let response_body = response_body.to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        request
    });
    (endpoint, server)
}

/// 按 Content-Length 读取一次性 HTTP 请求，避免等待客户端关闭连接。
fn read_http_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "fake CNB server received incomplete HTTP request");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.eq_ignore_ascii_case("content-length"))
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk).unwrap();
        assert!(read > 0, "fake CNB server received incomplete HTTP body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    String::from_utf8(bytes[..header_end + content_length].to_vec()).unwrap()
}

/// adapter 层失败必须把原始错误带进 judge_notes。
///
/// 之前这里是 `Err(_) => "model adapter completion failed"`，错误被整个丢掉。
/// 在 reasoning_effort 的 A/B 里，7 个 trial 因此变成无法归因的 runner_error：
/// 既看不出是超时、HTTP 错误，还是模型被静默替换，只能重跑。诊断本身是我们自己
/// 构造的确定性文本（且已 redact token），进报告是安全的。
#[test]
fn test_m58_runner_surfaces_adapter_error_in_judge_notes() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let case = fixture_llm_cases(&cases).into_iter().next().unwrap();
    let output_dir = unique_test_output_dir("m58-adapter-error-note");
    let config = fixture_runner_config(output_dir.clone());
    // 空队列的 FakeModelAdapter 会以确定性文案失败，等价于 live run 里的 adapter 层错误。
    let mut adapter = FakeModelAdapter::from_responses(vec![]);

    let report = run_case(&case, &mut adapter, &config).unwrap();

    assert_eq!(report.status, "error");
    assert_eq!(report.failure_classes, vec!["runner_error".to_string()]);
    let note = report.judge_notes.join(" ");
    assert!(
        note.contains("fake model response queue exhausted"),
        "runner_error 的 judge_notes 必须包含 adapter 原始错误，实际为: {note}"
    );

    std::fs::remove_dir_all(output_dir).unwrap();
}

/// 提示词不能强制模型说出某个 case 明令禁止的词。
///
/// `readonly_page_check` 禁止「按钮」，而 runner 的回答形状 override 曾经硬性要求每个
/// 页面回答都 literal 包含「按钮」并使用固定标签——那条 case 因此 9/9 全灭，无论工具
/// 输出多准确都不可能通过。这和文件里已经记过两次的 budget / intent 是同一类问题：
/// 接口不能一边要求、一边惩罚。回答形状只能跟着工具输出走，不能替某一个页面的样子
/// 立规矩。
#[test]
fn test_m58_bootstrap_never_mandates_a_forbidden_word() {
    let cases = m58_ai_eval::load_eval_cases(std::path::Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .expect("load cases");
    for case in &cases {
        // 只看 runner 自己的提示词，不含 SKILL.md：SKILL.md 是工具文档，本来就会出现
        // 「按钮」这类词，它没有强制模型把这个词写进答案。
        let prompt = m58_ai_eval::build_bootstrap_message("", case);
        let forbidden = case
            .value
            .get("answer_assertions")
            .and_then(|a| a.get("must_not_include"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for word in forbidden {
            let word = word.as_str().unwrap_or_default();
            assert!(
                !prompt.contains(word),
                "case {} 禁止「{word}」，提示词里却出现了这个词——模型会照抄",
                case.case_id
            );
        }
    }
}

/// 确定性的「没有」不该被当成必须降级表达的诊断。
#[test]
fn test_m58_determinate_absence_does_not_demand_hedging() {
    // 「这页没有写入目标」是判定，不是证据不足；要求它说「可能」会和同一条 case 要求
    // 的「只读」直接打架。
    assert!(!m58_ai_eval::diagnostic_demands_hedging(
        "NO_WRITE_TARGETS: Page has no detected write targets"
    ));
    assert!(!m58_ai_eval::diagnostic_demands_hedging("NO_ENTRYPOINTS"));
    // 语义没解析出来、输出被截断，仍然必须降级。
    assert!(m58_ai_eval::diagnostic_demands_hedging(
        "UNKNOWN_ACTION_TYPE: Unknown action type 'x' encountered"
    ));
    assert!(m58_ai_eval::diagnostic_demands_hedging("OUTPUT_TRUNCATED"));
    // 没登记过的 code 一律按保守处理。
    assert!(m58_ai_eval::diagnostic_demands_hedging("SOME_FUTURE_CODE"));
}
