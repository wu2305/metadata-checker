#![cfg(feature = "cli-local")]
//! M57 Phase 2：真实项目 release 模式基线基准（只用于人工压测，不参与默认回归）。

use std::path::PathBuf;
use std::time::Instant;

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::graph_store::GraphWriteStore;
use metadata_checker::runtime::{GraphRuntime, ReadModelUpdateMode, RuntimeMode};

fn load_path(var_name: &str, default_path: &str) -> PathBuf {
    std::env::var(var_name)
        .unwrap_or_else(|_| default_path.to_string())
        .into()
}

#[test]
#[ignore = "manual benchmark for real project phase2 baseline"]
fn m57_real_project_release_dirty_curve() -> std::result::Result<(), String> {
    let project_dir = load_path(
        "M57_REAL_PROJECT_DIR",
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
    );
    let graph_db_path = load_path("M57_REAL_GRAPH_DB", "/private/tmp/m57-xiaoshouyi.graphdb");

    let project_meta = std::fs::metadata(&project_dir).map_err(|error| {
        format!(
            "Invalid M57_REAL_PROJECT_DIR {}: {error}",
            project_dir.display()
        )
    })?;
    if !project_meta.is_dir() {
        return Err(format!(
            "M57_REAL_PROJECT_DIR must be a directory: {}",
            project_dir.display()
        ));
    }

    let graph_meta = std::fs::metadata(&graph_db_path).map_err(|error| {
        format!(
            "Invalid M57_REAL_GRAPH_DB {}: {error}",
            graph_db_path.display()
        )
    })?;
    if !graph_meta.is_file() {
        return Err(format!(
            "M57_REAL_GRAPH_DB must be an existing graphdb file: {}",
            graph_db_path.display()
        ));
    }

    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &graph_db_path,
        Some(&project_dir),
        RuntimeMode::LongLived,
    )
    .map_err(|error| {
        format!(
            "Failed to load GraphRuntime from project_dir={} graph_db={}: {error}",
            project_dir.display(),
            graph_db_path.display()
        )
    })?;
    let runtime_load_ms = runtime.graph_load_ms;
    let full_read_model_ms = runtime.read_model_build_ms;

    let node_count = runtime.graph.node_count().map_err(|error| {
        format!(
            "Failed to read node count from {}: {error}",
            graph_db_path.display()
        )
    })?;
    let edge_count = runtime.graph.edge_count().map_err(|error| {
        format!(
            "Failed to read edge count from {}: {error}",
            graph_db_path.display()
        )
    })?;
    let topology_unchanged = true;

    let node_ids: Vec<String> = runtime
        .graph
        .iter_nodes()
        .map_err(|error| {
            format!(
                "Failed to iterate nodes in graph {}: {error}",
                graph_db_path.display()
            )
        })?
        .map(|node| node.id)
        .collect();
    assert_eq!(node_ids.is_empty(), false);

    let mut points = Vec::new();
    for dirty_count in [1_usize, 10, 100] {
        let mut candidate = GraphDB::open(&graph_db_path).map_err(|error| {
            format!(
                "Failed to open candidate graph {}: {error}",
                graph_db_path.display()
            )
        })?;
        let candidate_node_count = candidate
            .node_count()
            .map_err(|error| format!("Failed to read candidate node_count: {error}"))?;
        let candidate_edge_count = candidate
            .edge_count()
            .map_err(|error| format!("Failed to read candidate edge_count: {error}"))?;
        assert_eq!(
            (candidate_node_count, candidate_edge_count),
            (node_count, edge_count),
            "candidate topology must match baseline for dirty_count={dirty_count}"
        );

        let dirty_ids: Vec<String> = node_ids
            .iter()
            .take(dirty_count)
            .cloned()
            .collect::<Vec<_>>();
        for dirty_node_id in &dirty_ids {
            let mut node =
                metadata_checker::graph_store::GraphReadStore::get_node(&candidate, dirty_node_id)
                    .map_err(|error| {
                        format!(
                            "Failed to get node {dirty_node_id} from candidate graph {}: {error}",
                            graph_db_path.display()
                        )
                    })?
                    .ok_or_else(|| {
                        format!(
                            "Unable to find node in candidate graph {}: {dirty_node_id}",
                            graph_db_path.display()
                        )
                    })?;
            node.name = format!("{}-m57-real-benchmark", node.name);
            candidate
                .upsert_node(node)
                .map_err(|error| format!("Failed to upsert node {dirty_node_id}: {error}"))?;
        }

        let wall_started = Instant::now();
        let prepared = runtime
            .prepare_replacement(&candidate, &dirty_ids)
            .map_err(|error| {
                format!("prepare_replacement failed for dirty_count={dirty_count}: {error}")
            })?;
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
            "topology_unchanged": topology_unchanged,
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
        "candidate_mutation": "node_name_suffix",
        "topology_unchanged": topology_unchanged,
        "points": points,
    });
    eprintln!("M57_REAL_PHASE2_BASELINE {summary}");
    assert_eq!(points.len(), 3);
    Ok(())
}
