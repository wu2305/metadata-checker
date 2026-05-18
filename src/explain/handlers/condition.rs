use crate::graph::GraphDB;
use anyhow::Result;
use serde_json::Value;
use std::io::{self, Write};

/// 解释条件节点：输出完整条件依赖路径，包含上游依赖、下游影响与每段 evidence
pub(in crate::explain) fn explain_condition_graph(
    _graph: &GraphDB,
    node: &crate::graph::Node,
    outgoing: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    incoming: Vec<(&crate::graph::Node, &crate::graph::Edge)>,
    human: bool,
) -> Result<Value> {
    // 从节点 meta 提取条件核心信息
    let meta = node.meta.as_ref().unwrap_or(&serde_json::Value::Null);
    let cond_id = node.id.split('|').next_back().unwrap_or(&node.id);
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

    // 分类 outgoing 边：owner 边（operation="Conditions"） vs upstream 边（operation="DependsOn"）
    let mut upstream_deps: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();
    let mut downstream_owners: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();
    let mut downstream_effects: Vec<(&crate::graph::Node, &crate::graph::Edge)> = Vec::new();

    for (target, edge) in &outgoing {
        let is_owner_edge = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("operation"))
            .and_then(|v| v.as_str())
            == Some("Conditions");
        if is_owner_edge {
            downstream_owners.push((target, edge));
        } else if target.name == "totalRowCount__" {
            downstream_effects.push((target, edge));
        } else {
            upstream_deps.push((target, edge));
        }
    }

    // 构建上游依赖详情
    let upstream_dependencies: Vec<serde_json::Value> = upstream_deps
        .iter()
        .map(|(target, edge)| {
            let sym = edge.field_path.as_deref().unwrap_or("?");
            let edge_raw_expr = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .unwrap_or(raw_expr);
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            let reason = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "symbol": sym,
                "target_node_id": target.id,
                "target_node_type": format!("{:?}", target.node_type),
                "target_name": target.name,
                "raw_expr": edge_raw_expr,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": reason,
            })
        })
        .collect();

    // 构建下游 owner 详情
    let downstream_owner_details: Vec<serde_json::Value> = downstream_owners
        .iter()
        .map(|(target, edge)| {
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            let reason = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "owner_node_id": target.id,
                "owner_node_type": format!("{:?}", target.node_type),
                "owner_name": target.name,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": reason,
            })
        })
        .collect();

    // 构建下游 effect 详情（totalRowCount__ 等）
    let downstream_effect_details: Vec<serde_json::Value> = downstream_effects
        .iter()
        .map(|(target, edge)| {
            let edge_raw_expr = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("source_expr"))
                .and_then(|v| v.as_str())
                .unwrap_or(raw_expr);
            let edge_json_path = edge
                .meta
                .as_ref()
                .and_then(|m| m.get("json_path"))
                .and_then(|v| v.as_str())
                .unwrap_or(json_path);
            serde_json::json!({
                "effect_type": "totalRowCount__",
                "target_node_id": target.id,
                "target_node_type": format!("{:?}", target.node_type),
                "target_name": target.name,
                "raw_expr": edge_raw_expr,
                "json_path": edge_json_path,
                "source_file": node.path,
                "reason": "模型过滤条件决定命中行数",
            })
        })
        .collect();

    // 处理 incoming 边（当前 graph 中较少有指向 condition 的边，为健壮性保留）
    let incoming_nodes: Vec<serde_json::Value> = incoming
        .iter()
        .map(|(source, edge)| {
            serde_json::json!({
                "node_id": source.id,
                "node_type": format!("{:?}", source.node_type),
                "name": source.name,
                "edge_type": format!("{:?}", edge.edge_type),
                "field_path": edge.field_path,
                "source_file": source.path,
            })
        })
        .collect();

    let what = format!(
        "条件 {} ({}) 作用于 {}，表达式 '{}', 引用 {} 个上游符号，影响 {} 个下游对象",
        cond_id,
        condition_type,
        owner_id,
        normalized_expr,
        upstream_dependencies.len(),
        downstream_owner_details.len() + downstream_effect_details.len()
    );

    let summary = serde_json::json!({
        "what_is_it": what,
        "type": "condition",
        "condition_id": cond_id,
        "condition_type": condition_type,
        "effect_type": effect_type,
        "subject_type": subject_type,
        "owner_type": owner_type,
        "owner_id": owner_id,
        "raw_expr": raw_expr,
        "normalized_expr": normalized_expr,
        "json_path": json_path,
        "source_file": node.path,
        "referenced_symbols_count": referenced_symbols.len(),
        "upstream_dependency_count": upstream_dependencies.len(),
        "downstream_owner_count": downstream_owner_details.len(),
        "downstream_effect_count": downstream_effect_details.len(),
    });

    let details = serde_json::json!({
        "condition": {
            "condition_id": cond_id,
            "condition_type": condition_type,
            "effect_type": effect_type,
            "subject_type": subject_type,
            "raw_expr": raw_expr,
            "normalized_expr": normalized_expr,
            "json_path": json_path,
            "owner_type": owner_type,
            "owner_id": owner_id,
            "referenced_symbols": referenced_symbols,
            "source_file": node.path,
        },
        "upstream_dependencies": upstream_dependencies,
        "downstream_owners": downstream_owner_details,
        "downstream_effects": downstream_effect_details,
        "incoming_nodes": incoming_nodes,
    });

    let mut output = crate::output::AiOutput::new(crate::output::OutputKind::Explain, summary);
    output.query_target = Some(node.id.clone());
    output.details = Some(details);

    // 条件自身证据
    output.evidence.push(
        crate::output::Evidence::new(
            format!(
                "Condition {} (type={}, effect={}, subject={}) defined at {} with {} referenced symbols",
                cond_id, condition_type, effect_type, subject_type, node.path, referenced_symbols.len()
            ),
            "Graph condition node with full metadata from scanner",
        )
        .with_confidence(crate::output::Confidence::High)
        .with_node_id(&node.id)
        .with_source_file(&node.path)
        .with_json_path(json_path)
        .with_raw_expr(raw_expr),
    );

    // 上游依赖 evidence
    for (target, edge) in &upstream_deps {
        let sym = edge.field_path.as_deref().unwrap_or("?");
        let edge_raw_expr = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("source_expr"))
            .and_then(|v| v.as_str())
            .unwrap_or(raw_expr);
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Condition depends on {} via expression '{}'",
                    sym, edge_raw_expr
                ),
                "Condition references upstream symbol in raw expression",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_raw_expr(edge_raw_expr)
            .with_edge_type("DependsOn"),
        );
    }

    // 下游 owner evidence
    for (target, edge) in &downstream_owners {
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Condition {} controls {} (type={:?})",
                    cond_id, target.name, target.node_type
                ),
                "Condition determines owner component/action visibility or execution",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_edge_type("DependsOn"),
        );
    }

    // 下游 effect evidence（totalRowCount__ 等）
    for (target, edge) in &downstream_effects {
        let edge_raw_expr = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("source_expr"))
            .and_then(|v| v.as_str())
            .unwrap_or(raw_expr);
        let edge_json_path = edge
            .meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str())
            .unwrap_or(json_path);
        output.evidence.push(
            crate::output::Evidence::new(
                format!(
                    "Filter condition determines {} ({}), affecting row count gating",
                    target.name, edge_raw_expr
                ),
                "Model filter condition implicitly determines totalRowCount__ gating",
            )
            .with_confidence(crate::output::Confidence::High)
            .with_node_id(&target.id)
            .with_source_file(&node.path)
            .with_json_path(edge_json_path)
            .with_raw_expr(edge_raw_expr)
            .with_edge_type("DependsOn"),
        );
    }

    let output = output.validate();

    if human {
        let mut out = io::stdout();
        writeln!(out, "=== Explain: {} ===", node.id)?;
        writeln!(out, "What: {}", what)?;
        writeln!(out, "Path: {}", node.path)?;
        writeln!(out, "Condition Type: {}", condition_type)?;
        writeln!(out, "Effect: {}", effect_type)?;
        writeln!(out, "Raw Expr: {}", raw_expr)?;
        if !upstream_dependencies.is_empty() {
            writeln!(
                out,
                "
--- Upstream Dependencies ({}) ---",
                upstream_dependencies.len()
            )?;
            for d in &upstream_dependencies {
                writeln!(out, "  {:?}", d)?;
            }
        }
        if !downstream_owner_details.is_empty() {
            writeln!(
                out,
                "
--- Downstream Owners ({}) ---",
                downstream_owner_details.len()
            )?;
            for o in &downstream_owner_details {
                writeln!(out, "  {:?}", o)?;
            }
        }
        if !downstream_effect_details.is_empty() {
            writeln!(
                out,
                "
--- Downstream Effects ({}) ---",
                downstream_effect_details.len()
            )?;
            for e in &downstream_effect_details {
                writeln!(out, "  {:?}", e)?;
            }
        }
    }
    Ok(serde_json::to_value(output)?)
}
