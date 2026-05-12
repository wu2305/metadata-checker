use super::resolve_reference_path;
use crate::graph::{EdgeType, GraphDB, NodeType};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// 确保 model 和 field 节点存在，并建立 Contains 关系。
/// 返回 (model_id, field_id)。
fn ensure_model_field(
    graph: &mut GraphDB,
    model: &str,
    field: &str,
    model_path: &str,
    field_meta: Option<serde_json::Value>,
) -> (String, String) {
    let model_id = format!("model:{}", model);
    let field_id = format!("field:{}.{}", model, field);
    graph.add_node(
        model_id.clone(),
        NodeType::Model,
        model_path.to_string(),
        model.to_string(),
        None,
    );
    graph.add_node(
        field_id.clone(),
        NodeType::Field,
        model_path.to_string(),
        field.to_string(),
        field_meta,
    );
    graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
    (model_id, field_id)
}

/// 从模型路径中提取物理表名（去除路径前缀和 .tbl 后缀）。
/// - "$DATA:/主数据/fact_qwSidebar.tbl" -> "fact_qwSidebar"
/// - "data/table1.tbl" -> "table1"
/// - "model1.tbl" -> "model1"
fn resolve_physical_table_name(model_path: &str) -> Option<String> {
    let path = model_path.trim_end_matches('/');
    let stem = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())?;
    if stem.is_empty() {
        return None;
    }
    Some(stem)
}

/// 添加从 from_id 读取 model.field 的关系边。
fn add_model_read(
    graph: &mut GraphDB,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_meta: serde_json::Value,
    edge_type: EdgeType,
) {
    let (model_id, _field_id) = ensure_model_field(graph, model, field, model_path, None);
    graph.add_edge_with_meta(
        from_id,
        &model_id,
        edge_type.clone(),
        Some(format!("{}.{}", model, field)),
        Some(edge_meta.clone()),
    );

    // 同时创建到物理表的读取边（如果局部模型 ID 与物理表名不同）
    if let Some(physical_name) = resolve_physical_table_name(model_path) {
        if model != physical_name {
            let (phy_model_id, _) =
                ensure_model_field(graph, &physical_name, field, model_path, None);
            graph.add_edge_with_meta(
                from_id,
                &phy_model_id,
                edge_type,
                Some(format!("{}.{}", physical_name, field)),
                Some(edge_meta),
            );
        }
    }
}

/// 添加从 from_id 写入 model.field 的关系边。
fn add_model_write(
    graph: &mut GraphDB,
    from_id: &str,
    model: &str,
    field: &str,
    model_path: &str,
    edge_type: EdgeType,
    edge_meta: serde_json::Value,
) {
    let (model_id, _field_id) = ensure_model_field(graph, model, field, model_path, None);
    graph.add_edge_with_meta(
        from_id,
        &model_id,
        edge_type.clone(),
        Some(format!("{}.{}", model, field)),
        Some(edge_meta.clone()),
    );

    // 同时创建到物理表的写入边（如果局部模型 ID 与物理表名不同）
    if let Some(physical_name) = resolve_physical_table_name(model_path) {
        if model != physical_name {
            let (phy_model_id, _) =
                ensure_model_field(graph, &physical_name, field, model_path, None);
            graph.add_edge_with_meta(
                from_id,
                &phy_model_id,
                edge_type,
                Some(format!("{}.{}", physical_name, field)),
                Some(edge_meta),
            );
        }
    }
}

pub fn process_spg_file_from_value(
    graph: &mut GraphDB,
    rel_path: &str,
    raw_value: serde_json::Value,
) -> Result<Vec<String>> {
    let meta = crate::superpage::parse_superpage_from_value(raw_value)?;
    let mut node_ids = std::collections::HashSet::new();

    // Build model_id -> path mapping from page sources for accurate model references
    let source_path_map: std::collections::HashMap<String, String> = meta
        .sources
        .iter()
        .filter_map(|s| {
            let path = s.path.clone()?;
            Some((s.id.clone(), path))
        })
        .collect();

    let page_name = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Use normalized relative path for unique page_id to avoid collisions
    let page_id = format!("page:{}", rel_path.replace("\\", "/"));
    graph.add_node(
        page_id.clone(),
        NodeType::Page,
        rel_path.to_string(),
        page_name.clone(),
        None,
    );
    node_ids.insert(page_id.clone());

    // Process embedded DataFlow models in sources
    for source in &meta.sources {
        if source.model_type.as_deref() != Some("dataflow") {
            continue;
        }
        let Some(content) = &source.content else {
            continue;
        };
        let model_name = &source.id;
        let model_id = format!("model:{}", model_name);

        graph.add_node(
            model_id.clone(),
            NodeType::Model,
            rel_path.to_string(),
            model_name.clone(),
            Some(serde_json::json!({"modelType": "DataFlow", "embeddedIn": page_name})),
        );
        if !node_ids.contains(&model_id) {
            node_ids.insert(model_id.clone());
        }

        // Extract dimensions as fields
        if let Some(dimensions) = content.get("dimensions").and_then(|d| d.as_array()) {
            for dim in dimensions {
                if let Some(name) = dim.get("name").and_then(|n| n.as_str()) {
                    let field_id = format!("field:{}.{}", model_name, name);
                    graph.add_node(
                        field_id.clone(),
                        NodeType::Field,
                        rel_path.to_string(),
                        name.to_string(),
                        Some(dim.clone()),
                    );
                    if !node_ids.contains(&field_id) {
                        node_ids.insert(field_id.clone());
                    }
                    graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                }
            }
        }

        // Extract dataFlow input sources (external tables referenced by ModelTable nodes)
        if let Some(nodes) = content
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
        {
            for (_, node) in nodes {
                if let Some(module_table_path) =
                    node.get("moduleTablePath").and_then(|p| p.as_str())
                {
                    let ref_model = Path::new(module_table_path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| module_table_path.to_string());
                    let ref_model_id = format!("model:{}", ref_model);
                    graph.add_node(
                        ref_model_id.clone(),
                        NodeType::Model,
                        module_table_path.to_string(),
                        ref_model.clone(),
                        None,
                    );
                    graph.add_edge(
                        &model_id,
                        &ref_model_id,
                        EdgeType::DataflowInput,
                        Some(module_table_path.to_string()),
                    );
                }
            }
        }
    }

    // Build a map of component id -> submitField for action processing
    let mut submit_field_map: HashMap<String, String> = HashMap::new();
    for comp in &meta.components {
        // Skip components explicitly marked as not submitting data
        if comp.submit_data == Some(false) {
            continue;
        }
        if let Some(sf) = comp.properties.get("submitField") {
            submit_field_map.insert(comp.id.clone(), sf.clone());
        }
    }

    let mut expr_map: std::collections::HashMap<&str, Vec<&crate::superpage::ComponentExpr>> =
        std::collections::HashMap::new();
    for expr in &meta.expressions {
        expr_map.entry(&expr.component_id).or_default().push(expr);
    }
    // Process components and their expressions
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        graph.add_node(
            comp_id.clone(),
            NodeType::Component,
            rel_path.to_string(),
            comp.id.clone(),
            Some(serde_json::json!({"component_type": comp.component_type})),
        );
        node_ids.insert(comp_id.clone());
        graph.add_edge(&page_id, &comp_id, EdgeType::Contains, None);

        // Process expressions (reads)
        if let Some(exprs) = expr_map.get(comp.id.as_str()) {
            for expr in exprs {
                for ref_type in &expr.refs {
                    match ref_type {
                        crate::superpage::RefType::ModelField(model, field) => {
                            let model_path = source_path_map
                                .get(model.as_str())
                                .map(|p| p.to_string())
                                .unwrap_or_else(|| format!("{}.tbl", model));
                            let edge_meta = serde_json::json!({
                                "reason": format!("Component '{}' reads from model '{}'", comp.id, model),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "Reads",
                                "target_model": model,
                                "target_field": field,
                                "source_expr": expr.raw_expr,
                            });
                            add_model_read(
                                graph,
                                &comp_id,
                                model,
                                field,
                                &model_path,
                                edge_meta,
                                EdgeType::Reads,
                            );
                        }
                        crate::superpage::RefType::ComponentValue(target_id) => {
                            let target_comp_id = format!(
                                "comp:{}|{}",
                                rel_path.replace(r"\", "/"),
                                target_id
                            );
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on component '{}' via field '{}'",
                                    comp.id, target_id, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_component": target_id,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            graph.add_edge_with_meta(
                                &comp_id,
                                &target_comp_id,
                                EdgeType::DependsOn,
                                Some(format!("comp:{}.value", target_id)),
                                Some(edge_meta),
                            );
                        }
                        crate::superpage::RefType::Param(param_name) => {
                            let param_id = format!(
                                "param:{}|{}",
                                rel_path.replace(r"\", "/"),
                                param_name
                            );
                            graph.add_node(
                                param_id.clone(),
                                NodeType::Field,
                                rel_path.to_string(),
                                param_name.clone(),
                                Some(serde_json::json!({"kind": "param"})),
                            );
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on param '{}' via field '{}'",
                                    comp.id, param_name, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_param": param_name,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            graph.add_edge_with_meta(
                                &comp_id,
                                &param_id,
                                EdgeType::DependsOn,
                                Some(format!("param:{}", param_name)),
                                Some(edge_meta),
                            );
                        }
                        crate::superpage::RefType::UserProperty(prop) => {
                            let user_id = format!("user:{}", prop);
                            graph.add_node(
                                user_id.clone(),
                                NodeType::Field,
                                "system".to_string(),
                                format!("$user.{}", prop),
                                Some(serde_json::json!({"kind": "user_property"})),
                            );
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on user property '$user.{}' via field '{}'",
                                    comp.id, prop, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_user_property": prop,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            graph.add_edge_with_meta(
                                &comp_id,
                                &user_id,
                                EdgeType::DependsOn,
                                Some(format!("$user.{}", prop)),
                                Some(edge_meta),
                            );
                        }
                        crate::superpage::RefType::SystemVar(var_name) => {
                            let sys_id = format!("system:{}", var_name);
                            graph.add_node(
                                sys_id.clone(),
                                NodeType::Field,
                                "system".to_string(),
                                format!("${}", var_name),
                                Some(serde_json::json!({"kind": "system_var"})),
                            );
                            let edge_meta = serde_json::json!({
                                "reason": format!(
                                    "Component '{}' depends on system variable '${}' via field '{}'",
                                    comp.id, var_name, expr.field
                                ),
                                "actor_kind": "component",
                                "actor_id": comp.id,
                                "operation": "DependsOn",
                                "target_system_var": var_name,
                                "source_expr": expr.raw_expr,
                                "source_field": expr.field,
                            });
                            graph.add_edge_with_meta(
                                &comp_id,
                                &sys_id,
                                EdgeType::DependsOn,
                                Some(format!("${}", var_name)),
                                Some(edge_meta),
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Process submitField writes (implicit writes from component bindings)
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);

        if let Some(submit_field) = comp.properties.get("submitField") {
            let parts: Vec<&str> = submit_field.split('.').collect();
            if parts.len() >= 2 {
                let model = parts[0];
                let model_path = source_path_map
                    .get(model)
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| format!("{}.tbl", model));
                let field = parts[1..].join(".");

                let submit_meta = serde_json::json!({
                    "reason": format!("Component '{}' binds submitField to model '{}'", comp.id, model),
                    "actor_kind": "component",
                    "actor_id": comp.id,
                    "operation": "Writes",
                    "target_model": model,
                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                    "source_expr": format!("{}.{}", model, field),
                });
                add_model_write(
                    graph,
                    &comp_id,
                    model,
                    &field,
                    &model_path,
                    EdgeType::Writes,
                    submit_meta,
                );
            }
        }
    }

    // Process embedsuperpage components (page embedding)
    for comp in &meta.components {
        if comp.component_type != "embedsuperpage" {
            continue;
        }
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);

        // Try to resolve resPath as integer index into referenceResources
        if let Some(ref res_path_str) = comp.res_path
            && let Ok(ref_idx) = res_path_str.parse::<usize>()
            && let Some(target_rel) =
                resolve_reference_path(rel_path, ref_idx, &meta.reference_resources)
        {
            let target_name = Path::new(&target_rel)
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| target_rel.clone());
            let target_page_id = format!("page:{}", target_rel.replace(r"\", "/"));
            graph.add_node(
                target_page_id.clone(),
                NodeType::Page,
                target_rel.clone(),
                target_name.clone(),
                None,
            );
            graph.add_edge(
                &comp_id,
                &target_page_id,
                EdgeType::EmbedsPage,
                Some(target_rel.clone()),
            );
        }
    }

    // Process actions (explicit writes from interactions)
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        for action in &comp.actions {
            let action_id = format!(
                "action:{}|{}|{}",
                rel_path.replace("\\", "/"),
                comp.id,
                action.id
            );
            graph.add_node(
                action_id.clone(),
                NodeType::Action,
                rel_path.to_string(),
                format!("{}:{}", action.action_type, action.id),
                Some(serde_json::json!({
                    "waitPrev": action.wait_prev,
                    "condition": action.condition,
                    "conditionExp": action.condition_exp,
                    "triggerType": action.trigger_type,
                })),
            );
            node_ids.insert(action_id.clone());
            graph.add_edge(&comp_id, &action_id, EdgeType::Triggers, None);

            match action.action_type.as_str() {
                "submitData" => {
                    let target_comps: Vec<String> = match action.submit_range.as_deref() {
                        Some("page") | Some("dataset") => {
                            // Submit ALL components with submitField
                            submit_field_map.keys().cloned().collect()
                        }
                        Some("component") | Some("dialog") => {
                            // Only submit explicitly specified components
                            action.submit_component.clone()
                        }
                        _ => {
                            // Default: if submitComponent specified, use it; otherwise collect all
                            if action.submit_component.is_empty() {
                                submit_field_map.keys().cloned().collect()
                            } else {
                                action.submit_component.clone()
                            }
                        }
                    };
                    for target_comp_id in target_comps {
                        if let Some(submit_field) = submit_field_map.get(&target_comp_id) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if parts.len() >= 2 {
                                let model = parts[0];
                                let model_path = source_path_map
                                    .get(model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let field = parts[1..].join(".");
                                let action_meta = serde_json::json!({
                                    "reason": format!("Action 'submitData' writes to field '{}'", field),
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "ActionWrites",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                });
                                add_model_write(
                                    graph,
                                    &action_id,
                                    model,
                                    &field,
                                    &model_path,
                                    EdgeType::ActionWrites,
                                    action_meta,
                                );
                            }
                        }
                    }
                }
                "updateData" | "insertData" | "deleteData" => {
                    if let Some(ref data_set) = action.data_set {
                        let data_set_path = source_path_map
                            .get(data_set)
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| format!("{}.tbl", data_set));
                        for (field_name, field_value, value_type) in &action.field_values {
                            let ud_meta = serde_json::json!({
                                "reason": format!("Action '{}' writes to field '{}'", action.action_type, field_name),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "ActionWrites",
                                "trigger": action.trigger_type,
                                "target_model": data_set,
                                "target_field": field_name,
                                "source_expr": field_value,
                            });
                            add_model_write(
                                graph,
                                &action_id,
                                data_set,
                                field_name,
                                &data_set_path,
                                EdgeType::ActionWrites,
                                ud_meta,
                            );
                            // If value_type is "exp", parse expression refs for dependency analysis
                            if value_type == "exp" {
                                let refs = crate::superpage::parse_expression_refs(field_value);
                                for ref_type in refs {
                                    if let crate::superpage::RefType::ModelField(model, field) =
                                        ref_type
                                    {
                                        let model_path = source_path_map
                                            .get(&model)
                                            .map(|p| p.to_string())
                                            .unwrap_or_else(|| format!("{}.tbl", model));
                                        let read_meta = serde_json::json!({
                                                "actor_kind": "action",
                                                "actor_id": action_id,
                                                "operation": "Reads",
                                                "trigger": action.trigger_type,
                                                "target_model": model,
                                                "target_field": field,
                                        "source_expr": format!("{}.{}", model, field),
                                                "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                            });
                                        add_model_read(
                                            graph,
                                            &action_id,
                                            &model,
                                            &field,
                                            &model_path,
                                            read_meta,
                                            EdgeType::ActionReads,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                "link" if action.target_type.as_str() == "app" => {
                    // Try to resolve path as integer index into referenceResources
                    if let Some(ref path_str) = action.path
                        && let Ok(ref_idx) = path_str.parse::<usize>()
                        && let Some(target_rel) =
                            resolve_reference_path(rel_path, ref_idx, &meta.reference_resources)
                    {
                        let target_name = Path::new(&target_rel)
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| target_rel.clone());
                        let target_page_id = format!("page:{}", target_rel.replace(r"\", "/"));
                        graph.add_node(
                            target_page_id.clone(),
                            NodeType::Page,
                            target_rel.clone(),
                            target_name.clone(),
                            None,
                        );
                        let opens_meta = serde_json::json!({
                            "reason": format!("Link action opens page '{}'", target_name),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionNavigates",
                            "trigger": action.trigger_type,
                            "target_model": target_name,
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &target_page_id,
                            EdgeType::ActionNavigates,
                            Some(target_rel.clone()),
                            Some(opens_meta),
                        );

                        // Process parameter passing via data array
                        for (param_name, param_value) in &action.data {
                            let param_id = format!("param:{}/{}", target_name, param_name);
                            graph.add_node(
                                param_id.clone(),
                                NodeType::Field,
                                target_rel.clone(),
                                param_name.clone(),
                                None,
                            );
                            let pass_meta = serde_json::json!({
                                "reason": format!("Link action passes param '{}'", param_name),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "PassesParam",
                                "trigger": action.trigger_type,
                                "target_field": param_name,
                                "source_expr": param_value,
                            });
                            graph.add_edge_with_meta(
                                &action_id,
                                &param_id,
                                EdgeType::PassesParam,
                                Some(param_value.clone()),
                                Some(pass_meta),
                            );
                            // Parse expression refs from param_value for dependency analysis
                            let refs = crate::superpage::parse_expression_refs(param_value);
                            for ref_type in refs {
                                if let crate::superpage::RefType::ModelField(model, field) =
                                    ref_type
                                {
                                    let model_path = source_path_map
                                        .get(&model)
                                        .map(|p| p.to_string())
                                        .unwrap_or_else(|| format!("{}.tbl", model));
                                    let read_meta = serde_json::json!({
                                        "actor_kind": "action",
                                        "actor_id": action_id,
                                        "operation": "Reads",
                                        "trigger": action.trigger_type,
                                        "target_model": model,
                                        "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                        "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                    });
                                    add_model_read(
                                        graph,
                                        &action_id,
                                        &model,
                                        &field,
                                        &model_path,
                                        read_meta,
                                        EdgeType::ActionReads,
                                    );
                                }
                            }
                        }
                    }
                }
                "setParamValue" => {
                    for (param_name, param_value) in &action.params {
                        let param_id =
                            format!("param:{}|{}", rel_path.replace("\\", "/"), param_name);
                        graph.add_node(
                            param_id.clone(),
                            NodeType::Field,
                            rel_path.to_string(),
                            param_name.clone(),
                            None,
                        );
                        let sets_meta = serde_json::json!({
                            "reason": format!("setParamValue sets param '{}'", param_name),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionSetsParam",
                            "trigger": action.trigger_type,
                            "target_field": param_name,
                            "source_expr": param_value,
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &param_id,
                            EdgeType::ActionSetsParam,
                            Some(param_value.clone()),
                            Some(sets_meta),
                        );
                        // Parse expression refs from param_value for dependency analysis
                        let refs = crate::superpage::parse_expression_refs(param_value);
                        for ref_type in refs {
                            if let crate::superpage::RefType::ModelField(model, field) = ref_type {
                                let model_path = source_path_map
                                    .get(&model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let read_meta = serde_json::json!({
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "Reads",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                    "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                });
                                add_model_read(
                                    graph,
                                    &action_id,
                                    &model,
                                    &field,
                                    &model_path,
                                    read_meta,
                                    EdgeType::ActionReads,
                                );
                            }
                        }
                    }
                }
                "showComponent" | "hideComponent" => {
                    for target_comp in &action.target_component {
                        let target_comp_id =
                            format!("comp:{}|{}", rel_path.replace(r"\", "/"), target_comp);
                        let ctrl_meta = serde_json::json!({
                            "reason": format!("Action '{}' controls component '{}'", action.action_type, target_comp),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "target_component": target_comp,
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &target_comp_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(ctrl_meta),
                        );
                    }
                }
                "showDialog" => {
                    if let Some(ref dialog_id) = action.dialog {
                        let dialog_comp_id =
                            format!("comp:{}|{}", rel_path.replace(r"\", "/"), dialog_id);
                        // 确保目标节点存在
                        if !graph.node_indices.contains_key(&dialog_comp_id) {
                            graph.add_node(
                                dialog_comp_id.clone(),
                                NodeType::Component,
                                rel_path.to_string(),
                                dialog_id.clone(),
                                None,
                            );
                        }
                        let dialog_meta = serde_json::json!({
                            "reason": format!("Action 'showDialog' opens dialog '{}'", dialog_id),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "dialog": dialog_id,
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &dialog_comp_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(dialog_meta),
                        );
                    }
                }
                "closeDialog" => {
                    let close_meta = serde_json::json!({
                        "reason": "Action 'closeDialog' closes current dialog",
                        "actor_kind": "action",
                        "actor_id": action_id,
                        "operation": "ActionControlsComponent",
                        "trigger": action.trigger_type,
                    });
                    graph.add_edge_with_meta(
                        &action_id,
                        &comp_id,
                        EdgeType::ActionControlsComponent,
                        None,
                        Some(close_meta),
                    );
                }
                "switchPanel" => {
                    if let Some(ref pb) = action.panelbook {
                        let panelbook_id = format!("comp:{}|{}", rel_path.replace(r"\", "/"), pb);
                        // 确保目标节点存在
                        if !graph.node_indices.contains_key(&panelbook_id) {
                            graph.add_node(
                                panelbook_id.clone(),
                                NodeType::Component,
                                rel_path.to_string(),
                                pb.clone(),
                                None,
                            );
                        }
                        let ctrl_meta = serde_json::json!({
                            "reason": format!("Action 'switchPanel' controls panelbook '{}'", pb),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionControlsComponent",
                            "trigger": action.trigger_type,
                            "panelbook": pb,
                            "panel": action.panel,
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &panelbook_id,
                            EdgeType::ActionControlsComponent,
                            None,
                            Some(ctrl_meta),
                        );
                    }
                }
                "validateData" => {
                    let target_comps: Vec<String> = match action.submit_range.as_deref() {
                        Some("page") | Some("dataset") => {
                            submit_field_map.keys().cloned().collect()
                        }
                        Some("component") | Some("dialog") => action.submit_component.clone(),
                        _ => {
                            if action.submit_component.is_empty() {
                                submit_field_map.keys().cloned().collect()
                            } else {
                                action.submit_component.clone()
                            }
                        }
                    };
                    for target_comp_id in target_comps {
                        if let Some(submit_field) = submit_field_map.get(&target_comp_id) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if parts.len() >= 2 {
                                let model = parts[0];
                                let model_path = source_path_map
                                    .get(model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                let field = parts[1..].join(".");
                                let val_meta = serde_json::json!({
                                    "reason": format!("Action 'validateData' validates field '{}'", field),
                                    "actor_kind": "action",
                                    "actor_id": action_id,
                                    "operation": "ActionValidates",
                                    "trigger": action.trigger_type,
                                    "target_model": model,
                                    "target_field": field,
                                    "source_expr": format!("{}.{}", model, field),
                                });
                                add_model_write(
                                    graph,
                                    &action_id,
                                    model,
                                    &field,
                                    &model_path,
                                    EdgeType::ActionValidates,
                                    val_meta,
                                );
                            }
                        }
                    }
                }
                "resetData" | "newData" | "refreshModels" | "refreshData" | "loadData" => {
                    let mut targets: Vec<(String, String)> = Vec::new();
                    if let Some(ref ds) = action.data_set {
                        targets.push((ds.clone(), format!("{}.tbl", ds)));
                    }
                    for sc in &action.submit_component {
                        if let Some(submit_field) = submit_field_map.get(sc) {
                            let parts: Vec<&str> = submit_field.split('.').collect();
                            if !parts.is_empty() {
                                let model = parts[0].to_string();
                                let model_path = source_path_map
                                    .get(&model)
                                    .map(|p| p.to_string())
                                    .unwrap_or_else(|| format!("{}.tbl", model));
                                targets.push((model, model_path));
                            }
                        }
                    }
                    if targets.is_empty() && !action.submit_component.is_empty() {
                        for sc in &action.submit_component {
                            let comp_full_id =
                                format!("comp:{}|{}", rel_path.replace(r"\", "/"), sc);
                            let load_meta = serde_json::json!({
                                "reason": format!("Action '{}' loads/resets component '{}'", action.action_type, sc),
                                "actor_kind": "action",
                                "actor_id": action_id,
                                "operation": "ActionLoadsData",
                                "trigger": action.trigger_type,
                                "target_component": sc,
                            });
                            graph.add_edge_with_meta(
                                &action_id,
                                &comp_full_id,
                                EdgeType::ActionLoadsData,
                                None,
                                Some(load_meta),
                            );
                        }
                    }
                    for (model, model_path) in targets {
                        let load_meta = serde_json::json!({
                            "reason": format!("Action '{}' loads/resets model '{}'", action.action_type, model),
                            "actor_kind": "action",
                            "actor_id": action_id,
                            "operation": "ActionLoadsData",
                            "trigger": action.trigger_type,
                            "target_model": model,
                        });
                        add_model_write(
                            graph,
                            &action_id,
                            &model,
                            "*",
                            &model_path,
                            EdgeType::ActionLoadsData,
                            load_meta,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    // 写入 condition 节点和依赖边
    let conditions = crate::conditions::scan_conditions(&meta, Some(rel_path));
    for cond in &conditions {
        let cond_node_id = format!(
            "cond:{}|{}",
            rel_path.replace(r"\", "/"),
            cond.condition_id
        );
        let cond_name = format!("{:?}:{}", cond.condition_type, cond.condition_id);
        let cond_meta = serde_json::json!({
            "condition_type": format!("{:?}", cond.condition_type),
            "effect_type": format!("{:?}", cond.effect_type),
            "subject_type": format!("{:?}", cond.subject_type),
            "raw_expr": cond.raw_expr,
            "normalized_expr": cond.normalized_expr,
            "json_path": cond.json_path,
            "owner_type": format!("{:?}", cond.owner_type),
            "owner_id": cond.owner_id,
            "referenced_symbols": cond.referenced_symbols,
        });
        graph.add_node(
            cond_node_id.clone(),
            NodeType::Condition,
            cond.source_file.as_deref().unwrap_or(rel_path).to_string(),
            cond_name,
            Some(cond_meta),
        );
        if !node_ids.contains(&cond_node_id) {
            node_ids.insert(cond_node_id.clone());
        }

        // condition -> owner 边
        let owner_node_id = match cond.owner_type {
            crate::conditions::OwnerType::Component => {
                format!("comp:{}|{}", rel_path.replace(r"\", "/"), cond.owner_id)
            }
            crate::conditions::OwnerType::Action => {
                let parts: Vec<&str> = cond.owner_id.split(':').collect();
                if parts.len() >= 2 {
                    format!(
                        "action:{}|{}|{}",
                        rel_path.replace(r"\", "/"),
                        parts[0],
                        parts[1]
                    )
                } else {
                    continue;
                }
            }
            crate::conditions::OwnerType::ModelSource => {
                format!("model:{}", cond.owner_id)
            }
            crate::conditions::OwnerType::FieldDefault => {
                format!("comp:{}|{}", rel_path.replace(r"\", "/"), cond.owner_id)
            }
            crate::conditions::OwnerType::Page => {
                page_id.clone()
            }
        };
        let owner_edge_meta = serde_json::json!({
            "reason": format!("Condition '{}' belongs to owner '{}'", cond.condition_id, cond.owner_id),
            "actor_kind": "condition",
            "actor_id": cond.condition_id,
            "operation": "Conditions",
            "json_path": cond.json_path,
            "condition_type": format!("{:?}", cond.condition_type),
        });
        graph.add_edge_with_meta(
            &cond_node_id,
            &owner_node_id,
            EdgeType::DependsOn,
            Some(cond.json_path.clone()),
            Some(owner_edge_meta),
        );

        // condition -> upstream dependency 边
        for sym in &cond.referenced_symbols {
            let sym_parts: Vec<&str> = sym.splitn(2, ':').collect();
            if sym_parts.len() < 2 {
                continue;
            }
            let (kind, target_id) = (sym_parts[0], sym_parts[1]);
            let target_node_id = match kind {
                "param" => format!("param:{}|{}", rel_path.replace(r"\", "/"), target_id),
                "model" => {
                    let model_name = target_id.split('.').next().unwrap_or(target_id);
                    format!("model:{}", model_name)
                }
                "component" => format!("comp:{}|{}", rel_path.replace(r"\", "/"), target_id),
                "user" => format!("user:{}", target_id),
                "system" => format!("system:{}", target_id),
                _ => continue,
            };
            let dep_edge_meta = serde_json::json!({
                "reason": format!("Condition '{}' depends on symbol '{}'", cond.condition_id, sym),
                "actor_kind": "condition",
                "actor_id": cond.condition_id,
                "operation": "DependsOn",
                "source_expr": cond.raw_expr,
                "json_path": cond.json_path,
            });
            graph.add_edge_with_meta(
                &cond_node_id,
                &target_node_id,
                EdgeType::DependsOn,
                Some(sym.clone()),
                Some(dep_edge_meta),
            );
        }
    }

    // 建立 model.filter 与 model.totalRowCount__ 的隐式关系
    for cond in &conditions {
        if matches!(cond.condition_type, crate::conditions::ConditionType::SourceFilterExp)
            && matches!(cond.owner_type, crate::conditions::OwnerType::ModelSource)
        {
            let model_name = &cond.owner_id;
            let trc_field_id = format!("field:{}.totalRowCount__", model_name);
            let trc_model_id = format!("model:{}", model_name);
            graph.add_node(
                trc_field_id.clone(),
                NodeType::Field,
                rel_path.to_string(),
                "totalRowCount__".to_string(),
                Some(serde_json::json!({
                    "kind": "implicit",
                    "description": "模型过滤后的隐式行数字段",
                })),
            );
            graph.add_edge(&trc_model_id, &trc_field_id, EdgeType::Contains, None);

            let cond_node_id = format!(
                "cond:{}|{}",
                rel_path.replace(r"\", "/"),
                cond.condition_id
            );
            let trc_edge_meta = serde_json::json!({
                "reason": format!("Filter condition determines {}.totalRowCount__", model_name),
                "actor_kind": "condition",
                "actor_id": cond.condition_id.clone(),
                "operation": "DependsOn",
                "source_expr": cond.raw_expr.clone(),
                "json_path": cond.json_path.clone(),
            });
            graph.add_edge_with_meta(
                &cond_node_id,
                &trc_field_id,
                EdgeType::DependsOn,
                Some(format!("{}.totalRowCount__", model_name)),
                Some(trc_edge_meta),
            );
        }
    }

    // Process dwtable sources: establish DataflowInput from local model to physical table
    for source in &meta.sources {
        if source.model_type.as_deref() != Some("dwtable") {
            continue;
        }
        let Some(ref path) = source.path else {
            continue;
        };

        // Extract physical table name from path like "$DATA:/主数据/fact_qwSidebar.tbl"
        let physical_table = std::path::Path::new(path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path);
        let model_id = format!("model:{}", source.id);
        let physical_model_id = format!("model:{}", physical_table);
        graph.add_node(
            physical_model_id.clone(),
            NodeType::Model,
            path.clone(),
            physical_table.to_string(),
            Some(serde_json::json!({"modelType": "PhysicalTable", "sourcePath": path})),
        );
        graph.add_edge(
            &model_id,
            &physical_model_id,
            EdgeType::DataflowInput,
            Some(path.clone()),
        );
        // 建立反向边，使 BFS 能从物理表回溯到局部模型
        graph.add_edge(
            &physical_model_id,
            &model_id,
            EdgeType::DataflowOutput,
            Some(path.clone()),
        );
    }

    Ok(node_ids.into_iter().collect())
}
