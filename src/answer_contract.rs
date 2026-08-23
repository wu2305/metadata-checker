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
    /// M58：动作轴。组件的三类属性中，display 与 value 都能用 intent 在组件 target 上
    /// 寻址，唯独「点了之后触发什么、为什么点不动」此前必须同时换动词和换 target 文法
    /// （`--explain action:page|comp|actionId`），而 action id 不可能由用户问句给出。
    Action,
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
            "action" => Ok(Self::Action),
            other => anyhow::bail!(
                "Invalid intent '{}'. Expected: auto | display | value-source | writer | availability | context | action",
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
            Self::Action => "action",
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
    /// 组件上挂的动作及其门禁条件（owner_type = Action）。
    Action,
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
        TraversalIntent::Action => kind == AnswerFactKind::Action,
        // Auto 保持既有行为：action_facts 只在显式 --intent action 下输出，
        // 避免改动所有既有 auto 查询的输出体积。
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

fn primary_fact_path(intent: TraversalIntent, target_node: &Node) -> &'static str {
    match intent {
        TraversalIntent::Display => "display_facts",
        TraversalIntent::ValueSource => "value_source_facts",
        TraversalIntent::Writer => "writer_facts",
        TraversalIntent::Availability => "availability_facts",
        TraversalIntent::Context => "context_facts",
        TraversalIntent::Action => "action_facts",
        TraversalIntent::Auto => match target_node.node_type {
            NodeType::Component => "display_facts|value_source_facts",
            NodeType::Field => "value_source_facts|writer_facts",
            NodeType::Model => "availability_facts|model_io_facts",
            NodeType::Page => "display_facts|availability_facts",
            _ => "context_facts",
        },
    }
}

fn forbidden_fact_paths(intent: TraversalIntent) -> Vec<&'static str> {
    match intent {
        TraversalIntent::Display => vec![
            "value_source_facts",
            "writer_facts",
            "availability_facts",
            "model_io_facts",
        ],
        TraversalIntent::ValueSource => vec![
            "display_facts",
            "writer_facts",
            "availability_facts",
            "model_io_facts",
        ],
        TraversalIntent::Writer => vec![
            "display_facts",
            "value_source_facts",
            "availability_facts",
            "model_io_facts",
        ],
        TraversalIntent::Availability => vec![
            "display_facts",
            "value_source_facts",
            "writer_facts",
            "model_io_facts",
        ],
        TraversalIntent::Action => vec!["value_source_facts", "writer_facts", "model_io_facts"],
        TraversalIntent::Context | TraversalIntent::Auto => vec![],
    }
}

fn completion_status(
    budget: &str,
    primary_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
) -> &'static str {
    if budget == "compact" && rejected_path_count > 0 {
        "partial_due_to_truncation"
    } else if primary_path_count == 0 && candidate_path_count > 0 {
        "needs_followup"
    } else if primary_path_count == 0 {
        "partial_due_to_missing_proven_path"
    } else {
        "complete"
    }
}

fn completion_missing(
    budget: &str,
    primary_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if budget == "compact" && rejected_path_count > 0 {
        missing.push("rejected_paths_expanded_detail");
    }
    if primary_path_count == 0 && candidate_path_count > 0 {
        missing.push("proven_path");
    }
    if primary_path_count == 0 && candidate_path_count == 0 {
        missing.push("graph_relationship_path");
    }
    missing
}

fn completion_next_commands(
    intent: TraversalIntent,
    query_target: &str,
    budget: &str,
    primary_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
) -> Vec<serde_json::Value> {
    let mut commands = Vec::new();
    if budget == "compact" && rejected_path_count > 0 {
        commands.push(serde_json::json!({
            "command": "--explain-condition",
            "target": query_target,
            "budget": "normal",
            "intent": intent.as_str(),
            "reason": "compact 输出存在截断风险，需要 normal 展开 rejected/candidate 明细",
        }));
    }
    if primary_path_count == 0 && candidate_path_count > 0 {
        commands.push(serde_json::json!({
            "command": "--explain-condition",
            "target": query_target,
            "budget": "full",
            "intent": intent.as_str(),
            "reason": "只有 candidate 路径，没有 proven 路径，需要 full 核查候选链路",
        }));
    }
    if primary_path_count == 0 && candidate_path_count == 0 {
        commands.push(serde_json::json!({
            "command": "--context",
            "target": query_target,
            "budget": "normal",
            "reason": "未找到路径时先查看目标相邻关系，确认目标 ID 和边方向",
        }));
    }
    commands
}

fn intent_primary_fact_path(intent: TraversalIntent) -> &'static str {
    match intent {
        TraversalIntent::Display => "display_facts",
        TraversalIntent::ValueSource => "value_source_facts",
        TraversalIntent::Writer => "writer_facts",
        TraversalIntent::Availability => "availability_facts",
        TraversalIntent::Context => "context_facts",
        TraversalIntent::Action => "action_facts",
        TraversalIntent::Auto => "auto_facts",
    }
}

/// 构建 answer_contract，告诉模型当前输出中哪些 fact block 是主证据、哪些是禁止混入的。
///
/// M35.1 统一契约：每次 explain-condition 输出必须在 details.answer_contract 下携带此结构，
/// 避免模型在不同 intent 下混用 display/value-source/availability/writer 证据。
pub fn build_answer_contract(
    intent: TraversalIntent,
    query_target: &str,
    target_node: &Node,
    primary_path_count: usize,
    candidate_path_count: usize,
    rejected_path_count: usize,
    budget: &str,
    is_page_scoped_target: bool,
    resolved_model_target: Option<&str>,
    dataflow_model_id: Option<&str>,
) -> serde_json::Value {
    let primary_fact_path = primary_fact_path(intent, target_node);
    let forbidden_fact_paths = forbidden_fact_paths(intent);
    let status = completion_status(
        budget,
        primary_path_count,
        candidate_path_count,
        rejected_path_count,
    );
    let missing = completion_missing(
        budget,
        primary_path_count,
        candidate_path_count,
        rejected_path_count,
    );
    let next_commands = completion_next_commands(
        intent,
        query_target,
        budget,
        primary_path_count,
        candidate_path_count,
        rejected_path_count,
    );

    serde_json::json!({
        "intent": intent.as_str(),
        "target_scope": {
            "query_target": query_target,
            "node_id": target_node.id,
            "node_type": format!("{:?}", target_node.node_type),
            "source_file": target_node.path,
            "is_page_scoped_target": is_page_scoped_target,
            "resolved_model_target": resolved_model_target,
            "dataflow_model_id": dataflow_model_id,
        },
        "primary_fact_path": primary_fact_path,
        "forbidden_fact_paths": forbidden_fact_paths,
        "completion": {
            "status": status,
            "missing": missing,
            "next_commands": next_commands,
        },
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
        TraversalIntent::Action => "点击/触发这个组件之后会发生什么，什么条件会挡住它",
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
        TraversalIntent::Availability => {
            "用户追问数据可用性，应聚焦 filter、totalRowCount__ 和 DataFlow 输入"
        }
        TraversalIntent::Context => "用户请求周围上下文，应给出相关邻居和旁路信息",
        TraversalIntent::Action => "用户追问交互行为，应聚焦组件挂了哪些动作以及动作自身的门禁条件",
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
        TraversalIntent::Display => {
            "不要把 value_source_facts 或 writer_facts 的结论当作显示条件的证据"
        }
        TraversalIntent::ValueSource => {
            "不要把 display_conditions 当作值来源；candidate_inputs 不是 proven_physical_input"
        }
        TraversalIntent::Writer => {
            "不要把 display_facts 或 availability_facts 的结论混为写入链路证据"
        }
        TraversalIntent::Availability => "不要把 source_filters 标成组件 direct visibleCondition",
        TraversalIntent::Action => {
            "不要把组件自身的 visibleCondition/disableCondition 当作动作门禁；动作门禁的 owner_type 是 Action"
        }
        TraversalIntent::Context | TraversalIntent::Auto => "不要把 related_context 当作必要条件",
    };

    let next_best_action = if value_source_context.is_some() && primary_path_count == 0 {
        "检查 value_source_context 中的 dataflow_field_origin 或 table_source_path"
    } else if primary_path_count == 0 {
        "使用 --budget normal 或 full 重新查询以展开更多路径"
    } else {
        "先读 answer_facts 中的 primary_fact_path，再核查 proven_paths 的每一步"
    };

    let completion_status = if primary_path_count == 0 && candidate_path_count > 0 {
        "needs_followup"
    } else if primary_path_count == 0 {
        "partial_due_to_missing_proven_path"
    } else {
        "complete"
    };
    let required_followups = if primary_path_count == 0 {
        vec![serde_json::json!({
            "reason": what_is_missing,
            "expected_fact_path": primary_fact_path(intent, target_node),
        })]
    } else {
        Vec::new()
    };

    serde_json::json!({
        "question_kind": intent.as_str(),
        "target_scope": {
            "node_id": target_node.id,
            "node_type": format!("{:?}", target_node.node_type),
            "source_file": target_node.path,
        },
        "answer_with": primary_fact_path(intent, target_node),
        "do_not_use_as_primary_evidence": forbidden_fact_paths(intent),
        "required_followups": required_followups,
        "completion_status": completion_status,
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
        format!(
            "--explain-condition {{}} --budget {} --intent {}",
            required_budget_for_complete_answer, "auto"
        )
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
            "expected_fact_path": intent_primary_fact_path(intent),
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
            "expected_fact_path": intent_primary_fact_path(intent),
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
                "expected_fact_path": "value_source_facts",
                "must_run_for_complete_answer": false,
            }));
        }
    }

    if intent == TraversalIntent::Availability
        && dataflow_model_id.is_some()
        && !is_page_scoped_target
    {
        followups.push(serde_json::json!({
            "command": "--explain-condition",
            "target": format!("model:{}|{}", target_id.split('|').next().unwrap_or(target_id), dataflow_model_id.unwrap_or("")),
            "budget": "normal",
            "intent": "availability",
            "reason": "availability 查询应使用 page-scoped model target 以获取 DataFlow filter 和物理输入",
            "expected_fact_path": "availability_facts",
            "must_run_for_complete_answer": true,
        }));
    }

    if intent == TraversalIntent::Auto && proven_path_count == 0 && candidate_path_count == 0 {
        followups.push(serde_json::json!({
            "command": "--context",
            "target": target_id,
            "budget": "normal",
            "reason": "Auto intent 下未找到任何路径，建议先用 --context 探索周围关系",
            "expected_fact_path": "context_facts",
            "must_run_for_complete_answer": false,
        }));
    }

    followups
}

/// target 是否已经带节点前缀。
///
/// `--advise-query` 的 target 既可能是裸 ID（配合 `--advise-query-page`），
/// 也可能是已经完整的节点 ID。再拼一层前缀会产出 `model:comp:app/x.spg|button2`
/// 这种无效 target，模型照着执行必然失败。
fn has_node_prefix(target: &str) -> bool {
    const PREFIXES: [&str; 7] = [
        "comp:",
        "field:",
        "model:",
        "page:",
        "action:",
        "dataflow:",
        "cond:",
    ];
    PREFIXES.iter().any(|prefix| target.starts_with(prefix))
}

/// 按 question-kind 需要的节点类型给裸 target 补前缀；已带前缀的原样返回。
fn scoped_target(prefix: &str, page_scope: Option<&str>, target: &str) -> String {
    if has_node_prefix(target) {
        return target.to_string();
    }
    match page_scope {
        Some(page) => format!("{}{}|{}", prefix, page, target),
        None => format!("{}{}", prefix, target),
    }
}

/// `--advise-query` 支持的 question-kind 全集（含下划线别名）。
pub const ADVISE_QUERY_QUESTION_KINDS: [&str; 7] = [
    "display",
    "value-source",
    "availability",
    "writer",
    "action",
    "page-logic",
    "model-relationships",
];

fn normalize_question_kind(question_kind: &str) -> &str {
    match question_kind {
        "value_source" => "value-source",
        "page_logic" => "page-logic",
        "model_relationships" => "model-relationships",
        other => other,
    }
}

/// M35.10: 构建 --advise-query 的结构化输出
///
/// 根据 target、page_scope、question_kind 和 budget 生成推荐命令和契约信息。
/// 不做自然语言问题分类，只消费结构化 question-kind 参数。
///
/// M58：输出统一走 `AiOutput` 信封。此前是裸 JSON，任何按标准信封转发输出的
/// 消费方（例如 M58 runner 的 `filter_cli_output`）都只会拿到空对象。
pub fn build_advise_query_output(
    target: &str,
    page_scope: Option<&str>,
    question_kind: &str,
    budget: &str,
) -> serde_json::Value {
    let advice = build_advise_query_advice(target, page_scope, question_kind, budget);
    let kind = normalize_question_kind(question_kind);
    let recognized = ADVISE_QUERY_QUESTION_KINDS.contains(&kind);

    let primary_command = advice["primary_command"].as_str().unwrap_or("");
    let primary_target = advice["primary_target"].as_str().unwrap_or("");
    let recommended_budget = advice["recommended_budget"].as_str().unwrap_or("compact");

    let summary = serde_json::json!({
        "question_kind": kind,
        "primary_command": primary_command,
        "primary_target": primary_target,
        "recommended_budget": recommended_budget,
        "what_is_it": format!(
            "question-kind '{}' 应先执行 {} {}",
            kind, primary_command, primary_target
        ),
    });

    let mut out = crate::output::schema::AiOutput::new(
        crate::output::schema::OutputKind::QueryAdvice,
        summary,
    );
    out.query_target = Some(target.to_string());
    out.evidence.push(
        crate::output::schema::Evidence::new(
            format!("{} {}", primary_command, primary_target),
            "路由建议由 question-kind 确定性推导，不依赖自然语言理解",
        )
        .with_node_id(primary_target)
        .with_confidence(crate::output::schema::Confidence::High),
    );
    if !recognized {
        out.diagnostics.push(crate::output::schema::Diagnostic {
            severity: crate::output::schema::DiagnosticSeverity::Warning,
            code: "UNKNOWN_QUESTION_KIND".to_string(),
            message: format!(
                "未识别的 question-kind '{}'，已回退到 auto 建议",
                question_kind
            ),
            location: crate::output::schema::Location::default(),
            suggestion: Some(format!(
                "使用其中之一：{}",
                ADVISE_QUERY_QUESTION_KINDS.join(" | ")
            )),
            count: None,
            answer_impact: None,
            first_seen_phase: None,
        });
    }
    out.next_queries
        .push(crate::output::schema::format_next_query(
            &format!("{} {{}} --budget {}", primary_command, recommended_budget),
            primary_target,
        ));
    for followup in advice["followup_rules"].as_array().into_iter().flatten() {
        let command = followup["command"].as_str().unwrap_or("");
        // target 为 null 的规则（例如动作级 target 要等上一步输出）没有可执行命令，
        // 拼出来只会是 `--explain ''`，宁可不给。
        let followup_target = match followup["target"].as_str() {
            Some(target) if !target.is_empty() => target,
            _ => continue,
        };
        let followup_budget = followup["budget"].as_str().unwrap_or("normal");
        let next = crate::output::schema::format_next_query(
            &format!("{} {{}} --budget {}", command, followup_budget),
            followup_target,
        );
        if !out.next_queries.contains(&next) {
            out.next_queries.push(next);
        }
    }
    out.details = Some(advice);

    serde_json::to_value(out.validate()).unwrap_or(serde_json::Value::Null)
}

/// 路由建议载荷本体；作为 `AiOutput.details` 输出，键名与 M35.10 契约保持一致。
fn build_advise_query_advice(
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
            scoped_target("comp:", page_scope, target),
            "compact",
            "display_facts",
            vec!["value_source_facts", "writer_facts", "availability_facts", "model_io_facts"],
        ),
        "value-source" | "value_source" => (
            "--explain-condition",
            scoped_target("comp:", page_scope, target),
            "compact",
            "value_source_facts",
            vec!["display_facts", "writer_facts", "availability_facts", "model_io_facts"],
        ),
        "availability" => (
            "--explain-condition",
            scoped_target("model:", page_scope, target),
            "normal",
            "availability_facts",
            vec!["display_facts", "value_source_facts", "writer_facts", "model_io_facts"],
        ),
        "writer" => (
            "--explain-condition",
            scoped_target("field:", page_scope, target),
            "compact",
            "writer_facts",
            vec!["display_facts", "value_source_facts", "availability_facts", "model_io_facts"],
        ),
        // M58：动作轴。第一步仍打在组件上——用户问句给不出 action id，
        // 而组件级 action_facts 会把可执行的 `action:` target 交出来。
        "action" => (
            "--explain-condition",
            scoped_target("comp:", page_scope, target),
            "compact",
            "action_facts",
            vec!["value_source_facts", "writer_facts", "model_io_facts"],
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
            scoped_target("comp:", page_scope, target),
            "compact",
            "auto_facts",
            vec![],
        ),
    };

    let kind = normalize_question_kind(question_kind);
    let mut followup_rules = vec![if kind == "availability" {
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
            "intent": kind,
            "reason": "primary recommendation based on question-kind",
            "must_run_for_complete_answer": false,
        })
    }];

    if kind == "action" {
        // 动作级 target 只能由上一步的输出给出：用户问句里没有 action id，
        // 因此这条规则给的是取值路径而不是写死的 target。
        followup_rules.push(serde_json::json!({
            "command": "--explain",
            "target": serde_json::Value::Null,
            "target_from": "details.answer_facts.action_facts.actions[].action_target",
            "budget": "compact",
            "reason": "组件级查询先暴露 action target，再用 --explain 查单个动作做什么",
            "expected_fact_path": "details.triggers",
            "must_run_for_complete_answer": false,
        }));
    }

    serde_json::json!({
        "primary_command": primary_command,
        "primary_target": primary_target,
        "recommended_budget": recommended_budget,
        "requires_graphdb": true,
        "primary_fact_path": primary_fact_path,
        "forbidden_fact_paths": forbidden_paths,
        "followup_rules": followup_rules,
        "note": "--advise-query 只消费结构化 question-kind，不做自然语言理解",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已带前缀的 target 不得被再拼一层：`model:comp:...` 是无效 target，
    /// 模型照着建议执行必然失败。
    #[test]
    fn test_advise_query_does_not_double_prefix_target() {
        for kind in [
            "availability",
            "display",
            "value-source",
            "writer",
            "action",
        ] {
            let out = build_advise_query_output(
                "comp:app/actions_test.spg|button2",
                None,
                kind,
                "compact",
            );
            let target = out["details"]["primary_target"].as_str().unwrap();
            assert_eq!(
                target, "comp:app/actions_test.spg|button2",
                "question-kind {} 不应改写已带前缀的 target",
                kind
            );
        }
    }

    /// 裸 target 仍按 question-kind 补前缀。
    #[test]
    fn test_advise_query_scopes_bare_target() {
        let out = build_advise_query_output(
            "button2",
            Some("app/actions_test.spg"),
            "availability",
            "compact",
        );
        assert_eq!(
            out["details"]["primary_target"].as_str(),
            Some("model:app/actions_test.spg|button2")
        );
    }

    /// advise_query 必须走标准 AiOutput 信封，否则按信封转发的消费方只会拿到空对象。
    #[test]
    fn test_advise_query_emits_ai_output_envelope() {
        let out = build_advise_query_output("model:model1", None, "model-relationships", "compact");
        assert_eq!(out["kind"].as_str(), Some("QueryAdvice"));
        assert!(out["schema_version"].is_string());
        assert_eq!(out["query_target"].as_str(), Some("model:model1"));
        assert_eq!(
            out["summary"]["primary_command"].as_str(),
            Some("--query-model")
        );
        assert!(out["details"]["followup_rules"].is_array());
        assert!(!out["evidence"].as_array().unwrap().is_empty());
        assert!(
            out["next_queries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|q| q.as_str().unwrap_or("").starts_with("--query-model")),
            "next_queries 必须给出可执行命令"
        );
    }

    /// action 轴要给出两步：先组件级拿 action target，再动作级查动作本身。
    #[test]
    fn test_advise_query_action_kind_routes_component_first() {
        let out =
            build_advise_query_output("button2", Some("app/actions_test.spg"), "action", "compact");
        assert_eq!(
            out["details"]["primary_fact_path"].as_str(),
            Some("action_facts")
        );
        assert_eq!(
            out["details"]["primary_target"].as_str(),
            Some("comp:app/actions_test.spg|button2")
        );
        let rules = out["details"]["followup_rules"].as_array().unwrap();
        assert!(
            rules.iter().any(|rule| {
                rule["command"] == "--explain"
                    && rule["target_from"]
                        == "details.answer_facts.action_facts.actions[].action_target"
            }),
            "动作级 target 只能来自上一步输出，必须给取值路径"
        );
    }

    /// 未识别的 question-kind 不能静默回退，否则模型拿到的是错的路由。
    #[test]
    fn test_advise_query_reports_unknown_question_kind() {
        let out = build_advise_query_output("comp:app/x.spg|b1", None, "bogus", "compact");
        let codes: Vec<&str> = out["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["code"].as_str())
            .collect();
        assert!(codes.contains(&"UNKNOWN_QUESTION_KIND"));
    }
}
