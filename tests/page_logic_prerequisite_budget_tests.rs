#![cfg(feature = "cli-local")]

use metadata_checker::graph::{EdgeType, Node, NodeType};
use metadata_checker::graph_store::GraphWriteStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::query::{
    build_page_logic_availability_cache,
    build_query_page_logic_output_profiled_with_availability_cache,
    build_query_page_logic_output_profiled_with_dense_snapshot,
};
use serde_json::json;

const PAGE_ID: &str = "page:app/prerequisites.spg";
const PAGE_PATH: &str = "app/prerequisites.spg";
const COMPONENT_ID: &str = "comp:app/prerequisites.spg|input1";

/// 每组条件按递减影响分排序；模型只出现在末尾条件，验证裁剪不能丢掉其模型引用。
fn prerequisite_graph(group_size: usize) -> anyhow::Result<MemoryGraphStore> {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node(PAGE_ID, "prerequisites", NodeType::Page, PAGE_PATH);
    graph.add_test_node(COMPONENT_ID, "input1", NodeType::Component, PAGE_PATH);
    graph.add_test_edge(PAGE_ID, COMPONENT_ID, EdgeType::Contains, None);
    for (group, kind) in [
        ("display", "VisibleCondition"),
        ("data", "CalcCondition"),
        ("action", "ActionCondition"),
    ] {
        let model_id = format!("model:{group}_tail");
        graph.add_test_node(&model_id, &model_id, NodeType::Model, PAGE_PATH);
        for index in 0..group_size {
            let condition_id = format!("cond:{PAGE_PATH}|{group}_{index}");
            let references = if index + 1 == group_size {
                vec![format!("{model_id}.value")]
            } else {
                (0..group_size - index + 1)
                    .map(|reference| format!("comp:{PAGE_PATH}|source_{reference}"))
                    .collect()
            };
            graph.upsert_node(Node {
                id: condition_id.clone(),
                name: condition_id.clone(),
                node_type: NodeType::Condition,
                path: PAGE_PATH.to_string(),
                meta: Some(json!({
                    "condition_type": kind,
                    "raw_expr": "=true",
                    "normalized_expr": "true",
                    "json_path": format!("components.input1.{group}_{index}"),
                    "owner_type": "component",
                    "owner_id": COMPONENT_ID,
                    "subject_type": "component",
                    "effect_type": "Enable",
                    "referenced_symbols": references,
                })),
                origin_file: None,
            })?;
            graph.add_test_edge(&condition_id, COMPONENT_ID, EdgeType::DependsOn, None);
        }
    }
    Ok(graph)
}

/// 覆盖空组、未满、恰好上限及超限；每个预算都必须保持完整输出字节、总数和排序。
#[test]
fn compact_prerequisites_bound_cached_items_without_changing_answers() -> anyhow::Result<()> {
    for group_size in [0, 3, 5, 7] {
        let graph = prerequisite_graph(group_size)?;
        for budget in ["compact", "normal", "full"] {
            let cache =
                build_page_logic_availability_cache(&graph, None, PAGE_ID, None, budget, None)?;
            let retained = if budget == "compact" {
                group_size.min(5)
            } else {
                group_size
            };
            let footprint = cache.cache_footprint();
            assert_eq!(
                footprint.display_prerequisites, retained,
                "{budget}/{group_size}"
            );
            assert_eq!(
                footprint.data_prerequisites, retained,
                "{budget}/{group_size}"
            );
            assert_eq!(
                footprint.action_prerequisites, retained,
                "{budget}/{group_size}"
            );

            let (baseline, _) = build_query_page_logic_output_profiled_with_dense_snapshot(
                &graph, None, PAGE_ID, None, budget,
            )?;
            let (cached, profile) = build_query_page_logic_output_profiled_with_availability_cache(
                &graph,
                None,
                Some(&cache),
                None,
                PAGE_ID,
                None,
                budget,
            )?;
            assert_eq!(
                serde_json::to_vec(&cached)?,
                serde_json::to_vec(&baseline)?,
                "{budget}/{group_size}"
            );
            assert_eq!(profile.counter("prerequisites_read_model_used"), 1);
            for group in ["display", "data", "action"] {
                let field = format!("{group}_prerequisites");
                assert_eq!(
                    cached["summary"][format!("{field}_count")],
                    json!(group_size)
                );
                assert_eq!(profile.counter(&field), group_size as u64);
                let items = if budget == "compact" {
                    let detail = &cached["details"][&field];
                    assert_eq!(detail["total_count"], json!(group_size));
                    assert_eq!(detail["shown_count"], json!(retained));
                    assert_eq!(detail["remaining_count"], json!(group_size - retained));
                    assert_eq!(detail["truncated"], json!(group_size > retained));
                    &detail["items"]
                } else {
                    &cached["details"][&field]
                };
                let expected_ids: Vec<_> = (0..retained)
                    .map(|index| json!(format!("cond:{PAGE_PATH}|{group}_{index}")))
                    .collect();
                assert_eq!(
                    items
                        .as_array()
                        .expect("prerequisite items")
                        .iter()
                        .map(|item| item["evidence"]["node_id"].clone())
                        .collect::<Vec<_>>(),
                    expected_ids
                );
            }
            let model_count = if group_size == 0 { 0 } else { 2 };
            assert_eq!(cached["summary"]["key_model_count"], json!(model_count));
            if budget == "compact" {
                assert_eq!(
                    cached["details"]["key_model_availability"]["total_count"],
                    json!(model_count)
                );
                assert_eq!(
                    cached["details"]["key_model_availability"]["shown_count"],
                    json!(model_count)
                );
            }
        }
    }
    Ok(())
}

/// compact 缓存不能误用于 normal/full；否则尾部条件会在非截断输出中消失。
#[test]
fn prerequisite_cache_budget_mismatch_rebuilds_complete_groups() -> anyhow::Result<()> {
    let graph = prerequisite_graph(7)?;
    let cache = build_page_logic_availability_cache(&graph, None, PAGE_ID, None, "compact", None)?;
    for budget in ["normal", "full"] {
        let (baseline, _) = build_query_page_logic_output_profiled_with_dense_snapshot(
            &graph, None, PAGE_ID, None, budget,
        )?;
        let (actual, profile) = build_query_page_logic_output_profiled_with_availability_cache(
            &graph,
            None,
            Some(&cache),
            None,
            PAGE_ID,
            None,
            budget,
        )?;
        assert_eq!(profile.counter("prerequisites_read_model_used"), 0);
        assert_eq!(
            actual["details"]["display_prerequisites"]
                .as_array()
                .map(Vec::len),
            Some(7)
        );
        assert_eq!(serde_json::to_vec(&actual)?, serde_json::to_vec(&baseline)?);
    }
    Ok(())
}
