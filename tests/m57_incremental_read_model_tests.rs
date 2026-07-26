#![cfg(feature = "cli-local")]
//! M57 Phase 2：dirty/deleted 驱动的派生 read-model 增量更新契约。
//!
//! 这些测试先锁定三件事：稳定 node set 的增量路径必须被选中；
//! Dense/Facts/PageDep 与 full rebuild 等价；1/10/100 dirty 规模必须可观测。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use metadata_checker::dense_graph::DenseGraphSnapshot;
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::query::{
    MaterializedAvailabilityFactsIndex, PageDependencyIndex,
    build_query_page_logic_output_profiled_with_availability_cache,
};
use metadata_checker::runtime::{GraphRuntime, ReadModelUpdateMode, RuntimeMode};
use metadata_checker::scanner::indexer::ProjectIndexer;

fn test_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-m57-phase2-{name}-{}-{nanos}",
        std::process::id()
    ))
}

fn page_spg(value: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "sources": [
            {"id": "model_a", "modelType": "dwtable", "path": "data/table.tbl"}
        ],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [
                {"id": "input1", "type": "input", "submitField": "model_a.name"},
                {"id": "text1", "type": "text", "value": value}
            ]
        }
    })
    .to_string()
}

fn canonical_graph(store: &dyn GraphReadStore) -> anyhow::Result<Vec<String>> {
    let mut rows = Vec::new();
    for node in store.iter_nodes()? {
        let mut edges = store
            .get_node_edges(&node.id)?
            .map(|neighbors| {
                neighbors
                    .outgoing
                    .into_iter()
                    .map(|view| serde_json::to_string(&view.edge).expect("serialize edge"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        edges.sort();
        rows.push(serde_json::to_string(&(node, edges))?);
    }
    rows.sort();
    Ok(rows)
}

fn canonicalize_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                canonicalize_json(item);
            }
            items.sort_by_key(|item| item.to_string());
        }
        serde_json::Value::Object(map) => {
            for item in map.values_mut() {
                canonicalize_json(item);
            }
        }
        _ => {}
    }
}

fn build_project(name: &str) -> anyhow::Result<(PathBuf, PathBuf)> {
    let project_dir = test_root(name);
    std::fs::create_dir_all(project_dir.join("app"))?;
    std::fs::create_dir_all(project_dir.join("data"))?;
    std::fs::write(
        project_dir.join("app/page.spg"),
        page_spg("${model_a.name}"),
    )?;
    std::fs::write(
        project_dir.join("data/table.tbl"),
        r#"{"dimensions":[{"name":"id"},{"name":"name"}]}"#,
    )?;
    let db_path = project_dir.join("graph.redb");
    ProjectIndexer::scan(&project_dir, &db_path)?;
    Ok((project_dir, db_path))
}

/// 稳定 node set 的增量 read model 必须与 full rebuild 等价。
#[test]
fn m57_incremental_read_model_matches_full_rebuild() -> anyhow::Result<()> {
    let (project_dir, db_path) = build_project("equivalence")?;
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&project_dir),
        RuntimeMode::LongLived,
    )?;

    std::fs::write(
        project_dir.join("app/page.spg"),
        page_spg("changed literal"),
    )?;
    let prepared_index = ProjectIndexer::prepare(&project_dir, &db_path)?;
    assert_eq!(prepared_index.deleted_node_ids.is_empty(), true);
    assert_eq!(prepared_index.dirty_node_ids.is_empty(), false);

    let prepared =
        runtime.prepare_replacement(&prepared_index.graph, &prepared_index.dirty_node_ids)?;
    assert_eq!(
        prepared.read_model_update_mode,
        ReadModelUpdateMode::Incremental
    );

    let full_dense = DenseGraphSnapshot::from_graph(&prepared_index.graph)?;
    assert_eq!(
        canonical_graph(
            prepared
                .read_model
                .dense_graph
                .as_deref()
                .expect("dense model")
        )?,
        canonical_graph(&full_dense)?,
        "incremental dense snapshot must equal full rebuild"
    );

    let full_facts = MaterializedAvailabilityFactsIndex::build(&prepared_index.graph)?;
    assert_eq!(
        prepared.read_model.availability_facts.node_count,
        full_facts.node_count
    );
    assert_eq!(
        prepared.read_model.availability_facts.condition_count,
        full_facts.condition_count
    );

    let full_page_index = PageDependencyIndex::build(&prepared_index.graph)?;
    for node in prepared_index.graph.iter_nodes()? {
        assert_eq!(
            prepared
                .read_model
                .page_dependency_index
                .pages_for_node(&node.id),
            full_page_index.pages_for_node(&node.id),
            "page dependency mapping must equal full rebuild for {}",
            node.id
        );
    }

    let page_id = "page:app/page.spg";
    let (mut incremental_output, _) =
        build_query_page_logic_output_profiled_with_availability_cache(
            &prepared_index.graph,
            prepared.read_model.dense_graph.as_deref(),
            None,
            Some(&prepared.read_model.availability_facts),
            page_id,
            Some(&project_dir),
            "normal",
        )?;
    let (mut full_output, _) = build_query_page_logic_output_profiled_with_availability_cache(
        &prepared_index.graph,
        Some(&full_dense),
        None,
        Some(&full_facts),
        page_id,
        Some(&project_dir),
        "normal",
    )?;
    canonicalize_json(&mut incremental_output);
    canonicalize_json(&mut full_output);
    assert_eq!(incremental_output, full_output);

    let _ = std::fs::remove_dir_all(project_dir);
    Ok(())
}

/// 记录 1/10/100 dirty 规模，作为后续真实项目曲线的统一输出格式。
#[test]
fn m57_incremental_read_model_records_dirty_scale_curve() -> anyhow::Result<()> {
    let project_dir = Path::new("tests/fixtures/test_project");
    let db_path = test_root("curve").with_extension("graphdb");
    ProjectIndexer::scan(project_dir, &db_path)?;
    let candidate = GraphDB::open(&db_path)?;
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(project_dir),
        RuntimeMode::LongLived,
    )?;
    let node_ids: Vec<String> = candidate.iter_nodes()?.map(|node| node.id).collect();
    assert_eq!(node_ids.is_empty(), false);

    let mut curve = BTreeMap::new();
    for dirty_count in [1_usize, 10, 100] {
        let dirty_ids: Vec<String> = node_ids.iter().cycle().take(dirty_count).cloned().collect();
        let started = Instant::now();
        let prepared = runtime.prepare_replacement(&candidate, &dirty_ids)?;
        let elapsed_ms = started.elapsed().as_millis();
        assert_eq!(
            prepared.read_model_update_mode,
            ReadModelUpdateMode::Incremental
        );
        curve.insert(dirty_count, (elapsed_ms, prepared.read_model_build_ms));
        eprintln!(
            "M57_PHASE2 dirty_count={} wall_ms={} read_model_ms={}",
            dirty_count, elapsed_ms, prepared.read_model_build_ms
        );
    }
    assert_eq!(curve.len(), 3);

    let _ = std::fs::remove_file(db_path);
    Ok(())
}
