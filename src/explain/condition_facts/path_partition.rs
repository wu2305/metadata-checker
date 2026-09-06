use crate::answer_contract::TraversalIntent;

fn path_edge_types(path: &serde_json::Value) -> Vec<String> {
    path.get("segments")
        .and_then(|v| v.as_array())
        .map(|segments| {
            segments
                .iter()
                .filter_map(|segment| {
                    segment
                        .get("edge")
                        .and_then(|edge| edge.get("edge_type"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn path_has_any_edge(path: &serde_json::Value, edge_types: &[&str]) -> bool {
    let edges = path_edge_types(path);
    edges
        .iter()
        .any(|edge| edge_types.iter().any(|allowed| edge == allowed))
}

fn path_reject_reason_for_intent(
    path: &serde_json::Value,
    intent: TraversalIntent,
) -> Option<&'static str> {
    let edges = path_edge_types(path);
    if edges.is_empty() {
        return Some("missing_path_segments");
    }

    match intent {
        TraversalIntent::Display => {
            if edges
                .iter()
                .all(|edge| matches!(edge.as_str(), "DependsOn" | "Contains"))
            {
                None
            } else {
                Some("edge_type_not_allowed_for_display_intent")
            }
        }
        TraversalIntent::ValueSource => {
            if edges.iter().all(|edge| {
                matches!(
                    edge.as_str(),
                    "Reads" | "ActionReads" | "FieldAlias" | "DataflowOutput" | "DataflowInput"
                )
            }) {
                None
            } else {
                Some("edge_type_not_allowed_for_value_source_intent")
            }
        }
        TraversalIntent::Writer => {
            let has_writer_edge =
                path_has_any_edge(path, &["Writes", "ActionWrites", "FieldWrite"]);
            // Contains：model→field 结构性桥接，与 Reads/FieldAlias 同属 writer 链路允许边。
            let only_writer_chain_edges = edges.iter().all(|edge| {
                matches!(
                    edge.as_str(),
                    "Reads" | "Contains" | "FieldAlias" | "Writes" | "ActionWrites" | "FieldWrite"
                )
            });
            if has_writer_edge && only_writer_chain_edges {
                None
            } else if !has_writer_edge {
                Some("writer_intent_requires_write_edge")
            } else {
                Some("edge_type_not_allowed_for_writer_intent")
            }
        }
        TraversalIntent::Availability => {
            if edges
                .iter()
                .all(|edge| matches!(edge.as_str(), "DependsOn" | "DataflowInput"))
            {
                None
            } else {
                Some("edge_type_not_allowed_for_availability_intent")
            }
        }
        TraversalIntent::Action => {
            if edges
                .iter()
                .all(|edge| matches!(edge.as_str(), "Triggers" | "ActionWrites" | "DependsOn"))
            {
                None
            } else {
                Some("edge_type_not_allowed_for_action_intent")
            }
        }
        TraversalIntent::Context | TraversalIntent::Auto => None,
    }
}

fn rejected_path(path: serde_json::Value, reject_reason: &str) -> serde_json::Value {
    let mut value = path;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "reject_reason".to_string(),
            serde_json::json!(reject_reason),
        );
    }
    value
}

/// 将路径选择结果按当前 intent 分为证明、候选、拒绝三类
pub(in crate::explain) fn partition_paths_for_intent(
    selection: crate::path::PathSelectionResult,
    intent: TraversalIntent,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
) {
    let mut proven_paths = Vec::new();
    let mut candidate_paths = Vec::new();
    let mut rejected_paths = Vec::new();

    for path in selection.primary_paths {
        let path_json = path.to_json();
        if let Some(reason) = path_reject_reason_for_intent(&path_json, intent) {
            rejected_paths.push(rejected_path(path_json, reason));
        } else {
            proven_paths.push(path_json);
        }
    }

    for path in selection
        .candidate_paths
        .into_iter()
        .chain(selection.supporting_paths)
        .chain(selection.related_context)
    {
        let path_json = path.to_json();
        if let Some(reason) = path_reject_reason_for_intent(&path_json, intent) {
            rejected_paths.push(rejected_path(path_json, reason));
        } else {
            candidate_paths.push(path_json);
        }
    }

    for path in selection.rejected_paths {
        rejected_paths.push(rejected_path(path.to_json(), "path_selector_rejected"));
    }

    (proven_paths, candidate_paths, rejected_paths)
}
