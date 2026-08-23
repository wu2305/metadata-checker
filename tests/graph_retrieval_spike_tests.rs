//! M58.4 轻量图召回确定性对比测试。

use anyhow::{Context, Result};
use metadata_checker::answer_contract::TraversalIntent;
use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_retrieval::{
    GraphRetrievalConfig, GraphRetrievalStrategy, GraphSeed, typed_personalized_page_rank,
};
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// Spike fixture 根结构。
#[derive(Debug, Deserialize)]
struct FixtureSuite {
    cases: Vec<FixtureCase>,
}

/// 单个公平对照 case。
#[derive(Debug, Deserialize)]
struct FixtureCase {
    case_id: String,
    intent: String,
    description: String,
    top_k: usize,
    hop_depth: usize,
    nodes: Vec<FixtureNode>,
    edges: Vec<FixtureEdge>,
    seed_scores: Vec<GraphSeed>,
    relevant_node_ids: Vec<String>,
    gold_terminal_ids: Vec<String>,
}

/// Fixture 节点的最小表示。
#[derive(Debug, Deserialize)]
struct FixtureNode {
    id: String,
    node_type: NodeType,
}

/// Fixture 边的最小表示。
#[derive(Debug, Deserialize)]
struct FixtureEdge {
    from: String,
    to: String,
    edge_type: EdgeType,
}

/// 单个 variant 的结构化指标。
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ComparisonRow {
    case_id: String,
    description: String,
    variant: String,
    top_node_ids: Vec<String>,
    relevant_recall_at_k: f64,
    precision_at_k: f64,
    gold_terminal_recall_at_k: f64,
    candidate_pool_count: usize,
    iterations: Option<usize>,
    converged: Option<bool>,
}

/// 单个 variant 的聚合指标。
#[derive(Debug, Clone, PartialEq, Serialize)]
struct AggregateRow {
    variant: String,
    mean_relevant_recall_at_k: f64,
    mean_precision_at_k: f64,
    mean_gold_terminal_recall_at_k: f64,
}

/// 可稳定序列化的完整对比报告。
#[derive(Debug, Clone, PartialEq, Serialize)]
struct ComparisonReport {
    cases: Vec<ComparisonRow>,
    aggregates: Vec<AggregateRow>,
}

/// 读取已冻结的确定性 fixture。
fn load_suite() -> Result<FixtureSuite> {
    serde_json::from_str(include_str!("fixtures/graph_retrieval_spike_cases.json"))
        .context("解析 graph retrieval spike fixture 失败")
}

/// 按 fixture 构造现有 GraphReadStore 实现。
fn build_graph(case: &FixtureCase) -> MemoryGraphStore {
    let mut graph = MemoryGraphStore::new();
    for node in &case.nodes {
        graph.add_test_node(&node.id, &node.id, node.node_type.clone(), "m58.4-fixture");
    }
    for edge in &case.edges {
        graph.add_test_edge(&edge.from, &edge.to, edge.edge_type.clone(), None);
    }
    graph
}

/// 计算 seed-only 对照，返回 top-k 与截断前候选数。
fn rank_seed_only(
    graph: &dyn GraphReadStore,
    seeds: &[GraphSeed],
    top_k: usize,
) -> Result<(Vec<String>, usize)> {
    let mut merged_scores = HashMap::<String, f64>::new();
    for seed in seeds {
        if seed.score > 0.0 && graph.get_node(&seed.node_id)?.is_some() {
            *merged_scores.entry(seed.node_id.clone()).or_default() += seed.score;
        }
    }
    let mut ranked = merged_scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    let candidate_pool_count = ranked.len();
    ranked.truncate(top_k);
    Ok((
        ranked.into_iter().map(|(node_id, _)| node_id).collect(),
        candidate_pool_count,
    ))
}

/// 计算双向无权 N-hop 对照，按最短 hop 和节点 ID 稳定排序。
fn rank_unweighted_hop(
    graph: &dyn GraphReadStore,
    seeds: &[GraphSeed],
    hop_depth: usize,
    top_k: usize,
) -> Result<(Vec<String>, usize)> {
    let mut positive_seeds = seeds
        .iter()
        .filter(|seed| seed.score > 0.0)
        .map(|seed| seed.node_id.clone())
        .collect::<Vec<_>>();
    positive_seeds.sort();
    positive_seeds.dedup();

    let mut depths = HashMap::<String, usize>::new();
    let mut queue = VecDeque::<String>::new();
    for node_id in positive_seeds {
        if graph.get_node(&node_id)?.is_some() {
            depths.insert(node_id.clone(), 0);
            queue.push_back(node_id);
        }
    }

    while let Some(node_id) = queue.pop_front() {
        let depth = depths[&node_id];
        if depth >= hop_depth {
            continue;
        }
        let Some(neighbors) = graph.get_node_edges(&node_id)? else {
            continue;
        };
        let mut neighbor_ids = neighbors
            .outgoing
            .into_iter()
            .chain(neighbors.incoming)
            .map(|edge_view| edge_view.node.id)
            .collect::<Vec<_>>();
        neighbor_ids.sort();
        neighbor_ids.dedup();
        for neighbor_id in neighbor_ids {
            if depths.contains_key(&neighbor_id) {
                continue;
            }
            depths.insert(neighbor_id.clone(), depth + 1);
            queue.push_back(neighbor_id);
        }
    }

    let candidate_pool_count = depths.len();
    let mut ranked = depths.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(top_k);
    Ok((
        ranked.into_iter().map(|(node_id, _)| node_id).collect(),
        candidate_pool_count,
    ))
}

/// 为一组 top-k 候选计算固定分母指标。
fn comparison_row(
    case: &FixtureCase,
    variant: &str,
    top_node_ids: Vec<String>,
    candidate_pool_count: usize,
    iterations: Option<usize>,
    converged: Option<bool>,
) -> ComparisonRow {
    let relevant = case
        .relevant_node_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let gold = case
        .gold_terminal_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let relevant_hits = top_node_ids
        .iter()
        .filter(|node_id| relevant.contains(node_id.as_str()))
        .count();
    let gold_hits = top_node_ids
        .iter()
        .filter(|node_id| gold.contains(node_id.as_str()))
        .count();

    ComparisonRow {
        case_id: case.case_id.clone(),
        description: case.description.clone(),
        variant: variant.to_string(),
        relevant_recall_at_k: relevant_hits as f64 / relevant.len() as f64,
        precision_at_k: relevant_hits as f64 / top_node_ids.len() as f64,
        gold_terminal_recall_at_k: gold_hits as f64 / gold.len() as f64,
        top_node_ids,
        candidate_pool_count,
        iterations,
        converged,
    }
}

/// 汇总一个 variant 的三类均值。
fn aggregate_variant(rows: &[ComparisonRow], variant: &str) -> AggregateRow {
    let selected = rows
        .iter()
        .filter(|row| row.variant == variant)
        .collect::<Vec<_>>();
    let denominator = selected.len() as f64;

    AggregateRow {
        variant: variant.to_string(),
        mean_relevant_recall_at_k: selected
            .iter()
            .map(|row| row.relevant_recall_at_k)
            .sum::<f64>()
            / denominator,
        mean_precision_at_k: selected.iter().map(|row| row.precision_at_k).sum::<f64>()
            / denominator,
        mean_gold_terminal_recall_at_k: selected
            .iter()
            .map(|row| row.gold_terminal_recall_at_k)
            .sum::<f64>()
            / denominator,
    }
}

/// 对同一 fixture 运行三组公平对照。
fn build_report() -> Result<ComparisonReport> {
    let mut suite = load_suite()?;
    suite
        .cases
        .sort_by(|left, right| left.case_id.cmp(&right.case_id));

    let mut rows = Vec::new();
    for case in &suite.cases {
        let graph = build_graph(case);
        let (seed_nodes, seed_pool_count) = rank_seed_only(&graph, &case.seed_scores, case.top_k)?;
        rows.push(comparison_row(
            case,
            "seed_only",
            seed_nodes,
            seed_pool_count,
            None,
            None,
        ));

        let (hop_nodes, hop_pool_count) =
            rank_unweighted_hop(&graph, &case.seed_scores, case.hop_depth, case.top_k)?;
        rows.push(comparison_row(
            case,
            "unweighted_hop",
            hop_nodes,
            hop_pool_count,
            None,
            None,
        ));

        let intent = TraversalIntent::parse(&case.intent)?;
        let ppr = typed_personalized_page_rank(
            &graph,
            intent,
            &case.seed_scores,
            GraphRetrievalConfig {
                top_k: case.top_k,
                ..GraphRetrievalConfig::default()
            },
        )?;
        rows.push(comparison_row(
            case,
            "typed_ppr",
            ppr.ranked_nodes
                .into_iter()
                .map(|node| node.node_id)
                .collect(),
            ppr.candidate_pool_count,
            Some(ppr.iterations),
            Some(ppr.converged),
        ));
    }

    let aggregates = ["seed_only", "unweighted_hop", "typed_ppr"]
        .into_iter()
        .map(|variant| aggregate_variant(&rows, variant))
        .collect();

    Ok(ComparisonReport {
        cases: rows,
        aggregates,
    })
}

/// typed PPR 必须满足预先登记的确定性 go 条件。
#[test]
fn graph_retrieval_spike_compares_three_variants_and_meets_gate() -> Result<()> {
    let report = build_report()?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    let typed_rows = report
        .cases
        .iter()
        .filter(|row| row.variant == "typed_ppr")
        .collect::<Vec<_>>();

    assert_eq!(typed_rows.len(), 3);
    for row in typed_rows {
        assert_eq!(row.gold_terminal_recall_at_k, 1.0, "case={}", row.case_id);
        assert_eq!(row.converged, Some(true), "case={}", row.case_id);
    }

    let hop = report
        .aggregates
        .iter()
        .find(|row| row.variant == "unweighted_hop")
        .expect("报告必须包含 hop 聚合");
    let typed = report
        .aggregates
        .iter()
        .find(|row| row.variant == "typed_ppr")
        .expect("报告必须包含 typed PPR 聚合");
    assert_eq!(
        typed.mean_relevant_recall_at_k >= hop.mean_relevant_recall_at_k,
        true
    );
    assert_eq!(typed.mean_precision_at_k > hop.mean_precision_at_k, true);

    Ok(())
}

/// 相同输入必须产生字节级一致的结构化报告。
#[test]
fn graph_retrieval_spike_report_is_deterministic() -> Result<()> {
    let first = serde_json::to_vec_pretty(&build_report()?)?;
    let second = serde_json::to_vec_pretty(&build_report()?)?;

    assert_eq!(first, second);
    Ok(())
}

/// 分数并列时必须按节点 ID 升序打破平局。
#[test]
fn graph_retrieval_spike_breaks_score_ties_by_node_id() -> Result<()> {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node("field:b", "b", NodeType::Field, "fixture");
    graph.add_test_node("field:a", "a", NodeType::Field, "fixture");
    let result = typed_personalized_page_rank(
        &graph,
        TraversalIntent::Context,
        &[
            GraphSeed {
                node_id: "field:b".to_string(),
                score: 1.0,
            },
            GraphSeed {
                node_id: "field:a".to_string(),
                score: 1.0,
            },
        ],
        GraphRetrievalConfig {
            top_k: 2,
            ..GraphRetrievalConfig::default()
        },
    )?;

    assert_eq!(
        result
            .ranked_nodes
            .into_iter()
            .map(|node| node.node_id)
            .collect::<Vec<_>>(),
        vec!["field:a", "field:b"]
    );
    Ok(())
}

/// 显式 baseline 必须与既有 ExplainCondition 输出完全一致。
#[test]
fn graph_retrieval_baseline_preserves_explain_contract() -> Result<()> {
    let suite = load_suite()?;
    let case = &suite.cases[2];
    let graph = build_graph(case);
    let target_id = &case.seed_scores[0].node_id;

    let legacy = metadata_checker::explain::build_explain_condition_output_with_intent(
        &graph,
        target_id,
        "normal",
        TraversalIntent::Action,
    )?;
    let explicit =
        metadata_checker::explain::build_explain_condition_output_with_intent_and_retrieval(
            &graph,
            target_id,
            "normal",
            TraversalIntent::Action,
            GraphRetrievalStrategy::Baseline,
        )?;

    assert_eq!(legacy, explicit);
    assert_eq!(
        explicit
            .get("details")
            .and_then(|details| details.get("experimental_graph_retrieval"))
            .is_none(),
        true
    );
    Ok(())
}

/// typed-PPR treatment 必须暴露候选上下文，但明确声明候选不是事实。
#[test]
fn graph_retrieval_typed_treatment_exposes_candidate_only_context() -> Result<()> {
    let suite = load_suite()?;
    let case = &suite.cases[2];
    let graph = build_graph(case);
    let target_id = &case.seed_scores[0].node_id;

    let output =
        metadata_checker::explain::build_explain_condition_output_with_intent_and_retrieval(
            &graph,
            target_id,
            "normal",
            TraversalIntent::Action,
            GraphRetrievalStrategy::TypedPpr,
        )?;
    let context = output
        .get("details")
        .and_then(|details| details.get("experimental_graph_retrieval"))
        .context("typed treatment 必须输出 experimental_graph_retrieval")?;

    assert_eq!(
        context.get("strategy").and_then(|value| value.as_str()),
        Some("typed_ppr")
    );
    assert_eq!(
        context
            .get("evidence_role")
            .and_then(|value| value.as_str()),
        Some("candidate_only")
    );
    assert_eq!(
        context.get("converged").and_then(|value| value.as_bool()),
        Some(true)
    );
    let candidate_ids = context
        .get("ranked_candidates")
        .and_then(|value| value.as_array())
        .context("typed treatment 必须输出候选数组")?
        .iter()
        .filter_map(|candidate| candidate.get("node_id").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(candidate_ids.contains(&"action:submit.validate"), true);
    assert_eq!(candidate_ids.contains(&target_id.as_str()), false);
    Ok(())
}
