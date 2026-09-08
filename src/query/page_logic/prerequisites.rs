use crate::graph::Node;
use crate::graph_store::GraphReadStore;
use crate::perf_profile::PerfProfile;
use anyhow::Result;
use std::time::Instant;

/// 页面级条件前置条件分组
#[derive(Clone, PartialEq)]
pub(super) struct PagePrerequisites {
    pub(super) display_prerequisites: Vec<serde_json::Value>,
    pub(super) data_prerequisites: Vec<serde_json::Value>,
    pub(super) action_prerequisites: Vec<serde_json::Value>,
    /// 完整集合的数量，不随 compact 缓存裁剪而变化。
    pub(super) display_total: usize,
    pub(super) data_total: usize,
    pub(super) action_total: usize,
    /// 按完整 display/data 排序结果首次出现的模型，包含被裁掉的条件引用。
    pub(super) model_ids: Vec<String>,
}

/// 收集组件、动作和数据源模型关联的条件前置条件
pub(super) fn collect_page_prerequisites(
    graph: &dyn GraphReadStore,
    page_path: &str,
    child_components: &[Node],
    child_actions: &[Node],
    data_sources: &[serde_json::Value],
    budget: &str,
    for_cache_materialization: bool,
    profile: &mut Option<&mut PerfProfile>,
) -> Result<PagePrerequisites> {
    let mut display_prerequisites = Vec::new();
    let mut data_prerequisites = Vec::new();
    let mut action_prerequisites = Vec::new();
    let mut seen_conditions: std::collections::HashSet<String> = std::collections::HashSet::new();

    let stage_started = Instant::now();
    for comp in child_components {
        collect_from_node(
            graph,
            page_path,
            &comp.id,
            &mut seen_conditions,
            &mut display_prerequisites,
            &mut data_prerequisites,
            &mut action_prerequisites,
        )?;
    }
    record_ms_counter(profile, "prerequisites_component_scan_ms", stage_started);
    super::set_profile_counter(
        profile,
        "prerequisites_component_nodes",
        child_components.len(),
    );

    let stage_started = Instant::now();
    for action in child_actions {
        collect_from_node(
            graph,
            page_path,
            &action.id,
            &mut seen_conditions,
            &mut display_prerequisites,
            &mut data_prerequisites,
            &mut action_prerequisites,
        )?;
    }
    record_ms_counter(profile, "prerequisites_action_scan_ms", stage_started);
    super::set_profile_counter(profile, "prerequisites_action_nodes", child_actions.len());

    let stage_started = Instant::now();
    let mut data_source_targets = Vec::new();
    let mut seen_data_source_targets = std::collections::HashSet::new();
    for ds in data_sources {
        if let Some(target_id) = ds.get("target_id").and_then(|v| v.as_str()) {
            if seen_data_source_targets.insert(target_id.to_string()) {
                data_source_targets.push(target_id.to_string());
            }
        }
    }
    for target_id in &data_source_targets {
        collect_from_node(
            graph,
            page_path,
            target_id,
            &mut seen_conditions,
            &mut display_prerequisites,
            &mut data_prerequisites,
            &mut action_prerequisites,
        )?;
    }
    record_ms_counter(profile, "prerequisites_data_source_scan_ms", stage_started);
    super::set_profile_counter(
        profile,
        "prerequisites_data_source_nodes",
        data_source_targets.len(),
    );

    let stage_started = Instant::now();
    sort_by_impact(&mut display_prerequisites);
    sort_by_impact(&mut data_prerequisites);
    sort_by_impact(&mut action_prerequisites);
    record_ms_counter(profile, "prerequisites_sort_ms", stage_started);
    super::set_profile_counter(
        profile,
        "prerequisites_unique_conditions",
        seen_conditions.len(),
    );

    // 先保存输出依赖的完整统计与模型顺序，再缩减常驻 JSON；不把裁剪后的长度当总数。
    let display_total = display_prerequisites.len();
    let data_total = data_prerequisites.len();
    let action_total = action_prerequisites.len();
    let mut model_ids = Vec::new();
    let mut seen_models = std::collections::HashSet::new();
    for prerequisite in display_prerequisites.iter().chain(&data_prerequisites) {
        if let Some(references) = prerequisite["depends_on"].as_array() {
            for reference in references {
                if let Some(model_id) = reference.as_str().and_then(super::model_id_from_reference)
                {
                    if seen_models.insert(model_id.clone()) {
                        model_ids.push(model_id);
                    }
                }
            }
        }
    }
    if for_cache_materialization && budget == "compact" {
        for group in [
            &mut display_prerequisites,
            &mut data_prerequisites,
            &mut action_prerequisites,
        ] {
            group.truncate(5);
            group.shrink_to_fit();
        }
    }

    Ok(PagePrerequisites {
        display_prerequisites,
        data_prerequisites,
        action_prerequisites,
        display_total,
        data_total,
        action_total,
        model_ids,
    })
}

fn record_ms_counter(profile: &mut Option<&mut PerfProfile>, name: &str, started_at: Instant) {
    super::set_profile_counter(profile, name, started_at.elapsed().as_millis() as usize);
}

fn collect_from_node(
    graph: &dyn GraphReadStore,
    page_path: &str,
    node_id: &str,
    seen_conditions: &mut std::collections::HashSet<String>,
    display_prerequisites: &mut Vec<serde_json::Value>,
    data_prerequisites: &mut Vec<serde_json::Value>,
    action_prerequisites: &mut Vec<serde_json::Value>,
) -> Result<()> {
    if let Some(neighbors) = graph.get_node_edges(node_id)? {
        let incoming = neighbors.incoming;
        for view in incoming {
            let source = view.node;
            let edge = view.edge;
            if matches!(source.node_type, crate::graph::NodeType::Condition)
                && matches!(edge.edge_type, crate::graph::EdgeType::DependsOn)
                && !seen_conditions.contains(&source.id)
                && source.path == page_path
            {
                seen_conditions.insert(source.id.clone());
                if let Some(prereq) = build_prerequisite(&source, &edge) {
                    push_prerequisite(
                        prereq,
                        display_prerequisites,
                        data_prerequisites,
                        action_prerequisites,
                    );
                }
            }
        }
    }
    Ok(())
}

fn push_prerequisite(
    prereq: serde_json::Value,
    display_prerequisites: &mut Vec<serde_json::Value>,
    data_prerequisites: &mut Vec<serde_json::Value>,
    action_prerequisites: &mut Vec<serde_json::Value>,
) {
    let kind = prereq
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    match kind {
        "VisibleCondition" | "EnableCondition" | "MaskCondition" => {
            display_prerequisites.push(prereq);
        }
        "SourceFilterExp" | "SourceFilterClause" | "ItemFilter" | "CalcCondition" | "CalcExp"
        | "DefaultValueExp" | "FieldExp" => {
            data_prerequisites.push(prereq);
        }
        "ActionConditionExp" | "ActionCondition" | "SubmitCondition" | "SubmitPageCondition" => {
            action_prerequisites.push(prereq);
        }
        _ => {
            display_prerequisites.push(prereq);
        }
    }
}

fn build_prerequisite(
    cond_node: &crate::graph::Node,
    edge: &crate::graph::Edge,
) -> Option<serde_json::Value> {
    let meta = cond_node.meta.as_ref()?;
    let condition_type = meta
        .get("condition_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let raw_expr = meta.get("raw_expr").and_then(|v| v.as_str()).unwrap_or("");
    let normalized_expr = meta
        .get("normalized_expr")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let json_path = meta.get("json_path").and_then(|v| v.as_str()).unwrap_or("");
    let owner_type = meta
        .get("owner_type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let owner_id = meta.get("owner_id").and_then(|v| v.as_str()).unwrap_or("");
    let effect_type = meta
        .get("effect_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let subject_type = meta
        .get("subject_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let referenced_symbols: Vec<String> = meta
        .get("referenced_symbols")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let source_file = cond_node.path.clone();

    let affected_model_count = referenced_symbols
        .iter()
        .filter(|s| s.starts_with("model:"))
        .count();
    let affected_component_count = referenced_symbols
        .iter()
        .filter(|s| s.starts_with("comp:"))
        .count();
    let is_entrypoint = matches!(subject_type, "action") || matches!(effect_type, "execute");
    let is_main_panel = matches!(subject_type, "component") && !matches!(effect_type, "execute");
    let affects_row_count = matches!(effect_type, "Filter") || condition_type.contains("Filter");

    let confidence = if !json_path.is_empty() && !raw_expr.is_empty() {
        "high"
    } else {
        "medium"
    };

    let mut diagnostics: Vec<serde_json::Value> = Vec::new();
    if raw_expr.trim().is_empty() {
        diagnostics.push(serde_json::json!({
            "code": "EMPTY_CONDITION",
            "message": "条件表达式为空",
        }));
    }

    Some(serde_json::json!({
        "kind": condition_type,
        "target": owner_id,
        "owner_type": owner_type,
        "subject_type": subject_type,
        "effect_type": effect_type,
        "raw_expr": raw_expr,
        "normalized_expr": normalized_expr,
        "json_path": json_path,
        "source_file": source_file,
        "depends_on": referenced_symbols,
        "impact": {
            "affected_component_count": affected_component_count,
            "affected_model_count": affected_model_count,
            "is_entrypoint": is_entrypoint,
            "is_main_panel": is_main_panel,
            "affects_row_count": affects_row_count,
            "impact_score": affected_model_count + affected_component_count + if is_entrypoint { 3 } else if is_main_panel { 2 } else { 1 },
        },
        "evidence": {
            "node_id": cond_node.id,
            "edge_type": format!("{:?}", edge.edge_type),
            "json_path": json_path,
            "raw_expr": raw_expr,
            "source_file": cond_node.path,
        },
        "confidence": confidence,
        "diagnostics": diagnostics,
    }))
}

fn sort_by_impact(prereqs: &mut Vec<serde_json::Value>) {
    prereqs.sort_by(|a, b| {
        let a_score = a
            .get("impact")
            .and_then(|v| v.get("impact_score"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let b_score = b
            .get("impact")
            .and_then(|v| v.get("impact_score"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        b_score.cmp(&a_score)
    });
}
