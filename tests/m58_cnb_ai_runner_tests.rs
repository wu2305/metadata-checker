#![cfg(feature = "cli-local")]

#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use m58_ai_eval::{
    AgentTurn, ChatMessage, ChatRequest, CnbChatAdapter, CommandPolicy, CommandRequest,
    FakeModelAdapter, ModelAdapter, fixture_llm_cases, load_eval_cases, parse_agent_turn,
    redact_secret,
};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

/// 验证 M58 首批 fixture LLM case 数量与 active 状态。
#[test]
fn test_m58_fixture_llm_has_three_active_cases() {
    let cases = load_eval_cases(Path::new(
        "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
    ))
    .unwrap();
    let selected = fixture_llm_cases(&cases);

    assert_eq!(selected.len(), 3);
    assert!(selected.iter().all(|case| case.case_status == "active"));
    assert_eq!(selected[0].case_id, "page_purpose_actions_test");
    assert!(!selected[0].question.is_empty());
    assert_eq!(selected[0].difficulty, "basic");
    assert!(selected[0].value.is_object());
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
        budget: None,
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
    let response_body = r#"{"choices":[{"message":{"role":"assistant","content":"{\"kind\":\"final\",\"answer\":\"ok\"}"}}]}"#;
    let (endpoint, server) = spawn_fake_cnb_server("200 OK", response_body);
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
    assert_eq!(request_json["stream"], false);
    assert_eq!(request_json["messages"][0]["role"], "user");
    assert_eq!(request_json["messages"][0]["content"], "question");
}

/// 验证 CNB HTTP 错误和缺字段响应不会泄漏 token 或伪造成功。
#[test]
fn test_m58_cnb_adapter_redacts_http_error_and_rejects_invalid_response() {
    let token = "m58-error-secret";
    let (endpoint, error_server) = spawn_fake_cnb_server(
        "500 Internal Server Error",
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

    let (endpoint, missing_choice_server) = spawn_fake_cnb_server("200 OK", r#"{"choices":[]}"#);
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
fn spawn_fake_cnb_server(status: &str, response_body: &str) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_string();
    let response_body = response_body.to_string();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
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
