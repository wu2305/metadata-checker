//! 预期事实由静态源夹具人工推导，并通过 Grafeo / GraphReadStore 验收。
//! 本 slice 检查 source path 与 JSON path，不把 origin_file=None 当作 ownership 通过。
#![cfg(all(feature = "cli-local", feature = "grafeo-store"))]

use anyhow::{Result, anyhow};
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_store::{GraphReadStore, IndexStateStore};
use metadata_checker::scanner::indexer::ProjectIndexer;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type NodeFact = (String, String, String, String);
type EdgeFact = (String, String, String, Option<String>);

fn test_dir(label: &str) -> Result<PathBuf> {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = std::env::temp_dir().join(format!(
        "m59-grafeo-source-facts-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn copy_fixture(project: &Path, source: &str, destination: &str) -> Result<()> {
    let fixture_root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m59_grafeo_source_facts");
    let destination = project.join(destination);
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("fixture destination has no parent"))?;
    fs::create_dir_all(parent)?;
    fs::copy(fixture_root.join(source), destination)?;
    Ok(())
}

fn seed_project(project: &Path) -> Result<()> {
    copy_fixture(project, "app/a.spg", "app/a.spg")?;
    copy_fixture(project, "app/b.spg", "app/b.spg")?;
    copy_fixture(project, "db/orders.tbl", "db/orders.tbl")?;
    Ok(())
}

fn observed_nodes(store: &GrafeoGraphStore) -> Result<Vec<NodeFact>> {
    let mut facts = store
        .iter_nodes()?
        .map(|node| {
            (
                node.id,
                format!("{:?}", node.node_type),
                node.path,
                node.name,
            )
        })
        .collect::<Vec<_>>();
    facts.sort();
    Ok(facts)
}

fn observed_edges(store: &GrafeoGraphStore) -> Result<Vec<EdgeFact>> {
    let nodes = store.iter_nodes()?.collect::<Vec<_>>();
    let mut facts = Vec::new();
    for node in nodes {
        let neighbors = store
            .get_node_edges(&node.id)?
            .ok_or_else(|| anyhow!("missing edge view for iterated node {}", node.id))?;
        for view in neighbors.outgoing {
            facts.push((
                view.edge.from,
                view.edge.to,
                format!("{:?}", view.edge.edge_type),
                view.edge.field_path,
            ));
        }
    }
    facts.sort();
    Ok(facts)
}

fn expected_nodes(updated: bool) -> Vec<NodeFact> {
    let b_component = if updated { "label_b_v2" } else { "label_b" };
    let mut facts = vec![
        (
            "comp:app/a.spg|canvas".into(),
            "Component".into(),
            "app/a.spg".into(),
            "canvas".into(),
        ),
        (
            "comp:app/a.spg|embed_b".into(),
            "Component".into(),
            "app/a.spg".into(),
            "embed_b".into(),
        ),
        (
            "comp:app/a.spg|label_a".into(),
            "Component".into(),
            "app/a.spg".into(),
            "label_a".into(),
        ),
        (
            "comp:app/b.spg|canvas".into(),
            "Component".into(),
            "app/b.spg".into(),
            "canvas".into(),
        ),
        (
            format!("comp:app/b.spg|{b_component}"),
            "Component".into(),
            "app/b.spg".into(),
            b_component.into(),
        ),
        (
            "field:orders.含税金额".into(),
            "Field".into(),
            "db/orders.tbl".into(),
            "含税金额".into(),
        ),
        (
            "field:orders.订单号".into(),
            "Field".into(),
            "db/orders.tbl".into(),
            "订单号".into(),
        ),
        (
            "model:orders".into(),
            "Model".into(),
            "db/orders.tbl".into(),
            "orders".into(),
        ),
        (
            "page:app/a.spg".into(),
            "Page".into(),
            "app/a.spg".into(),
            "a".into(),
        ),
        (
            "page:app/b.spg".into(),
            "Page".into(),
            "app/b.spg".into(),
            "b".into(),
        ),
    ];
    facts.sort();
    facts
}

fn expected_edges() -> Vec<EdgeFact> {
    let mut facts = vec![
        (
            "page:app/a.spg".into(),
            "comp:app/a.spg|canvas".into(),
            "Contains".into(),
            None,
        ),
        (
            "page:app/a.spg".into(),
            "comp:app/a.spg|embed_b".into(),
            "Contains".into(),
            None,
        ),
        (
            "page:app/a.spg".into(),
            "comp:app/a.spg|label_a".into(),
            "Contains".into(),
            None,
        ),
        (
            "comp:app/a.spg|canvas".into(),
            "comp:app/a.spg|embed_b".into(),
            "Contains".into(),
            None,
        ),
        (
            "comp:app/a.spg|canvas".into(),
            "comp:app/a.spg|label_a".into(),
            "Contains".into(),
            None,
        ),
        (
            "comp:app/a.spg|embed_b".into(),
            "page:app/b.spg".into(),
            "EmbedsPage".into(),
            Some("app/b.spg".into()),
        ),
        (
            "page:app/b.spg".into(),
            "comp:app/b.spg|canvas".into(),
            "Contains".into(),
            None,
        ),
        (
            "page:app/b.spg".into(),
            "comp:app/b.spg|label_b".into(),
            "Contains".into(),
            None,
        ),
        (
            "comp:app/b.spg|canvas".into(),
            "comp:app/b.spg|label_b".into(),
            "Contains".into(),
            None,
        ),
        (
            "model:orders".into(),
            "field:orders.订单号".into(),
            "Contains".into(),
            None,
        ),
        (
            "model:orders".into(),
            "field:orders.含税金额".into(),
            "Contains".into(),
            None,
        ),
    ];
    facts.sort();
    facts
}

fn node_meta(store: &GrafeoGraphStore, id: &str) -> Result<Value> {
    store
        .get_node(id)?
        .ok_or_else(|| anyhow!("missing gold node {id}"))?
        .meta
        .ok_or_else(|| anyhow!("gold node {id} has no metadata"))
}

fn assert_state_paths(store: &GrafeoGraphStore) -> Result<()> {
    let states = IndexStateStore::load_file_states(store)?;
    let mut paths = states.keys().cloned().collect::<Vec<_>>();
    paths.sort();
    let expected_paths = vec![
        "app/a.spg".to_string(),
        "app/b.spg".to_string(),
        "db/orders.tbl".to_string(),
    ];
    assert_eq!(paths, expected_paths);

    let entries = store.load_scanner_diagnostic_entries()?;
    let mut diagnostic_paths = entries
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    diagnostic_paths.sort();
    assert_eq!(diagnostic_paths, expected_paths);
    let occurrences = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(occurrences.occurrences.len(), 0);
    assert_eq!(occurrences.legacy_files, Vec::<String>::new());
    assert_eq!(
        ProjectIndexer::merge_scanner_diagnostic_entries(&entries)?.len(),
        0
    );
    Ok(())
}

fn assert_source_gold(store: &GrafeoGraphStore, updated: bool) -> Result<()> {
    assert_eq!(observed_nodes(store)?, expected_nodes(updated));
    let edges = if updated {
        let mut edges = expected_edges();
        edges.retain(|edge| edge.0 != "page:app/b.spg" && edge.0 != "comp:app/b.spg|canvas");
        edges.extend([
            (
                "page:app/b.spg".into(),
                "comp:app/b.spg|canvas".into(),
                "Contains".into(),
                None,
            ),
            (
                "page:app/b.spg".into(),
                "comp:app/b.spg|label_b_v2".into(),
                "Contains".into(),
                None,
            ),
            (
                "comp:app/b.spg|canvas".into(),
                "comp:app/b.spg|label_b_v2".into(),
                "Contains".into(),
                None,
            ),
        ]);
        edges.sort();
        edges
    } else {
        expected_edges()
    };
    assert_eq!(observed_edges(store)?, edges);

    let embed = node_meta(store, "comp:app/a.spg|embed_b")?;
    assert_eq!(embed["component_type"], "embedsuperpage");
    assert_eq!(embed["json_path"], "canvas.components[0]");
    let label_a = node_meta(store, "comp:app/a.spg|label_a")?;
    assert_eq!(label_a["component_type"], "text");
    assert_eq!(label_a["json_path"], "canvas.components[1]");
    assert_eq!(label_a["properties"]["value"], "A");
    let label_b_id = if updated {
        "comp:app/b.spg|label_b_v2"
    } else {
        "comp:app/b.spg|label_b"
    };
    let label_b = node_meta(store, label_b_id)?;
    assert_eq!(label_b["component_type"], "text");
    assert_eq!(label_b["json_path"], "canvas.components[0]");
    assert_eq!(
        label_b["properties"]["value"],
        if updated { "B after source edit" } else { "B" }
    );

    let model = node_meta(store, "model:orders")?;
    assert_eq!(model["modelType"], "App");
    let order_number = node_meta(store, "field:orders.订单号")?;
    assert_eq!(order_number["dbfield"], "ORDER_NO");
    assert_eq!(order_number["dataType"], "C");
    assert_eq!(order_number["length"], 24);
    let amount = node_meta(store, "field:orders.含税金额")?;
    assert_eq!(amount["dbfield"], "TAX_AMOUNT");
    assert_eq!(amount["dataType"], "N");
    assert_eq!(amount["length"], 16);
    assert_eq!(store.get_node("comp:app/b.spg|label_b")?.is_none(), updated);
    Ok(())
}

#[test]
// 验证全量扫描、持久化重开和无变化重扫均满足同一份人工源事实 gold。
fn full_and_noop_scans_match_static_source_gold_after_reopen() -> Result<()> {
    let directory = test_dir("full")?;
    let project = directory.join("project");
    let database = directory.join("graph.grafeo");
    seed_project(&project)?;

    let first = ProjectIndexer::scan_with_diagnostics(&project, &database)?;
    assert_eq!(first.report.indexed, 3);
    assert_eq!(first.report.dirty, 3);
    {
        let store = GrafeoGraphStore::open(&database)?;
        assert_state_paths(&store)?;
        assert_source_gold(&store, false)?;
    }

    let noop = ProjectIndexer::scan_with_diagnostics(&project, &database)?;
    assert_eq!(noop.report.dirty, 0);
    assert_eq!(noop.report.unchanged, 3);
    {
        let store = GrafeoGraphStore::open(&database)?;
        assert_state_paths(&store)?;
        assert_source_gold(&store, false)?;
    }
    fs::remove_dir_all(directory)?;
    Ok(())
}

#[test]
// 验证被引页面改写会重扫引用者，并精确替换旧组件事实。
fn referenced_page_rewrite_matches_updated_source_gold_after_reopen() -> Result<()> {
    let directory = test_dir("update")?;
    let project = directory.join("project");
    let database = directory.join("graph.grafeo");
    seed_project(&project)?;
    ProjectIndexer::scan_with_diagnostics(&project, &database)?;
    {
        let store = GrafeoGraphStore::open(&database)?;
        assert_source_gold(&store, false)?;
    }

    copy_fixture(&project, "app/b-updated.spg", "app/b.spg")?;
    let updated = ProjectIndexer::scan_with_diagnostics(&project, &database)?;
    assert_eq!(updated.report.dirty, 2);
    assert_eq!(updated.report.indexed, 3);
    {
        let store = GrafeoGraphStore::open(&database)?;
        assert_state_paths(&store)?;
        assert_source_gold(&store, true)?;
    }
    fs::remove_dir_all(directory)?;
    Ok(())
}
