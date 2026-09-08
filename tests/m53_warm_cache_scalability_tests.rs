#![cfg(feature = "cli-local")]

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::query::{
    PageDependencyIndex, build_page_logic_availability_cache,
    build_query_page_logic_output_profiled_with_availability_cache,
    build_query_page_logic_output_profiled_with_dense_snapshot, collect_page_logic_nodes_for_test,
};
use metadata_checker::runtime::{
    GraphRuntime, RuntimeMode, page_logic_cache_map_clone_elements,
    page_logic_cache_warm_structural_writes, reset_page_logic_cache_write_counters,
};
use metadata_checker::scanner::scan_project;
use serde_json::Value;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

mod common;

fn fixture_runtime(test_name: &str) -> anyhow::Result<(GraphRuntime, std::path::PathBuf)> {
    let (temp_dir, db_path) = common::build_fixture_graphdb();
    let runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )?;
    let _ = test_name;
    Ok((runtime, temp_dir))
}

fn collect_fixture_page_ids(graph: &GraphDB) -> anyhow::Result<Vec<String>> {
    Ok(graph
        .iter_nodes()?
        .filter(|node| matches!(node.node_type, metadata_checker::graph::NodeType::Page))
        .map(|node| node.id)
        .collect())
}

#[test]
fn m53_warm_cache_avoids_quadratic_map_clones() -> anyhow::Result<()> {
    let (mut runtime, _temp_dir) = fixture_runtime("avoid-quadratic-clones")?;
    let page_ids = collect_fixture_page_ids(&runtime.graph)?;
    assert!(!page_ids.is_empty(), "fixture must contain page nodes");

    let mut targets = Vec::new();
    let budgets = [
        "compact",
        "normal",
        "full",
        "compact-1",
        "compact-2",
        "compact-3",
        "normal-1",
        "normal-2",
        "full-1",
        "full-2",
        "warm-a",
        "warm-b",
        "warm-c",
        "warm-d",
        "warm-e",
        "warm-f",
        "warm-g",
        "warm-h",
        "warm-i",
        "warm-j",
    ];
    for (index, budget) in budgets.iter().enumerate() {
        let page_id = &page_ids[index % page_ids.len()];
        targets.push((page_id.clone(), (*budget).to_string()));
    }
    assert!(
        targets.len() >= 20,
        "test must warm at least 20 cache entries"
    );

    reset_page_logic_cache_write_counters();
    let writes_before = page_logic_cache_warm_structural_writes();
    for (page_id, budget) in &targets {
        runtime.warm_page_logic_availability(page_id, budget)?;
    }

    assert_eq!(
        page_logic_cache_map_clone_elements(),
        0,
        "make_mut warm path must not deep-clone page_logic_availability maps"
    );
    assert_eq!(
        page_logic_cache_warm_structural_writes() - writes_before,
        targets.len(),
        "each warm should perform exactly one structural write"
    );
    Ok(())
}

#[test]
fn m53_warm_page_logic_batch_matches_single_warm_cache_content() -> anyhow::Result<()> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-batch-eq-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    let temp_dir = db_path
        .parent()
        .expect("db path must have parent")
        .to_path_buf();

    let page_id = "page:app/actions_test.spg";
    let targets = [
        (page_id.to_string(), "compact".to_string()),
        (page_id.to_string(), "normal".to_string()),
        (page_id.to_string(), "full".to_string()),
    ];

    let mut batch_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )?;
    reset_page_logic_cache_write_counters();
    let writes_before = page_logic_cache_warm_structural_writes();
    let report = batch_runtime.warm_page_logic_batch(&targets)?;
    assert_eq!(report.structural_writes, 1, "batch warm must write once");
    assert!(report.pages.iter().all(|page| page.success));
    assert_eq!(
        page_logic_cache_warm_structural_writes() - writes_before,
        1,
        "batch warm should record one structural write"
    );

    let mut loop_runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )?;
    let mut loop_snapshots = Vec::new();
    for (target_page, budget) in &targets {
        loop_runtime.warm_page_logic_availability(target_page, budget)?;
        let read_model = loop_runtime.read_model.as_ref().unwrap();
        let key = format!("{target_page}\n{budget}");
        let cache = read_model
            .page_logic_availability
            .get(&key)
            .expect("loop warm must store cache");
        loop_snapshots.push((key, cache.warm_cache_test_snapshot()));
    }
    let batch_model = batch_runtime.read_model.as_ref().unwrap();
    for (key, loop_snapshot) in loop_snapshots {
        let batch_snapshot = batch_model
            .page_logic_availability
            .get(&key)
            .expect("batch cache must exist")
            .warm_cache_test_snapshot();
        assert_eq!(
            batch_snapshot, loop_snapshot,
            "batch warm cache snapshot must match loop warm for {key}"
        );
    }

    for (target_page, budget) in &targets {
        if *budget == "full" {
            continue;
        }
        let key = format!("{target_page}\n{budget}");
        use metadata_checker::runtime::{RuntimeQueryCommand, RuntimeQueryRequest};
        let request = RuntimeQueryRequest {
            command: RuntimeQueryCommand::QueryPageLogic,
            target: target_page.clone(),
            budget: budget.clone(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        };
        let single_output = loop_runtime.query(request.clone())?.result;
        let batch_output = batch_runtime.query(request)?.result;
        assert_eq!(
            batch_output, single_output,
            "batch warm query output must match loop warm for {key}"
        );
    }
    Ok(())
}

#[test]
fn m53_warm_cache_budget_output_byte_equivalence() -> anyhow::Result<()> {
    let graph = {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "metadata-checker-m53-budget-eq-{}-{nanos}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        scan_project(
            std::path::Path::new("tests/fixtures/test_project"),
            &db_path,
        )?;
        GraphDB::open(&db_path)?
    };
    let page_id = "page:app/actions_test.spg";
    let dense_snapshot = metadata_checker::dense_graph::DenseGraphSnapshot::from_graph(&graph)?;

    for budget in ["compact", "normal", "full"] {
        let availability_cache = build_page_logic_availability_cache(
            &graph,
            Some(&dense_snapshot),
            page_id,
            None,
            budget,
            None,
        )?;

        let (baseline, _) = build_query_page_logic_output_profiled_with_dense_snapshot(
            &graph,
            Some(&dense_snapshot),
            page_id,
            None,
            budget,
        )?;
        let (cached, _) = build_query_page_logic_output_profiled_with_availability_cache(
            &graph,
            Some(&dense_snapshot),
            Some(&availability_cache),
            None,
            page_id,
            None,
            budget,
        )?;

        assert_eq!(
            serde_json::to_vec(&cached)?,
            serde_json::to_vec(&baseline)?,
            "{budget} warm-cache-hit output must be byte-equivalent to non-warm path"
        );
    }
    Ok(())
}

#[test]
fn m53_compact_warm_cache_trims_unused_path_categories() -> anyhow::Result<()> {
    let graph = {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "metadata-checker-m53-compact-trim-{}-{nanos}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        scan_project(
            std::path::Path::new("tests/fixtures/test_project"),
            &db_path,
        )?;
        GraphDB::open(&db_path)?
    };
    let page_id = "page:app/actions_test.spg";
    let dense_snapshot = metadata_checker::dense_graph::DenseGraphSnapshot::from_graph(&graph)?;

    let compact_cache = build_page_logic_availability_cache(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        "compact",
        None,
    )?;
    let full_cache = build_page_logic_availability_cache(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        "full",
        None,
    )?;

    let compact_footprint = compact_cache.cache_footprint();
    let full_footprint = full_cache.cache_footprint();
    assert_eq!(
        compact_footprint.candidate_paths, 0,
        "compact warm cache must not retain candidate_paths"
    );
    assert_eq!(
        compact_footprint.supporting_paths, 0,
        "compact warm cache must not retain supporting_paths"
    );
    assert_eq!(
        compact_footprint.rejected_paths, 0,
        "compact warm cache must not retain rejected_paths"
    );
    assert_eq!(
        full_footprint.candidate_paths > 0,
        true,
        "fixture must produce candidates before compact trimming"
    );
    assert_eq!(
        full_footprint.rejected_paths > 0,
        true,
        "fixture must produce rejected paths before compact trimming"
    );
    assert_eq!(full_footprint.primary_paths > 0, true);
    assert_eq!(full_footprint.related_context > 0, true);
    assert_eq!(
        compact_footprint.primary_paths,
        full_footprint.primary_paths
    );
    assert_eq!(
        compact_footprint.related_context,
        full_footprint.related_context
    );

    let normal_cache = build_page_logic_availability_cache(
        &graph,
        Some(&dense_snapshot),
        page_id,
        None,
        "normal",
        None,
    )?;
    let normal_footprint = normal_cache.cache_footprint();
    assert_eq!(normal_footprint.candidate_paths, 0);
    assert_eq!(normal_footprint.rejected_paths, 0);
    assert_eq!(normal_footprint.primary_paths, full_footprint.primary_paths);
    assert_eq!(
        normal_footprint.related_context,
        full_footprint.related_context
    );
    Ok(())
}

/// 四条同页条件路径中有两条被提升为主路径，剩余两条用于验证 supporting 桶的预算边界。
#[test]
fn m53_warm_cache_keeps_supporting_paths_only_for_normal_and_full() -> anyhow::Result<()> {
    use metadata_checker::graph::{EdgeType, NodeType};
    use metadata_checker::memory_graph_store::MemoryGraphStore;

    let mut graph = MemoryGraphStore::new();
    let page_path = "app/trim.spg";
    let page_id = "page:app/trim.spg";
    let input_id = "comp:app/trim.spg|input1";
    let dependent_id = "comp:app/trim.spg|input2";
    for (node_id, kind) in [
        (page_id, NodeType::Page),
        (input_id, NodeType::Component),
        (dependent_id, NodeType::Component),
        ("model:trim", NodeType::Model),
    ] {
        graph.add_test_node(node_id, node_id, kind, page_path);
    }
    graph.add_test_edge(page_id, input_id, EdgeType::Contains, None);
    graph.add_test_edge(page_id, dependent_id, EdgeType::Contains, None);
    graph.add_test_edge(input_id, "model:trim", EdgeType::Reads, None);
    for index in 0..4 {
        let condition_id = format!("cond:app/trim.spg|gate_{index}");
        graph.add_test_node(&condition_id, &condition_id, NodeType::Condition, page_path);
        graph.add_test_edge(input_id, &condition_id, EdgeType::DependsOn, None);
        graph.add_test_edge(&condition_id, dependent_id, EdgeType::DependsOn, None);
    }
    let dense = metadata_checker::dense_graph::DenseGraphSnapshot::from_graph(&graph)?;
    for (budget, expected_supporting) in [("full", 2), ("normal", 2), ("compact", 0)] {
        let cache =
            build_page_logic_availability_cache(&graph, Some(&dense), page_id, None, budget, None)?;
        let footprint = cache.cache_footprint();
        assert_eq!(footprint.supporting_paths, expected_supporting, "{budget}");
        assert_eq!(
            footprint.primary_paths, 3,
            "{budget} must retain primary paths"
        );
        let (baseline, _) = build_query_page_logic_output_profiled_with_dense_snapshot(
            &graph,
            Some(&dense),
            page_id,
            None,
            budget,
        )?;
        let (cached, profile) = build_query_page_logic_output_profiled_with_availability_cache(
            &graph,
            Some(&dense),
            Some(&cache),
            None,
            page_id,
            None,
            budget,
        )?;
        assert_eq!(profile.counter("path_read_model_used"), 1);
        assert_eq!(
            serde_json::to_vec(&cached)?,
            serde_json::to_vec(&baseline)?,
            "{budget} cache must preserve path contents"
        );
    }
    Ok(())
}

#[test]
fn m53_page_dependency_index_matches_collect_page_logic_nodes() -> anyhow::Result<()> {
    let graph = {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "metadata-checker-m53-index-coverage-{}-{nanos}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        scan_project(
            std::path::Path::new("tests/fixtures/test_project"),
            &db_path,
        )?;
        GraphDB::open(&db_path)?
    };
    let graph_store: &dyn GraphReadStore = &graph;
    let index = PageDependencyIndex::build(graph_store)?;
    let page_id = "page:app/actions_test.spg";
    let (collected_components, collected_actions) =
        collect_page_logic_nodes_for_test(graph_store, page_id)?;

    let mut expected_nodes = HashSet::from([page_id.to_string()]);
    for component in collected_components {
        expected_nodes.insert(component.id);
    }
    for action in collected_actions {
        expected_nodes.insert(action.id);
    }

    for node_id in &expected_nodes {
        let indexed_pages = index.pages_for_node(node_id);
        assert!(
            indexed_pages.contains(page_id),
            "index must map {node_id} to {page_id}"
        );
    }

    for node_id in expected_nodes {
        let indexed_pages = index.pages_for_node(&node_id);
        assert_eq!(
            indexed_pages,
            HashSet::from([page_id.to_string()]),
            "collect_page_logic_nodes node {node_id} should only map to its page"
        );
    }
    Ok(())
}

#[test]
fn m53_invalidate_pages_for_dirty_nodes_removes_only_affected_pages() -> anyhow::Result<()> {
    let (mut runtime, _temp_dir) = fixture_runtime("invalidate-dirty-nodes")?;
    let page_a = "page:app/actions_test.spg";
    let page_b = "page:app/page_relations.spg";

    runtime.warm_page_logic_availability(page_a, "compact")?;
    runtime.warm_page_logic_availability(page_a, "normal")?;
    runtime.warm_page_logic_availability(page_b, "compact")?;
    runtime.warm_page_logic_availability(page_b, "full")?;

    let graph_store: &dyn GraphReadStore = &runtime.graph;
    let (collected_components, collected_actions) =
        collect_page_logic_nodes_for_test(graph_store, page_a)?;
    let dirty_node = collected_components
        .first()
        .map(|node| node.id.clone())
        .or_else(|| collected_actions.first().map(|node| node.id.clone()))
        .expect("page_a must have at least one component or action for invalidation test");

    let removed = runtime.invalidate_pages_for_dirty_nodes(&[dirty_node])?;
    assert_eq!(removed, vec![page_a.to_string()]);

    let read_model = runtime
        .read_model
        .as_ref()
        .expect("runtime must keep read model after invalidation");
    assert!(!read_model.has_page_logic_availability(page_a, "compact"));
    assert!(!read_model.has_page_logic_availability(page_a, "normal"));
    assert!(read_model.has_page_logic_availability(page_b, "compact"));
    assert!(read_model.has_page_logic_availability(page_b, "full"));
    Ok(())
}

#[test]
fn m53_invalidate_shared_node_affects_all_dependent_pages() -> anyhow::Result<()> {
    let graph = {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let db_path = std::env::temp_dir().join(format!(
            "metadata-checker-m53-shared-node-{}-{nanos}.db",
            std::process::id(),
        ));
        let _ = std::fs::remove_file(&db_path);
        scan_project(
            std::path::Path::new("tests/fixtures/test_project"),
            &db_path,
        )?;
        GraphDB::open(&db_path)?
    };
    let graph_store: &dyn GraphReadStore = &graph;
    let index = PageDependencyIndex::build(graph_store)?;

    let mut shared_node: Option<String> = None;
    let page_ids = collect_fixture_page_ids(&graph)?;
    for node in graph.iter_nodes()? {
        let pages = index.pages_for_node(&node.id);
        if pages.len() >= 2 {
            shared_node = Some(node.id);
            break;
        }
    }

    let Some(shared_node) = shared_node else {
        // ponytail: fixture 若无跨页共享节点，用 page 节点本身覆盖多 budget 失效路径。
        let (mut runtime, _temp_dir) = fixture_runtime("shared-node-fallback")?;
        let page_id = "page:app/actions_test.spg";
        runtime.warm_page_logic_availability(page_id, "compact")?;
        runtime.warm_page_logic_availability(page_id, "normal")?;
        runtime.warm_page_logic_availability("page:app/page_relations.spg", "compact")?;
        let removed = runtime.invalidate_pages_for_dirty_nodes(&[page_id.to_string()])?;
        assert_eq!(removed, vec![page_id.to_string()]);
        let read_model = runtime.read_model.as_ref().unwrap();
        assert!(!read_model.has_page_logic_availability(page_id, "compact"));
        assert!(!read_model.has_page_logic_availability(page_id, "normal"));
        assert!(read_model.has_page_logic_availability("page:app/page_relations.spg", "compact"));
        return Ok(());
    };

    let affected_pages: Vec<String> = index
        .affected_pages(&[shared_node.clone()])
        .into_iter()
        .collect();
    assert!(
        affected_pages.len() >= 2,
        "shared node {shared_node} must affect multiple pages"
    );

    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-shared-runtime-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    let temp_dir = db_path.parent().unwrap().to_path_buf();
    let mut runtime = GraphRuntime::load_with_project_dir_and_mode(
        &db_path,
        Some(&temp_dir),
        RuntimeMode::LongLived,
    )?;

    for page_id in &page_ids {
        runtime.warm_page_logic_availability(page_id, "compact")?;
    }
    let removed = runtime.invalidate_pages_for_dirty_nodes(&[shared_node])?;
    for page_id in &affected_pages {
        assert!(
            removed.iter().any(|removed_page| removed_page == page_id),
            "shared node invalidation must remove {page_id}"
        );
        let read_model = runtime.read_model.as_ref().unwrap();
        assert!(
            !read_model.has_page_logic_availability(page_id, "compact"),
            "warm cache for affected page {page_id} must be removed"
        );
    }

    let unaffected: Vec<String> = page_ids
        .into_iter()
        .filter(|page_id| !affected_pages.contains(page_id))
        .collect();
    let read_model = runtime.read_model.as_ref().unwrap();
    for page_id in unaffected {
        assert!(
            read_model.has_page_logic_availability(&page_id, "compact"),
            "unaffected page {page_id} must keep warm cache"
        );
    }
    Ok(())
}

/// 手动 profiling：`M53_PROFILE=1 cargo test ... m53_xiaoshouyi_warm_cache_footprint -- --ignored --nocapture`
#[test]
#[ignore = "manual profiling helper"]
fn m53_xiaoshouyi_warm_cache_footprint() -> anyhow::Result<()> {
    if std::env::var("M53_PROFILE").is_err() {
        return Ok(());
    }
    let project_dir = std::path::Path::new(
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
    );
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m53-footprint-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(project_dir, &db_path)?;
    let graph = GraphDB::open(&db_path)?;
    let dense_snapshot = metadata_checker::dense_graph::DenseGraphSnapshot::from_graph(&graph)?;
    let page_id = "page:app/售后.app/绑定车辆/会员已注册.spg";
    for budget in ["compact", "normal", "full"] {
        let cache = build_page_logic_availability_cache(
            &graph,
            Some(&dense_snapshot),
            page_id,
            Some(project_dir),
            budget,
            None,
        )?;
        println!("{budget} footprint: {:?}", cache.cache_footprint());
    }
    Ok(())
}
