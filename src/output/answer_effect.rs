//! 诊断对「答案」的影响。
//!
//! 诊断此前只面向工具维护者：`UNKNOWN_ACTION_TYPE` 的 suggestion 是
//! "Check if this action type is supported by metadata-checker"——这是说给我听的，
//! 不是说给消费输出的模型听的。模型拿到一条它看不懂后果的诊断，只能选择忽略，
//! 于是「遇到诊断要保守表达」这条要求永远落不了地。
//!
//! 这里把 code 映射成一句「它对结论意味着什么」。映射只按 code 走，不认得任何
//! 具体问题或用例，构造诊断的 100 多处调用点一个都不用改。

/// 一条诊断对结论的影响类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerImpact {
    /// 语义没解析出来，涉及它的结论不确定。
    Uncertain,
    /// 输出被截断/抽样，结论只覆盖了一部分数据。
    Partial,
    /// 「没有」本身是一个确定的判定，不是数据缺失。
    Determinate,
    /// 寻址问题：影响的是这条命令能不能答，而不是答案的置信度。
    Addressing,
    /// 路由过程的说明：命令是怎么被送到节点上的，与答案内容无关。
    Routing,
}

impl AnswerImpact {
    fn as_str(self) -> &'static str {
        match self {
            Self::Uncertain => "uncertain",
            Self::Partial => "partial",
            Self::Determinate => "determinate",
            Self::Addressing => "addressing",
            Self::Routing => "routing",
        }
    }
}

/// 诊断 code 对结论的影响。未登记的 code 返回 None——宁可不说，也不要瞎说一句
/// 「这可能影响结论」让模型对所有答案都降级。
pub fn answer_effect(code: &str) -> Option<(AnswerImpact, &'static str)> {
    use AnswerImpact::{Addressing, Determinate, Partial, Routing, Uncertain};
    let entry = match code {
        // ---- 语义不确定：结论要保守回答 ----
        "UNKNOWN_ACTION_TYPE" => (
            Uncertain,
            "存在未识别的 action 类型，这些 action 的语义不确定（action_category=unknown）；涉及它们的结论应保守回答，不要给出确定性判断。",
        ),
        "UNKNOWN_QUESTION_KIND" => (
            Uncertain,
            "问题类型未识别，返回的是通用事实；结论应保守回答。",
        ),
        "EXPR_PARSE_ERROR" | "EXPR_UNPARSED" | "LINEAGE_EXPR_UNPARSED" => (
            Uncertain,
            "有表达式没能解析，其依赖和取值语义不确定；涉及该表达式的结论应保守回答。",
        ),
        "EXPR_UNRESOLVED_REF" => (
            Uncertain,
            "表达式里有未解析的引用，实际依赖可能多于列出的部分；结论应保守回答。",
        ),
        "EXPR_UNSUPPORTED_FUNCTION" => (
            Uncertain,
            "表达式用到了未支持的函数，其求值语义不确定；结论应保守回答。",
        ),
        "LINEAGE_SOURCE_MISSING" => (
            Uncertain,
            "字段血缘的上游没有全部解析出来，来源可能不完整；不要断言这就是全部来源。",
        ),
        "UNRESOLVED_MODEL_WRITE" => (
            Uncertain,
            "有写入操作的目标模型没解析出来，写入目标清单可能不完整。",
        ),
        "UNRESOLVED_PAGE_NAVIGATION" => (
            Uncertain,
            "有跳转的目标页面没解析出来，跳转清单可能不完整。",
        ),
        "MODEL_UNRESOLVED" => (
            Uncertain,
            "引用的模型没解析出来，涉及该模型的结论应保守回答。",
        ),
        "VISIBILITY_RULE_UNRESOLVED" => (
            Uncertain,
            "可见性规则没解析完整，显示条件的结论应保守回答。",
        ),
        "ACTION_FLOW_INCOMPLETE" => (
            Uncertain,
            "动作链路没有解析完整，后续影响可能多于列出的部分。",
        ),
        "DATAFLOW_INPUT_PATH_UNRESOLVED" => {
            (Uncertain, "DataFlow 的输入路径没解析出来，数据来源不确定。")
        }
        "CYCLE_DEPENDENCY" => (
            Uncertain,
            "依赖里存在环，遍历在环处停止；链路结论应保守回答。",
        ),
        "EMPTY_CONDITION" => (Uncertain, "存在空的条件表达式，无法判断它实际是否生效。"),
        "PARSE_ERROR" | "METADATA_PARSE_ERROR" => (
            Uncertain,
            "有元数据没解析成功，本次结论建立在不完整的数据上，应保守回答。",
        ),
        "EVIDENCE_INCOMPLETE" | "EVIDENCE_LOCATION_MISSING" | "EDGE_EVIDENCE_UNAVAILABLE" => (
            Uncertain,
            "部分结论缺少可核对的证据位置，引用时要说明证据不完整。",
        ),
        "AMBIGUOUS_RESOLUTION" => (
            Uncertain,
            "名字匹配到多个节点，本次结果未必是你要的那个；结论应保守回答。",
        ),

        // ---- 部分数据：结论只覆盖了展示出来的部分 ----
        "OUTPUT_TRUNCATED" | "PRIMARY_PATHS_TRUNCATED" => (
            Partial,
            "输出被截断，列出的不是全集；不要根据它断言「只有这些」，需要全集就提高 --budget。",
        ),
        "EVIDENCE_SAMPLED" => (
            Partial,
            "证据是抽样的，不是全集；可以引用，但不要据此断言数量。",
        ),
        "PAGE_INPUTS_DEFERRED" => (
            Partial,
            "页面输入没有在本次展开，相关结论只覆盖了已展开的部分。",
        ),

        // ---- 「没有」是确定判定，不是数据缺失 ----
        "NO_WRITE_TARGETS" => (
            Determinate,
            "已确认该页面没有写入目标：页面不写数据，用户无法通过它修改数据。",
        ),
        "NO_ENTRYPOINTS" => (
            Determinate,
            "已确认该页面没有用户可触发入口：没有任何交互入口，用户无法在此页面发起操作。",
        ),
        "DATAFLOW_NO_INPUTS" => (Determinate, "已确认该 DataFlow 没有输入。"),
        "DATAFLOW_NO_OUTPUT" => (Determinate, "已确认该 DataFlow 没有输出。"),
        "NO_MATCHES_FOUND" => (
            Determinate,
            "已确认没有匹配项；换个关键词再定位，不要凭空断言目标存在。",
        ),

        // ---- 寻址：影响的是这条命令，不是答案置信度 ----
        "TARGET_NOT_FOUND" | "COMPONENT_NOT_FOUND" | "DOCUMENT_NOT_FOUND" => (
            Addressing,
            "target 没定位到；改用 candidate_targets 里的写法重试，不要重复同一条命令。",
        ),
        "AMBIGUOUS_TARGET" => (
            Addressing,
            "target 匹配到多个节点；从 candidate_targets 里挑一个完整写法重试，不要重复同一条命令。",
        ),
        // ---- 路由说明：与答案内容无关 ----
        "RESOLVED_TARGET" => (
            Routing,
            "你写的 target 已按图中真实节点补全，命令查的就是你要的节点；这不影响结论的确定性。",
        ),
        "SUPPLEMENT_UNAVAILABLE" => (
            Partial,
            "有一个补充事实块没取到，本次结论只覆盖已返回的部分。",
        ),
        _ => return None,
    };
    Some(entry)
}

/// 一组诊断对整体答案的置信度结论。
///
/// `level` 只由「不确定」和「部分数据」两类决定：确定性的「没有」和寻址问题都不该
/// 让模型给一个本来能确定的答案降级。
pub struct AnswerConfidence {
    pub level: &'static str,
    pub statement: String,
    pub reasons: Vec<(String, &'static str, &'static str)>,
}

/// 从一批诊断 code 归纳置信度。传入顺序即输出顺序，重复 code 只保留第一条。
pub fn summarize_confidence<'a>(codes: impl IntoIterator<Item = &'a str>) -> AnswerConfidence {
    let mut reasons: Vec<(String, &'static str, &'static str)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let (mut uncertain, mut partial) = (false, false);
    for code in codes {
        let Some((impact, effect)) = answer_effect(code) else {
            continue;
        };
        match impact {
            AnswerImpact::Uncertain => uncertain = true,
            AnswerImpact::Partial => partial = true,
            _ => {}
        }
        if seen.insert(code.to_string()) {
            reasons.push((code.to_string(), impact.as_str(), effect));
        }
    }
    let (level, statement) = match (uncertain, partial) {
        (true, _) => (
            "reduced",
            "本次输出存在语义不确定的诊断，涉及它们的结论应保守回答，不要给出确定性判断。",
        ),
        (false, true) => (
            "partial",
            "本次输出是部分数据，结论只覆盖已展示的部分，不要断言「只有这些」。",
        ),
        (false, false) => ("full", "本次输出没有降低置信度的诊断，可以按事实直接作答。"),
    };
    AnswerConfidence {
        level,
        statement: statement.to_string(),
        reasons,
    }
}

/// 置信度块的 JSON 形态。
pub fn confidence_value<'a>(codes: impl IntoIterator<Item = &'a str>) -> serde_json::Value {
    let confidence = summarize_confidence(codes);
    serde_json::json!({
        "level": confidence.level,
        "statement": confidence.statement,
        "reasons": confidence
            .reasons
            .iter()
            .map(|(code, impact, effect)| serde_json::json!({
                "code": code,
                "impact": impact,
                "effect": effect,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_codes_stay_silent() {
        assert!(answer_effect("SOME_FUTURE_CODE").is_none());
        let confidence = summarize_confidence(["SOME_FUTURE_CODE"]);
        assert_eq!(confidence.level, "full");
        assert!(confidence.reasons.is_empty());
    }

    #[test]
    fn semantic_uncertainty_outranks_truncation() {
        let confidence = summarize_confidence(["OUTPUT_TRUNCATED", "UNKNOWN_ACTION_TYPE"]);
        assert_eq!(confidence.level, "reduced");
        assert_eq!(confidence.reasons.len(), 2);
    }

    #[test]
    fn a_confirmed_absence_does_not_lower_confidence() {
        // 「没有写入目标」是判定结果，不是数据缺失：正因为如此才能回答「只读」。
        let confidence = summarize_confidence(["NO_WRITE_TARGETS", "NO_ENTRYPOINTS"]);
        assert_eq!(confidence.level, "full");
        assert_eq!(confidence.reasons.len(), 2);
    }

    #[test]
    fn repeated_codes_are_reported_once() {
        let confidence = summarize_confidence(["UNKNOWN_ACTION_TYPE", "UNKNOWN_ACTION_TYPE"]);
        assert_eq!(confidence.reasons.len(), 1);
    }
}
