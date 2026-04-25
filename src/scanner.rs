use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::graph::{EdgeType, FileState, GraphDB, NodeType};
use crate::superpage::parse_superpage;

/// Scan a project directory and build/update the graph database.
pub fn scan_project(
    project_dir: &Path,
    db_path: &Path,
) -> Result<()> {
    let mut graph = GraphDB::open(db_path)?;
    let prev_states = graph.load_file_states().unwrap_or_default();

    // Collect all .spg and .tbl files
    let mut files = Vec::new();
    collect_files(project_dir, project_dir, &mut files)?;

    // Determine which files changed or were deleted
    let mut dirty_files = Vec::new();
    let mut current_paths = HashMap::new();
    for path in &files {
        let rel = path.strip_prefix(project_dir).unwrap_or(path).to_string_lossy().to_string();
        current_paths.insert(rel.clone(), path.clone());

        let metadata = fs::metadata(path)?;
        let mtime = metadata.modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let size = metadata.len();

        match prev_states.get(&rel) {
            Some(state) if state.mtime == mtime && state.size == size => {
                // File unchanged, skip
            }
            _ => {
                dirty_files.push((rel, path.clone()));
            }
        }
    }

    // Find deleted files
    let mut deleted_files = Vec::new();
    for (rel, state) in &prev_states {
        if !current_paths.contains_key(rel) {
            deleted_files.push((rel.clone(), state.node_ids.clone()));
        }
    }

    eprintln!(
        "Indexed {} files | Unchanged: {} | Dirty: {} | Deleted: {}",
        files.len(),
        files.len() - dirty_files.len(),
        dirty_files.len(),
        deleted_files.len()
    );

    // Process dirty files
    let mut new_states = prev_states.clone();
    for (rel, path) in &dirty_files {
        // Remove old nodes if updating
        if let Some(old_state) = prev_states.get(rel) {
            for node_id in &old_state.node_ids {
                if let Some(idx) = graph.node_indices.get(node_id).copied() {
                    graph.graph.remove_node(idx);
                    graph.node_indices.remove(node_id);
                }
            }
        }

        let node_ids = if path.extension().map(|e| e == "spg").unwrap_or(false) {
            process_spg_file(&mut graph, rel, path)?
        } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
            process_tbl_file(&mut graph, rel, path)?
        } else {
            Vec::new()
        };

        let metadata = fs::metadata(path)?;
        let mtime = metadata.modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let size = metadata.len();

        new_states.insert(
            rel.clone(),
            FileState {
                file_path: rel.clone(),
                file_hash: format!("{}-{}", mtime, size),
                mtime,
                size,
                node_ids,
            },
        );
    }

    // Persist graph and file states
    graph.persist(&new_states)?;
    Ok(())
}

fn collect_files(dir: &Path, base: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, base, files)?;
            } else if path.extension().map(|e| e == "spg" || e == "tbl").unwrap_or(false) {
                files.push(path);
            }
        }
    }
    Ok(())
}

/// Resolve a reference path from referenceResources to an absolute path.
/// Handles relative paths (../, ./) and $TAPP: prefix.
fn resolve_reference_path(rel_path: &str, ref_idx: usize, reference_resources: &[String]) -> Option<String> {
    let ref_path = reference_resources.get(ref_idx)?;
    if ref_path.starts_with("$TAPP:") {
        // $TAPP:/path/to/page.spg → resolve relative to project root
        let path = ref_path.strip_prefix("$TAPP:").unwrap_or(ref_path);
        Some(path.trim_start_matches('/').to_string())
    } else {
        // Relative path: resolve based on current .spg directory
        let current_dir = Path::new(rel_path).parent()?;
        let resolved = current_dir.join(ref_path);
        Some(resolved.to_string_lossy().to_string().replace('\\', "/"))
    }
}

fn process_spg_file(
    graph: &mut GraphDB,
    rel_path: &str,
    path: &Path,
) -> Result<Vec<String>> {
    let meta = parse_superpage(path)?;
    let mut node_ids = Vec::new();

    let page_name = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let page_id = format!("page:{}", page_name);
    graph.add_node(
        page_id.clone(),
        NodeType::Page,
        rel_path.to_string(),
        page_name.clone(),
        None,
    );
    node_ids.push(page_id.clone());

    // Process embedded DataFlow models in sources
    for source in &meta.sources {
        if source.model_type.as_deref() != Some("dataflow") {
            continue;
        }
        let Some(content) = &source.content else { continue; };
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
            node_ids.push(model_id.clone());
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
                        node_ids.push(field_id.clone());
                    }
                    graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                }
            }
        }

        // Extract dataFlow input sources (external tables referenced by ModelTable nodes)
        if let Some(nodes) = content.get("dataFlow").and_then(|d| d.get("nodes")).and_then(|n| n.as_object()) {
            for (_, node) in nodes {
                if let Some(module_table_path) = node.get("moduleTablePath").and_then(|p| p.as_str()) {
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
                    graph.add_edge(&model_id, &ref_model_id, EdgeType::DataflowInput, Some(module_table_path.to_string()));
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

    // Process components and their expressions
    for comp in &meta.components {
        let comp_id = format!("comp:{}/{}", page_name, comp.id);
        graph.add_node(
            comp_id.clone(),
            NodeType::Component,
            rel_path.to_string(),
            comp.id.clone(),
            None,
        );
        node_ids.push(comp_id.clone());
        graph.add_edge(&page_id, &comp_id, EdgeType::Contains, None);

        // Process expressions (reads)
        for expr in &meta.expressions {
            if expr.component_id != comp.id {
                continue;
            }
            for ref_type in &expr.refs {
                match ref_type {
                    crate::superpage::RefType::ModelField(model, field) => {
                        let model_id = format!("model:{}", model);
                        let field_id = format!("field:{}.{}", model, field);
                        graph.add_node(
                            model_id.clone(),
                            NodeType::Model,
                            format!("{}.tbl", model),
                            model.clone(),
                            None,
                        );
                        graph.add_node(
                            field_id.clone(),
                            NodeType::Field,
                            format!("{}.tbl", model),
                            format!("{}", field),
                            None,
                        );
                        graph.add_edge(&comp_id, &model_id, EdgeType::Reads, Some(format!("{}.{}", model, field)));
                        graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                    }
                    _ => {}
                }
            }
        }
    }

    // Process submitField writes (implicit writes from component bindings)
    for comp in &meta.components {
        let comp_id = format!("comp:{}/{}", page_name, comp.id);
        
        if let Some(submit_field) = comp.properties.get("submitField") {
            let parts: Vec<&str> = submit_field.split('.').collect();
            if parts.len() >= 2 {
                let model = parts[0];
                let field = parts[1..].join(".");
                let model_id = format!("model:{}", model);
                let field_id = format!("field:{}.{}", model, field);
                
                graph.add_node(
                    model_id.clone(),
                    NodeType::Model,
                    format!("{}.tbl", model),
                    model.to_string(),
                    None,
                );
                graph.add_node(
                    field_id.clone(),
                    NodeType::Field,
                    format!("{}.tbl", model),
                    field.clone(),
                    None,
                );
                graph.add_edge(&comp_id, &model_id, EdgeType::Writes, Some(format!("{}.{}", model, field)));
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
            }
        }
    }

    // Process embedsuperpage components (page embedding)
    for comp in &meta.components {
        if comp.component_type != "embedsuperpage" {
            continue;
        }
        let comp_id = format!("comp:{}/{}", page_name, comp.id);
        
        // Try to resolve resPath as integer index into referenceResources
        if let Some(ref res_path_str) = comp.res_path {
            if let Ok(ref_idx) = res_path_str.parse::<usize>() {
                if let Some(target_rel) = resolve_reference_path(rel_path, ref_idx, &meta.reference_resources) {
                    let target_name = Path::new(&target_rel)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| target_rel.clone());
                    let target_page_id = format!("page:{}", target_name);
                    graph.add_node(
                        target_page_id.clone(),
                        NodeType::Page,
                        target_rel.clone(),
                        target_name.clone(),
                        None,
                    );
                    graph.add_edge(&comp_id, &target_page_id, EdgeType::EmbedsPage, Some(target_rel.clone()));
                }
            }
        }
    }

    // Process actions (explicit writes from interactions)
    for comp in &meta.components {
        let comp_id = format!("comp:{}/{}", page_name, comp.id);
        for action in &comp.actions {
            let action_id = format!("action:{}/{}/{}", page_name, comp.id, action.id);
            graph.add_node(
                action_id.clone(),
                NodeType::Action,
                rel_path.to_string(),
                format!("{}:{}", action.action_type, action.id),
                None,
            );
            node_ids.push(action_id.clone());
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
                                let field = parts[1..].join(".");
                                let model_id = format!("model:{}", model);
                                let field_id = format!("field:{}.{}", model, field);
                                graph.add_node(model_id.clone(), NodeType::Model, format!("{}.tbl", model), model.to_string(), None);
                                graph.add_node(field_id.clone(), NodeType::Field, format!("{}.tbl", model), field.clone(), None);
                                graph.add_edge(&action_id, &model_id, EdgeType::ActionWrites, Some(format!("{}.{}", model, field)));
                                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                            }
                        }
                    }
                }
                "updateData" | "insertData" | "deleteData" => {
                    if let Some(ref data_set) = action.data_set {
                        let model_id = format!("model:{}", data_set);
                        graph.add_node(model_id.clone(), NodeType::Model, format!("{}.tbl", data_set), data_set.clone(), None);
                        for (field_name, field_value, value_type) in &action.field_values {
                            let field_id = format!("field:{}.{}", data_set, field_name);
                            graph.add_node(field_id.clone(), NodeType::Field, format!("{}.tbl", data_set), field_name.clone(), None);
                            graph.add_edge(&action_id, &model_id, EdgeType::ActionWrites, Some(format!("{}.{}", data_set, field_name)));
                            graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                            // If value_type is "exp", parse expression refs for dependency analysis
                            if value_type == "exp" {
                                let refs = crate::superpage::parse_expression_refs(field_value);
                                for ref_type in refs {
                                    if let crate::superpage::RefType::ModelField(model, field) = ref_type {
                                        let ref_model_id = format!("model:{}", model);
                                        let ref_field_id = format!("field:{}.{}", model, field);
                                        graph.add_node(ref_model_id.clone(), NodeType::Model, format!("{}.tbl", model), model.clone(), None);
                                        graph.add_node(ref_field_id.clone(), NodeType::Field, format!("{}.tbl", model), field.clone(), None);
                                        graph.add_edge(&action_id, &ref_model_id, EdgeType::Reads, Some(format!("{}.{}", model, field)));
                                        graph.add_edge(&ref_model_id, &ref_field_id, EdgeType::Contains, None);
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
                                    if let Some(target_rel) = resolve_reference_path(rel_path, ref_idx, &meta.reference_resources) {
                                        let target_name = Path::new(&target_rel)
                                            .file_stem()
                                            .map(|s| s.to_string_lossy().to_string())
                                            .unwrap_or_else(|| target_rel.clone());
                                        let target_page_id = format!("page:{}", target_name);
                                        graph.add_node(
                                            target_page_id.clone(),
                                            NodeType::Page,
                                            target_rel.clone(),
                                            target_name.clone(),
                                            None,
                                        );
                                        graph.add_edge(&action_id, &target_page_id, EdgeType::OpensPage, Some(target_rel.clone()));
                                        
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
                                            graph.add_edge(&action_id, &param_id, EdgeType::PassesParam, Some(param_value.clone()));
                                            // Parse expression refs from param_value for dependency analysis
                                            let refs = crate::superpage::parse_expression_refs(param_value);
                                            for ref_type in refs {
                                                if let crate::superpage::RefType::ModelField(model, field) = ref_type {
                                                    let ref_model_id = format!("model:{}", model);
                                                    let ref_field_id = format!("field:{}.{}", model, field);
                                                    graph.add_node(ref_model_id.clone(), NodeType::Model, format!("{}.tbl", model), model.clone(), None);
                                                    graph.add_node(ref_field_id.clone(), NodeType::Field, format!("{}.tbl", model), field.clone(), None);
                                                    graph.add_edge(&action_id, &ref_model_id, EdgeType::Reads, Some(format!("{}.{}", model, field)));
                                                    graph.add_edge(&ref_model_id, &ref_field_id, EdgeType::Contains, None);
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
                        let param_id = format!("param:{}/{}", page_name, param_name);
                        graph.add_node(
                            param_id.clone(),
                            NodeType::Field,
                            rel_path.to_string(),
                            param_name.clone(),
                            None,
                        );
                        graph.add_edge(&action_id, &param_id, EdgeType::SetsParam, Some(param_value.clone()));
                        // Parse expression refs from param_value for dependency analysis
                        let refs = crate::superpage::parse_expression_refs(param_value);
                        for ref_type in refs {
                            if let crate::superpage::RefType::ModelField(model, field) = ref_type {
                                let ref_model_id = format!("model:{}", model);
                                let ref_field_id = format!("field:{}.{}", model, field);
                                graph.add_node(ref_model_id.clone(), NodeType::Model, format!("{}.tbl", model), model.clone(), None);
                                graph.add_node(ref_field_id.clone(), NodeType::Field, format!("{}.tbl", model), field.clone(), None);
                                graph.add_edge(&action_id, &ref_model_id, EdgeType::Reads, Some(format!("{}.{}", model, field)));
                                graph.add_edge(&ref_model_id, &ref_field_id, EdgeType::Contains, None);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }


    Ok(node_ids)
}

fn process_tbl_file(
    graph: &mut GraphDB,
    rel_path: &str,
    path: &Path,
) -> Result<Vec<String>> {
    let mut node_ids = Vec::new();
    let content = fs::read_to_string(path).ok().unwrap_or_default();
    if content.is_empty() {
        return Ok(node_ids);
    }

    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(node_ids),
    };

    // Use file stem as model identifier
    let model_name = Path::new(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let model_id = format!("model:{}", model_name);
    let is_dataflow = value.get("dataFlow").is_some();
    let model_type = if is_dataflow { "DataFlow" } else { "App" };

    graph.add_node(
        model_id.clone(),
        NodeType::Model,
        rel_path.to_string(),
        model_name.clone(),
        Some(serde_json::json!({"modelType": model_type})),
    );
    node_ids.push(model_id.clone());

    // Process dimensions (fields)
    if let Some(dimensions) = value.get("dimensions").and_then(|d| d.as_array()) {
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
                node_ids.push(field_id.clone());
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
            }
        }
    }

    // For DataFlow type: process output physical table (dbTableName)
    if is_dataflow {
        if let Some(db_table_name) = value.get("properties").and_then(|p| p.get("dbTableName")).and_then(|v| v.as_str()) {
            let output_model_id = format!("model:{}", db_table_name);
            graph.add_node(
                output_model_id.clone(),
                NodeType::Model,
                format!("{}.tbl", db_table_name),
                db_table_name.to_string(),
                Some(serde_json::json!({"modelType": "PhysicalTable"})),
            );
            graph.add_edge(&model_id, &output_model_id, EdgeType::OutputsTo, Some(db_table_name.to_string()));
        }
    }

    // For DataFlow type: process input sources (nodes referencing external tables)
    if is_dataflow {
        if let Some(nodes) = value.get("dataFlow").and_then(|d| d.get("nodes")).and_then(|n| n.as_object()) {
            // First pass: create internal node entries and record inputNodes for subGraph expansion
            let mut internal_deps: HashMap<String, Vec<String>> = HashMap::new();
            for (node_id, node) in nodes {
                if let Some(input_nodes) = node.get("inputNodes").and_then(|v| v.as_array()) {
                    let deps: Vec<String> = input_nodes.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    if !deps.is_empty() {
                        internal_deps.insert(node_id.clone(), deps);
                    }
                }
            }
            // Store internal deps in model metadata for subGraph expansion
            if let Some(model_node) = graph.graph.node_weight_mut(*graph.node_indices.get(&model_id).unwrap()) {
                let mut meta = model_node.meta.clone().unwrap_or(serde_json::Value::Null);
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("internalDeps".to_string(), serde_json::to_value(&internal_deps).unwrap_or(serde_json::Value::Null));
                }
                model_node.meta = Some(meta);
            }
            // Second pass: create DataflowInput edges for ModelTable nodes
            for (_, node) in nodes {
                if let Some(module_table_path) = node.get("moduleTablePath").and_then(|p| p.as_str()) {
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
                    graph.add_edge(&model_id, &ref_model_id, EdgeType::DataflowInput, Some(module_table_path.to_string()));
                }
            }
        }
    }

    Ok(node_ids)
}
