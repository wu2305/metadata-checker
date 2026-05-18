use crate::graph::{EdgeType, GraphDB, NodeType};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

pub fn process_tbl_file_from_string(
    graph: &mut GraphDB,
    rel_path: &str,
    content: &str,
) -> Result<Vec<String>> {
    let mut node_ids = std::collections::HashSet::new();
    if content.is_empty() {
        return Ok(node_ids.into_iter().collect());
    }

    let value: serde_json::Value = match serde_json::from_str(content) {
        Ok(v) => v,
        Err(_) => return Ok(node_ids.into_iter().collect()),
    };

    // Use file stem as model identifier
    let model_name = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let model_id = format!("model:{}", model_name);
    let is_dataflow = value.get("dataFlow").is_some();
    let model_type = if is_dataflow { "DataFlow" } else { "App" };

    let mut model_meta = serde_json::json!({"modelType": model_type});
    // Store dimensions in model meta so chain lineage can resolve field mappings
    if let Some(dims) = value.get("dimensions")
        && let Some(obj) = model_meta.as_object_mut()
    {
        obj.insert("dimensions".to_string(), dims.clone());
    }
    graph.add_node(
        model_id.clone(),
        NodeType::Model,
        rel_path.to_string(),
        model_name.clone(),
        Some(model_meta),
    );
    node_ids.insert(model_id.clone());

    // Process dimensions (fields)
    if let Some(dimensions) = value.get("dimensions").and_then(|d| d.as_array()) {
        for dim in dimensions {
            if let Some(name) = dim.get("name").and_then(|n| n.as_str()) {
                let field_id = format!("field:{}.{}", model_name, name);
                let mut field_meta = dim.clone();
                // 添加字段来源信息到 meta
                if let Some(input_field) = dim.get("inputField").and_then(|v| v.as_str())
                    && let Some(obj) = field_meta.as_object_mut()
                {
                    obj.insert(
                        "source_input_field".to_string(),
                        serde_json::json!(input_field),
                    );
                }
                if let Some(exp) = dim.get("exp").and_then(|v| v.as_str())
                    && let Some(obj) = field_meta.as_object_mut()
                {
                    obj.insert("source_expr".to_string(), serde_json::json!(exp));
                    // 粗粒度解析表达式中的字段引用
                    let refs = crate::superpage::parse_expression_refs(exp);
                    let ref_models: Vec<String> = refs
                        .iter()
                        .filter_map(|r| match r {
                            crate::superpage::RefType::ModelField(m, _) => Some(m.clone()),
                            _ => None,
                        })
                        .collect();
                    if !ref_models.is_empty() {
                        obj.insert(
                            "source_expr_models".to_string(),
                            serde_json::json!(ref_models),
                        );
                    }
                }
                graph.add_node(
                    field_id.clone(),
                    NodeType::Field,
                    rel_path.to_string(),
                    name.to_string(),
                    Some(field_meta),
                );
                node_ids.insert(field_id.clone());
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
            }
        }
    }

    // Process output physical table (dbTableName) for both App and DataFlow
    if let Some(db_table_name) = value
        .get("properties")
        .and_then(|p| p.get("dbTableName"))
        .and_then(|v| v.as_str())
    {
        let output_model_id = format!("model:{}", db_table_name);
        let db_table_path = format!("{}.tbl", db_table_name);
        // Merge with existing meta if node already exists (e.g., physical table also has its own .tbl file)
        let output_meta = if let Some(existing) = graph.get_node(&output_model_id) {
            let mut merged = existing.meta.clone().unwrap_or(serde_json::json!({}));
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("modelType".to_string(), serde_json::json!("PhysicalTable"));
            }
            Some(merged)
        } else {
            Some(serde_json::json!({"modelType": "PhysicalTable"}))
        };
        graph.add_node(
            output_model_id.clone(),
            NodeType::Model,
            db_table_path.clone(),
            db_table_name.to_string(),
            output_meta,
        );
        graph.add_edge(
            &model_id,
            &output_model_id,
            EdgeType::OutputsTo,
            Some(db_table_name.to_string()),
        );
        // Also create field nodes for the output physical table so field-level lineage works
        if let Some(dims) = value.get("dimensions").and_then(|d| d.as_array()) {
            for dim in dims {
                if let Some(name) = dim.get("name").and_then(|n| n.as_str()) {
                    let field_id = format!("field:{}.{}", db_table_name, name);
                    let mut field_meta = dim.clone();
                    // 粗粒度解析表达式中的字段引用（如果有exp）
                    if let Some(exp) = dim.get("exp").and_then(|v| v.as_str()) {
                        let refs = crate::superpage::parse_expression_refs(exp);
                        let ref_models: Vec<String> = refs
                            .iter()
                            .filter_map(|r| match r {
                                crate::superpage::RefType::ModelField(m, _) => Some(m.clone()),
                                _ => None,
                            })
                            .collect();
                        if let Some(obj) = field_meta.as_object_mut() {
                            obj.insert("source_expr".to_string(), serde_json::json!(exp));
                            if !ref_models.is_empty() {
                                obj.insert(
                                    "source_expr_models".to_string(),
                                    serde_json::json!(ref_models),
                                );
                            }
                        }
                    }
                    if let Some(input_field) = dim.get("inputField").and_then(|v| v.as_str())
                        && let Some(obj) = field_meta.as_object_mut()
                    {
                        obj.insert(
                            "source_input_field".to_string(),
                            serde_json::json!(input_field),
                        );
                    }
                    graph.add_node(
                        field_id.clone(),
                        NodeType::Field,
                        db_table_path.clone(),
                        name.to_string(),
                        Some(field_meta),
                    );
                    graph.add_edge(&output_model_id, &field_id, EdgeType::Contains, None);
                }
            }
        }
    }

    // For DataFlow type: process explicit depends (dependencies on other .tbl files)
    if is_dataflow
        && let Some(depends) = value
            .get("properties")
            .and_then(|p| p.get("depends"))
            .and_then(|v| v.as_array())
    {
        for dep in depends {
            if let Some(dep_path) = dep.as_str() {
                let dep_model = Path::new(dep_path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| dep_path.to_string());
                let dep_model_id = format!("model:{}", dep_model);
                graph.add_node(
                    dep_model_id.clone(),
                    NodeType::Model,
                    dep_path.to_string(),
                    dep_model.clone(),
                    Some(serde_json::json!({"modelType": "DataFlowDependency"})),
                );
                graph.add_edge(
                    &model_id,
                    &dep_model_id,
                    EdgeType::DataflowInput,
                    Some(dep_path.to_string()),
                );
            }
        }
    }

    // For DataFlow type: process input sources (nodes referencing external tables)
    if is_dataflow
        && let Some(nodes) = value
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
    {
        // First pass: build alias map, field mappings, and internal deps
        let mut internal_deps: HashMap<String, Vec<String>> = HashMap::new();
        let mut alias_map: HashMap<String, String> = HashMap::new();
        let mut node_fields: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        let mut node_types: HashMap<String, String> = HashMap::new();
        let mut node_filters: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
        let mut node_table_paths: HashMap<String, String> = HashMap::new();

        for (node_id, node) in nodes {
            // Record alias -> node_id mapping
            if let Some(alias) = node.get("alias").and_then(|v| v.as_str()) {
                alias_map.insert(alias.to_string(), node_id.clone());
            }
            // Record node type
            if let Some(node_type) = node.get("type").and_then(|v| v.as_str()) {
                node_types.insert(node_id.clone(), node_type.to_string());
            }
            if let Some(module_table_path) = node.get("moduleTablePath").and_then(|v| v.as_str()) {
                node_table_paths.insert(node_id.clone(), module_table_path.to_string());
            }
            // Record inputNodes dependencies
            if let Some(input_nodes) = node.get("inputNodes").and_then(|v| v.as_array()) {
                let deps: Vec<String> = input_nodes
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                if !deps.is_empty() {
                    internal_deps.insert(node_id.clone(), deps);
                }
                // Record field mappings for each node
                if let Some(fields) = node.get("fields").and_then(|v| v.as_array()) {
                    let mut field_records: Vec<serde_json::Value> = Vec::new();
                    for field in fields {
                        let name = field.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let dbfield = field.get("dbfield").and_then(|v| v.as_str()).unwrap_or("");
                        let original_field = field.get("originalField").and_then(|v| v.as_str());
                        let original_node = field.get("originalNode").and_then(|v| v.as_str());
                        let exp = field.get("exp").and_then(|v| v.as_str());

                        // Include steps-based AddField expressions
                        let mut step_exp: Option<String> = None;
                        if let Some(steps) = node.get("steps").and_then(|v| v.as_array()) {
                            for step in steps {
                                if step.get("type").and_then(|v| v.as_str()) == Some("AddField")
                                    && let Some(add_field) = step.get("addField")
                                    && add_field.get("name").and_then(|v| v.as_str()) == Some(name)
                                    && let Some(e) = add_field.get("exp").and_then(|v| v.as_str())
                                {
                                    step_exp = Some(e.to_string());
                                }
                            }
                        }

                        let input_field = field.get("inputField").and_then(|v| v.as_str());
                        let mut record = serde_json::json!({
                            "name": name,
                            "dbfield": dbfield,
                        });
                        if let Some(of) = original_field {
                            record["originalField"] = serde_json::Value::String(of.to_string());
                        }
                        if let Some(on) = original_node {
                            record["originalNode"] = serde_json::Value::String(on.to_string());
                        }
                        if let Some(e) = exp {
                            record["exp"] = serde_json::Value::String(e.to_string());
                        } else if let Some(e) = step_exp {
                            record["exp"] = serde_json::Value::String(e.to_string());
                        }
                        if let Some(input) = input_field {
                            record["inputField"] = serde_json::Value::String(input.to_string());
                        }
                        field_records.push(record);
                    }
                    if !field_records.is_empty() {
                        node_fields.insert(node_id.clone(), field_records);
                    }
                }
            }

            if let Some(filter) = node.get("filter").and_then(|v| v.as_object()) {
                if let Some(clauses) = filter.get("clauses").and_then(|v| v.as_array()) {
                    let has_filter = !clauses.is_empty();
                    if has_filter {
                        node_filters
                            .insert(node_id.clone(), clauses.iter().cloned().collect::<Vec<_>>());
                    }
                }
            }

            // Store all DataFlow metadata for subGraph expansion
            if let Some(model_node) = graph
                .graph
                .node_weight_mut(*graph.node_indices.get(&model_id).unwrap())
            {
                let mut meta = model_node.meta.clone().unwrap_or(serde_json::Value::Null);
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert(
                        "internalDeps".to_string(),
                        serde_json::to_value(&internal_deps).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "aliasMap".to_string(),
                        serde_json::to_value(&alias_map).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "nodeFields".to_string(),
                        serde_json::to_value(&node_fields).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "nodeTypes".to_string(),
                        serde_json::to_value(&node_types).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "nodeTablePaths".to_string(),
                        serde_json::to_value(&node_table_paths).unwrap_or(serde_json::Value::Null),
                    );
                    obj.insert(
                        "nodeFilters".to_string(),
                        serde_json::to_value(&node_filters).unwrap_or(serde_json::Value::Null),
                    );
                    // Store dimensions for fallback when nodeFields is absent
                    if let Some(dims) = value.get("dimensions") {
                        obj.insert(
                            "dimensions".to_string(),
                            serde_json::to_value(dims).unwrap_or(serde_json::Value::Null),
                        );
                    }
                }
                model_node.meta = Some(meta);
            }
            // Second pass: create DataflowInput edges for ModelTable nodes
            for (_, node) in nodes {
                if let Some(module_table_path) =
                    node.get("moduleTablePath").and_then(|p| p.as_str())
                {
                    // Extract referenced model name from path like "$DATA:/售后/fact_serviceappointments.tbl"
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
                    // DataflowInput edge: this DataFlow reads from ref_model
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

    Ok(node_ids.into_iter().collect())
}
