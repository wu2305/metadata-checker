#![cfg(feature = "cli-local")]

use metadata_checker::graph::{Node, NodeType};
use metadata_checker::graph_store::{
    GraphNeighbors, GraphReadStore, GraphStoreError, GraphStoreResult,
};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::query::{build_page_logic_availability_cache, build_query_page_logic_output};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

const PAGE_ID: &str = "page:app/side_context.spg";
const SAME_PAGE_TARGET_ID: &str = PAGE_ID;
const OTHER_PAGE_ID: &str = "page:app/other.spg";
const COMPONENT_A_ID: &str = "comp:app/side_context.spg|button_a";
const COMPONENT_B_ID: &str = "comp:app/side_context.spg|button_b";

/// 仅注入边读取行为的 GraphReadStore 测试替身。
struct InjectedEdgeGraph<'a> {
    inner: &'a MemoryGraphStore,
    edge_reads: RefCell<HashMap<String, usize>>,
    fail_on: Option<(String, usize)>,
    missing_adjacencies: HashSet<String>,
}

impl<'a> InjectedEdgeGraph<'a> {
    /// 构造在指定组件的指定次边读取时失败的测试图。
    fn failing_on(inner: &'a MemoryGraphStore, node_id: &str, read: usize) -> Self {
        Self {
            inner,
            edge_reads: RefCell::new(HashMap::new()),
            fail_on: Some((node_id.to_string(), read)),
            missing_adjacencies: HashSet::new(),
        }
    }

    /// 构造把指定组件的邻接视为缺失、但仍允许查询继续的测试图。
    fn missing_for(inner: &'a MemoryGraphStore, node_ids: &[&str]) -> Self {
        Self {
            inner,
            edge_reads: RefCell::new(HashMap::new()),
            fail_on: None,
            missing_adjacencies: node_ids.iter().map(|id| (*id).to_string()).collect(),
        }
    }

    /// 返回指定节点的底层边读取次数，帮助证明失败发生在目标阶段。
    fn edge_reads_for(&self, node_id: &str) -> usize {
        self.edge_reads.borrow().get(node_id).copied().unwrap_or(0)
    }
}

impl GraphReadStore for InjectedEdgeGraph<'_> {
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
        if self
            .fail_on
            .as_ref()
            .is_some_and(|(target, read)| target == node_id && *read == read_count)
        {
            return Err(GraphStoreError::ReadFailed {
                reason: format!("injected edge read failure for {node_id}"),
            });
        }
        if self.missing_adjacencies.contains(node_id) {
            return Ok(None);
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

/// 构造包含同页边、跨页边和可去重重复边的最小页面图。
fn side_context_graph() -> MemoryGraphStore {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node(PAGE_ID, "当前页面", NodeType::Page, "app/side_context.spg");
    graph.add_test_node(OTHER_PAGE_ID, "其他页面", NodeType::Page, "app/other.spg");
    graph.add_test_node(
        COMPONENT_A_ID,
        "按钮 A",
        NodeType::Component,
        "app/side_context.spg",
    );
    graph.add_test_node(
        COMPONENT_B_ID,
        "按钮 B",
        NodeType::Component,
        "app/side_context.spg",
    );
    graph.add_test_edge(
        PAGE_ID,
        COMPONENT_A_ID,
        metadata_checker::graph::EdgeType::Contains,
        None,
    );
    graph.add_test_edge(
        PAGE_ID,
        COMPONENT_B_ID,
        metadata_checker::graph::EdgeType::Contains,
        None,
    );
    graph.add_test_edge(
        COMPONENT_A_ID,
        OTHER_PAGE_ID,
        metadata_checker::graph::EdgeType::OpensPage,
        Some("button_a.open"),
    );
    graph.add_test_edge(
        COMPONENT_A_ID,
        OTHER_PAGE_ID,
        metadata_checker::graph::EdgeType::EmbedsPage,
        Some("button_a.embed"),
    );
    graph.add_test_edge(
        COMPONENT_A_ID,
        SAME_PAGE_TARGET_ID,
        metadata_checker::graph::EdgeType::OpensPage,
        Some("button_a.same_page"),
    );
    graph.add_test_edge(
        COMPONENT_B_ID,
        OTHER_PAGE_ID,
        metadata_checker::graph::EdgeType::OpensPage,
        Some("button_b.open"),
    );
    graph.add_test_edge(
        COMPONENT_B_ID,
        SAME_PAGE_TARGET_ID,
        metadata_checker::graph::EdgeType::EmbedsPage,
        Some("button_b.same_page"),
    );
    graph
}

/// 提取页面逻辑输出中的 side-context 明细。
fn side_contexts(output: &Value) -> Vec<&Value> {
    output["details"]["related_context"]
        .as_array()
        .expect("page logic output must contain related_context")
        .iter()
        .filter(|item| item["reason"] == "跨页面旁路关系")
        .collect()
}

#[test]
fn page_logic_includes_deduplicated_cross_page_context_and_excludes_same_page_edges()
-> anyhow::Result<()> {
    let graph = side_context_graph();
    let output = build_query_page_logic_output(&graph, PAGE_ID, None, "normal")?;
    let contexts = side_contexts(&output);

    // A 到同一目标页的两种边只能形成一条旁路上下文，B 仍保留独立来源的一条。
    assert_eq!(contexts.len(), 2);
    assert_eq!(
        contexts
            .iter()
            .filter(|context| {
                context["from"] == COMPONENT_A_ID && context["to"] == OTHER_PAGE_ID
            })
            .count(),
        1
    );
    assert_eq!(
        contexts
            .iter()
            .filter(|context| {
                context["from"] == COMPONENT_B_ID && context["to"] == OTHER_PAGE_ID
            })
            .count(),
        1
    );
    assert_eq!(
        contexts
            .iter()
            .any(|context| context["to"] == SAME_PAGE_TARGET_ID),
        false
    );
    Ok(())
}

#[test]
fn page_logic_treats_missing_component_adjacency_as_empty_side_context() -> anyhow::Result<()> {
    let graph = side_context_graph();
    let injected = InjectedEdgeGraph::missing_for(&graph, &[COMPONENT_A_ID, COMPONENT_B_ID]);
    let output = build_query_page_logic_output(&injected, PAGE_ID, None, "normal")?;

    assert_eq!(side_contexts(&output).is_empty(), true);
    Ok(())
}

#[test]
fn cold_page_logic_propagates_side_context_edge_read_failure() {
    let graph = side_context_graph();
    // 页面节点收集、edge bundle 和 prerequisites 共读取 A 五次；第六次进入 side-context。
    let injected = InjectedEdgeGraph::failing_on(&graph, COMPONENT_A_ID, 6);
    let error = build_query_page_logic_output(&injected, PAGE_ID, None, "normal")
        .expect_err("cold page logic must propagate side-context edge read failure");
    let message = format!("{error:#}");

    assert_eq!(injected.edge_reads_for(COMPONENT_A_ID), 6);
    assert_eq!(
        message.contains("cross-page side context"),
        true,
        "{message}"
    );
    assert_eq!(message.contains(COMPONENT_A_ID), true);
    assert_eq!(message.contains("injected edge read failure"), true);
}

#[test]
fn warm_page_logic_propagates_side_context_edge_read_failure() {
    let graph = side_context_graph();
    // warm builder 还会在 path summary 前扫描一次 prerequisites，因此失败点为第六次。
    let injected = InjectedEdgeGraph::failing_on(&graph, COMPONENT_A_ID, 6);
    let result =
        build_page_logic_availability_cache(&injected, None, PAGE_ID, None, "normal", None);
    let error = result
        .err()
        .expect("warm builder must propagate side-context read failure");
    let message = format!("{error:#}");

    assert_eq!(injected.edge_reads_for(COMPONENT_A_ID), 6);
    assert_eq!(
        message.contains("cross-page side context"),
        true,
        "{message}"
    );
    assert_eq!(message.contains(COMPONENT_A_ID), true);
    assert_eq!(message.contains("injected edge read failure"), true);
}
