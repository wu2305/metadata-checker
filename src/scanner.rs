use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use twox_hash::XxHash64;

use crate::graph::{EdgeType, FileState, GraphDB, NodeType};

/// 项目目录扫描模块
///
/// 递归扫描项目目录下的所有 .spg 和 .tbl 文件：
/// - .spg：解析组件、表达式、动作，构建 Page/Component/Model/Field 节点
/// - .tbl：解析 App 类型（可写）和 DataFlow 类型（只读加工流程）
///
/// 支持增量更新：对比文件 mtime/size/hash，只重新处理变更文件。

/// Scan a project directory and build/update the graph database.
pub fn scan_project(project_dir: &Path, db_path: &Path) -> Result<()> {
    let mut graph = GraphDB::open(db_path)?;
    let prev_states = graph.load_file_states().unwrap_or_default();

    // Collect all .spg and .tbl files
    let mut files = Vec::new();
    collect_files(project_dir, project_dir, &mut files)?;

    // Determine which files changed or were deleted
    let mut dirty_files = Vec::new();
    let mut current_paths = HashMap::new();
    for path in &files {
        let rel = path
            .strip_prefix(project_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        current_paths.insert(rel.clone(), path.clone());

        let content_bytes = fs::read(path)?;
        let mut hasher = XxHash64::default();
        hasher.write(&content_bytes);
        let file_hash = format!("{:x}", hasher.finish());

        match prev_states.get(&rel) {
            Some(state) if state.file_hash == file_hash => {
                // File unchanged, skip
            }
            _ => {
                dirty_files.push((rel, path.clone(), content_bytes));
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
    for (rel, node_ids) in &deleted_files {
        graph.remove_nodes_by_ids(node_ids);
        new_states.remove(rel);
    }

    if dirty_files.is_empty() && deleted_files.is_empty() {
        eprintln!("No graph changes detected; skip persistence");
        return Ok(());
    }

    for (rel, path, content_bytes) in &dirty_files {
        // Remove old nodes if updating
        if let Some(old_state) = prev_states.get(rel) {
            graph.remove_nodes_by_ids(&old_state.node_ids);
        }

        let node_ids = if path.extension().map(|e| e == "spg").unwrap_or(false) {
            let raw_value: serde_json::Value = serde_json::from_slice(content_bytes)?;
            process_spg_file_from_value(&mut graph, rel, raw_value)?
        } else if path.extension().map(|e| e == "tbl").unwrap_or(false) {
            let content = String::from_utf8_lossy(content_bytes);
            process_tbl_file_from_string(&mut graph, rel, &content)?
        } else {
            Vec::new()
        };

        // Reuse hash from scan loop
        let mut hasher = XxHash64::default();
        hasher.write(content_bytes);
        let file_hash = format!("{:x}", hasher.finish());

        let metadata = fs::metadata(path)?;
        let mtime = metadata
            .modified()?
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let size = metadata.len();

        new_states.insert(
            rel.clone(),
            FileState {
                file_path: rel.clone(),
                file_hash,
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

/// 递归收集目录下所有 .spg 和 .tbl 文件
fn collect_files(dir: &Path, base: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, base, files)?;
            } else if path
                .extension()
                .map(|e| e == "spg" || e == "tbl")
                .unwrap_or(false)
            {
                files.push(path);
            }
        }
    }
    Ok(())
}

/// Resolve a reference path from referenceResources to an absolute path.
/// Handles relative paths (../, ./) and $TAPP: prefix.
fn resolve_reference_path(
    rel_path: &str,
    ref_idx: usize,
    reference_resources: &[String],
) -> Option<String> {
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

/// 解析单个 .spg 文件并写入图
fn process_spg_file_from_value(
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
                            graph.add_edge(
                                &comp_id,
                                &model_id,
                                EdgeType::Reads,
                                Some(format!("{}.{}", model, field)),
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
                    let target_page_id = format!("page:{}", target_name);
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
                                graph.add_edge(
                                    &action_id,
                                    &model_id,
                                    EdgeType::ActionWrites,
                                    Some(format!("{}.{}", model, field)),
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
                                        graph.add_edge(
                                            &action_id,
                                            &ref_model_id,
                                            EdgeType::Reads,
                                            Some(format!("{}.{}", model, field)),
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
                                        let target_page_id = format!("page:{}", target_name);
                                        graph.add_node(
                                            target_page_id.clone(),
                                            NodeType::Page,
                                            target_rel.clone(),
                                            target_name.clone(),
                                            None,
                                        );
                                        graph.add_edge(
                                            &action_id,
                                            &target_page_id,
                                            EdgeType::OpensPage,
                                            Some(target_rel.clone()),
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
                                            graph.add_edge(
                                                &action_id,
                                                &param_id,
                                                EdgeType::PassesParam,
                                                Some(param_value.clone()),
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
                                                    graph.add_edge(
                                                        &action_id,
                                                        &ref_model_id,
                                                        EdgeType::Reads,
                                                        Some(format!("{}.{}", model, field)),
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
                        graph.add_edge(
                            &action_id,
                            &param_id,
                            EdgeType::SetsParam,
                            Some(param_value.clone()),
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
                                graph.add_edge(
                                    &action_id,
                                    &ref_model_id,
                                    EdgeType::Reads,
                                    Some(format!("{}.{}", model, field)),
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

/// 解析单个 .tbl 文件并写入图
fn process_tbl_file_from_string(
    graph: &mut GraphDB,
    rel_path: &str,
    content: &str,
) -> Result<Vec<String>> {
    let mut node_ids = std::collections::HashSet::new();
    if content.is_empty() {
        return Ok(node_ids.into_iter().collect());
    }

    let value: serde_json::Value = match serde_json::from_str(&content) {
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

    graph.add_node(
        model_id.clone(),
        NodeType::Model,
        rel_path.to_string(),
        model_name.clone(),
        Some(serde_json::json!({"modelType": model_type})),
    );
    node_ids.insert(model_id.clone());

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
                node_ids.insert(field_id.clone());
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
            }
        }
    }

    // For DataFlow type: process output physical table (dbTableName)
    if is_dataflow {
        if let Some(db_table_name) = value
            .get("properties")
            .and_then(|p| p.get("dbTableName"))
            .and_then(|v| v.as_str())
        {
            let output_model_id = format!("model:{}", db_table_name);
            let db_table_path = format!("{}.tbl", db_table_name);
            graph.add_node(
                output_model_id.clone(),
                NodeType::Model,
                db_table_path,
                db_table_name.to_string(),
                Some(serde_json::json!({"modelType": "PhysicalTable"})),
            );
            graph.add_edge(
                &model_id,
                &output_model_id,
                EdgeType::OutputsTo,
                Some(db_table_name.to_string()),
            );
        }
    }

    // For DataFlow type: process input sources (nodes referencing external tables)
    if is_dataflow {
        if let Some(nodes) = value
            .get("dataFlow")
            .and_then(|d| d.get("nodes"))
            .and_then(|n| n.as_object())
        {
            // First pass: build alias map, field mappings, and internal deps
            let mut internal_deps: HashMap<String, Vec<String>> = HashMap::new();
            let mut alias_map: HashMap<String, String> = HashMap::new();
            let mut node_fields: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
            let mut node_types: HashMap<String, String> = HashMap::new();

            for (node_id, node) in nodes {
                // Record alias -> node_id mapping
                if let Some(alias) = node.get("alias").and_then(|v| v.as_str()) {
                    alias_map.insert(alias.to_string(), node_id.clone());
                }
                // Record node type
                if let Some(node_type) = node.get("type").and_then(|v| v.as_str()) {
                    node_types.insert(node_id.clone(), node_type.to_string());
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
                                if step.get("type").and_then(|v| v.as_str()) == Some("AddField") {
                                    if let Some(add_field) = step.get("addField") {
                                        if add_field.get("name").and_then(|v| v.as_str())
                                            == Some(name)
                                        {
                                            if let Some(e) =
                                                add_field.get("exp").and_then(|v| v.as_str())
                                            {
                                                step_exp = Some(e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }

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
                        field_records.push(record);
                    }
                    if !field_records.is_empty() {
                        node_fields.insert(node_id.clone(), field_records);
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
