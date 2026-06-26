#![cfg(feature = "cli-local")]

use metadata_checker::dense_graph::DenseGraphSnapshot;
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, GraphStoreResult};
use metadata_checker::scanner::scan_project;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph(test_name: &str) -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-dense-{test_name}-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    GraphDB::open(&db_path)
}

fn canonical_neighbors(
    graph: &dyn GraphReadStore,
    node_id: &str,
) -> GraphStoreResult<(Vec<String>, Vec<String>)> {
    let neighbors = graph.get_node_edges(node_id)?.unwrap_or_else(|| {
        panic!("missing neighbors for {node_id}");
    });
    let mut outgoing: Vec<String> = neighbors
        .outgoing
        .into_iter()
        .map(|view| {
            format!(
                "{}>{}>{:?}>{}",
                view.edge.from,
                view.edge.to,
                view.edge.edge_type,
                view.edge.field_path.unwrap_or_default()
            )
        })
        .collect();
    let mut incoming: Vec<String> = neighbors
        .incoming
        .into_iter()
        .map(|view| {
            format!(
                "{}>{}>{:?}>{}",
                view.edge.from,
                view.edge.to,
                view.edge.edge_type,
                view.edge.field_path.unwrap_or_default()
            )
        })
        .collect();
    outgoing.sort();
    incoming.sort();
    Ok((outgoing, incoming))
}

#[test]
fn dense_graph_snapshot_preserves_graph_counts_and_neighbors() -> anyhow::Result<()> {
    let graph = fixture_graph("equivalence")?;
    let snapshot = DenseGraphSnapshot::from_graph(&graph)?;

    assert_eq!(snapshot.node_count()?, graph.node_count()?);
    assert_eq!(snapshot.edge_count()?, graph.edge_count()?);
    assert_eq!(snapshot.dense_node_count(), graph.node_count()?);
    assert_eq!(snapshot.dense_edge_count(), graph.edge_count()?);

    let nodes: Vec<_> = graph.iter_nodes()?.collect();
    for node in nodes {
        assert_eq!(
            serde_json::to_value(GraphReadStore::get_node(&snapshot, &node.id)?)?,
            serde_json::to_value(GraphReadStore::get_node(&graph, &node.id)?)?
        );
        assert_eq!(
            canonical_neighbors(&snapshot, &node.id)?,
            canonical_neighbors(&graph, &node.id)?,
            "dense snapshot neighbors changed for {}",
            node.id
        );
    }

    Ok(())
}
