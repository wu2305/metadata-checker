use crate::graph::Node;
use crate::graph_store::{GraphNeighbors, GraphReadStore, GraphStoreResult};
use crate::path::{PathFinder, PathSelector};
use crate::perf_profile::PerfProfile;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::Instant;

/// 页面逻辑输出中的路径选择结果
pub(super) struct PageLogicPaths {
    pub(super) primary_paths: Vec<serde_json::Value>,
    pub(super) related_context: Vec<serde_json::Value>,
    pub(super) candidate_paths: Vec<serde_json::Value>,
    pub(super) supporting_paths: Vec<serde_json::Value>,
    pub(super) rejected_paths: Vec<serde_json::Value>,
    pub(super) path_selection_diagnostics: Vec<String>,
}

/// 计算页面主链路、候选链路和旁路上下文
pub(super) fn build_page_logic_paths(
    graph: &dyn GraphReadStore,
    dense_snapshot: Option<&crate::dense_graph::DenseGraphSnapshot>,
    page_id: &str,
    page_node: &Node,
    child_components: &Vec<&Node>,
    child_actions: &Vec<&Node>,
    data_sources: &Vec<serde_json::Value>,
    write_targets: &Vec<serde_json::Value>,
    entrypoints: &Vec<serde_json::Value>,
    profile: &mut Option<&mut PerfProfile>,
) -> PageLogicPaths {
    super::set_profile_counter(profile, "dense_snapshot_build_ms", 0);
    if let Some(snapshot) = dense_snapshot {
        super::set_profile_counter(profile, "path_dense_graph_used", 1);
        super::set_profile_counter(profile, "dense_snapshot_nodes", snapshot.dense_node_count());
        super::set_profile_counter(profile, "dense_snapshot_edges", snapshot.dense_edge_count());
    } else {
        super::set_profile_counter(profile, "path_dense_graph_used", 0);
        super::set_profile_counter(profile, "dense_snapshot_nodes", 0);
        super::set_profile_counter(profile, "dense_snapshot_edges", 0);
    }

    let context_started = Instant::now();
    let path_graph_source: &dyn GraphReadStore = dense_snapshot
        .map(|snapshot| snapshot as &dyn GraphReadStore)
        .unwrap_or(graph);
    let path_graph = PathGraphCache::new(path_graph_source);
    record_ms_counter_with_min(profile, "path_context_build_ms", context_started, 1);

    let stage_started = Instant::now();
    let path_query = crate::path::AnchorExtractor::extract(
        &path_graph,
        page_id,
        page_node,
        child_components,
        child_actions,
        data_sources,
        write_targets,
        entrypoints,
    );
    record_ms_counter(profile, "path_anchor_extract_ms", stage_started);
    super::set_profile_counter(
        profile,
        "path_target_anchors",
        path_query.target_anchors.len(),
    );
    super::set_profile_counter(
        profile,
        "path_source_anchors",
        path_query.source_anchors.len(),
    );
    super::set_profile_counter(profile, "path_sink_anchors", path_query.sink_anchors.len());
    super::set_profile_counter(
        profile,
        "path_bridge_anchors",
        path_query.bridge_anchors.len(),
    );
    super::set_profile_counter(
        profile,
        "path_excluded_anchors",
        path_query.excluded_anchors.len(),
    );

    let stage_started = Instant::now();
    let finder = crate::path::BoundedCausalPathFinder::default();
    let mut candidates = finder.find_candidates(&path_graph, &path_query);
    let base_candidate_count = candidates.len();
    record_ms_counter(profile, "path_candidate_search_ms", stage_started);
    super::set_profile_counter(profile, "path_base_candidates", base_candidate_count);

    let stage_started = Instant::now();
    let mut field_candidate_count = 0usize;
    for ds in data_sources {
        let field_candidates =
            crate::path::build_field_causal_paths_for_data_source(&path_graph, page_node, ds);
        field_candidate_count += field_candidates.len();
        candidates.extend(field_candidates);
    }
    record_ms_counter(profile, "path_field_candidate_build_ms", stage_started);
    super::set_profile_counter(profile, "path_field_candidates", field_candidate_count);

    let stage_started = Instant::now();
    let candidates_before_dedup = candidates.len();
    {
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.path_id.clone()));
    }
    let candidates_after_dedup = candidates.len();
    record_ms_counter(profile, "path_dedup_ms", stage_started);
    super::set_profile_counter(
        profile,
        "path_candidates_before_dedup",
        candidates_before_dedup,
    );
    super::set_profile_counter(
        profile,
        "path_candidates_after_dedup",
        candidates_after_dedup,
    );
    super::set_profile_counter(
        profile,
        "path_dedup_removed",
        candidates_before_dedup.saturating_sub(candidates_after_dedup),
    );

    let stage_started = Instant::now();
    let selector = crate::path::RuleBasedPathSelector;
    let selection = selector.select(&path_query, candidates);
    record_ms_counter(profile, "path_classification_ms", stage_started);

    let stage_started = Instant::now();
    let mut primary_paths = Vec::new();
    let mut related_context = Vec::new();
    let mut candidate_paths = Vec::new();
    let mut supporting_paths = Vec::new();
    let mut rejected_paths = Vec::new();
    let mut path_selection_diagnostics = Vec::new();

    for p in selection.primary_paths {
        primary_paths.push(p.to_json());
    }
    for p in selection.candidate_paths {
        candidate_paths.push(p.to_json());
    }
    for p in selection.supporting_paths {
        supporting_paths.push(p.to_json());
    }
    for p in selection.related_context {
        related_context.push(p.to_json());
    }
    for p in selection.rejected_paths {
        rejected_paths.push(p.to_json());
    }
    for d in selection.selection_diagnostics {
        path_selection_diagnostics.push(d);
    }
    record_ms_counter(profile, "path_json_build_ms", stage_started);
    super::set_profile_counter(
        profile,
        "path_selection_diagnostics",
        path_selection_diagnostics.len(),
    );

    let stage_started = Instant::now();
    collect_cross_page_side_context(&path_graph, page_id, child_components, &mut related_context);
    record_ms_counter(profile, "path_side_context_ms", stage_started);
    super::set_profile_counter(profile, "path_graph_cache_hits", path_graph.cache_hits());

    let stage_started = Instant::now();
    sort_primary_paths_by_confidence(&mut primary_paths);
    record_ms_counter(profile, "path_sort_ms", stage_started);

    PageLogicPaths {
        primary_paths,
        related_context,
        candidate_paths,
        supporting_paths,
        rejected_paths,
        path_selection_diagnostics,
    }
}

fn record_ms_counter(profile: &mut Option<&mut PerfProfile>, name: &str, started_at: Instant) {
    super::set_profile_counter(profile, name, started_at.elapsed().as_millis() as usize);
}

fn record_ms_counter_with_min(
    profile: &mut Option<&mut PerfProfile>,
    name: &str,
    started_at: Instant,
    min: usize,
) {
    super::set_profile_counter(
        profile,
        name,
        (started_at.elapsed().as_millis() as usize).max(min),
    );
}

struct PathGraphCache<'a> {
    inner: &'a dyn GraphReadStore,
    nodes: RefCell<HashMap<String, Option<Node>>>,
    edges: RefCell<HashMap<String, Option<GraphNeighbors>>>,
    node_cache_hits: Cell<usize>,
    edge_cache_hits: Cell<usize>,
}

impl<'a> PathGraphCache<'a> {
    fn new(inner: &'a dyn GraphReadStore) -> Self {
        Self {
            inner,
            nodes: RefCell::new(HashMap::new()),
            edges: RefCell::new(HashMap::new()),
            node_cache_hits: Cell::new(0),
            edge_cache_hits: Cell::new(0),
        }
    }

    fn cache_hits(&self) -> usize {
        self.node_cache_hits.get() + self.edge_cache_hits.get()
    }
}

impl GraphReadStore for PathGraphCache<'_> {
    fn get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>> {
        if let Some(node) = self.nodes.borrow().get(node_id).cloned() {
            self.node_cache_hits.set(self.node_cache_hits.get() + 1);
            return Ok(node);
        }
        let node = self.inner.get_node(node_id)?;
        self.nodes
            .borrow_mut()
            .insert(node_id.to_string(), node.clone());
        Ok(node)
    }

    fn get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>> {
        if let Some(neighbors) = self.edges.borrow().get(node_id).cloned() {
            self.edge_cache_hits.set(self.edge_cache_hits.get() + 1);
            return Ok(neighbors);
        }
        let neighbors = self.inner.get_node_edges(node_id)?;
        self.edges
            .borrow_mut()
            .insert(node_id.to_string(), neighbors.clone());
        Ok(neighbors)
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

fn collect_cross_page_side_context(
    graph: &dyn GraphReadStore,
    page_id: &str,
    child_components: &[&Node],
    related_context: &mut Vec<serde_json::Value>,
) {
    let mut seen_ctx: std::collections::HashSet<String> = std::collections::HashSet::new();
    for comp in child_components {
        if let Some(neighbors) = graph.get_node_edges(&comp.id).ok().flatten() {
            for edge_view in neighbors.outgoing {
                let target = edge_view.node;
                let edge = edge_view.edge;
                if target.id.starts_with("page:") && target.id != page_id {
                    let ctx_key = format!("{}->{}", comp.id, target.id);
                    if !seen_ctx.contains(&ctx_key) {
                        seen_ctx.insert(ctx_key.clone());
                        related_context.push(serde_json::json!({
                            "from": comp.id,
                            "to": target.id,
                            "edge_type": format!("{:?}", edge.edge_type),
                            "field_path": edge.field_path,
                            "source_file": comp.path.clone(),
                            "confidence": "low",
                            "reason": "跨页面旁路关系",
                        }));
                    }
                }
            }
        }
    }
}

fn sort_primary_paths_by_confidence(primary_paths: &mut [serde_json::Value]) {
    primary_paths.sort_by(|a, b| {
        let a_conf = a
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        let b_conf = b
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("medium");
        match (a_conf, b_conf) {
            ("high", "medium") | ("high", "low") | ("medium", "low") => std::cmp::Ordering::Less,
            ("medium", "high") | ("low", "high") | ("low", "medium") => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
}
