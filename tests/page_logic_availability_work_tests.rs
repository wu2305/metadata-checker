#![cfg(feature = "cli-local")]

use metadata_checker::graph::{EdgeType, Node, NodeType};
use metadata_checker::graph_store::{
    GraphNeighbors, GraphReadStore, GraphStoreResult, GraphWriteStore,
};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::query::{
    build_page_logic_availability_cache, build_query_page_logic_output_profiled,
    build_query_page_logic_output_profiled_with_availability_cache,
};
use serde_json::json;
use std::cell::Cell;
use std::time::Duration;

const PAGE_ID: &str = "page:app/availability_work.spg";
const PAGE_PATH: &str = "app/availability_work.spg";
const COMPONENT_ID: &str = "comp:app/availability_work.spg|input1";
const MODEL_ID: &str = "model:only";
const MODEL_READ_DELAY_MS: u64 = 10;

/// 模型只由前置条件的符号引用引入，不作为页面 data source 或路径遍历边。
fn availability_graph() -> anyhow::Result<MemoryGraphStore> {
    let mut graph = MemoryGraphStore::new();
    graph.add_test_node(PAGE_ID, "availability_work", NodeType::Page, PAGE_PATH);
    graph.add_test_node(COMPONENT_ID, "input1", NodeType::Component, PAGE_PATH);
    graph.add_test_node(MODEL_ID, "only", NodeType::Model, PAGE_PATH);
    graph.add_test_edge(PAGE_ID, COMPONENT_ID, EdgeType::Contains, None);
    let condition_id = "cond:app/availability_work.spg|visible";
    graph.upsert_node(Node {
        id: condition_id.to_string(),
        name: "visible".to_string(),
        node_type: NodeType::Condition,
        path: PAGE_PATH.to_string(),
        meta: Some(json!({
            "condition_type": "VisibleCondition",
            "raw_expr": "=only.value",
            "normalized_expr": "only.value",
            "json_path": "components.input1.visibleCondition",
            "owner_id": COMPONENT_ID,
            "referenced_symbols": ["model:only.value"],
        })),
    })?;
    graph.add_test_edge(condition_id, COMPONENT_ID, EdgeType::DependsOn, None);
    Ok(graph)
}

/// 只延迟 availability 专用模型的读取，计数与计时来自实际 GraphReadStore 调用。
struct DelayedModelStore<'a> {
    inner: &'a MemoryGraphStore,
    model_reads: Cell<usize>,
}

impl GraphReadStore for DelayedModelStore<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        if node_id == MODEL_ID {
            self.model_reads.set(self.model_reads.get() + 1);
            std::thread::sleep(Duration::from_millis(MODEL_READ_DELAY_MS));
        }
        self.inner.get_node(node_id)
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
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

/// cold 必须计入受控读取延迟；warm 必须消除模型读取，而不仅仅把计时 counter 写为零。
#[test]
fn warm_availability_skips_model_reads_and_cold_timer_measures_them() -> anyhow::Result<()> {
    let graph = availability_graph()?;
    for budget in ["compact", "normal", "full"] {
        let cache = build_page_logic_availability_cache(&graph, None, PAGE_ID, None, budget, None)?;
        let measured = DelayedModelStore {
            inner: &graph,
            model_reads: Cell::new(0),
        };
        let (cold_output, cold_profile) =
            build_query_page_logic_output_profiled(&measured, PAGE_ID, None, budget)?;
        assert_eq!(
            measured.model_reads.get() > 0,
            true,
            "cold must reach the availability model"
        );
        assert_eq!(cold_output["summary"]["key_model_count"], 1);
        assert_eq!(
            cold_profile.counters.get("key_model_availability"),
            Some(&1)
        );
        assert_eq!(
            cold_profile.counters.get("availability_read_model_used"),
            Some(&0)
        );
        let cold_build_ms = *cold_profile
            .counters
            .get("availability_index_build_ms")
            .expect("cold build timer must be recorded");
        assert_eq!(
            cold_build_ms >= MODEL_READ_DELAY_MS,
            true,
            "timer must include injected model-read latency"
        );
        assert_eq!(
            cold_profile.counters.get("availability_context_build_ms"),
            Some(&cold_build_ms)
        );

        measured.model_reads.set(0);
        let (warm_output, warm_profile) =
            build_query_page_logic_output_profiled_with_availability_cache(
                &measured,
                None,
                Some(&cache),
                None,
                PAGE_ID,
                None,
                budget,
            )?;
        assert_eq!(
            measured.model_reads.get(),
            0,
            "warm must not rebuild availability from graph reads"
        );
        assert_eq!(
            warm_profile.counters.get("availability_read_model_used"),
            Some(&1)
        );
        assert_eq!(
            warm_profile.counters.get("availability_index_build_ms"),
            Some(&0)
        );
        assert_eq!(
            warm_profile.counters.get("availability_context_build_ms"),
            Some(&0)
        );
        assert_eq!(
            serde_json::to_vec(&warm_output)?,
            serde_json::to_vec(&cold_output)?
        );
    }
    Ok(())
}
