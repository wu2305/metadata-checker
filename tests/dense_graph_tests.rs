#![cfg(feature = "cli-local")]

use metadata_checker::dense_graph::DenseGraphSnapshot;
use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::{GraphNeighbors, GraphReadStore, GraphStoreResult};
use metadata_checker::scanner::scan_project;
use std::cell::Cell;
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
    let outgoing: Vec<String> = neighbors
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
    let incoming: Vec<String> = neighbors
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

struct CountingGraphStore<'a> {
    graph: &'a GraphDB,
    edge_reads: Cell<usize>,
}

impl GraphReadStore for CountingGraphStore<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<metadata_checker::graph::Node>> {
        GraphReadStore::get_node(self.graph, node_id)
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        self.edge_reads.set(self.edge_reads.get() + 1);
        GraphReadStore::get_node_edges(self.graph, node_id)
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        GraphReadStore::node_count(self.graph)
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        GraphReadStore::edge_count(self.graph)
    }

    fn iter_nodes(
        &self,
    ) -> GraphStoreResult<Box<dyn Iterator<Item = metadata_checker::graph::Node> + '_>> {
        GraphReadStore::iter_nodes(self.graph)
    }
}

#[test]
fn dense_graph_snapshot_reads_each_node_edge_set_once() -> anyhow::Result<()> {
    let graph = fixture_graph("single-edge-read")?;
    let counted = CountingGraphStore {
        edge_reads: Cell::new(0),
        graph: &graph,
    };

    let _snapshot = DenseGraphSnapshot::from_graph(&counted)?;

    assert_eq!(counted.edge_reads.get(), graph.node_count()?);
    Ok(())
}
