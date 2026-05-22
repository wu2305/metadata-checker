use crate::graph::GraphDB;
use crate::graph_store::GraphReadStore;
use crate::query::find_candidates;
use anyhow::{Context, Result};
use serde_json::json;
use std::any::Any;
use std::io::{self, Write};

mod diagnostics;
mod evidence;
mod graph_collect;
mod metadata;
mod path_summary;
mod prerequisites;

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

/// 从字段引用中提取模型 ID，例如 model:model1.name -> model:model1。
fn model_id_from_reference(reference: &str) -> Option<String> {
    let rest = reference.strip_prefix("model:")?;
    let model_part = rest.split('.').next().unwrap_or(rest).trim();
    if model_part.is_empty() {
        return None;
    }
    Some(format!("model:{}", model_part.trim_start_matches("model:")))
}

/// 将页面内局部模型 ID 转成 page-scoped 查询目标。
fn page_scoped_model_target(page_path: &str, model_id: &str) -> String {
    let model_part = model_id.strip_prefix("model:").unwrap_or(model_id);
    if model_part.contains('|') {
        format!("model:{}", model_part)
    } else {
        format!("model:{}|{}", page_path, model_part)
    }
}

/// availability_facts 的字段提升：保持 answer_facts 为主证据，同时便于模型快速定位。
fn cast_graph_db(graph: &dyn GraphReadStore) -> Result<&GraphDB> {
    (graph as &dyn Any)
        .downcast_ref::<GraphDB>()
        .with_context(|| "query_page_logic currently requires GraphDB-backed GraphReadStore")
}

fn build_key_model_availability_entry(
    graph: &GraphDB,
    page_path: &str,
    model_id: &str,
) -> Option<serde_json::Value> {
    let page_scoped_target = page_scoped_model_target(page_path, model_id);
    let scoped_result = crate::explain::build_explain_condition_output_with_intent(
        graph,
        &page_scoped_target,
        "compact",
        crate::explain::TraversalIntent::Availability,
    )
    .ok();
    let scoped_has_facts = scoped_result.as_ref().is_some_and(|result| {
        result
            .get("details")
            .and_then(|d| d.get("answer_facts"))
            .and_then(|a| a.get("availability_facts"))
            .is_some()
    });
    let (resolved_model_target, scope_warning, result) = if scoped_has_facts {
        (page_scoped_target.clone(), None, scoped_result?)
    } else {
        let result = crate::explain::build_explain_condition_output_with_intent(
            graph,
            model_id,
            "compact",
            crate::explain::TraversalIntent::Availability,
        )
        .ok()?;
        (
            model_id.to_string(),
            Some("page_scoped_target_not_resolved_fallback_to_global_model"),
            result,
        )
    };

    let direct_filters: Vec<serde_json::Value> = result
        .get("details")
        .and_then(|d| d.get("data_empty_gates"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("condition_scope").and_then(|v| v.as_str())
                        != Some("dataflow_internal")
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let availability_summary = result
        .get("details")
        .and_then(|d| d.get("answer_facts"))
        .and_then(|a| a.get("availability_facts"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let truncation = result
        .get("details")
        .and_then(|d| d.get("truncation_guard"))
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let empty_array = serde_json::Value::Array(Vec::new());
    let source_filters = availability_summary
        .get("source_filters")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let output_filters = availability_summary
        .get("output_filters")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let physical_inputs = availability_summary
        .get("physical_inputs")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let join_rules = availability_summary
        .get("join_rules")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());
    let union_rules = availability_summary
        .get("union_rules")
        .cloned()
        .unwrap_or_else(|| empty_array.clone());

    let mut row_semantics = Vec::new();
    let mut seen_semantics = std::collections::HashSet::new();
    for rule in join_rules
        .as_array()
        .into_iter()
        .flatten()
        .chain(union_rules.as_array().into_iter().flatten())
    {
        if let Some(row_semantic) = rule.get("row_semantic").and_then(|v| v.as_str()) {
            if seen_semantics.insert(row_semantic.to_string()) {
                row_semantics.push(row_semantic.to_string());
            }
        }
    }

    Some(serde_json::json!({
        "model_id": model_id,
        "page_scoped_target": page_scoped_target,
        "resolved_model_target": resolved_model_target,
        "scope_warning": scope_warning,
        "direct_filters": direct_filters,
        "dataflow_table": availability_summary.get("dataflow_table").cloned().unwrap_or(serde_json::Value::Null),
        "source_filters": source_filters,
        "output_filters": output_filters,
        "physical_inputs": physical_inputs,
        "join_rules": join_rules,
        "union_rules": union_rules,
        "row_semantics": row_semantics,
        "availability_summary": availability_summary,
        "truncation_guard": truncation,
    }))
}

/// 查询页面级逻辑摘要
///
/// 输出 page_inputs、data_sources、write_targets、entrypoints、action_flows、visibility_rules、navigation、risk_diagnostics。
pub fn build_query_page_logic_output(
    graph: &dyn GraphReadStore,
    page_id: &str,
    project_dir: Option<&std::path::Path>,
    budget: &str,
) -> Result<serde_json::Value> {
    let graph = cast_graph_db(graph)?;
    let is_compact = budget == "compact";
    let is_full = budget == "full";
    let page_node = match graph.get_node(page_id) {
        Some(n) => n,
        None => {
            let candidates = find_candidates(graph, page_id, 5)?;
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
    } = graph_collect::collect_page_logic_nodes(graph, page_id)?;
    let child_component_refs: Vec<&crate::graph::Node> = child_components.iter().collect();
    let child_action_refs: Vec<&crate::graph::Node> = child_actions.iter().collect();

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
    all_nodes.extend(child_components.iter());
    all_nodes.extend(child_actions.iter());

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
    let prerequisites::PagePrerequisites {
        display_prerequisites,
        data_prerequisites,
        action_prerequisites,
    } = prerequisites::collect_page_prerequisites(
        graph,
        &page_node.path,
        &child_components,
        &child_actions,
        &data_sources,
    )?;

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
        &child_component_refs,
        &child_action_refs,
        &data_sources,
        &write_targets,
        &entrypoints,
    );

    // ---- 6. Risk diagnostics ----
    let evidence_sample_limit = diagnostics::EVIDENCE_SAMPLE_LIMIT;
    let diagnostics::PageLogicDiagnostics {
        diagnostics,
        related_context_summary,
    } = {
        let graph_store: &dyn GraphReadStore = graph;
        diagnostics::build_page_logic_diagnostics(
            graph_store,
            page_id,
            &page_node,
            from_file,
            &entrypoints,
            &data_sources,
            &write_targets,
            &navigation,
            &visibility_rules,
            &action_flows,
            &mut primary_paths,
            &mut related_context,
        )
    };

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

    // M35.7: 从 data_sources 和 write_targets 中自动发现关键模型，
    // 并内嵌每个模型的 availability 摘要。
    let mut key_model_ids: Vec<String> = Vec::new();
    let mut seen_models: std::collections::HashSet<String> = std::collections::HashSet::new();
    for ds in &data_sources {
        if let Some(target_id) = ds.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    for wt in &write_targets {
        if let Some(target_id) = wt.get("target_id").and_then(|v| v.as_str()) {
            if target_id.starts_with("model:") && seen_models.insert(target_id.to_string()) {
                key_model_ids.push(target_id.to_string());
            }
        }
    }
    // 同时纳入显示/数据前置条件中引用的模型，避免只看 data_sources 漏掉 totalRowCount__ 类条件。
    for prereq in display_prerequisites
        .iter()
        .chain(data_prerequisites.iter())
    {
        if let Some(depends_on) = prereq.get("depends_on").and_then(|v| v.as_array()) {
            for dep in depends_on {
                let Some(dep) = dep.as_str() else {
                    continue;
                };
                let Some(model_id) = model_id_from_reference(dep) else {
                    continue;
                };
                if seen_models.insert(model_id.clone()) {
                    key_model_ids.push(model_id);
                }
            }
        }
    }

    let mut key_model_availability: Vec<serde_json::Value> = Vec::new();
    for model_id in &key_model_ids {
        if let Some(entry) = build_key_model_availability_entry(graph, &page_node.path, model_id) {
            key_model_availability.push(entry);
        }
    }

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
        "key_model_count": key_model_ids.len(),
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
            "key_model_availability": crate::output::brief::truncated_array(&key_model_availability, 3),
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
            "key_model_availability": key_model_availability.clone(),
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
            "key_model_availability": key_model_availability.clone(),
        })
    };

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::PageLogic, summary);
    output.query_target = Some(page_id.to_string());
    output.details = Some(details);
    output.diagnostics = diagnostics.clone();

    evidence::populate_page_logic_evidence(
        &mut output,
        page_id,
        &page_node.path,
        &entrypoints,
        &data_sources,
        &write_targets,
        &navigation,
        &visibility_rules,
        &action_flows,
        evidence_sample_limit,
    );

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
    graph: &dyn GraphReadStore,
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
