//! M11 图数据库路径、只读与并发可用性测试

use metadata_checker::output::schema::{AiOutput, OutputKind};
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    std::env::current_dir()
        .unwrap()
        .join("target/debug/metadata-checker")
}

fn run_cli(args: &[&str]) -> String {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("Failed to run metadata-checker");
    String::from_utf8(output.stdout).expect("Invalid UTF-8")
}

#[test]
fn test_default_graph_db_path_compatibility() {
    // Default path: tests/fixtures/test_project/.metadata-checker.graphdb
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
    ]);
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::ModelQuery);
    assert!(
        ai.diagnostics
            .iter()
            .all(|d| d.code != "GRAPH_DB_NOT_FOUND")
    );
}

#[test]
fn test_custom_graph_db_path_build_and_query() {
    let tmp = std::env::temp_dir().join("m11_test_custom.graphdb");
    let _ = std::fs::remove_file(&tmp);

    let build_out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
        "--graph-db-path",
        tmp.to_str().unwrap(),
    ]);
    assert!(build_out.contains("Graph database built"));
    assert!(tmp.exists(), "graphdb should exist at custom path");

    let query_out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
        "--graph-db-path",
        tmp.to_str().unwrap(),
    ]);
    let ai: AiOutput = serde_json::from_str(&query_out).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::ModelQuery);

    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_graph_db_not_found_returns_structured_diagnostic() {
    let tmp = std::env::temp_dir()
        .join("m11_missing_")
        .join("missing.graphdb");
    let _ = std::fs::remove_dir_all(tmp.parent().unwrap());

    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
        "--graph-db-path",
        tmp.to_str().unwrap(),
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::GraphDbCheck);
    assert!(
        ai.diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_DB_NOT_FOUND"),
        "expected GRAPH_DB_NOT_FOUND diagnostic, got: {:?}",
        ai.diagnostics
    );
    assert!(
        ai.next_queries.iter().any(|q| q.contains("--build-graph")),
        "next_queries should suggest build-graph"
    );
}

#[test]
fn test_check_graph_output_contract() {
    let output = run_cli(&[
        "--check-graph",
        "--graph-db-path",
        "tests/fixtures/test_project/.metadata-checker.graphdb",
    ]);
    let ai: AiOutput = serde_json::from_str(&output).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::GraphDbCheck);
    assert!(ai.summary.get("db_path").is_some());
    assert!(ai.summary.get("exists").is_some());
    assert!(ai.summary.get("readable").is_some());
    assert!(ai.summary.get("writable").is_some());
    assert!(ai.summary.get("needs_rebuild").is_some());
}

#[test]
fn test_check_graph_missing_db() {
    let tmp = std::env::temp_dir().join("m11_check_missing.graphdb");
    let _ = std::fs::remove_file(&tmp);

    let output = run_cli(&["--check-graph", "--graph-db-path", tmp.to_str().unwrap()]);
    let ai: AiOutput = serde_json::from_str(&output).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::GraphDbCheck);
    assert!(
        ai.diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_DB_NOT_FOUND"),
        "expected GRAPH_DB_NOT_FOUND"
    );
}

#[test]
fn test_consecutive_project_queries_no_lock_panic() {
    let tmp = std::env::temp_dir().join("m11_concurrent.graphdb");
    let _ = std::fs::remove_file(&tmp);

    // Build first
    let _ = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
        "--graph-db-path",
        tmp.to_str().unwrap(),
    ]);

    // Run multiple queries in sequence without explicit drop
    for _ in 0..3 {
        let out = run_cli(&[
            "--project-dir",
            "tests/fixtures/test_project",
            "--query-model",
            "model1",
            "--graph-db-path",
            tmp.to_str().unwrap(),
        ]);
        let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
        assert_eq!(
            ai.kind,
            OutputKind::ModelQuery,
            "consecutive query should succeed"
        );
        assert!(
            ai.diagnostics.iter().all(|d| d.code != "GRAPH_DB_LOCKED"),
            "should not hit lock"
        );
    }

    let _ = std::fs::remove_file(&tmp);
}

#[test]
#[ignore = "requires real project at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_build_graph_to_tmp_real_project() {
    let project = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
    if !std::path::Path::new(project).exists() {
        return; // skip if real project not available
    }
    let tmp = std::env::temp_dir().join("m11_real_project.graphdb");
    let _ = std::fs::remove_file(&tmp);

    let build_out = run_cli(&[
        "--project-dir",
        project,
        "--build-graph",
        "--graph-db-path",
        tmp.to_str().unwrap(),
    ]);
    assert!(
        build_out.contains("Graph database built"),
        "build failed: {}",
        build_out
    );

    // Run a few queries
    let queries = &[
        &[
            "--project-dir",
            project,
            "--query-page-logic",
            "page:app/actions_test.spg",
            "--graph-db-path",
            tmp.to_str().unwrap(),
        ],
        &[
            "--project-dir",
            project,
            "--query-model",
            "model1",
            "--graph-db-path",
            tmp.to_str().unwrap(),
        ],
        &[
            "--project-dir",
            project,
            "--explain",
            "comp:app/actions_test.spg|button1",
            "--graph-db-path",
            tmp.to_str().unwrap(),
        ],
    ];
    for args in queries {
        let out = run_cli(args.as_slice());
        let ai: AiOutput = serde_json::from_str(&out).expect("must be AiOutput");
        assert!(
            ai.diagnostics
                .iter()
                .all(|d| d.code != "GRAPH_DB_NOT_FOUND" && d.code != "GRAPH_DB_LOCKED"),
            "real project query failed: {:?}",
            ai.diagnostics
        );
    }

    let _ = std::fs::remove_file(&tmp);
}
