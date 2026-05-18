use crate::graph::GraphDB;
use crate::output::schema::{format_next_query, format_next_query_multi};
use anyhow::Result;
use serde_json::json;
use std::io::{self, Write};

mod graph_collect;
mod metadata;
mod path_summary;

/// 提取完整节点 ID 中的组件裸 ID（如 comp:app/a.spg|button1 -> button1）
fn component_short_id(node_id: &str) -> &str {
    node_id.split('|').next_back().unwrap_or(node_id)
}

/// 为 action 构造稳定的近似 json_path
fn action_json_path(component_id: &str, action_id: &str) -> String {
    format!(
        "canvas.components[id='{}'].actions[id='{}']",
        component_short_id(component_id),
        action_id
    )
}

/// 从边元数据提取字符串字段
fn edge_meta_str(edge: &crate::graph::Edge, key: &str) -> Option<String> {
    edge.meta
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

/// 按优先顺序提取对象中的字符串字段，自动跳过 null/非字符串
fn pick_str_field<'a>(obj: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| obj.get(*key).and_then(|v| v.as_str()))
}

/// 查询页面级逻辑摘要
///
/// 输出 page_inputs、data_sources、write_targets、entrypoints、action_flows、visibility_rules、navigation、risk_diagnostics。
pub fn build_query_page_logic_output(
    graph: &GraphDB,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<serde_json::Value> {
    let is_compact = budget == "compact";
    let is_full = budget == "full";
    let page_node = match graph.get_node(page_id) {
        Some(n) => n,
        None => {
            let candidates = graph.find_candidates(page_id, 5);
            let out = crate::output::schema::build_target_not_found_output(
                crate::output::schema::OutputKind::PageQuery,
                page_id,
                &candidates,
            );
            return Ok(serde_json::to_value(out)?);
        }
    };

    // ---- 1. 递归收集页面下所有 Component 节点，再收集它们 Triggers 出的 Action 节点 ----
    let graph_collect::PageLogicNodes {
        child_components,
        child_actions,
    } = graph_collect::collect_page_logic_nodes(graph, page_id);

    // ---- 2. 从原始文件读取：递归收集组件元数据、action 元数据、visibility_rules ----
    let metadata::PageFileMetadata {
        page_inputs,
        visibility_rules,
        from_file,
        component_json_paths,
        action_meta,
    } = metadata::load_page_file_metadata(project_dir, &page_node.path);

    // ---- 3. Entrypoints：只包含用户可触发组件（有 action 的 button/link 等） ----
    let mut entrypoints: Vec<serde_json::Value> = Vec::new();
    for comp in &child_components {
        if let Some((comp_out, _)) = graph.get_node_edges(&comp.id) {
            let has_trigger = comp_out
                .iter()
                .any(|(_, e)| matches!(e.edge_type, crate::graph::EdgeType::Triggers));
            if has_trigger {
                let comp_type = comp
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("component_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("component");
                let component_id = component_short_id(&comp.id);
                let json_path = component_json_paths
                    .get(component_id)
                    .cloned()
                    .unwrap_or_else(|| {
                        format!("canvas.components[id='{}']", component_short_id(&comp.id))
                    });
                entrypoints.push(json!({
                    "id": comp.id,
                    "name": comp.name,
                    "type": comp_type,
                    "source_file": comp.path,
                    "node_id": comp.id,
                    "edge_type": "Triggers",
                    "raw_expr": serde_json::Value::Null,
                    "json_path": format!("{}.actions[*]", json_path),
                }));
            }
        }
    }

    // ---- 4. 收集所有 Component & Action 的出边，用于 data_sources / write_targets / navigation ----
    let mut data_sources: Vec<serde_json::Value> = Vec::new();
    let mut write_targets: Vec<serde_json::Value> = Vec::new();
    let mut navigation: Vec<serde_json::Value> = Vec::new();
    let mut action_flows: Vec<serde_json::Value> = Vec::new();

    let mut all_nodes: Vec<&crate::graph::Node> = Vec::new();
    all_nodes.extend(child_components.iter().copied());
    all_nodes.extend(child_actions.iter().copied());

    for node in &all_nodes {
        if let Some((node_out, _)) = graph.get_node_edges(&node.id) {
            for (target, edge) in &node_out {
                match edge.edge_type {
                    crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                        let default_json_path =
                            if matches!(node.node_type, crate::graph::NodeType::Action) {
                                let (atype, aid) = node
                                    .name
                                    .split_once(':')
                                    .map(|(t, i)| (t.to_string(), i.to_string()))
                                    .unwrap_or_else(|| (node.name.clone(), node.name.clone()));
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.{}",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        &aid
                                    ),
                                    if atype == "setParamValue" || atype == "link" {
                                        "params[*].value"
                                    } else {
                                        "fieldValues[*].value"
                                    }
                                )
                            } else {
                                format!(
                                    "canvas.components[id='{}'].<expression>",
                                    component_short_id(&node.id)
                                )
                            };
                        data_sources.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or(default_json_path),
                        }));
                    }
                    crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                        let default_json_path =
                            if matches!(node.node_type, crate::graph::NodeType::Action) {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.fieldValues[*].value",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            } else {
                                format!(
                                    "canvas.components[id='{}'].submitField",
                                    component_short_id(&node.id)
                                )
                            };
                        write_targets.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or(default_json_path),
                        }));
                    }
                    crate::graph::EdgeType::OpensPage
                    | crate::graph::EdgeType::ActionNavigates
                    | crate::graph::EdgeType::EmbedsPage => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                if matches!(node.node_type, crate::graph::NodeType::Action) {
                                    let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                    let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                    format!(
                                        "{}.path",
                                        action_json_path(
                                            &format!("comp:{}|{}", page_node.path, parent_component),
                                            aid
                                        )
                                    )
                                } else {
                                    format!("canvas.components[id='{}'].resPath", component_short_id(&node.id))
                                }
                            }),
                        }));
                    }
                    crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::PassesParam => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                if matches!(node.node_type, crate::graph::NodeType::Action) {
                                    let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                    let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                    format!(
                                        "{}.params[*].value",
                                        action_json_path(
                                            &format!("comp:{}|{}", page_node.path, parent_component),
                                            aid
                                        )
                                    )
                                } else {
                                    format!(
                                        "canvas.components[id='{}'].params[*].value",
                                        component_short_id(&node.id)
                                    )
                                }
                            }),
                        }));
                    }
                    crate::graph::EdgeType::ActionValidates
                    | crate::graph::EdgeType::ActionLoadsData => {
                        data_sources.push(json!({
                            "source_component": if matches!(node.node_type, crate::graph::NodeType::Component) { Some(node.id.clone()) } else { None },
                            "source_action": if matches!(node.node_type, crate::graph::NodeType::Action) { Some(node.id.clone()) } else { None },
                            "model": target.name,
                            "field_path": edge.field_path,
                            "target_id": target.id,
                            "type": format!("{:?}", edge.edge_type),
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.<derived-target>",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            }),
                        }));
                    }
                    crate::graph::EdgeType::ActionControlsComponent => {
                        navigation.push(json!({
                            "from": node.id,
                            "to": target.id,
                            "to_name": target.name,
                            "type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": node.path,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "raw_expr": edge_meta_str(edge, "source_expr"),
                            "json_path": edge_meta_str(edge, "json_path").unwrap_or_else(|| {
                                let aid = node.name.split(':').nth(1).unwrap_or(node.name.as_str());
                                let parent_component = node.id.split('|').nth(1).unwrap_or("?");
                                format!(
                                    "{}.targetComponent[*]",
                                    action_json_path(
                                        &format!("comp:{}|{}", page_node.path, parent_component),
                                        aid
                                    )
                                )
                            }),
                        }));
                    }
                    _ => {}
                }
            }
        }
    }

    // ---- 5. Action flows：遍历 Action 节点，聚合 reads/writes/navigation ----
    for action in &child_actions {
        let (action_out, action_in) = graph
            .get_node_edges(&action.id)
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        let (action_type, action_id) = action
            .name
            .split_once(':')
            .map(|(t, i)| (t.to_string(), i.to_string()))
            .unwrap_or_else(|| (action.name.clone(), action.name.clone()));

        let parent_component = action_in.iter().find_map(|(src, e)| {
            if matches!(e.edge_type, crate::graph::EdgeType::Triggers)
                && matches!(src.node_type, crate::graph::NodeType::Component)
            {
                Some(src.id.clone())
            } else {
                None
            }
        });

        let mut reads = Vec::new();
        let mut writes = Vec::new();
        let mut nav = Vec::new();
        let mut sets_params = Vec::new();
        let mut passes_params = Vec::new();

        for (target, edge) in &action_out {
            match edge.edge_type {
                crate::graph::EdgeType::Reads | crate::graph::EdgeType::ActionReads => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::Writes | crate::graph::EdgeType::ActionWrites => {
                    writes.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::OpensPage
                | crate::graph::EdgeType::ActionNavigates
                | crate::graph::EdgeType::EmbedsPage => {
                    nav.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "type": format!("{:?}", edge.edge_type),
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::SetsParam | crate::graph::EdgeType::ActionSetsParam => {
                    sets_params.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "field_path": edge.field_path,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::PassesParam => {
                    passes_params.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "field_path": edge.field_path,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionValidates => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "validate": true,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionLoadsData => {
                    reads.push(json!({
                        "model": target.name,
                        "field_path": edge.field_path,
                        "target_id": target.id,
                        "load": true,
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                crate::graph::EdgeType::ActionControlsComponent => {
                    nav.push(json!({
                        "to": target.id,
                        "to_name": target.name,
                        "type": format!("{:?}", edge.edge_type),
                        "source_file": action.path,
                        "edge_type": format!("{:?}", edge.edge_type),
                        "raw_expr": edge_meta_str(edge, "source_expr"),
                        "json_path": edge_meta_str(edge, "json_path"),
                    }));
                }
                _ => {}
            }
        }

        // 用 parent_component 提取裸组件名构建 lookup key，避免同名 action 串线
        let lookup_key = parent_component
            .as_deref()
            .and_then(|cid| cid.split('|').next_back())
            .map(|comp_name| format!("{}|{}", comp_name, action_id))
            .unwrap_or_else(|| action_id.clone());
        let trigger_type = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("trigger_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("click")
            .to_string();
        let wait_prev = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("wait_prev"))
            .cloned();
        let action_path_hint = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("json_path"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                action_json_path(parent_component.as_deref().unwrap_or("?"), &action_id)
            });
        let action_source_file = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("source_file"))
            .and_then(|v| v.as_str())
            .unwrap_or(action.path.as_str())
            .to_string();
        let condition_raw = action_meta
            .get(&lookup_key)
            .and_then(|v| v.get("condition"))
            .and_then(|v| v.as_str());

        let action_category = crate::action_semantics::classify_action(&action_type);
        let comp_name = parent_component
            .as_deref()
            .and_then(|cid| cid.split('|').next_back())
            .unwrap_or("?");
        let semantic_summary = crate::action_semantics::build_semantic_summary(
            &action_type,
            comp_name,
            &action_id,
            &reads,
            &writes,
            &nav,
        );
        let blocks_on =
            crate::action_semantics::parse_wait_prev(wait_prev.as_ref().and_then(|v| v.as_str()));
        let condition_struct = crate::action_semantics::parse_condition(condition_raw);

        // 执行模型字段推断
        let wait_status = if wait_prev.as_ref().and_then(|v| v.as_str()).is_some() {
            "blocking"
        } else {
            "none"
        };
        let may_interrupt = condition_raw.is_some();
        let failure_behavior = if condition_raw.is_some() {
            "skip_if_condition_fails"
        } else {
            "proceed"
        };

        action_flows.push(json!({
            "action_id": action_id,
            "action_type": action_type,
            "action_category": action_category,
            "semantic_summary": semantic_summary,
            "component_id": parent_component,
            "trigger_type": trigger_type,
            "blocks_on": blocks_on,
            "wait_status": wait_status,
            "condition": condition_struct,
            "may_interrupt": may_interrupt,
            "failure_behavior": failure_behavior,
            "reads": reads,
            "writes": writes,
            "navigation": nav,
            "sets_params": sets_params,
            "passes_params": passes_params,
            "source_file": action_source_file,
            "node_id": action.id,
            "json_path": action_path_hint,
        }));
    }

    // ---- 5.5 收集页面级条件前置条件（M19） ----
    let mut display_prerequisites: Vec<serde_json::Value> = Vec::new();
    let mut data_prerequisites: Vec<serde_json::Value> = Vec::new();
    let mut action_prerequisites: Vec<serde_json::Value> = Vec::new();
    let mut seen_conditions: std::collections::HashSet<String> = std::collections::HashSet::new();

    /// 从 Condition 节点构建 prerequisite 结构
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

        // 影响范围计算：统计引用的模型数量和组件数量
        let affected_model_count = referenced_symbols
            .iter()
            .filter(|s| s.starts_with("model:"))
            .count();
        let affected_component_count = referenced_symbols
            .iter()
            .filter(|s| s.starts_with("comp:"))
            .count();
        let is_entrypoint = matches!(subject_type, "action") || matches!(effect_type, "execute");
        let is_main_panel =
            matches!(subject_type, "component") && !matches!(effect_type, "execute");
        let affects_row_count =
            matches!(effect_type, "Filter") || condition_type.contains("Filter");

        // confidence：有原始表达式和 JSON 路径时为 high
        let confidence = if !json_path.is_empty() && !raw_expr.is_empty() {
            "high"
        } else {
            "medium"
        };

        // 诊断：空表达式
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

    // 辅助：遍历节点入边，收集 condition
    let mut collect_from_node = |node_id: &str| {
        if let Some((_out, incoming)) = graph.get_node_edges(node_id) {
            for (source, edge) in &incoming {
                if matches!(source.node_type, crate::graph::NodeType::Condition)
                    && matches!(edge.edge_type, crate::graph::EdgeType::DependsOn)
                    && !seen_conditions.contains(&source.id)
                    && source.path == page_node.path
                {
                    seen_conditions.insert(source.id.clone());
                    if let Some(prereq) = build_prerequisite(source, edge) {
                        let kind = prereq
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        match kind {
                            "VisibleCondition" | "EnableCondition" | "MaskCondition" => {
                                display_prerequisites.push(prereq);
                            }
                            "SourceFilterExp" | "SourceFilterClause" | "ItemFilter"
                            | "CalcCondition" | "CalcExp" | "DefaultValueExp" | "FieldExp" => {
                                data_prerequisites.push(prereq);
                            }
                            "ActionConditionExp"
                            | "ActionCondition"
                            | "SubmitCondition"
                            | "SubmitPageCondition" => {
                                action_prerequisites.push(prereq);
                            }
                            _ => {
                                // 默认归入 display_prerequisites
                                display_prerequisites.push(prereq);
                            }
                        }
                    }
                }
            }
        }
    };

    // 从组件收集
    for comp in &child_components {
        collect_from_node(&comp.id);
    }
    // 从动作收集
    for action in &child_actions {
        collect_from_node(&action.id);
    }
    // 从数据源模型收集（model filter）
    for ds in &data_sources {
        if let Some(target_id) = ds.get("target_id").and_then(|v| v.as_str()) {
            collect_from_node(target_id);
        }
    }

    // 排序：按 impact_score 降序
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
    sort_by_impact(&mut display_prerequisites);
    sort_by_impact(&mut data_prerequisites);
    sort_by_impact(&mut action_prerequisites);

    // ---- 5.6 主链路抽取（M19.5）—— 使用路径计算领域模型 ----
    let path_summary::PageLogicPaths {
        mut primary_paths,
        mut related_context,
        candidate_paths,
        supporting_paths,
        rejected_paths,
        path_selection_diagnostics,
    } = path_summary::build_page_logic_paths(
        graph,
        page_id,
        &page_node,
        &child_components,
        &child_actions,
        &data_sources,
        &write_targets,
        &entrypoints,
    );

    // ---- 6. Risk diagnostics ----
    let mut diagnostics: Vec<crate::output::Diagnostic> = Vec::new();
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
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "PRIMARY_PATHS_TRUNCATED".to_string(),
            message: format!(
                "Primary paths limited to {}; {} overflow relations moved to related_context",
                PRIMARY_PATH_LIMIT, overflow_count
            ),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(
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
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "NO_WRITE_TARGETS".to_string(),
            message: "Page has no detected write targets".to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some("Verify if page is read-only or actions are not parsed".to_string()),
        });
    }

    if entrypoints.is_empty() {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "NO_ENTRYPOINTS".to_string(),
            message: "Page has no detected user entrypoints (buttons, links, etc.)".to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: Some("canvas.components[*].actions[*]".to_string()),
            },
            suggestion: Some("Check component action definitions".to_string()),
        });
    }

    if !from_file {
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Warning,
            code: "PAGE_INPUTS_DEFERRED".to_string(),
            message: "Raw page file is unavailable; page_inputs/visibility_rules may be incomplete"
                .to_string(),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(
                "Ensure --project-dir points to the project root containing this page file"
                    .to_string(),
            ),
        });
    }

    // UNRESOLVED_PAGE_NAVIGATION：导航目标页面不存在于图中
    for nav in &navigation {
        let to = nav.get("to").and_then(|v| v.as_str()).unwrap_or("");
        if to.starts_with("page:") && graph.get_node(to).is_none() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNRESOLVED_PAGE_NAVIGATION".to_string(),
                message: format!("Navigation target page '{}' not found in graph", to),
                location: crate::output::Location {
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
            });
        }
    }

    // UNRESOLVED_MODEL_WRITE：写入目标模型不存在于图中
    for wt in &write_targets {
        let target_id = wt.get("target_id").and_then(|v| v.as_str()).unwrap_or("");
        if target_id.starts_with("model:") && graph.get_node(target_id).is_none() {
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNRESOLVED_MODEL_WRITE".to_string(),
                message: format!("Write target model '{}' not found in graph", target_id),
                location: crate::output::Location {
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
            });
        }
    }

    // VISIBILITY_RULE_UNRESOLVED：visibility 规则中的表达式包含未解析或多义引用
    for rule in &visibility_rules {
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
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "VISIBILITY_RULE_UNRESOLVED".to_string(),
                message: format!(
                    "Visibility rule '{}' contains unresolved or ambiguous references",
                    expr
                ),
                location: crate::output::Location {
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
                suggestion: Some(
                    "Review the expression and verify each referenced component/model exists"
                        .to_string(),
                ),
            });
        }
    }

    // ACTION_FLOW_INCOMPLETE：仅对“通常应产生副作用”的动作类型发出提示
    for flow in &action_flows {
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
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "ACTION_FLOW_INCOMPLETE".to_string(),
                message: format!(
                    "Action {} reads but does not write; may be a query-only action",
                    flow.get("action_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                ),
                location: crate::output::Location {
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
        for flow in &action_flows {
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
            diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Warning,
                code: "UNKNOWN_ACTION_TYPE".to_string(),
                message: if count > 1 {
                    format!(
                        "Unknown action type '{}' encountered ({} occurrences)",
                        atype, count
                    )
                } else {
                    format!("Unknown action type '{}' encountered", atype)
                },
                location: crate::output::Location {
                    source_file: Some(source_file),
                    node_id,
                    json_path,
                },
                suggestion: Some(
                    "Check if this action type is supported by metadata-checker".to_string(),
                ),
            });
        }
    }

    // EVIDENCE_SAMPLED：明细数量大于 evidence 展开上限时提示 evidence 非全集
    let evidence_sample_limit = 5usize;
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
        diagnostics.push(crate::output::Diagnostic {
            severity: crate::output::DiagnosticSeverity::Info,
            code: "EVIDENCE_SAMPLED".to_string(),
            message: format!(
                "Evidence includes only a sample for: {}",
                sampled_parts.join(", ")
            ),
            location: crate::output::Location {
                source_file: Some(page_node.path.clone()),
                node_id: Some(page_id.to_string()),
                json_path: None,
            },
            suggestion: Some(
                "Use details arrays for full coverage; evidence is intentionally low-noise sampled"
                    .to_string(),
            ),
        });
    }

    // ---- 7. Summary & page_role ----
    let page_role = if entrypoints.is_empty() {
        "readonly_dashboard"
    } else if !write_targets.is_empty() && !navigation.is_empty() {
        "mixed_interaction_page"
    } else if !write_targets.is_empty() {
        "data_maintenance_page"
    } else if !navigation.is_empty() {
        "navigation_page"
    } else {
        "unknown"
    };

    let what_is_it = format!(
        "页面 {}，{} 个用户入口，读取 {} 个数据源，写入 {} 个目标，{} 个跳转",
        page_node.name,
        entrypoints.len(),
        data_sources.len(),
        write_targets.len(),
        navigation.len()
    );

    // Build top-N lists for brief mode
    let top_entrypoints: Vec<serde_json::Value> = entrypoints.iter().take(3).cloned().collect();
    let top_data_sources: Vec<serde_json::Value> = data_sources.iter().take(3).cloned().collect();
    let top_writes: Vec<serde_json::Value> = write_targets.iter().take(3).cloned().collect();
    let top_navigation: Vec<serde_json::Value> = navigation.iter().take(3).cloned().collect();

    // Build top-N prerequisite lists for readable summary
    let top_display_prerequisites: Vec<serde_json::Value> =
        display_prerequisites.iter().take(3).cloned().collect();
    let top_data_prerequisites: Vec<serde_json::Value> =
        data_prerequisites.iter().take(3).cloned().collect();
    let top_action_prerequisites: Vec<serde_json::Value> =
        action_prerequisites.iter().take(3).cloned().collect();
    // key_primary_paths 从已排序的 primary_paths 中取前 10 条（已按重要性排序）
    let key_primary_paths: Vec<serde_json::Value> =
        primary_paths.iter().take(10).cloned().collect();

    let summary = serde_json::json!({
        "page_id": page_id,
        "page_name": page_node.name,
        "what_is_it": what_is_it,
        "page_role": page_role,
        "entrypoint_count": entrypoints.len(),
        "data_source_count": data_sources.len(),
        "write_target_count": write_targets.len(),
        "navigation_count": navigation.len(),
        "risk_count": diagnostics.len(),
        "top_entrypoints": top_entrypoints,
        "top_data_sources": top_data_sources,
        "top_writes": top_writes,
        "top_navigation": top_navigation,
        "display_prerequisites_count": display_prerequisites.len(),
        "data_prerequisites_count": data_prerequisites.len(),
        "action_prerequisites_count": action_prerequisites.len(),
        "primary_paths_count": primary_paths.len(),
        "related_context_count": related_context.len(),
        "top_display_prerequisites": top_display_prerequisites,
        "top_data_prerequisites": top_data_prerequisites,
        "top_action_prerequisites": top_action_prerequisites,
        "key_primary_paths": key_primary_paths,
    });

    let risk_diagnostics: Vec<serde_json::Value> = diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "severity": format!("{:?}", d.severity),
                "code": d.code,
                "message": d.message,
                "location": d.location,
                "suggestion": d.suggestion,
            })
        })
        .collect();

    // M19-FIX: budget 差异输出
    let details = if is_compact {
        serde_json::json!({
            "page_inputs": crate::output::brief::truncated_array(&page_inputs, 5),
            "data_sources": crate::output::brief::truncated_array(&data_sources, 5),
            "write_targets": crate::output::brief::truncated_array(&write_targets, 5),
            "entrypoints": crate::output::brief::truncated_array(&entrypoints, 5),
            "action_flows": crate::output::brief::truncated_array(&action_flows, 5),
            "visibility_rules": crate::output::brief::truncated_array(&visibility_rules, 5),
            "navigation": crate::output::brief::truncated_array(&navigation, 5),
            "risk_diagnostics": crate::output::brief::truncated_array(&risk_diagnostics, 10),
            "display_prerequisites": crate::output::brief::truncated_array(&display_prerequisites, 5),
            "data_prerequisites": crate::output::brief::truncated_array(&data_prerequisites, 5),
            "action_prerequisites": crate::output::brief::truncated_array(&action_prerequisites, 5),
            "primary_paths": crate::output::brief::truncated_array(&primary_paths, 5),
            "related_context": crate::output::brief::truncated_array(&related_context, 3),
            "related_context_summary": related_context_summary.clone(),
        })
    } else if is_full {
        // full 模式：输出完整路径分类 + rejected + diagnostics
        serde_json::json!({
            "page_inputs": page_inputs.clone(),
            "data_sources": data_sources.clone(),
            "write_targets": write_targets.clone(),
            "entrypoints": entrypoints.clone(),
            "action_flows": action_flows.clone(),
            "visibility_rules": visibility_rules.clone(),
            "navigation": navigation.clone(),
            "risk_diagnostics": risk_diagnostics,
            "display_prerequisites": display_prerequisites.clone(),
            "data_prerequisites": data_prerequisites.clone(),
            "action_prerequisites": action_prerequisites.clone(),
            "primary_paths": primary_paths.clone(),
            "candidate_paths": candidate_paths.clone(),
            "supporting_paths": supporting_paths.clone(),
            "related_context": related_context.clone(),
            "rejected_paths": rejected_paths.clone(),
            "path_selection_diagnostics": path_selection_diagnostics.clone(),
            "related_context_summary": related_context_summary.clone(),
        })
    } else {
        // normal 模式：输出 primary + supporting，不输出 rejected
        serde_json::json!({
            "page_inputs": page_inputs.clone(),
            "data_sources": data_sources.clone(),
            "write_targets": write_targets.clone(),
            "entrypoints": entrypoints.clone(),
            "action_flows": action_flows.clone(),
            "visibility_rules": visibility_rules.clone(),
            "navigation": navigation.clone(),
            "risk_diagnostics": risk_diagnostics,
            "display_prerequisites": display_prerequisites.clone(),
            "data_prerequisites": data_prerequisites.clone(),
            "action_prerequisites": action_prerequisites.clone(),
            "primary_paths": primary_paths.clone(),
            "supporting_paths": supporting_paths.clone(),
            "related_context": related_context.clone(),
            "related_context_summary": related_context_summary.clone(),
        })
    };

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::PageLogic, summary);
    output.query_target = Some(page_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics.clone();

    output.evidence.push(
        crate::output::Evidence::new(
            format!("Page {} has {} entrypoints", page_id, entrypoints.len()),
            "Graph traversal: child components with Triggers edges",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(page_id)
        .with_source_file(&page_node.path)
        .with_json_path("canvas.components[*].actions[*]")
        .with_edge_type("Triggers"),
    );
    for ep in entrypoints.iter().take(evidence_sample_limit) {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Entrypoint {} ({}) can trigger page logic",
                    ep.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                    ep.get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("component")
                ),
                "Component has action definitions in page metadata",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_source_file(
                ep.get("source_file")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&page_node.path),
            )
            .with_node_id(ep.get("id").and_then(|v| v.as_str()).unwrap_or("?"))
            .with_edge_type(
                ep.get("edge_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Triggers"),
            )
            .with_json_path(
                ep.get("json_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("canvas.components[*].actions[*]"),
            ),
        );
    }
    if !data_sources.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} reads from {} data sources",
                    page_id,
                    data_sources.len()
                ),
                "Graph traversal: Reads edges from page children",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Reads/ActionReads"),
        );
        for ds in data_sources.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Read {} via {}",
                        ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?"),
                        pick_str_field(ds, &["source_action", "source_component"]).unwrap_or("?")
                    ),
                    "Relation extracted from graph edge under page scope",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    ds.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    pick_str_field(ds, &["source_action", "source_component"]).unwrap_or("?"),
                )
                .with_edge_type(
                    ds.get("edge_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Reads"),
                )
                .with_raw_expr(
                    ds.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .unwrap_or(ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?")),
                )
                .with_json_path(
                    ds.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].<expression>"),
                ),
            );
        }
    }
    if !write_targets.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} writes to {} targets", page_id, write_targets.len()),
                "Graph traversal: Writes/ActionWrites edges from page children",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Writes/ActionWrites"),
        );
        for wt in write_targets.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Write {} via {}",
                        wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?"),
                        pick_str_field(wt, &["source_action", "source_component"]).unwrap_or("?")
                    ),
                    "Write target inferred from action/submit binding edge",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    wt.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    pick_str_field(wt, &["source_action", "source_component"]).unwrap_or("?"),
                )
                .with_edge_type(
                    wt.get("edge_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Writes"),
                )
                .with_raw_expr(
                    wt.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .unwrap_or(wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?")),
                )
                .with_json_path(
                    wt.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*].fieldValues[*].value"),
                ),
            );
        }
    }
    if !navigation.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} has {} navigation relations",
                    page_id,
                    navigation.len()
                ),
                "Graph traversal: OpensPage/EmbedsPage/Param transfer edges",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Navigation"),
        );
        for nav_item in navigation.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Navigation {} -> {} ({})",
                        nav_item.get("from").and_then(|v| v.as_str()).unwrap_or("?"),
                        nav_item.get("to").and_then(|v| v.as_str()).unwrap_or("?"),
                        nav_item.get("type").and_then(|v| v.as_str()).unwrap_or("?")
                    ),
                    "Navigation relation extracted from action/component edges",
                )
                .with_confidence(crate::output::Confidence::High)
                .with_source_file(
                    nav_item
                        .get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(nav_item.get("from").and_then(|v| v.as_str()).unwrap_or("?"))
                .with_edge_type(
                    nav_item
                        .get("edge_type")
                        .or_else(|| nav_item.get("type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Navigation"),
                )
                .with_raw_expr(
                    nav_item.get("raw_expr").and_then(|v| v.as_str()).unwrap_or(
                        nav_item
                            .get("field_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                    ),
                )
                .with_json_path(
                    nav_item
                        .get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*].path"),
                ),
            );
        }
    }
    if !visibility_rules.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Page {} has {} visibility-related rules",
                    page_id,
                    visibility_rules.len()
                ),
                "Visibility conditions are collected recursively from component tree",
            )
            .with_confidence(crate::output::Confidence::Medium)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("VisibilityRule"),
        );
        for rule in visibility_rules.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Visibility rule {}.{}",
                        rule.get("component_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        rule.get("rule").and_then(|v| v.as_str()).unwrap_or("?")
                    ),
                    "Rule originates from component metadata field",
                )
                .with_confidence(match rule.get("confidence").and_then(|v| v.as_str()) {
                    Some("high") => crate::output::Confidence::High,
                    Some("low") => crate::output::Confidence::Low,
                    _ => crate::output::Confidence::Medium,
                })
                .with_source_file(
                    rule.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(
                    rule.get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?"),
                )
                .with_edge_type("VisibilityRule")
                .with_raw_expr(
                    rule.get("raw_expr")
                        .and_then(|v| v.as_str())
                        .or_else(|| rule.get("expression").and_then(|v| v.as_str()))
                        .unwrap_or("<non-string expression>"),
                )
                .with_json_path(
                    rule.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].visible"),
                ),
            );
        }
    }
    if !action_flows.is_empty() {
        output.evidence.push(
            crate::output::Evidence::new(
                format!("Page {} has {} action flows", page_id, action_flows.len()),
                "Graph traversal: Action nodes under page",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(page_id)
            .with_source_file(&page_node.path)
            .with_edge_type("Triggers"),
        );
        for flow in action_flows.iter().take(evidence_sample_limit) {
            output.evidence.push(
                crate::output::Evidence::new(
                    format!(
                        "Action flow {} ({})",
                        flow.get("action_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?"),
                        flow.get("action_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    ),
                    "Action flow assembled from action node edges plus raw action metadata",
                )
                .with_confidence(
                    if flow.get("action_category").and_then(|v| v.as_str()) == Some("unknown") {
                        crate::output::Confidence::Low
                    } else {
                        crate::output::Confidence::High
                    },
                )
                .with_source_file(
                    flow.get("source_file")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&page_node.path),
                )
                .with_node_id(flow.get("node_id").and_then(|v| v.as_str()).unwrap_or("?"))
                .with_edge_type("Triggers")
                .with_raw_expr(
                    flow.get("condition")
                        .and_then(|v| v.get("raw_expr"))
                        .and_then(|v| v.as_str())
                        .or_else(|| {
                            flow.get("blocks_on")
                                .and_then(|v| v.get("raw"))
                                .and_then(|v| v.as_str())
                        })
                        .unwrap_or("action execution metadata"),
                )
                .with_json_path(
                    flow.get("json_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("canvas.components[*].actions[*]"),
                ),
            );
        }
    }

    let mut nq = vec![
        format_next_query("--explain {} for page semantic summary", page_id),
        format_next_query_multi(
            "--context {} --depth 2 --budget normal for surrounding context",
            &[page_id, "2"],
        ),
    ];
    let mut model_names: Vec<String> = write_targets
        .iter()
        .filter_map(|wt| wt.get("target_id").and_then(|v| v.as_str()))
        .filter(|id| id.starts_with("model:"))
        .map(|id| id.strip_prefix("model:").unwrap_or(id).to_string())
        .collect();
    model_names.sort();
    model_names.dedup();
    for m in model_names.iter().take(3) {
        nq.push(format_next_query("--query-model {} for model details", m));
    }
    output.next_queries = nq;

    // Compact mode: add OUTPUT_TRUNCATED diagnostic and evidence_summary
    if is_compact {
        let truncated_arrays = [
            ("data_sources", data_sources.len(), 5),
            ("write_targets", write_targets.len(), 5),
            ("entrypoints", entrypoints.len(), 5),
            ("action_flows", action_flows.len(), 5),
            ("visibility_rules", visibility_rules.len(), 5),
            ("navigation", navigation.len(), 5),
        ];
        let truncated_parts: Vec<String> = truncated_arrays
            .iter()
            .filter(|(_, size, limit)| *size > *limit)
            .map(|(name, size, limit)| format!("{} {}>{}", name, size, limit))
            .collect();
        if !truncated_parts.is_empty() {
            output.diagnostics.push(crate::output::Diagnostic {
                severity: crate::output::DiagnosticSeverity::Info,
                code: "OUTPUT_TRUNCATED".to_string(),
                message: format!(
                    "Compact budget: arrays truncated for: {}",
                    truncated_parts.join(", ")
                ),
                location: crate::output::Location {
                    source_file: Some(page_node.path.clone()),
                    node_id: Some(page_id.to_string()),
                    json_path: None,
                },
                suggestion: Some(
                    "Use --budget normal or --budget full to see complete arrays".to_string(),
                ),
            });
        }

        // Add evidence_summary and key_findings to summary
        let evidence_summary =
            crate::output::brief::evidence_summary(&output.evidence, evidence_sample_limit);
        let key_findings =
            crate::output::brief::build_key_findings(&output.summary, &output.diagnostics);
        if let Some(obj) = output.summary.as_object_mut() {
            obj.insert("evidence_summary".to_string(), evidence_summary);
            obj.insert("key_findings".to_string(), serde_json::json!(key_findings));
        }
        // Compact mode: truncate evidence array to limit noise
        output.evidence.truncate(evidence_sample_limit);
    }
    let output = output.validate();
    Ok(serde_json::to_value(output)?)
}

/// 查询页面级逻辑摘要
///
/// CLI 包装器，负责调用 `build_query_page_logic_output` 并按 `human` 参数决定输出格式。
pub fn query_page_logic(
    graph: &GraphDB,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    human: bool,
    budget: &str,
) -> Result<()> {
    let output = build_query_page_logic_output(graph, page_id, project_dir, budget)?;
    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Page Logic: {} ===", page_id)?;
        if let Some(summary) = output.get("summary").and_then(|s| s.as_object()) {
            if let Some(name) = summary.get("page_name").and_then(|v| v.as_str()) {
                writeln!(out, "Name: {}", name)?;
            }
            if let Some(role) = summary.get("page_role").and_then(|v| v.as_str()) {
                writeln!(out, "Role: {}", role)?;
            }
            if let Some(what) = summary.get("what_is_it").and_then(|v| v.as_str()) {
                writeln!(out, "{}", what)?;
            }
        }
        if let Some(details) = output.get("details").and_then(|d| d.as_object()) {
            if let Some(eps) = details.get("entrypoints").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Entrypoints ({}) ---",
                    eps.len()
                )?;
                for ep in eps {
                    let id = ep.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = ep.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let t = ep.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  [{}] {} ({})", id, name, t)?;
                }
            }
            if let Some(dss) = details.get("data_sources").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Data Sources ({}) ---",
                    dss.len()
                )?;
                for ds in dss {
                    let fp = ds.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {}", fp)?;
                }
            }
            if let Some(wts) = details.get("write_targets").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Write Targets ({}) ---",
                    wts.len()
                )?;
                for wt in wts {
                    let fp = wt.get("field_path").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {}", fp)?;
                }
            }
            if let Some(flows) = details.get("action_flows").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Action Flows ({}) ---",
                    flows.len()
                )?;
                for flow in flows {
                    let aid = flow
                        .get("action_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let atype = flow
                        .get("action_type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let acat = flow
                        .get("action_category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    let summary = flow
                        .get("semantic_summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let cid = flow
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?");
                    writeln!(out, "  {} [{} | {}]", aid, atype, acat)?;
                    if !summary.is_empty() {
                        writeln!(out, "    summary: {}", summary)?;
                    }
                    writeln!(out, "    triggered by {}", cid)?;
                    if let Some(raw) = flow
                        .get("blocks_on")
                        .and_then(|b| b.get("raw"))
                        .and_then(|v| v.as_str())
                    {
                        writeln!(out, "    waits for: {}", raw)?;
                    }
                    if let Some(raw) = flow
                        .get("condition")
                        .and_then(|c| c.get("raw_expr"))
                        .and_then(|v| v.as_str())
                    {
                        writeln!(out, "    condition: {}", raw)?;
                    }
                    let writes = flow
                        .get("writes")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    let nav = flow
                        .get("navigation")
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    if writes > 0 {
                        writeln!(out, "    writes: {} target(s)", writes)?;
                    }
                    if nav > 0 {
                        writeln!(out, "    navigation: {} target(s)", nav)?;
                    }
                }
            }
            if let Some(nav) = details.get("navigation").and_then(|v| v.as_array()) {
                writeln!(
                    out,
                    "
--- Navigation ({}) ---",
                    nav.len()
                )?;
                for n in nav {
                    let from = n.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                    let to = n.get("to").and_then(|v| v.as_str()).unwrap_or("?");
                    let t = n.get("type").and_then(|v| v.as_str()).unwrap_or("?");
                    writeln!(out, "  {} -> {} ({})", from, to, t)?;
                }
            }
        }
        out.flush()?;
    } else {
        println!("{}", serde_json::to_string_pretty(&output)?);
    }
    Ok(())
}
