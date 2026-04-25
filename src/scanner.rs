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
                        let field_id = format!("field:{}.{})", model, field);
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
                        graph.add_edge(&comp_id, &model_id, EdgeType::Reads, Some(format!("{}.{})", model, field)));
                        graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
                    }
                    _ => {}
                }
            }
        }
    }

    // Process submitField writes
    for comp in &meta.components {
        let comp_id = format!("comp:{}/{}", page_name, comp.id);
        
        if let Some(submit_field) = comp.properties.get("submitField") {
            let parts: Vec<&str> = submit_field.split('.').collect();
            if parts.len() >= 2 {
                let model = parts[0];
                let field = parts[1..].join(".");
                let model_id = format!("model:{}", model);
                let field_id = format!("field:{}.{})", model, field);
                
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
                graph.add_edge(&comp_id, &model_id, EdgeType::Writes, Some(format!("{}.{})", model, field)));
                graph.add_edge(&model_id, &field_id, EdgeType::Contains, None);
            }
        }
    }

    Ok(node_ids)
}

fn process_tbl_file(
    _graph: &mut GraphDB,
    _rel_path: &str,
    _path: &Path,
) -> Result<Vec<String>> {
    // TODO: Parse .tbl file to extract model fields and DataFlow inputs
    Ok(Vec::new())
}