use crate::graph::{EdgeType, Node, NodeType};
use crate::graph_store::GraphWriteStore;
use crate::memory_graph_store::MemoryGraphStore;
use serde_json::json;

const PAGE_ID: &str = "page:app/cache_contract.spg";
const PAGE_PATH: &str = "app/cache_contract.spg";
const COMPONENT_ID: &str = "comp:app/cache_contract.spg|input1";
const MODEL_A_ID: &str = "model:orders";
const MODEL_B_ID: &str = "model:customers";

/// 构造包含三类前置条件和两个模型引用的真实 warm cache。
fn populated_cache() -> anyhow::Result<super::PageLogicAvailabilityCache> {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node(PAGE_ID, "cache_contract", NodeType::Page, PAGE_PATH);
    graph.add_test_node(COMPONENT_ID, "input1", NodeType::Component, PAGE_PATH);
    graph.add_test_node(MODEL_A_ID, "orders", NodeType::Model, PAGE_PATH);
    graph.add_test_node(MODEL_B_ID, "customers", NodeType::Model, PAGE_PATH);
    graph.add_test_edge(PAGE_ID, COMPONENT_ID, EdgeType::Contains, None);

    for (condition_id, condition_type, effect_type) in [
        (
            "cond:app/cache_contract.spg|visible",
            "VisibleCondition",
            "Visible",
        ),
        (
            "cond:app/cache_contract.spg|filter",
            "CalcCondition",
            "Filter",
        ),
        (
            "cond:app/cache_contract.spg|submit",
            "ActionCondition",
            "execute",
        ),
    ] {
        graph.upsert_node(Node {
            id: condition_id.to_string(),
            name: condition_id.to_string(),
            node_type: NodeType::Condition,
            path: PAGE_PATH.to_string(),
            meta: Some(json!({
                "condition_type": condition_type,
                "raw_expr": "=true",
                "normalized_expr": "true",
                "json_path": format!("canvas.components.input1.{condition_type}"),
                "owner_type": "component",
                "owner_id": COMPONENT_ID,
                "subject_type": "component",
                "effect_type": effect_type,
                "referenced_symbols": [
                    format!("{MODEL_A_ID}.id"),
                    format!("{MODEL_B_ID}.id"),
                ],
            })),
        origin_file: None})?;
        graph.add_test_edge(condition_id, COMPONENT_ID, EdgeType::DependsOn, None);
    }

    super::build_page_logic_availability_cache(&graph, None, PAGE_ID, None, "normal", None)
}

/// 确认候选只改变 prerequisites，其余 comparator 的前置字段保持一致。
fn assert_non_prerequisite_metadata_matches(
    left: &super::PageLogicAvailabilityCache,
    right: &super::PageLogicAvailabilityCache,
) {
    assert_eq!(left.page_id, right.page_id);
    assert_eq!(left.budget, right.budget);
    assert_eq!(left.batch.entries, right.batch.entries);
    assert_eq!(left.batch.fast_path_count, right.batch.fast_path_count);
    assert_eq!(left.batch.fallback_count, right.batch.fallback_count);
    assert_eq!(
        left.batch.condition_group_count,
        right.batch.condition_group_count
    );
    assert_eq!(left.batch.graph_cache_hits, right.batch.graph_cache_hits);
    assert_eq!(left.batch.materialized_hits, right.batch.materialized_hits);
    assert_eq!(left.paths.is_some(), right.paths.is_some());
    if let (Some(left_paths), Some(right_paths)) = (&left.paths, &right.paths) {
        assert_eq!(left_paths.primary_paths, right_paths.primary_paths);
        assert_eq!(left_paths.related_context, right_paths.related_context);
        assert_eq!(left_paths.candidate_paths, right_paths.candidate_paths);
        assert_eq!(left_paths.supporting_paths, right_paths.supporting_paths);
        assert_eq!(left_paths.rejected_paths, right_paths.rejected_paths);
        assert_eq!(
            left_paths.path_selection_diagnostics,
            right_paths.path_selection_diagnostics
        );
    }
}

/// 确认 retained JSON 条目相同，使差异只能归因于 prerequisites 元数据。
fn assert_retained_prerequisites_match(
    left: &super::PageLogicAvailabilityCache,
    right: &super::PageLogicAvailabilityCache,
) {
    let left_prerequisites = left.prerequisites.as_ref().expect("左侧应有前置条件");
    let right_prerequisites = right.prerequisites.as_ref().expect("右侧应有前置条件");
    assert_eq!(
        left_prerequisites.display_prerequisites,
        right_prerequisites.display_prerequisites
    );
    assert_eq!(
        left_prerequisites.data_prerequisites,
        right_prerequisites.data_prerequisites
    );
    assert_eq!(
        left_prerequisites.action_prerequisites,
        right_prerequisites.action_prerequisites
    );
}

/// 断言前置条件差异在两个比较方向都返回稳定原因。
fn assert_prerequisites_mismatch(
    left: &super::PageLogicAvailabilityCache,
    right: &super::PageLogicAvailabilityCache,
) {
    assert_non_prerequisite_metadata_matches(left, right);
    assert_eq!(
        left.warm_cache_diff_reason(right),
        Some("prerequisites mismatch".to_string())
    );
    assert_eq!(
        right.warm_cache_diff_reason(left),
        Some("prerequisites mismatch".to_string())
    );
    assert_eq!(left.equals_warm_cache(right), false);
    assert_eq!(right.equals_warm_cache(left), false);
}

/// retained 条目相同时，display_total 差异仍必须破坏 warm cache 等价性。
#[test]
fn warm_cache_comparison_detects_display_total_mismatch() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    altered
        .prerequisites
        .as_mut()
        .expect("warm cache 应物化前置条件")
        .display_total += 1;

    assert_retained_prerequisites_match(&cache, &altered);
    assert_prerequisites_mismatch(&cache, &altered);
    Ok(())
}

/// retained 条目相同时，data_total 差异仍必须破坏 warm cache 等价性。
#[test]
fn warm_cache_comparison_detects_data_total_mismatch() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    altered
        .prerequisites
        .as_mut()
        .expect("warm cache 应物化前置条件")
        .data_total += 1;

    assert_retained_prerequisites_match(&cache, &altered);
    assert_prerequisites_mismatch(&cache, &altered);
    Ok(())
}

/// retained 条目相同时，action_total 差异仍必须破坏 warm cache 等价性。
#[test]
fn warm_cache_comparison_detects_action_total_mismatch() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    altered
        .prerequisites
        .as_mut()
        .expect("warm cache 应物化前置条件")
        .action_total += 1;

    assert_retained_prerequisites_match(&cache, &altered);
    assert_prerequisites_mismatch(&cache, &altered);
    Ok(())
}

/// 模型 ID 数量相同但具体 ID 不同，必须被 prerequisites PartialEq 识别。
#[test]
fn warm_cache_comparison_detects_model_id_replacement() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    {
        let model_ids = &mut altered
            .prerequisites
            .as_mut()
            .expect("warm cache 应物化前置条件")
            .model_ids;
        assert_eq!(model_ids.len(), 2);
        model_ids[0] = "model:products".to_string();
    }

    assert_retained_prerequisites_match(&cache, &altered);
    assert_eq!(
        cache
            .prerequisites
            .as_ref()
            .expect("左侧前置条件")
            .model_ids
            .len(),
        altered
            .prerequisites
            .as_ref()
            .expect("右侧前置条件")
            .model_ids
            .len()
    );
    assert_prerequisites_mismatch(&cache, &altered);
    Ok(())
}

/// 模型 ID 数量相同但顺序不同，必须被 prerequisites PartialEq 识别。
#[test]
fn warm_cache_comparison_detects_model_id_order_change() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    altered
        .prerequisites
        .as_mut()
        .expect("warm cache 应物化前置条件")
        .model_ids
        .reverse();

    assert_retained_prerequisites_match(&cache, &altered);
    let original_ids = &cache
        .prerequisites
        .as_ref()
        .expect("左侧前置条件")
        .model_ids;
    let altered_ids = &altered
        .prerequisites
        .as_ref()
        .expect("右侧前置条件")
        .model_ids;
    assert_eq!(original_ids.len(), altered_ids.len());
    assert_eq!(original_ids == altered_ids, false);
    assert_prerequisites_mismatch(&cache, &altered);
    Ok(())
}

/// Some 与 None 的前置条件状态不同，必须返回 prerequisites mismatch。
#[test]
fn warm_cache_comparison_detects_some_none_prerequisites() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let mut altered = cache.clone();
    altered.prerequisites = None;

    assert_non_prerequisite_metadata_matches(&cache, &altered);
    assert_eq!(
        cache.warm_cache_diff_reason(&altered),
        Some("prerequisites mismatch".to_string())
    );
    assert_eq!(
        altered.warm_cache_diff_reason(&cache),
        Some("prerequisites mismatch".to_string())
    );
    assert_eq!(cache.equals_warm_cache(&altered), false);
    assert_eq!(altered.equals_warm_cache(&cache), false);
    Ok(())
}

/// 完全相等的 warm cache 克隆应双向通过，且直接验证 PagePrerequisites 的 PartialEq。
#[test]
fn warm_cache_comparison_accepts_exact_clone() -> anyhow::Result<()> {
    let cache = populated_cache()?;
    let clone = cache.clone();
    let prerequisites = cache.prerequisites.as_ref().expect("原缓存前置条件");
    let cloned_prerequisites = clone.prerequisites.as_ref().expect("克隆缓存前置条件");

    assert_eq!(prerequisites == cloned_prerequisites, true);
    assert_eq!(cache.warm_cache_diff_reason(&clone), None);
    assert_eq!(clone.warm_cache_diff_reason(&cache), None);
    assert_eq!(cache.equals_warm_cache(&clone), true);
    assert_eq!(clone.equals_warm_cache(&cache), true);
    Ok(())
}
