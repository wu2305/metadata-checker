use super::resolve_reference_path;
use crate::graph::{EdgeType, GraphDB, NodeType};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

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
        expr_map
            .entry(&expr.component_id)
            .or_insert_with(Vec::new)
            .push(expr);
    }
    // Process components and their expressions
    for comp in &meta.components {
        let comp_id = format!("comp:{}|{}", rel_path.replace("\\", "/"), comp.id);
        graph.add_node(
            comp_id.clone(),
            NodeType::Component,
            rel_path.to_string(),
            comp.id.clone(),
            None,
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
                            let model_id = format!("model:{}", model);
                            let field_id = format!("field:{}.{}", model, field);
                            graph.add_node(
                                model_id.clone(),
                                NodeType::Model,
                                model_path.clone(),
                                model.clone(),
                                None,
                            );
                            graph.add_node(
                                field_id.clone(),
                                NodeType::Field,
                                model_path,
                                format!("{}", field),
                                None,
                            );
                            let edge_meta = serde_json::json!({
                                "component_field": expr.field,
                                "raw_expr": expr.raw_expr,
                                "ref_token": format!("{}.{}", model, field),
                                "reason": format!("Component '{}' reads from model '{}'", comp.id, model),
                            });
                            graph.add_edge_with_meta(
                                &comp_id,
                                &model_id,
                                EdgeType::Reads,
                                Some(format!("{}.{}", model, field)),
                                Some(edge_meta),
                            );
                            graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
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
                let model_id = format!("model:{}", model);
                let field_id = format!("field:{}.{}", model, field);

                graph.add_node(
                    model_id.clone(),
                    NodeType::Model,
                    model_path.clone(),
                    model.to_string(),
                    None,
                );
                graph.add_node(
                    field_id.clone(),
                    NodeType::Field,
                    model_path,
                    field.clone(),
                    None,
                );
                graph.add_edge(
                    &comp_id,
                    &model_id,
                    EdgeType::Writes,
                    Some(format!("{}.{}", model, field)),
                );
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
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
        if let Some(ref res_path_str) = comp.res_path {
            if let Ok(ref_idx) = res_path_str.parse::<usize>() {
                if let Some(target_rel) =
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
                None,
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
                                let model_id = format!("model:{}", model);
                                let field_id = format!("field:{}.{}", model, field);
                                graph.add_node(
                                    model_id.clone(),
                                    NodeType::Model,
                                    model_path.clone(),
                                    model.to_string(),
                                    None,
                                );
                                graph.add_node(
                                    field_id.clone(),
                                    NodeType::Field,
                                    model_path.clone(),
                                    field.clone(),
                                    None,
                                );
                                let action_meta = serde_json::json!({
                                    "action_type": action.action_type,
                                    "trigger_type": action.trigger_type,
                                    "submit_range": action.submit_range,
                                    "source_component": comp.id,
                                    "reason": format!("Action '{}' writes to model '{}'", action.action_type, model),
                                });
                                graph.add_edge_with_meta(
                                    &action_id,
                                    &model_id,
                                    EdgeType::ActionWrites,
                                    Some(format!("{}.{}", model, field)),
                                    Some(action_meta),
                                );
                                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                            }
                        }
                    }
                }
                "updateData" | "insertData" | "deleteData" => {
                    if let Some(ref data_set) = action.data_set {
                        let model_id = format!("model:{}", data_set);
                        let data_set_path = format!("{}.tbl", data_set);
                        graph.add_node(
                            model_id.clone(),
                            NodeType::Model,
                            data_set_path.clone(),
                            data_set.clone(),
                            None,
                        );
                        for (field_name, field_value, value_type) in &action.field_values {
                            let field_id = format!("field:{}.{}", data_set, field_name);
                            graph.add_node(
                                field_id.clone(),
                                NodeType::Field,
                                data_set_path.clone(),
                                field_name.clone(),
                                None,
                            );
                            graph.add_edge(
                                &action_id,
                                &model_id,
                                EdgeType::ActionWrites,
                                Some(format!("{}.{}", data_set, field_name)),
                            );
                            graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
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
                                        let ref_model_id = format!("model:{}", model);
                                        let ref_field_id = format!("field:{}.{}", model, field);
                                        graph.add_node(
                                            ref_model_id.clone(),
                                            NodeType::Model,
                                            model_path.clone(),
                                            model.clone(),
                                            None,
                                        );
                                        graph.add_node(
                                            ref_field_id.clone(),
                                            NodeType::Field,
                                            model_path.clone(),
                                            field.clone(),
                                            None,
                                        );
                                        let read_meta = serde_json::json!({
                                            "action_type": action.action_type,
                                            "trigger_type": action.trigger_type,
                                            "source_component": comp.id,
                                            "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                        });
                                        graph.add_edge_with_meta(
                                            &action_id,
                                            &ref_model_id,
                                            EdgeType::Reads,
                                            Some(format!("{}.{}", model, field)),
                                            Some(read_meta),
                                        );
                                        graph.add_edge(
                                            &ref_model_id,
                                            &ref_field_id,
                                            EdgeType::Contains,
                                            None,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                "link" => {
                    match action.target_type.as_str() {
                        "app" => {
                            // Try to resolve path as integer index into referenceResources
                            if let Some(ref path_str) = action.path {
                                if let Ok(ref_idx) = path_str.parse::<usize>() {
                                    if let Some(target_rel) = resolve_reference_path(
                                        rel_path,
                                        ref_idx,
                                        &meta.reference_resources,
                                    ) {
                                        let target_name = Path::new(&target_rel)
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_else(|| target_rel.clone());
                                        let target_page_id =
                                            format!("page:{}", target_rel.replace(r"\", "/"));
                                        graph.add_node(
                                            target_page_id.clone(),
                                            NodeType::Page,
                                            target_rel.clone(),
                                            target_name.clone(),
                                            None,
                                        );
                                        let opens_meta = serde_json::json!({
                                            "action_type": action.action_type,
                                            "trigger_type": action.trigger_type,
                                            "target_type": action.target_type,
                                            "source_component": comp.id,
                                            "reason": format!("Link action opens page '{}'", target_name),
                                        });
                                        graph.add_edge_with_meta(
                                            &action_id,
                                            &target_page_id,
                                            EdgeType::OpensPage,
                                            Some(target_rel.clone()),
                                            Some(opens_meta),
                                        );

                                        // Process parameter passing via data array
                                        for (param_name, param_value) in &action.data {
                                            let param_id =
                                                format!("param:{}/{}", target_name, param_name);
                                            graph.add_node(
                                                param_id.clone(),
                                                NodeType::Field,
                                                target_rel.clone(),
                                                param_name.clone(),
                                                None,
                                            );
                                            let pass_meta = serde_json::json!({
                                                "param_name": param_name,
                                                "param_value": param_value,
                                                "action_type": action.action_type,
                                                "source_component": comp.id,
                                                "reason": format!("Link action passes param '{}'", param_name),
                                            });
                                            graph.add_edge_with_meta(
                                                &action_id,
                                                &param_id,
                                                EdgeType::PassesParam,
                                                Some(param_value.clone()),
                                                Some(pass_meta),
                                            );
                                            // Parse expression refs from param_value for dependency analysis
                                            let refs = crate::superpage::parse_expression_refs(
                                                param_value,
                                            );
                                            for ref_type in refs {
                                                if let crate::superpage::RefType::ModelField(
                                                    model,
                                                    field,
                                                ) = ref_type
                                                {
                                                    let model_path = source_path_map
                                                        .get(&model)
                                                        .map(|p| p.to_string())
                                                        .unwrap_or_else(|| {
                                                            format!("{}.tbl", model)
                                                        });
                                                    let ref_model_id = format!("model:{}", model);
                                                    let ref_field_id =
                                                        format!("field:{}.{}", model, field);
                                                    graph.add_node(
                                                        ref_model_id.clone(),
                                                        NodeType::Model,
                                                        model_path.clone(),
                                                        model.clone(),
                                                        None,
                                                    );
                                                    graph.add_node(
                                                        ref_field_id.clone(),
                                                        NodeType::Field,
                                                        model_path.clone(),
                                                        field.clone(),
                                                        None,
                                                    );
                                                    let read_meta = serde_json::json!({
                                                        "action_type": action.action_type,
                                                        "trigger_type": action.trigger_type,
                                                        "source_component": comp.id,
                                                        "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                                    });
                                                    graph.add_edge_with_meta(
                                                        &action_id,
                                                        &ref_model_id,
                                                        EdgeType::Reads,
                                                        Some(format!("{}.{}", model, field)),
                                                        Some(read_meta),
                                                    );
                                                    graph.add_edge(
                                                        &ref_model_id,
                                                        &ref_field_id,
                                                        EdgeType::Contains,
                                                        None,
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
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
                            "param_name": param_name,
                            "param_value": param_value,
                            "action_type": action.action_type,
                            "trigger_type": action.trigger_type,
                            "source_component": comp.id,
                            "reason": format!("setParamValue sets param '{}'", param_name),
                        });
                        graph.add_edge_with_meta(
                            &action_id,
                            &param_id,
                            EdgeType::SetsParam,
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
                                let ref_model_id = format!("model:{}", model);
                                let ref_field_id = format!("field:{}.{}", model, field);
                                graph.add_node(
                                    ref_model_id.clone(),
                                    NodeType::Model,
                                    model_path.clone(),
                                    model.clone(),
                                    None,
                                );
                                graph.add_node(
                                    ref_field_id.clone(),
                                    NodeType::Field,
                                    model_path.clone(),
                                    field.clone(),
                                    None,
                                );
                                let read_meta = serde_json::json!({
                                    "action_type": action.action_type,
                                    "trigger_type": action.trigger_type,
                                    "source_component": comp.id,
                                    "reason": format!("Action '{}' reads from model '{}'", action.action_type, model),
                                });
                                graph.add_edge_with_meta(
                                    &action_id,
                                    &ref_model_id,
                                    EdgeType::Reads,
                                    Some(format!("{}.{}", model, field)),
                                    Some(read_meta),
                                );
                                graph.add_edge(
                                    &ref_model_id,
                                    &ref_field_id,
                                    EdgeType::Contains,
                                    None,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    Ok(node_ids.into_iter().collect())
}
