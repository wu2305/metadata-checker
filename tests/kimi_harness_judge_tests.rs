#![cfg(feature = "cli-local")]

#[path = "support/kimi_smoke_judge.rs"]
mod kimi_smoke_judge;
#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use anyhow::{Context, Result, bail};
use kimi_smoke_judge::{
    AssertionVerdict, CaseJudgement, ForbiddenCheck, MAX_ANSWER_CHARS, SMOKE_QUESTION_CASES,
    SmokeCase, StandardAnswer, Verdict, build_judge_request, extract_final_answer, judge_case,
    load_smoke_cases, parse_judge_response, render_judge_markdown,
};
use m58_ai_eval::{CnbChatAdapter, FakeModelAdapter};
use std::path::{Path, PathBuf};

/// 构造一个带三条 must、一条 should、两条禁令的样例 case。
fn sample_case() -> SmokeCase {
    SmokeCase {
        case_id: "test_case".to_string(),
        question: "text41 什么时候显示？".to_string(),
        standard_answer: StandardAnswer {
            summary: "text41 继承 panel35 的显示条件。".to_string(),
            must_mention: vec![
                "text41 自身没有直接显示条件".to_string(),
                "继承 panel35.visibleCondition".to_string(),
                "model11.totalRowCount__ > 0".to_string(),
            ],
            should_mention: vec!["supporting_context 不是必要显示门控".to_string()],
            must_not_mention: vec![
                "text41 自己配置了 visibleCondition".to_string(),
                "DataFlow 字段来源就是显示条件".to_string(),
            ],
        },
    }
}

/// 三条 must 全 supported、无禁令违反的标准 judge JSON 响应。
fn all_supported_response() -> String {
    serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "答案明说自身无直接条件"},
            {"id": 2, "verdict": "supported", "note": "答案提到继承 panel35"},
            {"id": 3, "verdict": "supported", "note": "答案给出 totalRowCount 表达式"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
        "overall_note": "答案完整",
    })
    .to_string()
}

/// 验证 case 文件加载：真实冒烟 fixture 读出 8 个 case 及其 standard_answer 断言。
#[test]
fn test_load_smoke_cases_reads_real_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json");
    let cases = load_smoke_cases(&path).unwrap();
    assert_eq!(cases.len(), 8);
    let first = &cases[0];
    assert_eq!(first.case_id, "xiaoshouyi_text41_display_conditions");
    assert!(!first.question.is_empty());
    assert_eq!(first.standard_answer.must_mention.len(), 5);
    assert_eq!(first.standard_answer.should_mention.len(), 2);
    assert_eq!(first.standard_answer.must_not_mention.len(), 4);
    assert!(!first.standard_answer.summary.is_empty());
}

/// 验证 case JSON 缺字段时按 serde(default) 容忍，不报错。
#[test]
fn test_load_smoke_cases_tolerates_missing_fields() {
    let temp_path = std::env::temp_dir().join(format!(
        "kimi-smoke-judge-cases-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&temp_path, r#"{"cases":[{"case_id":"c1"}]}"#).unwrap();
    let cases = load_smoke_cases(&temp_path).unwrap();
    std::fs::remove_file(&temp_path).unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case_id, "c1");
    assert_eq!(cases[0].question, "");
    assert_eq!(cases[0].standard_answer.must_mention.len(), 0);
}

/// 验证六问映射与冒烟 fixture 中的 case_id 完全一致。
#[test]
fn test_smoke_question_cases_mapping_matches_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json");
    let cases = load_smoke_cases(&path).unwrap();
    assert_eq!(SMOKE_QUESTION_CASES.len(), 6);
    for (index, (question_no, case_id)) in SMOKE_QUESTION_CASES.iter().enumerate() {
        assert_eq!(*question_no, format!("q{}", index + 1));
        assert!(
            cases.iter().any(|case| case.case_id == *case_id),
            "fixture 中找不到 {question_no} 映射的 {case_id}"
        );
    }
}

/// 验证多行 transcript 提取：最后一个非空 assistant 拼接结果为最终答案。
#[test]
fn test_extract_final_answer_takes_last_assistant_text() {
    let transcript = concat!(
        "{\"role\":\"assistant\",\"message\":{\"content\":\"first answer\"}}\n",
        "{\"role\":\"user\",\"message\":{\"content\":\"ignore me\"}}\n",
        "{\"role\":\"assistant\",\"message\":{\"content\":\"final answer\"}}\n",
    );
    assert_eq!(extract_final_answer(transcript), "final answer");
}

/// 验证同行多个 content 字符串按序拼接，且末尾空白 assistant 不覆盖已有答案。
#[test]
fn test_extract_final_answer_concatenates_and_skips_blank() {
    let transcript = concat!(
        "{\"role\":\"assistant\",\"content\":\"Hello\",\"delta\":{\"content\":\" world\"}}\n",
        "{\"role\":\"assistant\",\"content\":\"   \"}\n",
    );
    assert_eq!(extract_final_answer(transcript), "Hello world");
}

/// 验证没有 assistant 行时返回空串。
#[test]
fn test_extract_final_answer_returns_empty_without_assistant() {
    let transcript = concat!(
        "{\"role\":\"user\",\"content\":\"question\"}\n",
        "{\"role\":\"system\",\"content\":\"prompt\"}\n",
    );
    assert_eq!(extract_final_answer(transcript), "");
}

/// 验证 content 嵌套在深层对象/数组里也能提取，非法 JSON 行被跳过。
#[test]
fn test_extract_final_answer_handles_nested_content_and_bad_lines() {
    let transcript = concat!(
        "{\"role\":\"assistant\",\"a\":{\"b\":[{\"content\":\"x\"},{\"c\":{\"content\":\"y\"}}]}\n",
        "this line mentions assistant but is not json\n",
        "{\"role\":\"assistant\",\"content\":\"!\"}\n",
    );
    assert_eq!(extract_final_answer(transcript), "!");
}

/// 验证 judge 请求包含问题、参考答案、每条断言原文、禁令与不可信数据提示。
#[test]
fn test_build_judge_request_contains_all_assertions() {
    let case = sample_case();
    let request = build_judge_request(&case, "模型答案文本");
    assert_eq!(request.messages.len(), 2);
    let system = &request.messages[0];
    assert_eq!(system.role, "system");
    assert!(system.content.contains("严格的答案评审"));
    assert!(system.content.contains("不可信数据"));

    let user = &request.messages[1];
    assert_eq!(user.role, "user");
    assert!(user.content.contains(&case.question));
    assert!(user.content.contains(&case.standard_answer.summary));
    for assertion in &case.standard_answer.must_mention {
        assert!(user.content.contains(assertion));
    }
    for assertion in &case.standard_answer.should_mention {
        assert!(user.content.contains(assertion));
    }
    for forbidden in &case.standard_answer.must_not_mention {
        assert!(user.content.contains(forbidden));
    }
    assert!(
        user.content
            .contains("<model_answer>\n模型答案文本\n</model_answer>")
    );
    // 先给 schema，再要求不要输出其他文字。
    let schema_pos = user.content.find("\"assertions\"").unwrap();
    let tail_pos = user.content.find("不要输出 JSON 以外的任何文字").unwrap();
    assert!(schema_pos < tail_pos);
}

/// 验证超长答案被截断到 MAX_ANSWER_CHARS，防止 prompt 膨胀。
#[test]
fn test_build_judge_request_truncates_long_answer() {
    let case = sample_case();
    let answer = "a".repeat(MAX_ANSWER_CHARS + 1000);
    let request = build_judge_request(&case, &answer);
    let user = &request.messages[1].content;
    let fenced = user
        .split("<model_answer>\n")
        .nth(1)
        .and_then(|rest| rest.split("\n</model_answer>").next())
        .unwrap();
    assert_eq!(fenced.chars().count(), MAX_ANSWER_CHARS);
    assert!(fenced.chars().all(|character| character == 'a'));
    assert!(user.contains("已截断"));
}

/// 验证纯 JSON 响应：全 supported 且无违规时 passed=true，字段计数正确。
#[test]
fn test_parse_judge_response_all_supported_passes() {
    let case = sample_case();
    let judgement = parse_judge_response(&case, &all_supported_response()).unwrap();
    assert_eq!(judgement.case_id, "test_case");
    assert!(judgement.passed);
    assert_eq!(judgement.must_supported, 3);
    assert_eq!(judgement.must_total, 3);
    assert_eq!(judgement.contradicted_count, 0);
    assert_eq!(judgement.bonus_supported, 0);
    assert_eq!(judgement.bonus_total, 1);
    assert_eq!(judgement.violations, 0);
    assert_eq!(judgement.verdicts.len(), 3);
    // 断言原文随判定记录带出，供报告直接展示。
    assert_eq!(
        judgement.verdicts[0].assertion,
        "text41 自身没有直接显示条件"
    );
    assert_eq!(judgement.overall_note, "答案完整");
}

/// 验证 ```json 围栏与前后多余文字被剥离后照常解析。
#[test]
fn test_parse_judge_response_strips_markdown_fence_and_prose() {
    let case = sample_case();
    let raw = format!(
        "好的，判定如下：\n```json\n{}\n```\n以上。",
        all_supported_response()
    );
    let judgement = parse_judge_response(&case, &raw).unwrap();
    assert!(judgement.passed);
    assert_eq!(judgement.must_supported, 3);
}

/// 验证 must 未全部 supported 时 passed=false，contradicted 单独计数。
#[test]
fn test_parse_judge_response_fails_when_must_not_fully_supported() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "not_mentioned", "note": "没提 panel35"},
            {"id": 3, "verdict": "contradicted", "note": "答案说成了 totalRowCount__ == 0"},
        ],
        "forbidden": [],
        "overall_note": "有缺漏",
    })
    .to_string();
    let judgement = parse_judge_response(&case, &raw).unwrap();
    assert!(!judgement.passed);
    assert_eq!(judgement.must_supported, 1);
    assert_eq!(judgement.contradicted_count, 1);
    assert_eq!(judgement.verdicts[2].verdict, Verdict::Contradicted);
}

/// 验证禁令被违反时 passed=false，violation 明细带禁令原文。
#[test]
fn test_parse_judge_response_fails_on_forbidden_violation() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "forbidden": [
            {"id": 1, "violated": true, "note": "答案声称 text41 自己配置了条件"},
        ],
        "overall_note": "有违禁表述",
    })
    .to_string();
    let judgement = parse_judge_response(&case, &raw).unwrap();
    assert!(!judgement.passed);
    assert_eq!(judgement.must_supported, 3);
    assert_eq!(judgement.violations, 1);
    assert!(judgement.forbidden_checks[0].violated);
    assert_eq!(
        judgement.forbidden_checks[0].assertion,
        "text41 自己配置了 visibleCondition"
    );
}

/// 验证未知 verdict 字符串直接报错，不静默归类。
#[test]
fn test_parse_judge_response_rejects_unknown_verdict() {
    let case = sample_case();
    let raw = r#"{"assertions":[{"id":1,"verdict":"partially","note":"x"}]}"#;
    let error = parse_judge_response(&case, raw).unwrap_err().to_string();
    assert!(error.contains("partially"));
    assert!(error.contains("无效"));
}

/// 验证响应完全不含 JSON 对象时报错。
#[test]
fn test_parse_judge_response_rejects_response_without_json() {
    let case = sample_case();
    let error = parse_judge_response(&case, "我无法判定")
        .unwrap_err()
        .to_string();
    assert!(error.contains("找不到 JSON 对象"));
}

/// 验证编号越界或重复的条目被忽略但记录在 overall_note 中，且不计入统计。
#[test]
fn test_parse_judge_response_ignores_out_of_range_ids_with_record() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
            {"id": 99, "verdict": "supported", "note": "幽灵条目"},
            {"id": 2, "verdict": "contradicted", "note": "重复编号"},
        ],
        "forbidden": [
            {"id": 0, "violated": true, "note": "非法编号"},
        ],
        "overall_note": "完成",
    })
    .to_string();
    let judgement = parse_judge_response(&case, &raw).unwrap();
    assert!(judgement.passed);
    assert_eq!(judgement.must_supported, 3);
    assert_eq!(judgement.violations, 0);
    assert!(judgement.overall_note.contains("完成"));
    assert!(judgement.overall_note.contains("忽略"));
    assert!(judgement.overall_note.contains("assertions#99"));
    assert!(judgement.overall_note.contains("forbidden#0"));
}

/// 验证 judge_case 端到端走 FakeModelAdapter：请求送达、响应解析为 CaseJudgement。
#[test]
fn test_judge_case_with_fake_adapter() {
    let case = sample_case();
    let mut adapter = FakeModelAdapter::from_responses(vec![all_supported_response()]);
    let judgement = judge_case(&case, "模型答案文本", &mut adapter).unwrap();
    assert!(judgement.passed);
    assert_eq!(judgement.must_supported, 3);
    // adapter 记录的请求就是 build_judge_request 的产出，模型占位符由真实 adapter 覆盖。
    let requests = adapter.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].messages[1].content.contains(&case.question));
    assert_eq!(requests[0].model, "kimi-smoke-judge");
}

/// 验证 Markdown 报告包含 case_id、PASS/FAIL 标记、逐条 verdict 与 violation 明细。
#[test]
fn test_render_judge_markdown_contains_key_sections() {
    let passing = CaseJudgement {
        case_id: "case_pass".to_string(),
        passed: true,
        must_supported: 1,
        must_total: 1,
        contradicted_count: 0,
        bonus_supported: 0,
        bonus_total: 0,
        violations: 0,
        verdicts: vec![AssertionVerdict {
            id: 1,
            verdict: Verdict::Supported,
            note: "覆盖".to_string(),
            assertion: "断言甲".to_string(),
        }],
        bonus_verdicts: Vec::new(),
        forbidden_checks: Vec::new(),
        overall_note: "通过".to_string(),
    };
    let failing = CaseJudgement {
        case_id: "case_fail".to_string(),
        passed: false,
        must_supported: 0,
        must_total: 1,
        contradicted_count: 1,
        bonus_supported: 0,
        bonus_total: 0,
        violations: 1,
        verdicts: vec![AssertionVerdict {
            id: 1,
            verdict: Verdict::Contradicted,
            note: "答案说反了".to_string(),
            assertion: "断言乙".to_string(),
        }],
        bonus_verdicts: Vec::new(),
        forbidden_checks: vec![ForbiddenCheck {
            id: 1,
            violated: true,
            note: "违反了禁令".to_string(),
            assertion: "禁令丙".to_string(),
        }],
        overall_note: "失败原因".to_string(),
    };
    let failing_for_escape = failing.clone();
    let markdown = render_judge_markdown(&[passing, failing]);
    assert!(markdown.contains("case_pass"));
    assert!(markdown.contains("case_fail"));
    assert!(markdown.contains("PASS"));
    assert!(markdown.contains("FAIL"));
    assert!(markdown.contains("断言甲"));
    assert!(markdown.contains("supported"));
    assert!(markdown.contains("contradicted"));
    assert!(markdown.contains("违反的禁令"));
    assert!(markdown.contains("禁令丙"));
    assert!(markdown.contains("失败原因"));
    // Markdown 表格分隔符被转义，断言文本不会破坏报告结构。
    let escaped = CaseJudgement {
        overall_note: "无".to_string(),
        verdicts: vec![AssertionVerdict {
            id: 1,
            verdict: Verdict::Supported,
            note: "含|管道".to_string(),
            assertion: "含|管道\n和换行".to_string(),
        }],
        ..failing_for_escape
    };
    let escaped_markdown = render_judge_markdown(&[escaped]);
    assert!(escaped_markdown.contains("含\\|管道"));
}

/// CNB pipeline 中对 kimi harness 六问冒烟 transcript 做语义判分；普通测试不运行。
///
/// env 契约：`CNB_TOKEN`（必需，缺则跳过）、`M58_CNB_REPO`（或 `CNB_REPO_SLUG` 兜底）、
/// `M58_CNB_MODEL`（judge 模型）、`M58_CNB_API_BASE`（可选）、
/// `KIMI_SMOKE_TRANSCRIPT_DIR`（默认 target/kimi-harness-smoke，目录不存在则跳过）、
/// `KIMI_JUDGE_OUTPUT_DIR`（默认与 transcript 目录相同）。
#[test]
#[ignore = "requires CNB_TOKEN, M58_CNB_REPO/CNB_REPO_SLUG, M58_CNB_MODEL and smoke transcripts"]
fn kimi_harness_smoke_judge_live() -> Result<()> {
    if std::env::var("CNB_TOKEN").is_err() {
        println!("跳过 kimi_harness_smoke_judge_live：缺少 CNB_TOKEN");
        return Ok(());
    }
    let transcript_dir = std::env::var_os("KIMI_SMOKE_TRANSCRIPT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/kimi-harness-smoke"));
    if !transcript_dir.is_dir() {
        println!(
            "跳过 kimi_harness_smoke_judge_live：transcript 目录不存在 {}",
            transcript_dir.display()
        );
        return Ok(());
    }

    // judge adapter 与 M58 runner 同一契约（CNB_TOKEN / M58_CNB_REPO / M58_CNB_MODEL /
    // M58_CNB_API_BASE），repo 额外允许 CNB_REPO_SLUG 兜底。不调 from_env：Rust 2024
    // 下补环境变量必须 unsafe，而显式读 env 调 new 行为完全一致。
    let token = std::env::var("CNB_TOKEN")?;
    let repo = std::env::var("M58_CNB_REPO")
        .or_else(|_| std::env::var("CNB_REPO_SLUG"))
        .context("缺少 M58_CNB_REPO（或 CNB_REPO_SLUG）")?;
    let model = std::env::var("M58_CNB_MODEL").context("缺少 M58_CNB_MODEL（judge 模型）")?;
    let endpoint =
        std::env::var("M58_CNB_API_BASE").unwrap_or_else(|_| "https://api.cnb.cool".to_string());
    let mut adapter = CnbChatAdapter::new(endpoint, repo, token, model)?;

    let cases_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json");
    let cases = load_smoke_cases(&cases_path)?;

    let output_dir = std::env::var_os("KIMI_JUDGE_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| transcript_dir.clone());
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("创建 judge 输出目录失败: {}", output_dir.display()))?;

    let mut judgements = Vec::new();
    let mut infra_errors: Vec<String> = Vec::new();
    for (question_no, case_id) in SMOKE_QUESTION_CASES {
        let Some(case) = cases.iter().find(|case| case.case_id == case_id) else {
            bail!("case 文件缺少六问映射的 {case_id}");
        };
        // 冒烟 stage 的标准文件名优先，t-q{n}.jsonl 是手工导出 transcript 的兼容名。
        let transcript_path = [
            transcript_dir.join(format!("smoke-transcript-{question_no}.jsonl")),
            transcript_dir.join(format!("t-{question_no}.jsonl")),
        ]
        .into_iter()
        .find(|path| path.is_file());
        let answer = match transcript_path {
            Some(path) => {
                let content = std::fs::read_to_string(&path)
                    .with_context(|| format!("读取 transcript 失败: {}", path.display()))?;
                extract_final_answer(&content)
            }
            None => {
                // transcript 缺失记 infra 占位，不让整场失败。
                let reason = format!("{question_no} transcript 缺失");
                println!("{reason}，记 infra 占位");
                judgements.push(infra_judgement(case, &reason));
                continue;
            }
        };
        if answer.trim().is_empty() {
            let reason = format!("{question_no} 最终答案为空");
            println!("{reason}，记 infra 占位");
            judgements.push(infra_judgement(case, &reason));
            continue;
        }
        match judge_case(case, &answer, &mut adapter) {
            Ok(judgement) => judgements.push(judgement),
            Err(error) => {
                // adapter/endpoint/judge 输出解析失败都是 infra 故障：记占位，
                // 报告落盘后让测试失败，infra 问题必须可见。
                let reason = format!("{question_no} judge 调用失败: {error:#}");
                println!("{reason}");
                judgements.push(infra_judgement(case, &reason));
                infra_errors.push(reason);
            }
        }
    }

    // 每 case 一行摘要，CI 靠 --nocapture 直接读日志；语义 pass/fail 只报告不断言。
    for ((question_no, _), judgement) in SMOKE_QUESTION_CASES.iter().zip(&judgements) {
        println!(
            "{} {} {} must={}/{} contradicted={} violations={}",
            question_no.to_uppercase(),
            judgement.case_id,
            status_label(judgement),
            judgement.must_supported,
            judgement.must_total,
            judgement.contradicted_count,
            judgement.violations,
        );
    }
    let markdown = render_judge_markdown(&judgements);
    std::fs::write(
        output_dir.join("judge.json"),
        format!("{}\n", serde_json::to_string_pretty(&judgements)?),
    )
    .with_context(|| "写入 judge.json 失败".to_string())?;
    std::fs::write(output_dir.join("judge.md"), &markdown)
        .with_context(|| "写入 judge.md 失败".to_string())?;
    println!("{markdown}");

    if !infra_errors.is_empty() {
        bail!("judge infra 故障: {}", infra_errors.join("; "));
    }
    Ok(())
}

/// 报告与日志中的状态标记：infra 占位与语义 PASS/FAIL 区分开。
fn status_label(judgement: &CaseJudgement) -> &'static str {
    if judgement.overall_note.starts_with("infra:") {
        "INFRA"
    } else if judgement.passed {
        "PASS"
    } else {
        "FAIL"
    }
}

/// 构造 infra 占位判分：transcript 缺失、答案为空或 judge 调用失败时记录。
fn infra_judgement(case: &SmokeCase, reason: &str) -> CaseJudgement {
    CaseJudgement {
        case_id: case.case_id.clone(),
        passed: false,
        must_supported: 0,
        must_total: case.standard_answer.must_mention.len(),
        contradicted_count: 0,
        bonus_supported: 0,
        bonus_total: case.standard_answer.should_mention.len(),
        violations: 0,
        verdicts: Vec::new(),
        bonus_verdicts: Vec::new(),
        forbidden_checks: Vec::new(),
        overall_note: format!("infra: {reason}"),
    }
}
