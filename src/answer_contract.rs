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
