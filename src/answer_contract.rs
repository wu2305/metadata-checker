use crate::graph::{Node, NodeType};
use anyhow::Result;

/// M33/M35 目标节点因果遍历意图。
///
/// 该类型是后续 `answer_contract` 与 `thinking_frame` 的协议入口：
/// CLI、stdio 与测试应复用同一套 intent 枚举，避免不同输出路径各自维护字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalIntent {
    Auto,
    Display,
    ValueSource,
    Writer,
    Availability,
    Context,
}

impl TraversalIntent {
    /// 从 CLI / stdio 字符串解析 intent。
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "auto" | "" => Ok(Self::Auto),
            "display" => Ok(Self::Display),
            "value-source" | "value_source" => Ok(Self::ValueSource),
            "writer" => Ok(Self::Writer),
            "availability" => Ok(Self::Availability),
            "context" => Ok(Self::Context),
            other => anyhow::bail!(
                "Invalid intent '{}'. Expected: auto | display | value-source | writer | availability | context",
                other
            ),
        }
    }

    /// 返回稳定输出用 intent 字符串。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Display => "display",
            Self::ValueSource => "value-source",
            Self::Writer => "writer",
            Self::Availability => "availability",
            Self::Context => "context",
        }
    }
}

/// answer_facts 中的事实块类型。
///
/// 该枚举用于决定某个 intent 下哪些 fact block 可以成为主证据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnswerFactKind {
    Display,
    ValueSource,
    Writer,
    Availability,
    Context,
    ModelIo,
}

/// 判断当前 intent 是否需要输出某类事实块。
///
/// `Auto` 模式仍保留旧行为：按目标节点类型选择默认事实块。显式 intent 则只启用
/// 对应事实块，用于降低模型把 display/value/availability/writer 证据混用的风险。
pub(crate) fn answer_fact_enabled(
    intent: TraversalIntent,
    target_node: &Node,
    kind: AnswerFactKind,
) -> bool {
    match intent {
        TraversalIntent::Display => kind == AnswerFactKind::Display,
        TraversalIntent::ValueSource => kind == AnswerFactKind::ValueSource,
        TraversalIntent::Writer => kind == AnswerFactKind::Writer,
        TraversalIntent::Availability => kind == AnswerFactKind::Availability,
        TraversalIntent::Context => kind == AnswerFactKind::Context,
        TraversalIntent::Auto => match target_node.node_type {
            NodeType::Component => {
                matches!(kind, AnswerFactKind::Display | AnswerFactKind::ValueSource)
            }
            NodeType::Field => {
                matches!(kind, AnswerFactKind::ValueSource | AnswerFactKind::Writer)
            }
            NodeType::Model => {
                matches!(kind, AnswerFactKind::Availability | AnswerFactKind::ModelIo)
            }
            NodeType::Page => {
                matches!(kind, AnswerFactKind::Display | AnswerFactKind::Availability)
            }
            _ => kind == AnswerFactKind::Context,
        },
    }
}

/// 构建 answer_contract，告诉模型当前输出中哪些 fact block 是主证据、哪些是禁止混入的。
///
/// M35.1 统一契约：每次 explain-condition 输出必须在 details.answer_contract 下携带此结构，
/// 避免模型在不同 intent 下混用 display/value-source/availability/writer 证据。
pub fn build_answer_contract(
    intent: TraversalIntent,
    target_node: &Node,
    primary_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
    budget: &str,
    is_page_scoped_target: bool,
    resolved_model_target: Option<&str>,
    dataflow_model_id: Option<&str>,
) -> serde_json::Value {
    let primary_fact_path = match intent {
        TraversalIntent::Display => "display_facts",
        TraversalIntent::ValueSource => "value_source_facts",
        TraversalIntent::Writer => "writer_facts",
        TraversalIntent::Availability => "availability_facts",
        TraversalIntent::Context => "context_facts",
        TraversalIntent::Auto => match target_node.node_type {
            NodeType::Component => "display_facts|value_source_facts",
            NodeType::Field => "value_source_facts|writer_facts",
            NodeType::Model => "availability_facts|model_io_facts",
            NodeType::Page => "display_facts|availability_facts",
            _ => "context_facts",
        },
    };

    let forbidden_fact_paths: Vec<&str> = match intent {
        TraversalIntent::Display => vec!["value_source_facts", "writer_facts", "availability_facts", "model_io_facts"],
        TraversalIntent::ValueSource => vec!["display_facts", "writer_facts", "availability_facts", "model_io_facts"],
        TraversalIntent::Writer => vec!["display_facts", "value_source_facts", "availability_facts", "model_io_facts"],
        TraversalIntent::Availability => vec!["display_facts", "value_source_facts", "writer_facts", "model_io_facts"],
        TraversalIntent::Context => vec![],
        TraversalIntent::Auto => vec![],
    };

    serde_json::json!({
        "primary_fact_path": primary_fact_path,
        "forbidden_fact_paths": forbidden_fact_paths,
        "proven_path_count": primary_path_count,
        "candidate_path_count": candidate_path_count,
        "rejected_path_count": rejected_path_count,
        "must_read_summary_first": true,
        "budget_used": budget,
        "is_page_scoped_target": is_page_scoped_target,
        "resolved_model_target": resolved_model_target,
        "dataflow_model_id": dataflow_model_id,
    })
}

/// 构建 thinking_frame，固化 Skill 中的分析思考模式到 CLI/stdio 输出协议。
///
/// M35.2：让模型每次先读结构化 thinking_frame，再按工具给出的 required_followups 和 truncation_guard 行动。
pub fn build_thinking_frame(
    intent: TraversalIntent,
    target_node: &Node,
    primary_path_count: usize,
    candidate_path_count: usize,
    value_source_context: &Option<serde_json::Value>,
) -> serde_json::Value {
    let primary_question_type = match intent {
        TraversalIntent::Display => "为什么这个组件显示/隐藏",
        TraversalIntent::ValueSource => "这个组件/字段的值从哪里来",
        TraversalIntent::Writer => "谁写入或生成了这个目标",
        TraversalIntent::Availability => "这个模型在什么条件下有数据",
        TraversalIntent::Context => "这个目标周围有哪些相关上下文",
        TraversalIntent::Auto => "自动判断目标的核心问题类型",
    };

    let why_this_intent = match intent {
        TraversalIntent::Display => {
            if target_node.node_type == NodeType::Component {
                    "目标是一个组件，优先解释它的显示/启用条件"
                } else {
                    "目标是字段或模型，但用户问的是 display 条件"
                }
        }
        TraversalIntent::ValueSource => "用户明确追问值来源，应优先追踪 Reads/FieldAlias 链路",
        TraversalIntent::Writer => "用户明确追问写入链路，应优先追踪 Writes/ActionWrites 链路",
        TraversalIntent::Availability => "用户追问数据可用性，应聚焦 filter、totalRowCount__ 和 DataFlow 输入",
        TraversalIntent::Context => "用户请求周围上下文，应给出相关邻居和旁路信息",
        TraversalIntent::Auto => "未指定 intent，按目标节点类型自动选择默认事实块",
    };

    let what_is_missing = if primary_path_count == 0 && candidate_path_count == 0 {
        "未找到任何 proven 或 candidate 路径；可能需要扩展 budget 或检查目标 ID 是否正确"
    } else if primary_path_count == 0 {
        "没有 proven 路径，只有 candidate；需要人工核查 candidate_inputs 或补充字段级来源信息"
    } else if candidate_path_count > 0 {
        "有 proven 路径，但存在 candidate 路径未完全确认；建议核查 candidate_inputs"
    } else {
        "proven 路径已找到，但 compact 模式下可能隐藏了 supporting_context 和 related_context 明细"
    };

    let what_to_avoid = match intent {
        TraversalIntent::Display => "不要把 value_source_facts 或 writer_facts 的结论当作显示条件的证据",
        TraversalIntent::ValueSource => "不要把 display_conditions 当作值来源；candidate_inputs 不是 proven_physical_input",
        TraversalIntent::Writer => "不要把 display_facts 或 availability_facts 的结论混为写入链路证据",
        TraversalIntent::Availability => "不要把 source_filters 标成组件 direct visibleCondition",
        TraversalIntent::Context | TraversalIntent::Auto => "不要把 related_context 当作必要条件",
    };

    let next_best_action = if value_source_context.is_some() && primary_path_count == 0 {
        "检查 value_source_context 中的 dataflow_field_origin 或 table_source_path"
    } else if primary_path_count == 0 {
        "使用 --budget normal 或 full 重新查询以展开更多路径"
    } else {
        "先读 answer_facts 中的 primary_fact_path，再核查 proven_paths 的每一步"
    };

    serde_json::json!({
        "primary_question_type": primary_question_type,
        "why_this_intent": why_this_intent,
        "what_is_missing": what_is_missing,
        "what_to_avoid": what_to_avoid,
        "next_best_action": next_best_action,
    })
}

/// 构建 truncation_guard，标记 compact 模式下是否存在截断风险。
///
/// M35.9：当任一 readers/writers/related/dataflow 数组超过 compact limit，
/// 或同一模型跨页面读写数量超过普通样本展示能力时，标记 safe_to_answer_full_relationships=false。
pub fn build_truncation_guard(
    budget: &str,
    proven_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
    supporting_context_count: usize,
    related_context_count: usize,
) -> serde_json::Value {
    let compact_limits = serde_json::json!({
        "max_paths": 3,
        "max_steps_per_path": 6,
        "max_supporting_context": 5,
        "max_related_context": 5,
        "max_blocking_conditions": 10,
        "max_data_empty_gates": 10,
    });

    let mut truncated_sections: Vec<String> = Vec::new();
    let is_compact = budget == "compact";

    if is_compact {
        if proven_path_count > 3 {
            truncated_sections.push("primary_path".to_string());
        }
        if candidate_path_count > 3 {
            truncated_sections.push("candidate_paths".to_string());
        }
        if rejected_path_count > 0 {
            truncated_sections.push("rejected_paths".to_string());
        }
        if supporting_context_count > 5 {
            truncated_sections.push("supporting_context".to_string());
        }
        if related_context_count > 5 {
            truncated_sections.push("related_context".to_string());
        }
    }

    let is_complete = truncated_sections.is_empty();
    let safe_to_answer_full_relationships = is_complete || (!is_compact);

    let required_budget_for_complete_answer = if safe_to_answer_full_relationships {
        budget
    } else {
        "normal"
    };

    let recommended_rerun = if !safe_to_answer_full_relationships {
        format!("--explain-condition {{}} --budget {} --intent {}", required_budget_for_complete_answer, "auto")
    } else {
        String::new()
    };

    serde_json::json!({
        "is_complete": is_complete,
        "safe_to_answer_full_relationships": safe_to_answer_full_relationships,
        "required_budget_for_complete_answer": required_budget_for_complete_answer,
        "truncated_sections": truncated_sections,
        "recommended_rerun": recommended_rerun,
        "compact_limits": compact_limits,
    })
}

/// 构建 required_followups，列出当前输出不足以完整回答时必须继续执行的查询。
///
/// M35.3：当 proven_path_count=0、candidate_path_count>0、或 truncation_guard 标记不完整时，
/// 生成 must_run_for_complete_answer=true 的 followup 命令。
pub fn build_required_followups(
    intent: TraversalIntent,
    target_id: &str,
    budget: &str,
    proven_path_count: usize,
    candidate_path_count: usize,
    value_source_context: &Option<serde_json::Value>,
    is_page_scoped_target: bool,
    dataflow_model_id: Option<&str>,
    safe_to_answer: bool,
) -> Vec<serde_json::Value> {
    let mut followups: Vec<serde_json::Value> = Vec::new();

    if !safe_to_answer && budget == "compact" {
        followups.push(serde_json::json!({
            "command": "--explain-condition",
            "target": target_id,
            "budget": "normal",
            "intent": intent.as_str(),
            "reason": "truncation_guard 标记 compact 输出不完整，需要升级到 normal 展开完整路径",
            "must_run_for_complete_answer": true,
        }));
    }

    if proven_path_count == 0 && candidate_path_count > 0 {
        followups.push(serde_json::json!({
            "command": "--explain-condition",
            "target": target_id,
            "budget": "full",
            "intent": intent.as_str(),
            "reason": "只有 candidate 路径没有 proven 路径，需要 full budget 核查所有候选",
            "must_run_for_complete_answer": true,
        }));
    }

    if value_source_context.is_some() {
        let has_proven = value_source_context
            .as_ref()
            .and_then(|v| v.get("proven_physical_input"))
            .is_some();
        if !has_proven {
            followups.push(serde_json::json!({
                "command": "--explain-condition",
                "target": target_id,
                "budget": "normal",
                "intent": "value-source",
                "reason": "value_source_context 存在但没有 proven_physical_input，需要专门核查 value-source",
                "must_run_for_complete_answer": false,
            }));
        }
    }

    if intent == TraversalIntent::Availability && dataflow_model_id.is_some() && !is_page_scoped_target {
        followups.push(serde_json::json!({
            "command": "--explain-condition",
            "target": format!("model:{}|{}", target_id.split('|').next().unwrap_or(target_id), dataflow_model_id.unwrap_or("")),
            "budget": "normal",
            "intent": "availability",
            "reason": "availability 查询应使用 page-scoped model target 以获取 DataFlow filter 和物理输入",
            "must_run_for_complete_answer": true,
        }));
    }

    if intent == TraversalIntent::Auto && proven_path_count == 0 && candidate_path_count == 0 {
        followups.push(serde_json::json!({
            "command": "--context",
            "target": target_id,
            "budget": "normal",
            "reason": "Auto intent 下未找到任何路径，建议先用 --context 探索周围关系",
            "must_run_for_complete_answer": false,
        }));
    }

    followups
}

/// M35.10: 构建 --advise-query 的结构化输出
///
/// 根据 target、page_scope、question_kind 和 budget 生成推荐命令和契约信息。
/// 不做自然语言问题分类，只消费结构化 question-kind 参数。
pub fn build_advise_query_output(
    target: &str,
    page_scope: Option<&str>,
    question_kind: &str,
    _budget: &str,
) -> serde_json::Value {
    let (primary_command, primary_target, recommended_budget, primary_fact_path, forbidden_paths): (
        &str,
        String,
        &str,
        &str,
        Vec<&str>,
    ) = match question_kind {
        "display" => (
            "--explain-condition",
            if let Some(page) = page_scope {
                format!("comp:{}|{}", page, target)
            } else {
                target.to_string()
            },
            "compact",
            "display_facts",
            vec!["value_source_facts", "writer_facts", "availability_facts", "model_io_facts"],
        ),
        "value-source" | "value_source" => (
            "--explain-condition",
            if let Some(page) = page_scope {
                format!("comp:{}|{}", page, target)
            } else {
                target.to_string()
            },
            "compact",
            "value_source_facts",
            vec!["display_facts", "writer_facts", "availability_facts", "model_io_facts"],
        ),
        "availability" => (
            "--explain-condition",
            if let Some(page) = page_scope {
                format!("model:{}|{}", page, target)
            } else {
                format!("model:{}", target)
            },
            "normal",
            "availability_facts",
            vec!["display_facts", "value_source_facts", "writer_facts", "model_io_facts"],
        ),
        "writer" => (
            "--explain-condition",
            if let Some(page) = page_scope {
                format!("field:{}|{}", page, target)
            } else {
                target.to_string()
            },
            "compact",
            "writer_facts",
            vec!["display_facts", "value_source_facts", "availability_facts", "model_io_facts"],
        ),
        "page-logic" | "page_logic" => (
            "--query-page-logic",
            if let Some(page) = page_scope {
                page.to_string()
            } else {
                target.to_string()
            },
            "compact",
            "page_logic_summary",
            vec!["display_facts", "value_source_facts", "writer_facts", "model_io_facts"],
        ),
        "model-relationships" | "model_relationships" => (
            "--query-model",
            target.to_string(),
            "normal",
            "model_io_facts",
            vec!["display_facts", "value_source_facts", "writer_facts", "availability_facts"],
        ),
        _ => (
            "--explain-condition",
            if let Some(page) = page_scope {
                format!("comp:{}|{}", page, target)
            } else {
                target.to_string()
            },
            "compact",
            "auto_facts",
            vec![],
        ),
    };

    let followup = if question_kind == "availability" {
        serde_json::json!({
            "command": primary_command,
            "target": primary_target,
            "budget": "full",
            "intent": "availability",
            "reason": "availability 需要展开 DataFlow filter 和物理输入明细",
            "must_run_for_complete_answer": true,
        })
    } else {
        serde_json::json!({
            "command": primary_command,
            "target": primary_target,
            "budget": recommended_budget,
            "intent": question_kind,
            "reason": "primary recommendation based on question-kind",
            "must_run_for_complete_answer": false,
        })
    };

    serde_json::json!({
        "primary_command": primary_command,
        "primary_target": primary_target,
        "recommended_budget": recommended_budget,
        "requires_graphdb": true,
        "primary_fact_path": primary_fact_path,
        "forbidden_fact_paths": forbidden_paths,
        "followup_rules": [followup],
        "note": "--advise-query 只消费结构化 question-kind，不做自然语言理解",
    })
}

