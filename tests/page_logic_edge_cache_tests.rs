#![cfg(feature = "cli-local")]

use metadata_checker::graph::{EdgeType, Node, NodeType};
use metadata_checker::graph_store::{
    GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::query::{
    build_page_logic_availability_cache, build_query_page_logic_output,
    build_query_page_logic_output_profiled_with_availability_cache,
};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;

const PAGE_ID: &str = "page:app/cache_contract.spg";
const COMPONENT_ID: &str = "comp:app/cache_contract.spg|button1";
const ACTION_ID: &str = "action:app/cache_contract.spg|button1|action1";

/// 只统计 GraphReadStore 的实际边读取，所有图数据仍由现有内存实现提供。
struct CountingGraphStore<'a> {
    inner: &'a MemoryGraphStore,
    edge_reads: RefCell<HashMap<String, usize>>,
    fail_node: Option<String>,
}

impl CountingGraphStore<'_> {
    fn edge_reads_for(&self, node_id: &str) -> usize {
        self.edge_reads.borrow().get(node_id).copied().unwrap_or(0)
    }
}

impl GraphReadStore for CountingGraphStore<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        self.inner.get_node(node_id)
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        let read_count = {
            let mut edge_reads = self.edge_reads.borrow_mut();
            let count = edge_reads.entry(node_id.to_string()).or_insert(0);
            *count += 1;
            *count
        };
        if self.fail_node.as_deref() == Some(node_id) && read_count == 1 {
            return Err(GraphStoreError::ReadFailed {
                reason: format!("test edge read failure for {node_id}"),
            });
        }
        self.inner.get_node_edges(node_id)
    }

    fn node_count(&self) -> GraphStoreResult<usize> {
        self.inner.node_count()
    }

    fn edge_count(&self) -> GraphStoreResult<usize> {
        self.inner.edge_count()
    }

    fn iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>> {
        self.inner.iter_nodes()
    }
}

fn minimal_page_graph() -> MemoryGraphStore {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node(
        PAGE_ID,
        "cache_contract",
        NodeType::Page,
        "app/cache_contract.spg",
    );
    graph.add_test_node(
        COMPONENT_ID,
        "button1",
        NodeType::Component,
        "app/cache_contract.spg",
    );
    graph.add_test_node(
        ACTION_ID,
        "submitData:action1",
        NodeType::Action,
        "app/cache_contract.spg",
    );
    graph.add_test_edge(PAGE_ID, COMPONENT_ID, EdgeType::Contains, None);
    graph.add_test_edge(COMPONENT_ID, ACTION_ID, EdgeType::Triggers, None);
    graph
}

fn action_flows(output: &Value) -> &Vec<Value> {
    output["details"]["action_flows"]
        .as_array()
        .expect("page logic output must contain action_flows")
}

#[test]
fn page_logic_reuses_action_edges_between_edge_scan_and_action_flow() -> anyhow::Result<()> {
    let graph = minimal_page_graph();
    let counted = CountingGraphStore {
        inner: &graph,
        edge_reads: RefCell::new(HashMap::new()),
        fail_node: None,
    };

    let output = build_query_page_logic_output(&counted, PAGE_ID, None, "normal")?;
    let flows = action_flows(&output);
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0]["action_id"], "action1");
    assert_eq!(
        output["details"]["entrypoints"].as_array().map(Vec::len),
        Some(1)
    );

    // 图闭包、edge scan、前置条件和路径阶段各读取一次 action 邻接。
    // action flow 必须复用 edge scan 的结果，不能增加第五次底层读取。
    assert_eq!(counted.edge_reads_for(ACTION_ID), 4);
    Ok(())
}

#[test]
fn page_logic_preserves_graph_read_errors() {
    let graph = minimal_page_graph();
    let counted = CountingGraphStore {
        inner: &graph,
        edge_reads: RefCell::new(HashMap::new()),
        fail_node: Some(ACTION_ID.to_string()),
    };

    let error = build_query_page_logic_output(&counted, PAGE_ID, None, "normal")
        .expect_err("an underlying GraphReadStore error must reach the query caller");
    assert!(error.to_string().contains("test edge read failure"));
    assert_eq!(counted.edge_reads_for(ACTION_ID), 1);
}

#[test]
fn page_logic_warm_helper_remains_usable_with_local_edge_cache() -> anyhow::Result<()> {
    let graph = minimal_page_graph();
    let counted = CountingGraphStore {
        inner: &graph,
        edge_reads: RefCell::new(HashMap::new()),
        fail_node: None,
    };

    let cache = build_page_logic_availability_cache(&counted, None, PAGE_ID, None, "normal", None)?;
    assert_eq!(cache.warm_stage_ms("edge_scan").is_some(), true);
    let baseline = build_query_page_logic_output(&graph, PAGE_ID, None, "normal")?;
    let (cached, profile) = build_query_page_logic_output_profiled_with_availability_cache(
        &graph,
        None,
        Some(&cache),
        None,
        PAGE_ID,
        None,
        "normal",
    )?;
    assert_eq!(
        cached, baseline,
        "warm cache must preserve the complete page output"
    );
    assert_eq!(profile.counter("availability_read_model_used"), 1);
    assert_eq!(profile.counter("prerequisites_read_model_used"), 1);
    assert_eq!(profile.counter("path_read_model_used"), 1);
    Ok(())
}
