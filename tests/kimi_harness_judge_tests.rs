#![cfg(feature = "cli-local")]

#[path = "support/kimi_smoke_judge.rs"]
mod kimi_smoke_judge;
#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use anyhow::{Context, Result, bail};
use kimi_smoke_judge::{
    AssertionVerdict, CaseJudgement, ForbiddenCheck, MAX_ANSWER_CHARS, SmokeCase, StandardAnswer,
    TRIAL_RECORD_FIELDS, TrialJudgement, Verdict, build_judge_request, extract_final_answer,
    judge_case, load_smoke_cases, parse_judge_response, parse_trial_records, render_judge_markdown,
    resolve_transcript_path, smoke_subset,
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
        smoke: Default::default(),
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

/// 验证冒烟子集直接从 fixture 的 `smoke` 字段导出，且顺序连续无重复。
#[test]
fn test_smoke_subset_derives_from_fixture() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json");
    let cases = load_smoke_cases(&path).unwrap();
    let subset = smoke_subset(&cases).unwrap();
    assert_eq!(subset.len(), 6);
    for (index, case) in subset.iter().enumerate() {
        assert_eq!(
            case.smoke.order,
            index as u32 + 1,
            "{} 的 order 与位置不符，order 必须从 1 起连续",
            case.case_id
        );
        assert!(
            case.smoke.question_template.contains("{project_dir}"),
            "{} 的 question_template 缺少 {{project_dir}} 占位符",
            case.case_id
        );
    }
}

/// 验证没有任何 `smoke.enabled` 时报错。
///
/// 空子集会让判分产出「零 case、全绿」的报告——那比失败更危险，它看起来像通过。
#[test]
fn test_smoke_subset_rejects_empty_selection() {
    let cases = vec![SmokeCase {
        case_id: "c1".to_string(),
        question: String::new(),
        standard_answer: StandardAnswer::default(),
        smoke: Default::default(),
    }];
    assert!(smoke_subset(&cases).is_err());
}

/// 一行合法 record，字段顺序与 `.cnb.yml` 的 node 发射器一致。
fn sample_record_line() -> String {
    r#"{"case_id":"c1","order":1,"difficulty":"easy","variant":"baseline","trial":1,"exit_code":0,"wall_clock_ms":1234,"tool_calls":3,"metadata_checker_invocations":2,"raw_fallback_calls":1,"raw_fallback":true,"transcript_bytes":4096,"transcript_path":"out/t.jsonl","stderr_path":"out/t.log"}"#
        .to_string()
}

/// 从 `.cnb.yml` 里那段 node 发射器抽出它实际写出的字段名。
///
/// 定位 `JSON.stringify({` 到配对的 `})`，逐行取 `key:` 或简写 `key,`。
fn cnb_emitted_record_fields() -> Vec<String> {
    let cnb = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cnb.yml"))
        .expect("读取 .cnb.yml 失败");
    let start = cnb
        .find("process.stdout.write(JSON.stringify({")
        .expect(".cnb.yml 里找不到 records.jsonl 的 node 发射器；若已改写请同步本测试");
    let body_start = start + cnb[start..].find('{').unwrap();
    let body_start = body_start + cnb[body_start..].find('{').unwrap() + 1;
    let end = body_start
        + cnb[body_start..]
            .find("}) + \"\\n\"")
            .expect("node 发射器的对象字面量没有正常结束");

    cnb[body_start..end]
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let name = trimmed
                .split_once(':')
                .map(|(key, _)| key)
                .unwrap_or_else(|| trimmed.trim_end_matches(','));
            let name = name.trim();
            let is_ident = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            is_ident.then(|| name.to_string())
        })
        .collect()
}

/// `TRIAL_RECORD_FIELDS` 与结构体本身同步。
///
/// 清单是给跨语言比对用的手写副本，漂了就等于比对了个寂寞。用一行合法 JSON
/// 逐字段删除、断言每次都反序列化失败，来证明清单里每一项都确实是必需字段。
#[test]
fn test_trial_record_field_list_matches_struct() {
    let line = sample_record_line();
    let full: serde_json::Value = serde_json::from_str(&line).unwrap();
    // serde_json 的 Map 按 key 排序，样例行的书写顺序在这里已经丢了，只能比集合；
    // 顺序一致性由 test_cnb_record_emitter_matches_trial_record_schema 保证。
    let mut keys: Vec<&str> = full
        .as_object()
        .unwrap()
        .keys()
        .map(|k| k.as_str())
        .collect();
    keys.sort_unstable();
    let mut expected: Vec<&str> = TRIAL_RECORD_FIELDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "样例 record 的字段与 TRIAL_RECORD_FIELDS 不一致"
    );

    for field in TRIAL_RECORD_FIELDS {
        let mut reduced = full.clone();
        reduced.as_object_mut().unwrap().remove(field);
        assert!(
            parse_trial_records(&reduced.to_string()).is_err(),
            "删掉 {field} 后仍能解析，说明它不是必需字段，清单与结构体已漂移"
        );
    }
}

/// `.cnb.yml` 的 node 发射器与 Rust 侧 schema 字段名一致。
///
/// 这是本组测试真正拦住的东西：产出方是 shell 里的 JS，消费方是 Rust，
/// 中间没有编译期联系。一个 `transcript_byte`（少个 s）今天要等 Phase 2
/// 读 records 时才炸，那时 pipeline 的算力已经花完了。
#[test]
fn test_cnb_record_emitter_matches_trial_record_schema() {
    let emitted = cnb_emitted_record_fields();
    assert_eq!(
        emitted, TRIAL_RECORD_FIELDS,
        ".cnb.yml 发射的 record 字段与 TrialRecord 不一致（左：.cnb.yml，右：Rust）"
    );
}

/// `.cnb.yml` 的 run.json 必须记下 fixture/gold 身份、未设置的温度、以及
/// kimi-code 浮动策略。少记一项，跨次实验就无法判断是改了 gold 还是改了 SKILL。
#[test]
fn test_cnb_run_json_records_eval_identity() {
    let cnb = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cnb.yml"))
        .expect("读取 .cnb.yml 失败");
    let start = cnb
        .find("const manifest = {")
        .expect(".cnb.yml 里找不到 run.json 的 manifest 对象；若已改写请同步本测试");
    let body = &cnb[start..];
    for needle in [
        "fixture_path:",
        "fixture_sha256:",
        "agent_temperature: null",
        "judge_temperature: null",
        "kimi_pin_policy: \"float\"",
    ] {
        assert!(
            body.contains(needle),
            "run.json manifest 缺少 {needle}：\n{}",
            &body[..body.len().min(800)]
        );
    }
    assert!(
        cnb.contains("FIXTURE_SHA=\"$(sha256sum \"$CASES_JSON\""),
        "fixture_sha256 必须由 sha256sum 对 CASES_JSON 算出，不能手填"
    );
}

/// 阶段 B 必须是同一 `api_trigger_kimi_harness_smoke` 的下一 stage，复用
/// workspace / cargo target / smoke 目录。另起 pipeline 会重装 kimi、重编 CLI。
#[test]
fn test_cnb_stage_b_judge_follows_smoke_in_same_pipeline() {
    let cnb = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cnb.yml"))
        .expect("读取 .cnb.yml 失败");
    let smoke = cnb
        .find("name: kimi-code harness smoke run")
        .expect("找不到阶段 A smoke run");
    let judge = cnb
        .find("name: kimi-code harness judge")
        .expect("找不到阶段 B judge");
    assert!(
        smoke < judge,
        "judge 必须紧跟在同一 pipeline 的 smoke run 之后，不能另起 api_trigger"
    );
    assert!(
        !cnb.contains("api_trigger_kimi_harness_judge"),
        "不要为判分再开一条 api_trigger：runner env 必须由本 pipeline 复用"
    );
    let smoke_script = &cnb[smoke..judge];
    assert!(
        !smoke_script.contains("kimi_harness_judge_tests -- --ignored"),
        "阶段 A 不应再跑 live judge；判分属于下一 stage"
    );
    let judge_script = &cnb[judge..];
    let judge_end = judge_script
        .find("endStages:")
        .expect("judge stage 之后应是 endStages");
    let judge_script = &judge_script[..judge_end];
    assert!(
        judge_script.contains("kimi_harness_judge_tests -- --ignored"),
        "阶段 B 必须跑 live judge"
    );
    assert!(
        !judge_script.contains("install.sh"),
        "阶段 B 不得重装 kimi-code"
    );
    assert!(
        !judge_script.contains("cargo build --release"),
        "阶段 B 不得重编 release 二进制"
    );
    assert!(
        judge_script.contains("fixture_sha mismatch"),
        "阶段 B 必须核对 run.json 的 fixture_sha256"
    );
}

/// `.cnb.yml` 产出的 transcript / stderr 文件名必须带 variant 与 trial。
///
/// 与上一条测试同一动机，换一个失效模式：文件名只按 case_id 命名时，同一 stage 内
/// 的第二个 trial 会覆盖第一个的答案——records 有 N 行、磁盘上只剩 1 份，judge 评的
/// 是幸存者，报告读起来完全正常。这是**跑完一整轮才可能被察觉**的静默丢数据，
/// 因此把它钉在 `cargo test` 上，而不是等 Phase 2 打开多 trial 时靠肉眼发现。
#[test]
fn test_cnb_transcript_names_carry_variant_and_trial() {
    let cnb = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cnb.yml"))
        .expect("读取 .cnb.yml 失败");
    assert!(
        cnb.contains(r#"RUN_TAG="${CASE_ID}__${VARIANT}__t${TRIAL}""#),
        ".cnb.yml 的 RUN_TAG 不再由 case_id + variant + trial 组成"
    );
    assert!(
        cnb.contains(r#"TRANSCRIPT="$OUT_DIR/smoke-transcript-${RUN_TAG}.jsonl""#),
        "transcript 文件名未使用 RUN_TAG：多 trial 会互相覆盖"
    );
    assert!(
        cnb.contains(r#"STDERR_LOG="$OUT_DIR/smoke-stderr-${RUN_TAG}.log""#),
        "stderr 文件名未使用 RUN_TAG：多 trial 会互相覆盖"
    );
    // 清理 glob 必须仍覆盖新命名，否则上一轮产物会混进本轮报告。
    assert!(
        cnb.contains(r#"rm -f "$OUT_DIR"/smoke-transcript-*.jsonl"#),
        "开跑清理的 glob 与 transcript 命名不再匹配"
    );
}

/// 换取 commit 附件上传链接的成功码必须是 201。
///
/// OpenAPI `PostCommitAssetUploadURL` 只声明 201 Created。写成 200 时干跑 stub
/// 若也回 200 会假绿，而 CNB 实跑返回 201 会被当成失败——2026-08-20 四件附件
/// 因此全部没归档。这条守卫把契约钉在 `.cnb.yml` 原文上。
#[test]
fn test_cnb_archive_upload_url_accepts_201() {
    let cnb = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(".cnb.yml"))
        .expect("读取 .cnb.yml 失败");
    assert!(
        cnb.contains(r#"if [ "$META_CODE" != "201" ]; then"#),
        ".cnb.yml 换取上传链接不再把 201 当成功（OpenAPI PostCommitAssetUploadURL）"
    );
    assert!(
        !cnb.contains(r#"if [ "$META_CODE" != "200" ]; then"#),
        ".cnb.yml 又把换取上传链接的成功码写成了 200"
    );
}

/// 合法 records.jsonl 正常解析，空行被跳过。
#[test]
fn test_parse_trial_records_accepts_valid_lines() {
    let second = sample_record_line()
        .replace("\"c1\"", "\"c2\"")
        .replace("\"order\":1", "\"order\":2");
    let jsonl = format!("{}\n\n{}\n", sample_record_line(), second);
    let records = parse_trial_records(&jsonl).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].case_id, "c1");
    assert_eq!(records[0].wall_clock_ms, 1234);
    assert_eq!(records[1].order, 2);
}

/// 多出的字段判红：写错的键会同时表现为「多一个未知字段」，不能静默吞掉。
#[test]
fn test_parse_trial_records_rejects_unknown_field() {
    let line = sample_record_line().replace("\"tool_calls\"", "\"toolCalls\"");
    let err = parse_trial_records(&line).unwrap_err().to_string();
    assert!(err.contains("TrialRecord schema"), "实际错误：{err}");
}

/// 计时缺失时 node 侧会写出 `null`（`Number("") - Number("")` = NaN），必须判红。
#[test]
fn test_parse_trial_records_rejects_null_wall_clock() {
    let line = sample_record_line().replace("\"wall_clock_ms\":1234", "\"wall_clock_ms\":null");
    assert!(parse_trial_records(&line).is_err());
}

/// 负墙钟同样是计时出错，不是一个可以照单全收的数。
#[test]
fn test_parse_trial_records_rejects_negative_wall_clock() {
    let line = sample_record_line().replace("\"wall_clock_ms\":1234", "\"wall_clock_ms\":-5");
    let err = parse_trial_records(&line).unwrap_err().to_string();
    assert!(err.contains("wall_clock_ms"), "实际错误：{err}");
}

/// 空文件判红：下游会据此产出「零 trial 全绿」的空报告。
#[test]
fn test_parse_trial_records_rejects_empty_input() {
    assert!(parse_trial_records("\n\n").is_err());
}

/// (case_id, variant, trial) 撞号判红：配对网格会悄悄覆盖掉其中一条。
#[test]
fn test_parse_trial_records_rejects_duplicate_key() {
    let jsonl = format!("{}\n{}\n", sample_record_line(), sample_record_line());
    let err = parse_trial_records(&jsonl).unwrap_err().to_string();
    assert!(err.contains("重复"), "实际错误：{err}");
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
    assert!(
        user.content
            .contains("遗漏、未提及、没有解释某项关系，绝不构成禁令违反")
    );
    assert!(
        user.content
            .contains("不能用参考答案中有而被评审答案中没有的内容作依据")
    );
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
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
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
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": true, "note": "答案声称 text41 自己配置了条件"},
            {"id": 2, "violated": false, "note": "未违规"},
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

/// 验证编号越界或重复的条目按 judge 基础设施故障报错——无效编号意味着对应断言
/// 没被查，静默忽略会把「judge 漏判」变成「答案没问题」的假通过。
#[test]
fn test_parse_judge_response_rejects_out_of_range_or_duplicate_ids() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
            {"id": 99, "verdict": "supported", "note": "幽灵条目"},
            {"id": 2, "verdict": "contradicted", "note": "重复编号"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
            {"id": 0, "violated": true, "note": "非法编号"},
        ],
        "overall_note": "完成",
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("无效或重复编号"), "{error}");
    assert!(error.contains("assertions#99"), "{error}");
    assert!(error.contains("forbidden#0"), "{error}");
}

/// 验证 judge 漏检全部禁令时按基础设施故障报错，而不是拿 `violations == 0` 假通过。
#[test]
fn test_parse_judge_response_errors_when_forbidden_omitted() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "overall_note": "忘了查禁令",
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("禁令检查不完整"), "{error}");
    assert!(error.contains("0/2"), "{error}");
}

/// 验证禁令只覆盖了一部分同样报错——漏检的禁令不可能违规，必然酿成假通过。
#[test]
fn test_parse_judge_response_errors_on_partial_forbidden_coverage() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "只查了一条"},
        ],
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("禁令检查不完整"), "{error}");
    assert!(error.contains("1/2"), "{error}");
}

/// 验证禁令条目缺 violated 字段直接报错——violated 是判分必需字段，默认成 false
/// 会把「judge 漏查」记成「未违反」并通过完整性检查，缺失必须是 judge infra 错误。
#[test]
fn test_parse_judge_response_rejects_missing_violated_field() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "查过"},
            {"id": 2, "note": "只写了 note 忘了布尔"},
        ],
    })
    .to_string();
    // {:#} 展开 anyhow 错误链：顶层 context 是「不是合法 JSON」，字段名在 serde 的 source 里。
    let error = format!("{:#}", parse_judge_response(&case, &raw).unwrap_err());
    assert!(error.contains("missing field `violated`"), "{error}");
}

/// 验证 must 漏判时按基础设施故障报错——少判一条就有断言没被查，pass 会是假通过。
#[test]
fn test_parse_judge_response_errors_when_must_incomplete() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("must 判定不完整"), "{error}");
    assert!(error.contains("2/3"), "{error}");
}

/// 验证 bonus 漏判同样报错：bonus 不计 pass，但漏判说明 judge 没按清单逐条检查，
/// 其余判定也不可信。
#[test]
fn test_parse_judge_response_errors_when_bonus_incomplete() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("bonus 判定不完整"), "{error}");
    assert!(error.contains("0/1"), "{error}");
}

/// 验证布尔与解释矛盾的禁令判定报错：`violated=true` 但 note 自写「故violated=false」
/// 是 CNB 实跑出现过的样本——结构化字段与自声明总有一侧是错的，输出不可信。
#[test]
fn test_parse_judge_response_rejects_self_contradicting_forbidden() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "覆盖"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": true, "note": "答案措辞像违规，但细看不构成，故violated=false"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("自相矛盾"), "{error}");
    assert!(error.contains("forbidden#1"), "{error}");
}

/// 验证断言判定与 note 自声明矛盾时报错：`verdict=supported` 与「supported=false」冲突。
#[test]
fn test_parse_judge_response_rejects_self_contradicting_assertion() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "答案没提到，supported=false"},
            {"id": 2, "verdict": "supported", "note": "覆盖"},
            {"id": 3, "verdict": "supported", "note": "覆盖"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "未违规"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
    })
    .to_string();
    let error = parse_judge_response(&case, &raw).unwrap_err().to_string();
    assert!(error.contains("自相矛盾"), "{error}");
    assert!(error.contains("assertions#1"), "{error}");
}

/// 验证叙述性 note 不误伤：没有 `字段=值` 形态的显式自声明时不做矛盾比对。
#[test]
fn test_parse_judge_response_tolerates_narrative_note() {
    let case = sample_case();
    let raw = serde_json::json!({
        "assertions": [
            {"id": 1, "verdict": "supported", "note": "答案说自身无直接条件，这点成立"},
            {"id": 2, "verdict": "supported", "note": "不算完全准确，但继承了 panel35 的意思到了"},
            {"id": 3, "verdict": "supported", "note": "不支持过度解读，表达式一致"},
        ],
        "bonus": [
            {"id": 1, "verdict": "not_mentioned", "note": "未展开"},
        ],
        "forbidden": [
            {"id": 1, "violated": false, "note": "不构成违反"},
            {"id": 2, "violated": false, "note": "未违规"},
        ],
    })
    .to_string();
    let judgement = parse_judge_response(&case, &raw).unwrap();
    assert!(judgement.passed);
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
    let markdown = render_judge_markdown(
        &[
            TrialJudgement::new("baseline", 1, passing),
            TrialJudgement::new("baseline", 1, failing),
        ],
        &[],
    );
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
    // 没有 records 时两个 trial 都算「缺少匹配 record」，行为分层全部为空。
    assert!(
        markdown.contains("answer_quality（task_score）：1/2 trial"),
        "{markdown}"
    );
    assert!(markdown.contains("tool_adherence：0/0 trial"), "{markdown}");
    assert!(
        markdown.contains("tool_assisted_quality：0/0 trial"),
        "{markdown}"
    );
    assert!(markdown.contains("三个率分母固定"), "{markdown}");
    assert!(markdown.contains("缺少匹配 record 的 trial"), "{markdown}");
    assert!(
        markdown.contains("case_pass/baseline/t1、case_fail/baseline/t1"),
        "{markdown}"
    );
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
    let escaped_markdown =
        render_judge_markdown(&[TrialJudgement::new("baseline", 1, escaped)], &[]);
    assert!(escaped_markdown.contains("含\\|管道"));
}

/// 验证得分口径：三个固定分母率 + 四个互斥行为分层。
///
/// `answer_quality` 含 raw-only PASS；`tool_assisted_quality` 不含。
/// 禁止再按行为筛分母（已废弃的 tool_score）。
#[test]
fn test_render_judge_markdown_reports_fixed_denominator_strata() {
    let judged = |case_id: &str, passed: bool| CaseJudgement {
        case_id: case_id.to_string(),
        passed,
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
            assertion: "断言".to_string(),
        }],
        bonus_verdicts: Vec::new(),
        forbidden_checks: Vec::new(),
        overall_note: "完成".to_string(),
    };
    let judgements = vec![
        TrialJudgement::new("baseline", 1, judged("case_tool_pass", true)),
        TrialJudgement::new("baseline", 1, judged("case_raw_pass", true)),
        TrialJudgement::new("baseline", 1, judged("case_mixed_fail", false)),
    ];
    let record = |case_id: &str, mc: u32, raw_calls: u32, raw: bool| {
        format!(
            r#"{{"case_id":"{case_id}","order":1,"difficulty":"easy","variant":"baseline","trial":1,"exit_code":0,"wall_clock_ms":100,"tool_calls":3,"metadata_checker_invocations":{mc},"raw_fallback_calls":{raw_calls},"raw_fallback":{raw},"transcript_bytes":100,"transcript_path":"t.jsonl","stderr_path":"t.log"}}"#
        )
    };
    let records = parse_trial_records(&format!(
        "{}\n{}\n{}\n",
        record("case_tool_pass", 3, 0, false),
        record("case_raw_pass", 0, 2, true),
        record("case_mixed_fail", 2, 1, true),
    ))
    .unwrap();
    let markdown = render_judge_markdown(&judgements, &records);
    // answer_quality 分母固定为全部非 INFRA trial，含绕过工具完成的。
    assert!(
        markdown.contains("answer_quality（task_score）：2/3 trial"),
        "{markdown}"
    );
    // 单 trial 时 case_stable_pass 与 answer_quality 同源，分母是 case 数。
    assert!(
        markdown.contains("case_stable_pass：2/3 case"),
        "{markdown}"
    );
    // 三个率：tool 2/3（tool_pass + mixed_fail），assisted 1/3（只有 tool_pass）。
    assert!(markdown.contains("tool_adherence：2/3 trial"), "{markdown}");
    assert!(
        markdown.contains("tool_assisted_quality：1/3 trial"),
        "{markdown}"
    );
    assert!(markdown.contains("三个率分母固定"), "{markdown}");
    assert!(markdown.contains("LLMOps 主指标"), "{markdown}");
    // 行为分层分母固定：三个 trial 各自进入唯一分层，fallback 的 PASS 不再被移出分母。
    assert!(
        markdown.contains(
            "tool-only（mc>0，无 raw fallback）：1 trial，PASS 1 —— case_tool_pass/baseline/t1"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "raw-only（mc=0，绕过工具直读原始文件）：1 trial，PASS 1 —— case_raw_pass/baseline/t1"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "mixed（mc>0 且有 raw fallback）：1 trial，PASS 0 —— case_mixed_fail/baseline/t1"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains("no-tool（mc=0 且无 raw fallback）：0 trial"),
        "{markdown}"
    );
    // raw_fallback 只作诊断信号的声明必须在报告里。
    assert!(markdown.contains("启发式诊断信号"), "{markdown}");
}

/// 验证同一 case 的多个 trial 各自判分、各自配对：一份判分只配一条 record，
/// 不同 trial 的结论不互相顶替。
///
/// 这条测试此前编码的正是缺陷本身：只给一份 case 级判分、两条 record，然后断言
/// 「两个分层各 PASS 1」——同一个结论被算了两票。真实语义是每个 trial 是一次
/// 独立运行，可以一次通过一次失败。
#[test]
fn test_render_judge_markdown_scores_each_trial_independently() {
    let judged = |passed: bool| CaseJudgement {
        case_id: "case_multi".to_string(),
        passed,
        must_supported: if passed { 1 } else { 0 },
        must_total: 1,
        contradicted_count: 0,
        bonus_supported: 0,
        bonus_total: 0,
        violations: 0,
        verdicts: vec![AssertionVerdict {
            id: 1,
            verdict: if passed {
                Verdict::Supported
            } else {
                Verdict::NotMentioned
            },
            note: "依据".to_string(),
            assertion: "断言".to_string(),
        }],
        bonus_verdicts: Vec::new(),
        forbidden_checks: Vec::new(),
        overall_note: "完成".to_string(),
    };
    let record = |trial: u32, mc: u32| {
        format!(
            r#"{{"case_id":"case_multi","order":1,"difficulty":"easy","variant":"baseline","trial":{trial},"exit_code":0,"wall_clock_ms":100,"tool_calls":3,"metadata_checker_invocations":{mc},"raw_fallback_calls":0,"raw_fallback":false,"transcript_bytes":100,"transcript_path":"t{trial}.jsonl","stderr_path":"t{trial}.log"}}"#
        )
    };
    // 故意按 trial 2 在前的顺序输入：配对按三元组，不得依赖顺序。
    let records = parse_trial_records(&format!("{}\n{}\n", record(2, 0), record(1, 3))).unwrap();
    let markdown = render_judge_markdown(
        &[
            TrialJudgement::new("baseline", 1, judged(true)),
            TrialJudgement::new("baseline", 2, judged(false)),
        ],
        &records,
    );
    // trial 1 通过、trial 2 失败：分层各自记自己那条 record 的结论。
    assert!(
        markdown.contains(
            "tool-only（mc>0，无 raw fallback）：1 trial，PASS 1 —— case_multi/baseline/t1"
        ),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "no-tool（mc=0 且无 raw fallback）：1 trial，PASS 0 —— case_multi/baseline/t2"
        ),
        "{markdown}"
    );
    // 一个 case 只要有 trial 失败就不算稳定通过。
    assert!(
        markdown.contains("answer_quality（task_score）：1/2 trial"),
        "{markdown}"
    );
    assert!(
        markdown.contains("case_stable_pass：0/1 case"),
        "{markdown}"
    );
    // 每个 trial 一节，工具调用只列自己那条 record。
    assert!(
        markdown.contains("## case_multi/baseline/t1 — PASS"),
        "{markdown}"
    );
    assert!(
        markdown.contains("## case_multi/baseline/t2 — FAIL"),
        "{markdown}"
    );
}

/// 验证两侧配不上的情况都被点名：判分找不到 record（工具统计未知），
/// record 找不到判分（算力已花却在报告里消失）。两种都曾是静默的。
#[test]
fn test_render_judge_markdown_names_unpaired_sides() {
    let judgement = CaseJudgement {
        case_id: "case_judged".to_string(),
        passed: true,
        must_supported: 1,
        must_total: 1,
        contradicted_count: 0,
        bonus_supported: 0,
        bonus_total: 0,
        violations: 0,
        verdicts: Vec::new(),
        bonus_verdicts: Vec::new(),
        forbidden_checks: Vec::new(),
        overall_note: "完成".to_string(),
    };
    let record = |case_id: &str, trial: u32| {
        format!(
            r#"{{"case_id":"{case_id}","order":1,"difficulty":"easy","variant":"baseline","trial":{trial},"exit_code":0,"wall_clock_ms":100,"tool_calls":1,"metadata_checker_invocations":1,"raw_fallback_calls":0,"raw_fallback":false,"transcript_bytes":100,"transcript_path":"t.jsonl","stderr_path":"t.log"}}"#
        )
    };
    // record 是 case_judged/t2 与 case_unjudged/t1，判分是 case_judged/t1：三者两两配不上。
    let records = parse_trial_records(&format!(
        "{}\n{}\n",
        record("case_judged", 2),
        record("case_unjudged", 1)
    ))
    .unwrap();
    let markdown =
        render_judge_markdown(&[TrialJudgement::new("baseline", 1, judgement)], &records);
    assert!(
        markdown.contains("缺少匹配 record 的 trial（工具统计未知）：case_judged/baseline/t1"),
        "{markdown}"
    );
    assert!(
        markdown.contains("有 record 但没有判分的 trial"),
        "{markdown}"
    );
    assert!(
        markdown.contains("case_judged/baseline/t2、case_unjudged/baseline/t1"),
        "{markdown}"
    );
}

/// 验证 transcript 定位以 record 的 `transcript_path` 为准：按 case_id 重拼路径是
/// 独立猜测，两侧一旦分叉（文件名加了 variant/trial）就会安静地评错一份答案。
#[test]
fn test_resolve_transcript_path_prefers_recorded_path() {
    // 仓库内既有惯例：不引 tempfile，直接在系统临时目录下开一个唯一子目录。
    let dir = std::env::temp_dir().join(format!(
        "metadata-checker-judge-transcript-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let recorded = dir.join("smoke-transcript-case_a__baseline__t2.jsonl");
    std::fs::write(&recorded, "{}\n").unwrap();
    let record_line = format!(
        r#"{{"case_id":"case_a","order":1,"difficulty":"easy","variant":"baseline","trial":2,"exit_code":0,"wall_clock_ms":100,"tool_calls":1,"metadata_checker_invocations":1,"raw_fallback_calls":0,"raw_fallback":false,"transcript_bytes":3,"transcript_path":"{}","stderr_path":"x.log"}}"#,
        recorded.display()
    );
    let records = parse_trial_records(&record_line).unwrap();
    // 记录里的绝对路径存在，直接采用。
    assert_eq!(
        resolve_transcript_path(&records[0], Path::new("/nonexistent")),
        Some(recorded.clone())
    );

    // 记录路径不存在时按 basename 落到 transcript 目录：artifact 下载到别处也能重判。
    let moved = dir.join("elsewhere");
    std::fs::create_dir(&moved).unwrap();
    let moved_file = moved.join("smoke-transcript-case_a__baseline__t2.jsonl");
    std::fs::rename(&recorded, &moved_file).unwrap();
    assert_eq!(
        resolve_transcript_path(&records[0], &moved),
        Some(moved_file)
    );

    // 两处都没有就返回 None，由调用方记 infra 占位，不猜第三种路径。
    std::fs::remove_dir_all(&moved).unwrap();
    assert_eq!(resolve_transcript_path(&records[0], &dir), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// CNB pipeline 中对 kimi harness 六问冒烟 transcript 做语义判分；普通测试不运行。
///
/// env 契约：`CNB_TOKEN`（必需，缺则跳过）、`CNB_REPO_SLUG`（仓库）、
/// `KIMI_JUDGE_MODEL`（AI IDE v2 judge 模型）、`KIMI_JUDGE_API_BASE`（可选）、
/// `KIMI_SMOKE_TRANSCRIPT_DIR`（默认 target/kimi-harness-smoke，目录不存在则跳过）、
/// `KIMI_JUDGE_OUTPUT_DIR`（默认与 transcript 目录相同）。
#[test]
#[ignore = "requires CNB_TOKEN, CNB_REPO_SLUG, KIMI_JUDGE_MODEL and smoke transcripts"]
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

    // judge 使用 AI IDE v2 内部路由，与被测 agent 的 CNB AI Chat 路由和模型分离。
    // 不调 from_env：该构造器固定指向 OpenAPI `/-/ai`，而 judge 必须使用
    // `/-/ai-ide/v2` 的直传 Authorization 契约。
    let token = std::env::var("CNB_TOKEN")?;
    let repo = std::env::var("CNB_REPO_SLUG").context("缺少 CNB_REPO_SLUG")?;
    let model = std::env::var("KIMI_JUDGE_MODEL")
        .context("缺少 KIMI_JUDGE_MODEL（AI IDE v2 judge 模型）")?;
    let endpoint = std::env::var("KIMI_JUDGE_API_BASE")
        .or_else(|_| std::env::var("CNB_API_ENDPOINT"))
        .unwrap_or_else(|_| "https://api.cnb.cool".to_string());
    let mut adapter = CnbChatAdapter::new_ai_ide_v2(endpoint, repo, token, model)?;

    let cases_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json");
    let cases = load_smoke_cases(&cases_path)?;

    let output_dir = std::env::var_os("KIMI_JUDGE_OUTPUT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| transcript_dir.clone());
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("创建 judge 输出目录失败: {}", output_dir.display()))?;

    // records.jsonl 是阶段 A 交给所有下游的契约文件，在这里就地校验一次，
    // 并作为**判分的驱动列表**：一条 record = 一次真实运行 = 一份要判的答案。
    // 判分 stage 是 pipeline 里最后一个读它的 Rust 环节；放到 Phase 2 才发现字段
    // 对不上，意味着一整轮 pipeline 的算力已经花掉了。文件不存在不算错——
    // 单独重跑判分时本来就没有它，那时退回按 case 发现 transcript。
    let records_path = transcript_dir.join("records.jsonl");
    let mut trial_records = Vec::new();
    if records_path.is_file() {
        let content = std::fs::read_to_string(&records_path)
            .with_context(|| format!("读取 records 失败: {}", records_path.display()))?;
        trial_records = parse_trial_records(&content)
            .with_context(|| format!("records.jsonl 校验失败: {}", records_path.display()))?;
        println!(
            "records.jsonl 校验通过：{} 条 trial，总墙钟 {} ms",
            trial_records.len(),
            trial_records.iter().map(|r| r.wall_clock_ms).sum::<i64>()
        );
    } else {
        println!("未找到 {}，跳过 records 校验", records_path.display());
    }

    let subset = smoke_subset(&cases)?;
    let mut judgements: Vec<TrialJudgement> = Vec::new();
    let mut infra_errors: Vec<String> = Vec::new();

    // 判一份答案：读 transcript、抽最终答案、调 judge。三种失败都记 infra 占位
    // 并计入 infra_errors——评测基础设施失效时测试必须失败，不能拿占位报告当成功。
    let mut judge_one = |case: &SmokeCase,
                         label: &str,
                         path: Option<PathBuf>,
                         judgements: &mut Vec<TrialJudgement>,
                         infra_errors: &mut Vec<String>,
                         variant: &str,
                         trial: u32|
     -> Result<()> {
        let Some(path) = path else {
            let reason = format!("{label} transcript 缺失");
            println!("{reason}，记 infra 占位");
            judgements.push(TrialJudgement::new(
                variant,
                trial,
                infra_judgement(case, &reason),
            ));
            infra_errors.push(reason);
            return Ok(());
        };
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("读取 transcript 失败: {}", path.display()))?;
        let answer = extract_final_answer(&content);
        if answer.trim().is_empty() {
            let reason = format!("{label} 最终答案为空");
            println!("{reason}，记 infra 占位");
            judgements.push(TrialJudgement::new(
                variant,
                trial,
                infra_judgement(case, &reason),
            ));
            infra_errors.push(reason);
            return Ok(());
        }
        match judge_case(case, &answer, &mut adapter) {
            Ok(judgement) => judgements.push(TrialJudgement::new(variant, trial, judgement)),
            Err(error) => {
                let reason = format!("{label} judge 调用失败: {error:#}");
                println!("{reason}");
                judgements.push(TrialJudgement::new(
                    variant,
                    trial,
                    infra_judgement(case, &reason),
                ));
                infra_errors.push(reason);
            }
        }
        Ok(())
    };

    if trial_records.is_empty() {
        // 手工重判路径：没有 records 就没有 (variant, trial) 身份，按 case 发现
        // transcript 并明确记为 unrecorded/t0，报告里一眼看得出这不是流水线产物。
        for case in &subset {
            let question_no = format!("q{}", case.smoke.order);
            // 兼容名只在这条路径上保留：流水线产出的文件名带 variant/trial，
            // 由 records 驱动，不走这里。
            let path = [
                transcript_dir.join(format!("smoke-transcript-{}.jsonl", case.case_id)),
                transcript_dir.join(format!("smoke-transcript-{question_no}.jsonl")),
                transcript_dir.join(format!("t-{question_no}.jsonl")),
            ]
            .into_iter()
            .find(|path| path.is_file());
            if let Some(path) = &path {
                println!(
                    "警告：{} 无 records.jsonl，按 case 发现 transcript {}，身份记为 unrecorded/t0",
                    case.case_id,
                    path.display()
                );
            }
            judge_one(
                case,
                &question_no,
                path,
                &mut judgements,
                &mut infra_errors,
                "unrecorded",
                0,
            )?;
        }
    } else {
        // 流水线路径：逐 record 判分。transcript 路径取自 record 而不是按 case_id
        // 重拼——重拼是一次独立猜测，两侧一旦分叉（文件名带上了 variant/trial 而
        // 判分侧没跟上）就会安静地评到另一份答案上。
        for record in &trial_records {
            let label = format!("{}/{}/t{}", record.case_id, record.variant, record.trial);
            let Some(case) = subset
                .iter()
                .find(|case| case.case_id == record.case_id)
                .copied()
            else {
                // record 指向 case 文件里没有的 case_id：两侧已经不是同一份 fixture，
                // 继续判分只会产出无法归因的报告。
                let reason = format!("{label} 在 case 文件的 smoke 子集里没有对应 case");
                println!("{reason}");
                infra_errors.push(reason);
                continue;
            };
            let path = resolve_transcript_path(record, &transcript_dir);
            judge_one(
                case,
                &label,
                path,
                &mut judgements,
                &mut infra_errors,
                &record.variant,
                record.trial,
            )?;
        }
        // 子集里一条 record 都没有的 case：阶段 A 根本没跑到它。
        for case in &subset {
            if !trial_records
                .iter()
                .any(|record| record.case_id == case.case_id)
            {
                let reason = format!("{} 没有任何 record：阶段 A 未产出该 case", case.case_id);
                println!("{reason}，记 infra 占位");
                judgements.push(TrialJudgement::new(
                    "unrecorded",
                    0,
                    infra_judgement(case, &reason),
                ));
                infra_errors.push(reason);
            }
        }
    }

    // 每 trial 一行摘要，CI 靠 --nocapture 直接读日志；语义 pass/fail 只报告不断言。
    // 标识是 (case_id, variant, trial)，不再和一份外部顺序表按位置 zip——那种耦合
    // 在任一侧跳过元素时会静默错位，把 A 的分数印成 B 的。
    for trial_judgement in &judgements {
        println!(
            "{}/{}/t{} {} must={}/{} contradicted={} violations={}",
            trial_judgement.judgement.case_id,
            trial_judgement.variant,
            trial_judgement.trial,
            status_label(&trial_judgement.judgement),
            trial_judgement.judgement.must_supported,
            trial_judgement.judgement.must_total,
            trial_judgement.judgement.contradicted_count,
            trial_judgement.judgement.violations,
        );
    }
    let markdown = render_judge_markdown(&judgements, &trial_records);
    std::fs::write(
        output_dir.join("judge.json"),
        format!("{}\n", serde_json::to_string_pretty(&judgements)?),
    )
    .with_context(|| "写入 judge.json 失败".to_string())?;
    std::fs::write(output_dir.join("judge.md"), &markdown)
        .with_context(|| "写入 judge.md 失败".to_string())?;
    // judge 文本可能转述被测答案。CNB stage 会先扫描并脱敏整个输出目录，再决定是否
    // 打印 judge.md；这里不能通过 --nocapture 绕过那条日志边界。

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
