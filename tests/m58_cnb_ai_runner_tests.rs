#![cfg(feature = "cli-local")]

#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use m58_ai_eval::{
    AgentTurn, CaseReport, ChatMessage, ChatRequest, CnbChatAdapter, CommandPolicy, CommandRequest,
    CommandTrace, FakeModelAdapter, JudgeResult, ModelAdapter, RunReport, RunnerConfig,
    fixture_llm_cases, judge_answer, load_eval_cases, parse_agent_turn, redact_secret, run_case,
    run_fixture_llm_cases, validate_fixture_llm_case_count, write_run_report,
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
        r#"{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg","args":[],"budget":null}"#,
    )
    .unwrap();
    assert_eq!(
        command,
        AgentTurn::Command(CommandRequest {
            command_kind: "--query-page-logic".to_string(),
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
        command_kind: "--query-page-logic".to_string(),
        target: "page:app/actions_test.spg".to_string(),
        args: Vec::new(),
        budget: Some("compact".to_string()),
    };

    let validated = policy.validate(&request, &[]).unwrap();
    assert_eq!(validated.step_index(), 0);
    assert_eq!(validated.binary_path(), binary_path.as_path());
    assert_eq!(
        validated.argv(),
        vec![
            OsString::from("--non-human"),
            OsString::from("--project-dir"),
            project_dir.into_os_string(),
            OsString::from("--graph-db-path"),
            graph_db_path.into_os_string(),
            OsString::from("--query-page-logic"),
            OsString::from("page:app/actions_test.spg"),
            OsString::from("--budget"),
            OsString::from("compact"),
        ]
    );
    assert!(policy.validate(&request, &[0]).is_err());
    assert!(policy.validate(&request, &[0, 1]).is_err());
    assert!(
        policy
            .validate(
                &CommandRequest {
                    target: "page:app/other.spg".to_string(),
                    ..request
                },
                &[],
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
                command_kind: "--query-page-logic".to_string(),
                target: "page:app/dataflow_embedded.spg".to_string(),
                args: Vec::new(),
                budget: Some("compact".to_string()),
            },
            &[],
        )
        .unwrap();
    assert_eq!(validated.argv().last(), Some(&OsString::from("compact")));

    let default_validated = policy
        .validate(
            &CommandRequest {
                command_kind: "--query-page-logic".to_string(),
                target: "page:app/dataflow_embedded.spg".to_string(),
                args: Vec::new(),
                budget: None,
            },
            &[],
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
    assert_eq!(
        report.judge_notes,
        vec!["model adapter completion failed".to_string()]
    );
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
    assert_eq!(parsed["schema_version"], "1.3.0");
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
            .contains_key("page_logic -> --query-page-logic"),
        false
    );
    // 反过来，accepted 命令必须出现在 route usage 里，两张表互补且不重叠。
    assert_eq!(
        report.command_route_usage["page_logic -> --query-page-logic (primary)"],
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
    assert!(bootstrap.contains("smallest allowed query"));
    assert!(bootstrap.contains("先按问题意图选命令"));
    assert!(bootstrap.contains("页面整体逻辑 -> --query-page-logic"));
    assert!(bootstrap.contains("单组件/按钮/动作 -> --explain"));
    assert!(bootstrap.contains("writer/value-source/condition -> --explain-condition"));
    assert!(bootstrap.contains("点击/按钮/组件/动作 -> --explain"));
    assert!(bootstrap.contains("页面整体问题才允许 --query-page-logic"));
    assert!(bootstrap.contains("裸 field 的值/来源/写入 -> --explain"));
    assert!(bootstrap.contains("page:app/<relative-file>.spg；不得删除 app/ 或 .spg"));
    assert!(bootstrap.contains("budget 只能放在 JSON 顶层字段，不能放进 args"));
    assert!(bootstrap.contains("主证据块为空或 result=null 时，不要直接作答"));
    assert!(bootstrap.contains("用同一 target 执行 --explain 作为受限 fallback"));
    assert!(bootstrap.contains("compact"));
    assert!(bootstrap.contains("compact / normal / full"));
    assert!(bootstrap.contains("页面目标保留 `page:app/<relative-file>.spg`"));
    assert!(!bootstrap.contains("compact path"));
    assert!(bootstrap.contains("next user message as the only evidence"));
    assert!(bootstrap.contains("enough evidence"));
    assert!(bootstrap.contains("final answer 必须包含至少一个 literal section name"));
    assert!(bootstrap.contains("页面整体回答至少说明入口/写入计数和一个 action"));
    assert!(bootstrap.contains("按钮/动作回答至少说明组件、action 和写入目标"));
    assert!(bootstrap.contains("字段回答至少说明字段和写入者或来源"));
    assert!(
        bootstrap.contains(
            "裸 field 的一次 compact --explain 已有 summary 和 evidence 后立即返回 final"
        )
    );
    assert!(bootstrap.contains("M58 runner routing override"));
    assert!(bootstrap.contains("裸 field 必须使用 --explain"));
    assert!(bootstrap.contains(
        "按钮/点击问题必须使用 --explain 和 comp:app/<relative-file>.spg|<component-id>"
    ));
    assert!(bootstrap.contains("页面整体最终回答必须明确写出用户入口、写入目标和 action"));
    assert!(bootstrap.contains("page final 必须 literal 包含 用户入口、按钮、写入目标、action"));
    assert!(
        bootstrap.contains(
            "页面 final 使用固定标签：用户入口：...；按钮：...；写入目标：...；action：..."
        )
    );
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
        r#"{"kind":"command","command_kind":"--query-page-logic","target":"page:<relative-page-path>.spg","args":[],"budget":"compact"}"#
    ));
    assert!(bootstrap.contains("page:app/<relative-file>.spg"));
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
        "target": "page:app/not-in-plan.spg",
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
        .find(|case| case.case_id == "fixture_condition_input1_visible_disabled")
        .expect("case 必须存在")
        .clone();
    let alternative = &case.value["minimal_command_plan"][0]["alternatives"][0];
    assert_eq!(alternative["args"][0], "--intent");

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
        "answer": "见 summary：input1 的 visibleCondition 依赖 input2，\
                   disableCondition 依赖 input1 自身取值。",
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
    assert_eq!(trace.args, vec!["--intent".to_string(), "auto".to_string()]);
    assert!(
        !trace.output_sections.is_empty(),
        "备选命令必须真的执行过 CLI 并回传 section"
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
    let report = run_fixture_llm_cases(&cases, &mut adapter, &config).unwrap();
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
        command_kind: "--query-page-logic".to_string(),
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
