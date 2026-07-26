#![cfg(feature = "cli-local")]

#[path = "support/m58_ai_eval.rs"]
mod m58_ai_eval;

use m58_ai_eval::{fixture_llm_cases, load_eval_cases};
use std::path::Path;

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
