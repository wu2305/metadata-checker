//! M58 CI tester 的评测 runner 支持代码。

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::Path;

/// M58 评测分层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalTier {
    /// 只校验 CLI 结构化输出，不调用 LLM。
    FixtureStructural,
    /// 使用 fixture 项目调用真实或 fake LLM。
    FixtureLlm,
    /// 仅允许人工或 Agent 执行的真实项目 case。
    RealManual,
}

impl EvalTier {
    /// 从 JSON 字符串解析评测分层。
    fn parse(value: &str, case_id: &str) -> Result<Self> {
        match value {
            "fixture_structural" => Ok(Self::FixtureStructural),
            "fixture_llm" => Ok(Self::FixtureLlm),
            "real_manual" => Ok(Self::RealManual),
            _ => bail!("case {case_id} 的 tier '{value}' 无效"),
        }
    }
}

/// 已加载的评测 case；原始 JSON 保留给判分器读取既有断言。
#[derive(Debug, Clone)]
pub(crate) struct EvalCase {
    /// case 稳定标识。
    pub(crate) case_id: String,
    /// 交给模型的问题。
    pub(crate) question: String,
    /// 评测分层。
    pub(crate) tier: EvalTier,
    /// case 生命周期状态。
    pub(crate) case_status: String,
    /// case 难度。
    pub(crate) difficulty: String,
    /// 原始 case JSON。
    pub(crate) value: Value,
}

/// 加载并校验 M58 评测 case。
pub(crate) fn load_eval_cases(path: &Path) -> Result<Vec<EvalCase>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取评测 case 失败: {}", path.display()))?;
    let root: Value = serde_json::from_str(&content)
        .with_context(|| format!("解析评测 case JSON 失败: {}", path.display()))?;
    let cases = root
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("评测 JSON 缺少 cases 数组"))?;

    cases
        .iter()
        .map(|value| {
            let case_id = value
                .get("case_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("评测 case 缺少 case_id"))?;
            let question = value
                .get("question")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("case {case_id} 缺少 question"))?;
            let tier = match value.get("tier").and_then(Value::as_str) {
                Some(raw) => EvalTier::parse(raw, case_id)?,
                None => {
                    if value
                        .get("requires_real_project")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        EvalTier::RealManual
                    } else {
                        EvalTier::FixtureStructural
                    }
                }
            };

            Ok(EvalCase {
                case_id: case_id.to_string(),
                question: question.to_string(),
                tier,
                case_status: value
                    .get("case_status")
                    .and_then(Value::as_str)
                    .unwrap_or("active")
                    .to_string(),
                difficulty: value
                    .get("difficulty")
                    .and_then(Value::as_str)
                    .unwrap_or("basic")
                    .to_string(),
                value: value.clone(),
            })
        })
        .collect()
}

/// 选择 active 的 fixture LLM case。
pub(crate) fn fixture_llm_cases(cases: &[EvalCase]) -> Vec<EvalCase> {
    cases
        .iter()
        .filter(|case| {
            case.tier == EvalTier::FixtureLlm && case.case_status == "active"
        })
        .cloned()
        .collect()
}
