#![cfg(feature = "cli-local")]
//! M57 Phase 2：真实项目 release 模式基线基准（只用于人工压测，不参与默认回归）。

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::runtime::{GraphRuntime, ReadModelUpdateMode, RuntimeMode};

fn load_path(var_name: &str, default_path: &str) -> PathBuf {
    std::env::var(var_name)
        .unwrap_or_else(|_| default_path.to_string())
        .into()
}

#[test]
#[ignore = "manual benchmark for real project phase2 baseline"]
fn m57_real_project_release_dirty_curve() -> Result<()> {
    let project_dir = load_path(
        "M57_REAL_PROJECT_DIR",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
    );
    let graph_db_path = load_path("M57_REAL_GRAPH_DB", "/private/tmp/m57-xiaoshouyi.graphdb");

    let project_meta = std::fs::metadata(&project_dir)
        .with_context(|| format!("Invalid M57_REAL_PROJECT_DIR: {}", project_dir.display()))?;
    if !project_meta.is_dir() {
        bail!(
            "M57_REAL_PROJECT_DIR must be a directory: {}",
            project_dir.display()
        );
    }

    let graph_meta = std::fs::metadata(&graph_db_path)
        .with_context(|| format!("Invalid M57_REAL_GRAPH_DB: {}", graph_db_path.display()))?;
    if !graph_meta.is_file() {
        bail!(
            "M57_REAL_GRAPH_DB must be an existing graphdb file: {}",
            graph_db_path.display()
        );
    }

    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &graph_db_path,
        Some(&project_dir),
        RuntimeMode::LongLived,
    )?;
    let runtime_load_ms = runtime.graph_load_ms;
    let full_read_model_ms = runtime.read_model_build_ms;

    let node_count = runtime
        .graph
        .node_count()
        .with_context(|| format!("Failed to read node count: {}", graph_db_path.display()))?;
    let edge_count = runtime
        .graph
        .edge_count()
        .with_context(|| format!("Failed to read edge count: {}", graph_db_path.display()))?;

    let node_ids: Vec<String> = runtime.graph.iter_nodes()?.map(|node| node.id).collect();
    assert_eq!(node_ids.is_empty(), false);

    let mut points = Vec::new();
    for dirty_count in [1_usize, 10, 100] {
        let dirty_ids = node_ids
            .iter()
            .take(dirty_count)
            .cloned()
            .collect::<Vec<_>>();
        let wall_started = Instant::now();
        let prepared = runtime.prepare_replacement(&runtime.graph, &dirty_ids)?;
        let wall_ms = wall_started.elapsed().as_millis();
        assert_eq!(
            prepared.read_model_update_mode,
            ReadModelUpdateMode::Incremental,
            "dirty_count={} must use Incremental mode",
            dirty_count
        );
        points.push(serde_json::json!({
            "dirty_count": dirty_count,
            "wall_ms": wall_ms,
            "read_model_build_ms": prepared.read_model_build_ms,
            "dense_snapshot_build_ms": prepared.dense_snapshot_build_ms,
            "availability_facts_build_ms": prepared.availability_facts_build_ms,
            "page_dependency_index_build_ms": prepared.page_dependency_index_build_ms,
        }));
    }

    let summary = serde_json::json!({
        "schema_version": "1.0",
        "kind": "m57_real_project_release_dirty_curve",
        "project_dir": project_dir.display().to_string(),
        "graph_db": graph_db_path.display().to_string(),
        "node_count": node_count,
        "edge_count": edge_count,
        "runtime_load_ms": runtime_load_ms,
        "full_read_model_ms": full_read_model_ms,
        "points": points,
    });
    eprintln!("M57_REAL_PHASE2_BASELINE {summary}");
    assert_eq!(points.len(), 3);
    Ok(())
}
