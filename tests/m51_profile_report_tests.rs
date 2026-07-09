#![cfg(feature = "cli-local")]

use metadata_checker::graph::GraphDB;
use metadata_checker::perf_report::{
    PageLogicProfileScenario, build_core_profile_report, build_page_logic_profile_report,
};
use metadata_checker::scanner::scan_project;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture_graph() -> anyhow::Result<GraphDB> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m51-profile-report-{}-{nanos}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    GraphDB::open(&db_path)
}

#[test]
fn m51_page_logic_profile_report_summarizes_stage_and_counter_costs() -> anyhow::Result<()> {
    let graph = fixture_graph()?;
    let scenario = PageLogicProfileScenario::new(
        "fixture_page_logic_compact",
        "page:app/page_relations.spg",
        "compact",
    );

    let report = build_page_logic_profile_report(
        &graph,
        Some(std::path::Path::new("tests/fixtures/test_project")),
        &[scenario],
        2,
    )?;

    assert_eq!(report.report_kind, "m51_page_logic_profile");
    assert_eq!(report.scenarios.len(), 1);
    let scenario_report = &report.scenarios[0];
    assert_eq!(scenario_report.scenario.name, "fixture_page_logic_compact");
    assert_eq!(scenario_report.samples.len(), 2);
    assert!(scenario_report.output_bytes > 0);
    assert!(
        scenario_report
            .samples
            .iter()
            .all(|sample| sample.profile.stage("edge_scan").is_some()),
        "every sample must retain raw stage profile"
    );
    assert!(
        scenario_report
            .stage_summary
            .iter()
            .any(|stage| stage.name == "edge_scan" && stage.sample_count == 2),
        "report must summarize edge scan stage cost"
    );
    let edges_scanned = scenario_report
        .counter_summary
        .get("edges_scanned")
        .expect("edges_scanned counter summary is required");
    assert_eq!(edges_scanned.sample_count, 2);
    assert!(edges_scanned.max > 0);
    assert!(
        scenario_report
            .cost_model
            .iter()
            .any(|model| model.hotspot == "key_model_availability"
                && model.purpose.contains("数据可用性")
                && model
                    .space_time_candidates
                    .iter()
                    .any(|item| item.contains("index"))),
        "page logic report should explain why availability is expensive and name index candidates"
    );
    assert!(
        scenario_report
            .cost_model
            .iter()
            .any(|model| model.hotspot == "path_summary"
                && model
                    .cost_drivers
                    .iter()
                    .any(|driver| driver.contains("path_"))),
        "page logic report should explain path_summary cost drivers"
    );
    Ok(())
}

#[test]
fn m51_core_profile_report_covers_rebuild_redb_and_runtime() -> anyhow::Result<()> {
    let report = build_core_profile_report(std::path::Path::new("tests/fixtures/test_project"), 1)?;

    assert_eq!(report.report_kind, "m51_core_profile");
    for scenario in ["rebuild", "redb", "runtime"] {
        let scenario_report = report
            .scenarios
            .iter()
            .find(|item| item.scenario == scenario)
            .unwrap_or_else(|| panic!("missing {scenario} scenario"));
        assert_eq!(scenario_report.samples.len(), 1);
        assert!(
            !scenario_report.stage_summary.is_empty(),
            "{scenario} must summarize stage costs"
        );
        assert!(
            !scenario_report.cost_model.is_empty(),
            "{scenario} must include performance cost model"
        );
    }
    Ok(())
}

#[test]
fn m52_core_profile_report_records_detailed_stage_contract() -> anyhow::Result<()> {
    let report = build_core_profile_report(std::path::Path::new("tests/fixtures/test_project"), 1)?;

    let expected = [
        (
            "rebuild",
            [
                "rebuild_cold_discover",
                "rebuild_cold_dirty_detection",
                "rebuild_cold_parse",
                "rebuild_cold_graph_apply",
                "rebuild_cold_persist_commit",
                "rebuild_noop_dirty_detection",
            ]
            .as_slice(),
        ),
        (
            "redb",
            [
                "redb_open",
                "redb_load_file_states",
                "redb_node_write",
                "redb_edge_write",
                "redb_commit",
            ]
            .as_slice(),
        ),
        (
            "runtime",
            [
                "runtime_graphdb_load",
                "runtime_status_dispatch",
                "runtime_query_dispatch",
                "runtime_check_reload_unchanged",
            ]
            .as_slice(),
        ),
    ];

    for (scenario, stages) in expected {
        let scenario_report = report
            .scenarios
            .iter()
            .find(|item| item.scenario == scenario)
            .unwrap_or_else(|| panic!("missing {scenario} scenario"));
        for stage in stages {
            assert!(
                scenario_report
                    .stage_summary
                    .iter()
                    .any(|summary| summary.name == *stage),
                "{scenario} must include {stage}"
            );
        }
        if scenario == "runtime" {
            for counter in [
                "runtime_dense_snapshot_build_ms",
                "runtime_dense_snapshot_nodes",
                "runtime_read_model_build_ms",
                "runtime_long_lived_load_ms",
                "runtime_long_lived_read_model_build_ms",
                "runtime_availability_facts_build_ms",
                "runtime_page_dependency_index_build_ms",
                "runtime_long_lived_availability_warm_ms",
                "runtime_long_lived_query_dispatch_ms",
                "runtime_long_lived_warmed_query_dispatch_ms",
                "runtime_one_shot_total_ms_n10",
                "runtime_long_lived_total_ms_n10",
                "runtime_long_lived_warmed_total_ms_n10",
            ] {
                assert!(
                    scenario_report.counter_summary.contains_key(counter),
                    "runtime profile must include {counter}"
                );
            }
        }
    }
    Ok(())
}
