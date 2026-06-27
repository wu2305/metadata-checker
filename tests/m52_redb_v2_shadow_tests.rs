#![cfg(feature = "cli-local")]

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_redb_v2::{
    REDB_V2_SCHEMA_VERSION, build_v2_layout, hydrate_graph_from_v2, read_v2_layout,
    shadow_compare_v1_v2, write_v2_shadow,
};
use metadata_checker::scanner::scan_project;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph(test_name: &str) -> anyhow::Result<(GraphDB, std::path::PathBuf)> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m52-redb-v2-{test_name}-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    let graph = GraphDB::open(&db_path)?;
    Ok((graph, db_path))
}

/// 验证 persist 会写入 v2 shadow 表，且 hydrate 后与 v1 等价。
#[test]
fn m52_redb_v2_shadow_persist_round_trip_matches_v1() -> anyhow::Result<()> {
    let (v1, db_path) = fixture_graph("persist-round-trip")?;
    let file_states = v1.load_file_states()?;

    let layout = build_v2_layout(&v1, &file_states)?;
    assert_eq!(layout.meta.schema_version, REDB_V2_SCHEMA_VERSION);
    assert_eq!(layout.meta.node_count as usize, v1.node_indices.len());
    assert!(layout.meta.edge_count > 0);
    assert!(layout.meta.content_fingerprint > 0);

    let report = shadow_compare_v1_v2(&v1, &layout)?;
    assert!(
        report.equivalent,
        "in-memory v2 layout must match v1 before persist: {:?}",
        report
    );

    write_v2_shadow(&db_path, &layout)?;
    let loaded =
        read_v2_layout(&db_path)?.expect("persisted db must contain readable v2 shadow layout");
    assert_eq!(loaded, layout);

    let hydrated = hydrate_graph_from_v2(&loaded, db_path.to_string_lossy().as_ref())?;
    let hydrated_report = shadow_compare_v1_v2(&v1, &loaded)?;
    assert!(
        hydrated_report.equivalent,
        "hydrated v2 graph must match v1: {:?}",
        hydrated_report
    );
    assert_eq!(
        hydrated.node_indices.len(),
        v1.node_indices.len(),
        "hydrated node count must match v1"
    );
    Ok(())
}

/// 验证 scan/persist 会写入 v2 shadow，且 v1 打开路径不依赖 v2 可读性。
#[test]
fn m52_redb_v2_scan_writes_shadow_and_v1_still_opens() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("scan-writes-shadow")?;
    let layout = read_v2_layout(&db_path)?.expect("scan/persist must write v2 shadow tables");
    let report = shadow_compare_v1_v2(&graph, &layout)?;
    assert!(
        report.equivalent,
        "scan-written v2 shadow must match v1 graph: {:?}",
        report
    );

    let reopened = GraphDB::open(&db_path)?;
    assert_eq!(
        reopened.node_indices.len(),
        graph.node_indices.len(),
        "v1 open must succeed when v2 shadow is present"
    );
    Ok(())
}

/// 验证 fingerprint 不匹配时 read_v2_layout 返回 miss。
#[test]
fn m52_redb_v2_fingerprint_mismatch_returns_miss() -> anyhow::Result<()> {
    let (v1, db_path) = fixture_graph("fingerprint-mismatch")?;
    let file_states = v1.load_file_states()?;
    let mut layout = build_v2_layout(&v1, &file_states)?;
    write_v2_shadow(&db_path, &layout)?;

    layout.meta.content_fingerprint ^= 1;
    write_v2_shadow(&db_path, &layout)?;

    assert!(
        read_v2_layout(&db_path)?.is_none(),
        "tampered fingerprint must invalidate v2 shadow read"
    );

    let graph = GraphDB::open(&db_path)?;
    assert!(
        graph.node_indices.len() > 0,
        "v1 must remain readable when v2 fingerprint mismatches"
    );
    Ok(())
}
