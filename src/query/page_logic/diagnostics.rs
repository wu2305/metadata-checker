use crate::graph::Node;
use crate::graph_store::GraphReadStore;
use crate::output::{Diagnostic, DiagnosticSeverity, Location};

use super::pick_str_field;

/// evidence 中每类明细的默认采样上限
pub(super) const EVIDENCE_SAMPLE_LIMIT: usize = 5;

/// 页面逻辑诊断结果
pub(super) struct PageLogicDiagnostics {
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) related_context_summary: serde_json::Value,
}

/// 构建页面逻辑风险诊断，并执行主路径截断降级
pub(super) fn build_page_logic_diagnostics(
    graph: &dyn GraphReadStore,
    page_id: &str,
    page_node: &Node,
    from_file: bool,
    entrypoints: &[serde_json::Value],
    data_sources: &[serde_json::Value],
    write_targets: &[serde_json::Value],
    navigation: &[serde_json::Value],
    visibility_rules: &[serde_json::Value],
    action_flows: &[serde_json::Value],
    primary_paths: &mut Vec<serde_json::Value>,
    related_context: &mut Vec<serde_json::Value>,
) -> PageLogicDiagnostics {
    // ---- 6. Risk diagnostics ----
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    // 注意力漂移治理：限制 primary_paths 数量，超出的旁路关系降级到 related_context
    const PRIMARY_PATH_LIMIT: usize = 50;
    if primary_paths.len() > PRIMARY_PATH_LIMIT {
        let overflow = primary_paths.split_off(PRIMARY_PATH_LIMIT);
        let overflow_count = overflow.len();
        for mut path in overflow {
            if let Some(obj) = path.as_object_mut() {
                obj.insert(
                    "reason".to_string(),
                    serde_json::json!("旁路关系：超出主链路限制"),
                );
                obj.insert("confidence".to_string(), serde_json::json!("low"));
            }
            related_context.push(path);
        }
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: "PRIMARY_PATHS_TRUNCATED".to_string(),
            message: format!(
                "Primary paths limited to {}; {} overflow relations moved to related_context",
                PRIMARY_PATH_LIMIT, overflow_count
            ),
            location: Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(,
            count: None,
            answer_impact: None,
            first_seen_phase: None,
                "Use --budget full to see more relations, or focus on key_primary_paths in summary"
                    .to_string(),
            ),
        });
    }
    // ---- 5.7 注意力漂移治理：旁路关系统计（必须在 truncation 之后）
    let related_context_summary = serde_json::json!({
        "total_count": related_context.len(),
        "by_type": {
            "OpensPage": related_context.iter().filter(|v| v.get("edge_type").and_then(|e| e.as_str()) == Some("OpensPage")).count(),
            "EmbedsPage": related_context.iter().filter(|v| v.get("edge_type").and_then(|e| e.as_str()) == Some("EmbedsPage")).count(),
            "other": related_context.iter().filter(|v| {
                let et = v.get("edge_type").and_then(|e| e.as_str()).unwrap_or("");
                et != "OpensPage" && et != "EmbedsPage"
            }).count(),
        },
        "note": "related_context 不是必要条件，仅作参考",
    });

    if write_targets.is_empty() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: "NO_WRITE_TARGETS".to_string(),
            message: "Page has no detected write targets".to_string(),
            location: Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Verify if page is read-only or actions are not parsed".to_string()),
            count: None,
            answer_impact: None,
            first_seen_phase: None,
        });
    }

    if entrypoints.is_empty() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "NO_ENTRYPOINTS".to_string(),
            message: "Page has no detected user entrypoints (buttons, links, etc.)".to_string(),
            location: Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: Some("canvas.components[*].actions[*]".to_string()),
            },
            suggestion: Some("Check component action definitions".to_string()),
            count: None,
            answer_impact: None,
            first_seen_phase: None,
        });
    }

    if !from_file {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Warning,
            code: "PAGE_INPUTS_DEFERRED".to_string(),
            message: "Raw page file is unavailable; page_inputs/visibility_rules may be incomplete"
                .to_string(),
            location: Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(,
            count: None,
            answer_impact: None,
            first_seen_phase: None,
                "Ensure --project-dir points to the project root containing this page file"
                    .to_string(),
            ),
        });
    }

    // UNRESOLVED_PAGE_NAVIGATION：导航目标页面不存在于图中
    for nav in navigation {
        let to = nav.get("to").and_then(|v| v.as_str()).unwrap_or("");
        if to.starts_with("page:") && graph.get_node(to).is_ok_and(|node| node.is_none()) {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "UNRESOLVED_PAGE_NAVIGATION".to_string(),
                message: format!("Navigation target page '{}' not found in graph", to),
                location: Location {
                    source_file: nav
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: nav
                        .get("from")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: nav
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Check if target page exists in project".to_string()),
                count: None,
                answer_impact: None,
                first_seen_phase: None,
            });
        }
    }

    // UNRESOLVED_MODEL_WRITE：写入目标模型不存在于图中
    for wt in write_targets {
        let target_id = wt.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if target_id.starts_with("model:")
            && graph.get_node(target_id).is_ok_and(|node| node.is_none())
        {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "UNRESOLVED_MODEL_WRITE".to_string(),
                message: format!("Write target model '{}' not found in graph", target_id),
                location: Location {
                    source_file: wt
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: pick_str_field(wt, &["source_action", "source_component"])
                        .map(ToString::to_string),
                    json_path: wt
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Check if target model exists in project sources".to_string()),
                count: None,
                answer_impact: None,
                first_seen_phase: None,
            });
        }
    }

    // VISIBILITY_RULE_UNRESOLVED：visibility 规则中的表达式包含未解析或多义引用
    for rule in visibility_rules {
        let has_unresolved = rule
            .get("unresolved_refs")
            .and_then(|v| v.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        let has_unresolved_diag = rule
            .get("diagnostics")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter().any(|d| {
                    d.get("code").and_then(|v| v.as_str()) == Some("EXPR_UNRESOLVED_REF")
                        || d.get("code").and_then(|v| v.as_str()) == Some("EXPR_AMBIGUOUS_REF")
                })
            })
            .unwrap_or(false);
        if has_unresolved || has_unresolved_diag {
            let expr = rule
                .get("raw_expr")
                .and_then(|v| v.as_str())
                .or_else(|| rule.get("expression").and_then(|v| v.as_str()))
                .unwrap_or("<non-string expression>");
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "VISIBILITY_RULE_UNRESOLVED".to_string(),
                message: format!(
                    "Visibility rule '{}' contains unresolved or ambiguous references",
                    expr
                ),
                location: Location {
                    source_file: rule
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: rule
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: rule
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some(,
                count: None,
                answer_impact: None,
                first_seen_phase: None,
                    "Review the expression and verify each referenced component/model exists"
                        .to_string(),
                ),
            });
        }
    }

    // ACTION_FLOW_INCOMPLETE：仅对“通常应产生副作用”的动作类型发出提示
    for flow in action_flows {
        let reads = flow
            .get("reads")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let writes = flow
            .get("writes")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let nav_count = flow
            .get("navigation")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let action_type = flow
            .get("action_type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let action_category = flow
            .get("action_category")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let is_query_refresh_like = matches!(
            action_type,
            "loadData" | "resetData" | "refreshData" | "refreshModels" | "newData" | "validateData"
        ) || matches!(
            action_category,
            "data_read" | "data_refresh" | "data_initialization" | "validation"
        );
        let expects_side_effect = matches!(
            action_category,
            "data_write" | "param_mutation" | "api_call"
        ) || matches!(
            action_type,
            "submitData" | "insertData" | "updateData" | "deleteData" | "setParamValue" | "webAPI"
        );
        if reads > 0
            && writes == 0
            && nav_count == 0
            && expects_side_effect
            && !is_query_refresh_like
        {
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Info,
                code: "ACTION_FLOW_INCOMPLETE".to_string(),
                message: format!(
                    "Action {} reads but does not write; may be a query-only action",
                    flow.get("action_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ),
                location: Location {
                    source_file: flow
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    node_id: flow
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    json_path: flow
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                },
                suggestion: Some("Verify if this action should produce a write target".to_string()),
                count: None,
                answer_impact: None,
                first_seen_phase: None,
            });
        }
    }

    // UNKNOWN_ACTION_TYPE：存在未识别的 action 类型（聚合同类，避免刷屏）
    {
        let mut unknown_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut unknown_examples: std::collections::HashMap<
            String,
            (String, Option<String>, Option<String>, Option<String>),
        > = std::collections::HashMap::new();
        for flow in action_flows {
            if flow.get("action_category").and_then(|v| v.as_str()) == Some("unknown") {
                let atype = flow
                    .get("action_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string();
                *unknown_counts.entry(atype.clone()).or_insert(0) += 1;
                if !unknown_examples.contains_key(&atype) {
                    unknown_examples.insert(
                        atype,
                        (
                            flow.get("source_file")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            flow.get("component_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                            flow.get("json_path")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                            flow.get("action_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                        ),
                    );
                }
            }
        }
        for (atype, count) in unknown_counts {
            let (source_file, node_id, json_path, _action_id) =
                unknown_examples.get(&atype).cloned().unwrap_or_default();
            diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "UNKNOWN_ACTION_TYPE".to_string(),
                message: if count > 1 {
                    format!(
                        "Unknown action type '{}' encountered ({} occurrences)",
                        atype, count
                    )
                } else {
                    format!("Unknown action type '{}' encountered", atype)
                },
                location: Location {
                    source_file: Some(source_file),
                    node_id,
                    json_path,
                },
                suggestion: Some(,
                count: None,
                answer_impact: None,
                first_seen_phase: None,
                    "Check if this action type is supported by metadata-checker".to_string(),
                ),
            });
        }
    }

    // EVIDENCE_SAMPLED：明细数量大于 evidence 展开上限时提示 evidence 非全集
    let evidence_sample_limit = EVIDENCE_SAMPLE_LIMIT;
    let sampling_categories = [
        ("entrypoints", entrypoints.len()),
        ("data_sources", data_sources.len()),
        ("write_targets", write_targets.len()),
        ("navigation", navigation.len()),
        ("visibility_rules", visibility_rules.len()),
        ("action_flows", action_flows.len()),
    ];
    let sampled_parts: Vec<String> = sampling_categories
        .iter()
        .filter(|(_, size)| *size > evidence_sample_limit)
        .map(|(name, size)| format!("{name} {size}>{evidence_sample_limit}"))
        .collect();
    if !sampled_parts.is_empty() {
        diagnostics.push(Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: "EVIDENCE_SAMPLED".to_string(),
            message: format!(
                "Evidence includes only a sample for: {}",
                sampled_parts.join(", ")
            ),
            location: Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(,
            count: None,
            answer_impact: None,
            first_seen_phase: None,
                "Use details arrays for full coverage; evidence is intentionally low-noise sampled"
                    .to_string(),
            ),
        });
    }

    PageLogicDiagnostics {
        diagnostics,
        related_context_summary,
    }
}
