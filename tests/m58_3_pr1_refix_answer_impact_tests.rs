//! M58.3 PR1 refix：`answer_impact_for` 兜底分支从 `answer_effect` 派生，
//! 消除序列化 JSON 中 `answer_impact: "none"` 与置信度 `level: "partial"` 的矛盾。

use metadata_checker::diagnostics::{
    CODE_GRAPH_DB_PARTIAL_HYDRATE, CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE, answer_impact_for,
    envelope_diagnostic,
};
use metadata_checker::output::Location;
use metadata_checker::output::answer_effect::confidence_value;

/// `OUTPUT_TRUNCATED` 未在 `answer_impact_for` 显式列出，但 `answer_effect` 将其分类为 Partial，
/// 兜底派生后 `answer_impact` 应为 `partial`。
#[test]
fn output_truncated_fallback_derives_partial() {
    let diag = envelope_diagnostic(
        "OUTPUT_TRUNCATED",
        1,
        Location::default(),
        "output truncated",
    );
    assert_eq!(diag.answer_impact.as_deref(), Some("partial"));
}

/// 序列化后的 JSON 中 `answer_impact` 与置信度 `answer_effect.level` 语义一致：
/// 同一 code 不应出现 `answer_impact: "none"` 而 level 为 `partial` 的矛盾。
#[test]
fn serialized_json_answer_impact_matches_confidence_level() {
    let diag = envelope_diagnostic(
        "OUTPUT_TRUNCATED",
        1,
        Location::default(),
        "output truncated",
    );
    let value = serde_json::to_value(&diag).expect("信封诊断序列化不应失败");
    assert_eq!(value["answer_impact"], serde_json::json!("partial"));
    // answer_effect 字段为影响说明文本，存在即代表该 code 已登记
    assert!(value.get("answer_effect").is_some());

    let confidence = confidence_value(["OUTPUT_TRUNCATED"]);
    assert_eq!(confidence["level"], serde_json::json!("partial"));
    assert_eq!(
        value["answer_impact"].as_str(),
        Some(confidence["level"].as_str().expect("level 应为字符串"))
    );
}

/// 显式列出的 code 以 `answer_impact_for` 表为准，不受兜底派生影响：
/// `GRAPH_DB_V2_LAYOUT_UNREADABLE` 显式为 none（answer_effect 未登记该 code，派生也会给 none，
/// 但此处验证显式映射本身不被改动）。
#[test]
fn explicit_none_code_stays_none() {
    assert_eq!(
        answer_impact_for(CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE),
        "none"
    );
    let diag = envelope_diagnostic(
        CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE,
        1,
        Location::default(),
        "v2 layout unreadable",
    );
    assert_eq!(diag.answer_impact.as_deref(), Some("none"));
}

/// 显式列出的 partial code 保持 partial（与 answer_effect 的 Partial 分类一致）。
#[test]
fn explicit_partial_code_stays_partial() {
    assert_eq!(answer_impact_for(CODE_GRAPH_DB_PARTIAL_HYDRATE), "partial");
    let diag = envelope_diagnostic(
        CODE_GRAPH_DB_PARTIAL_HYDRATE,
        3,
        Location::default(),
        "partial hydrate",
    );
    assert_eq!(diag.answer_impact.as_deref(), Some("partial"));
}

/// 完全未知、answer_effect 也未登记的 code 兜底为 none（宁可不说，不瞎降级）。
#[test]
fn unknown_code_falls_back_to_none() {
    assert_eq!(answer_impact_for("SOME_FUTURE_CODE"), "none");
    let diag = envelope_diagnostic("SOME_FUTURE_CODE", 1, Location::default(), "future");
    assert_eq!(diag.answer_impact.as_deref(), Some("none"));
}
