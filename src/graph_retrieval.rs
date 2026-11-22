//! 轻量图召回实验模块
//!
//! M58.4：在现有确定性图上验证“同一批种子 + 按查询意图赋权的 PPR”。
//! 本模块只生成候选排序，不产生新事实，也不替代既有 path/evidence 证明链。

use crate::answer_contract::TraversalIntent;
use crate::graph::EdgeType;
use crate::graph_store::GraphReadStore;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// cli-local runtime 读取的实验图召回开关。
pub const EXPERIMENTAL_GRAPH_RETRIEVAL_ENV: &str = "METADATA_CHECKER_EXPERIMENTAL_GRAPH_RETRIEVAL";

/// 单个图召回种子。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphSeed {
    /// 图节点 ID。
    pub node_id: String,
    /// 外部检索器给出的非负相关度。
    pub score: f64,
}

/// typed PPR 的运行参数。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GraphRetrievalConfig {
    /// 每轮回到种子分布的概率。
    pub restart_probability: f64,
    /// L1 差值收敛阈值。
    pub tolerance: f64,
    /// 最大迭代轮数。
    pub max_iterations: usize,
    /// 最多返回的候选数。
    pub top_k: usize,
}

impl Default for GraphRetrievalConfig {
    fn default() -> Self {
        Self {
            restart_probability: 0.2,
            tolerance: 1e-10,
            max_iterations: 200,
            top_k: 10,
        }
    }
}

/// 单个 PPR 排名节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedGraphNode {
    /// 图节点 ID。
    pub node_id: String,
    /// PPR 稳态分数。
    pub score: f64,
    /// 归并前的原始种子分数；非种子为零。
    pub seed_score: f64,
}

/// typed PPR 的完整结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRetrievalResult {
    /// 按分数降序、节点 ID 升序排列的候选。
    pub ranked_nodes: Vec<RankedGraphNode>,
    /// 实际执行的迭代轮数。
    pub iterations: usize,
    /// 是否在最大轮数内达到阈值。
    pub converged: bool,
    /// 图中存在且分数为正的去重种子数。
    pub effective_seed_count: usize,
    /// 截断到 top-k 前的正分候选数。
    pub candidate_pool_count: usize,
}

/// ExplainCondition 的图召回策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphRetrievalStrategy {
    /// 保持既有查询与输出契约。
    Baseline,
    /// 追加按查询意图赋权的 PPR 候选。
    TypedPpr,
}

impl GraphRetrievalStrategy {
    /// 解析 runtime 实验开关；未设置时保持 baseline。
    pub fn parse(value: Option<&str>) -> Result<Self> {
        match value {
            None | Some("baseline") => Ok(Self::Baseline),
            Some("typed-ppr") => Ok(Self::TypedPpr),
            Some(other) => anyhow::bail!(
                "{} 只接受 baseline 或 typed-ppr，实际为 '{}'",
                EXPERIMENTAL_GRAPH_RETRIEVAL_ENV,
                other
            ),
        }
    }
}

/// 在现有类型图上执行按查询意图赋权的 Personalized PageRank。
///
/// 分数只用于候选排序。调用方仍须通过既有路径和证据契约证明最终结论。
pub fn typed_personalized_page_rank(
    graph: &dyn GraphReadStore,
    intent: TraversalIntent,
    seeds: &[GraphSeed],
    config: GraphRetrievalConfig,
) -> Result<GraphRetrievalResult> {
    validate_config(config)?;
    ensure!(!seeds.is_empty(), "图召回至少需要一个种子");

    let mut nodes = graph
        .iter_nodes()
        .context("读取图节点失败")?
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));

    let node_indexes = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect::<HashMap<_, _>>();

    let mut raw_seed_scores = vec![0.0; nodes.len()];
    for seed in seeds {
        ensure!(
            seed.score.is_finite() && seed.score >= 0.0,
            "图召回种子 '{}' 的分数必须是有限非负数",
            seed.node_id
        );
        if seed.score == 0.0 {
            continue;
        }
        if let Some(index) = node_indexes.get(&seed.node_id) {
            raw_seed_scores[*index] += seed.score;
        }
    }

    let seed_total = raw_seed_scores.iter().sum::<f64>();
    ensure!(
        seed_total > 0.0,
        "图召回没有可用种子：正分种子均不存在于当前图"
    );

    let effective_seed_count = raw_seed_scores.iter().filter(|score| **score > 0.0).count();
    let seed_distribution = raw_seed_scores
        .iter()
        .map(|score| score / seed_total)
        .collect::<Vec<_>>();
    let transitions = build_transitions(graph, intent, &nodes, &node_indexes)?;

    let mut rank = seed_distribution.clone();
    let mut iterations = 0;
    let mut converged = false;
    let follow_probability = 1.0 - config.restart_probability;

    for iteration in 1..=config.max_iterations {
        let mut next = seed_distribution
            .iter()
            .map(|score| config.restart_probability * score)
            .collect::<Vec<_>>();
        let mut dangling_mass = 0.0;

        for (source_index, outgoing) in transitions.iter().enumerate() {
            if outgoing.is_empty() {
                dangling_mass += rank[source_index];
                continue;
            }
            for &(target_index, probability) in outgoing {
                next[target_index] += follow_probability * rank[source_index] * probability;
            }
        }

        if dangling_mass > 0.0 {
            for (target_index, seed_probability) in seed_distribution.iter().enumerate() {
                next[target_index] += follow_probability * dangling_mass * seed_probability;
            }
        }

        let difference = rank
            .iter()
            .zip(next.iter())
            .map(|(before, after)| (before - after).abs())
            .sum::<f64>();
        rank = next;
        iterations = iteration;
        if difference <= config.tolerance {
            converged = true;
            break;
        }
    }

    let mut ranked_nodes = nodes
        .iter()
        .enumerate()
        .filter(|(index, _)| rank[*index] > f64::EPSILON)
        .map(|(index, node)| RankedGraphNode {
            node_id: node.id.clone(),
            score: rank[index],
            seed_score: raw_seed_scores[index],
        })
        .collect::<Vec<_>>();
    ranked_nodes.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    let candidate_pool_count = ranked_nodes.len();
    ranked_nodes.truncate(config.top_k);

    Ok(GraphRetrievalResult {
        ranked_nodes,
        iterations,
        converged,
        effective_seed_count,
        candidate_pool_count,
    })
}

/// 校验 PPR 参数，避免静默接受不收敛或无输出的配置。
fn validate_config(config: GraphRetrievalConfig) -> Result<()> {
    ensure!(
        config.restart_probability.is_finite()
            && config.restart_probability > 0.0
            && config.restart_probability <= 1.0,
        "restart_probability 必须位于 (0, 1] 区间"
    );
    ensure!(
        config.tolerance.is_finite() && config.tolerance > 0.0,
        "tolerance 必须是有限正数"
    );
    ensure!(config.max_iterations > 0, "max_iterations 必须大于零");
    ensure!(config.top_k > 0, "top_k 必须大于零");
    Ok(())
}

/// 从事实边构造稳定、归一化的查询向 transition。
fn build_transitions(
    graph: &dyn GraphReadStore,
    intent: TraversalIntent,
    nodes: &[crate::graph::Node],
    node_indexes: &HashMap<String, usize>,
) -> Result<Vec<Vec<(usize, f64)>>> {
    let mut weighted_arcs = vec![HashMap::<usize, f64>::new(); nodes.len()];

    for (source_index, node) in nodes.iter().enumerate() {
        let neighbors = graph
            .get_node_edges(&node.id)
            .with_context(|| format!("读取节点 '{}' 的邻接边失败", node.id))?;
        let Some(neighbors) = neighbors else {
            continue;
        };

        for edge_view in neighbors.outgoing {
            let Some(target_index) = node_indexes.get(&edge_view.node.id).copied() else {
                continue;
            };
            let (forward_weight, reverse_weight) =
                intent_edge_weights(intent, &edge_view.edge.edge_type);
            if forward_weight > 0.0 {
                *weighted_arcs[source_index].entry(target_index).or_default() += forward_weight;
            }
            if reverse_weight > 0.0 {
                *weighted_arcs[target_index].entry(source_index).or_default() += reverse_weight;
            }
        }
    }

    Ok(weighted_arcs
        .into_iter()
        .map(|arcs| {
            let total_weight = arcs.values().sum::<f64>();
            let mut normalized = arcs
                .into_iter()
                .map(|(target_index, weight)| (target_index, weight / total_weight))
                .collect::<Vec<_>>();
            normalized.sort_by_key(|(target_index, _)| *target_index);
            normalized
        })
        .collect())
}

/// 返回事实边在当前查询意图下的正向、反向权重。
fn intent_edge_weights(intent: TraversalIntent, edge_type: &EdgeType) -> (f64, f64) {
    match intent {
        TraversalIntent::Writer => match edge_type {
            EdgeType::FieldAlias => (0.8, 0.8),
            EdgeType::FieldWrite | EdgeType::Writes | EdgeType::ActionWrites => (0.15, 1.0),
            EdgeType::Triggers | EdgeType::ActionReads => (0.1, 0.1),
            EdgeType::Contains => (0.02, 0.02),
            _ => (0.0, 0.0),
        },
        TraversalIntent::ValueSource => match edge_type {
            EdgeType::Reads => (1.0, 0.25),
            EdgeType::FieldAlias => (0.9, 0.9),
            EdgeType::DependsOn => (0.9, 0.25),
            EdgeType::DataflowInput
            | EdgeType::DataflowInternal
            | EdgeType::DataflowOutput
            | EdgeType::OutputsTo => (0.8, 0.8),
            EdgeType::Contains => (0.02, 0.02),
            _ => (0.0, 0.0),
        },
        TraversalIntent::Availability => match edge_type {
            EdgeType::Reads => (1.0, 0.25),
            EdgeType::FieldAlias => (0.85, 0.85),
            EdgeType::DependsOn => (0.9, 0.25),
            EdgeType::DataflowInput
            | EdgeType::DataflowInternal
            | EdgeType::DataflowOutput
            | EdgeType::OutputsTo
            | EdgeType::ActionLoadsData => (0.85, 0.85),
            EdgeType::Contains => (0.02, 0.02),
            _ => (0.0, 0.0),
        },
        TraversalIntent::Display => match edge_type {
            EdgeType::DependsOn | EdgeType::ActionControlsComponent => (1.0, 0.8),
            EdgeType::Triggers => (0.1, 0.1),
            EdgeType::Contains => (0.02, 0.02),
            _ => (0.0, 0.0),
        },
        TraversalIntent::Action => match edge_type {
            EdgeType::Triggers => (1.0, 0.2),
            EdgeType::ActionReads | EdgeType::ActionWrites | EdgeType::FieldWrite => (0.8, 0.35),
            EdgeType::ActionNavigates
            | EdgeType::ActionSetsParam
            | EdgeType::ActionControlsComponent
            | EdgeType::ActionValidates
            | EdgeType::ActionLoadsData => (0.9, 0.4),
            EdgeType::DependsOn => (0.8, 0.2),
            EdgeType::Contains => (0.02, 0.02),
            _ => (0.0, 0.0),
        },
        // 这里刻意不用通配符：新增 EdgeType 必须先明确分类，不能自动获得高权。
        // 这把“未知关系不隐式传播”变成编译期约束。
        TraversalIntent::Context | TraversalIntent::Auto => match edge_type {
            EdgeType::Contains => (0.05, 0.05),
            EdgeType::EmbedsPage => (0.1, 0.1),
            EdgeType::Reads
            | EdgeType::Writes
            | EdgeType::Triggers
            | EdgeType::DataflowInput
            | EdgeType::ActionWrites
            | EdgeType::OpensPage
            | EdgeType::PassesParam
            | EdgeType::SetsParam
            | EdgeType::OutputsTo
            | EdgeType::DataflowInternal
            | EdgeType::DataflowOutput
            | EdgeType::FieldAlias
            | EdgeType::FieldWrite
            | EdgeType::ActionReads
            | EdgeType::ActionNavigates
            | EdgeType::ActionSetsParam
            | EdgeType::ActionControlsComponent
            | EdgeType::ActionValidates
            | EdgeType::ActionLoadsData
            | EdgeType::DependsOn
            | EdgeType::ExecutesScript
            | EdgeType::LinksScript => (0.5, 0.5),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeType;
    use crate::memory_graph_store::MemoryGraphStore;

    /// 构造只有一个 dangling 节点的最小图。
    fn dangling_graph() -> MemoryGraphStore {
        let mut graph = MemoryGraphStore::new();
        graph.add_test_node("field:a", "a", NodeType::Field, "fixture");
        graph
    }

    /// dangling mass 必须回灌种子，保证概率质量守恒。
    #[test]
    fn graph_retrieval_dangling_seed_conserves_probability() {
        let graph = dangling_graph();
        let result = typed_personalized_page_rank(
            &graph,
            TraversalIntent::Context,
            &[GraphSeed {
                node_id: "field:a".to_string(),
                score: 1.0,
            }],
            GraphRetrievalConfig {
                top_k: 1,
                ..GraphRetrievalConfig::default()
            },
        )
        .expect("dangling seed 应成功收敛");

        assert_eq!(result.converged, true);
        assert_eq!(result.ranked_nodes.len(), 1);
        assert_eq!(result.ranked_nodes[0].node_id, "field:a");
        assert_eq!((result.ranked_nodes[0].score - 1.0).abs() < 1e-12, true);
    }

    /// 全零或不存在的种子不得静默返回空结果。
    #[test]
    fn graph_retrieval_rejects_missing_positive_seed() {
        let graph = dangling_graph();
        let error = typed_personalized_page_rank(
            &graph,
            TraversalIntent::Context,
            &[GraphSeed {
                node_id: "field:missing".to_string(),
                score: 1.0,
            }],
            GraphRetrievalConfig::default(),
        )
        .expect_err("不存在的正分种子应报错");

        assert_eq!(error.to_string().contains("没有可用种子"), true);
    }

    /// 达到迭代上限时必须显式标记未收敛。
    #[test]
    fn graph_retrieval_marks_iteration_limit() {
        let mut graph = dangling_graph();
        graph.add_test_node("field:b", "b", NodeType::Field, "fixture");
        graph.add_test_edge("field:a", "field:b", EdgeType::FieldAlias, None);

        let result = typed_personalized_page_rank(
            &graph,
            TraversalIntent::Context,
            &[GraphSeed {
                node_id: "field:a".to_string(),
                score: 1.0,
            }],
            GraphRetrievalConfig {
                max_iterations: 1,
                tolerance: 1e-30,
                top_k: 2,
                ..GraphRetrievalConfig::default()
            },
        )
        .expect("一轮 PPR 应返回可观察结果");

        assert_eq!(result.iterations, 1);
        assert_eq!(result.converged, false);
    }

    /// Context/Auto 的已知语义边和结构边必须保持显式、不同档位。
    #[test]
    fn graph_retrieval_context_weights_are_explicit() {
        assert_eq!(
            intent_edge_weights(TraversalIntent::Context, &EdgeType::Reads),
            (0.5, 0.5)
        );
        assert_eq!(
            intent_edge_weights(TraversalIntent::Context, &EdgeType::Contains),
            (0.05, 0.05)
        );
        assert_eq!(
            intent_edge_weights(TraversalIntent::Auto, &EdgeType::EmbedsPage),
            (0.1, 0.1)
        );
    }

    /// runtime 策略解析必须保持未设置时 baseline，并只接受批准值。
    #[test]
    fn graph_retrieval_strategy_rejects_unknown_value() {
        assert_eq!(
            GraphRetrievalStrategy::parse(None).expect("未设置时应使用 baseline"),
            GraphRetrievalStrategy::Baseline
        );
        assert_eq!(
            GraphRetrievalStrategy::parse(Some("baseline")).expect("显式 baseline 应被接受"),
            GraphRetrievalStrategy::Baseline
        );
        assert_eq!(
            GraphRetrievalStrategy::parse(Some("typed-ppr")).expect("typed-ppr 应被接受"),
            GraphRetrievalStrategy::TypedPpr
        );
        let error =
            GraphRetrievalStrategy::parse(Some("typed_ppr")).expect_err("未知策略不得静默回退");
        assert_eq!(
            error.to_string().contains("只接受 baseline 或 typed-ppr"),
            true
        );
    }
}
