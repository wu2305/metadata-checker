#![cfg(feature = "cli-local")]

//! M11 图数据库路径、只读与并发可用性测试

use fs2::FileExt;
use metadata_checker::output::schema::{AiOutput, OutputKind};
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metadata-checker"))
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
        "--human",
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
        "--human",
        "--graph-db-path",
        tmp.to_str().unwrap(),
        "--graph-lock-timeout-ms",
        "30000",
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
            "page:app/测试.app/文件快速上传.spg",
            "--graph-db-path",
            tmp.to_str().unwrap(),
            "--graph-lock-timeout-ms",
            "30000",
        ],
        &[
            "--project-dir",
            project,
            "--query-model",
            "crm_customermanager",
            "--graph-db-path",
            tmp.to_str().unwrap(),
            "--graph-lock-timeout-ms",
            "30000",
        ],
        &[
            "--project-dir",
            project,
            "--explain",
            "page:app/测试.app/文件快速上传.spg",
            "--graph-db-path",
            tmp.to_str().unwrap(),
            "--graph-lock-timeout-ms",
            "30000",
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

#[test]
#[ignore = "requires real project at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_parallel_queries() {
    let project = "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";
    if !std::path::Path::new(project).exists() {
        return;
    }
    let tmp = std::env::temp_dir().join("m11_parallel.graphdb");
    let _ = std::fs::remove_file(&tmp);

    // Build first
    let build_out = run_cli(&[
        "--project-dir",
        project,
        "--build-graph",
        "--human",
        "--graph-db-path",
        tmp.to_str().unwrap(),
        "--graph-lock-timeout-ms",
        "30000",
    ]);
    assert!(
        build_out.contains("Graph database built"),
        "build failed: {}",
        build_out
    );

    // Run two queries in parallel threads
    let db_path1 = tmp.clone();
    let db_path2 = tmp.clone();
    let project1 = project.to_string();
    let project2 = project.to_string();

    let handle1 = std::thread::spawn(move || {
        run_cli(&[
            "--project-dir",
            &project1,
            "--query-model",
            "crm_customermanager",
            "--graph-db-path",
            db_path1.to_str().unwrap(),
            "--graph-lock-timeout-ms",
            "30000",
        ])
    });
    let handle2 = std::thread::spawn(move || {
        run_cli(&[
            "--project-dir",
            &project2,
            "--query-page-logic",
            "page:app/测试.app/文件快速上传.spg",
            "--graph-db-path",
            db_path2.to_str().unwrap(),
            "--graph-lock-timeout-ms",
            "30000",
        ])
    });

    let out1 = handle1.join().expect("thread 1 panicked");
    let out2 = handle2.join().expect("thread 2 panicked");

    for (label, out) in [("model", &out1), ("page_logic", &out2)] {
        eprintln!(
            "[TEST DEBUG] {} raw stdout (len={}): {}",
            label,
            out.len(),
            out
        );
        let ai: AiOutput = match serde_json::from_str(out) {
            Ok(v) => v,
            Err(e) => panic!(
                "parallel query {} parse error: {}. stdout: {}",
                label, e, out
            ),
        };
        for d in &ai.diagnostics {
            eprintln!(
                "[TEST DEBUG] {} diag: code={} msg={}",
                label, d.code, d.message
            );
        }
        assert!(
            ai.diagnostics
                .iter()
                .all(|d| d.code != "GRAPH_DB_NOT_FOUND" && d.code != "GRAPH_DB_LOCKED"),
            "parallel query {} failed: {:?}",
            label,
            ai.diagnostics
        );
    }

    let _ = std::fs::remove_file(&tmp);
}

/// 主动制造锁冲突，验证 CLI 返回 GRAPH_DB_LOCKED 结构化诊断
#[test]
fn test_graph_db_locked_returns_structured_diagnostic() {
    let db_path = std::env::temp_dir().join("m15_lock_test.graphdb");
    let lock_path = db_path.with_extension("graphdb.lock");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);

    // 先 build graph（--human：默认模式输出单 JSON，人类统计行只在 human 模式打印）
    let build_out = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--build-graph",
        "--human",
        "--graph-db-path",
        db_path.to_str().unwrap(),
    ]);
    assert!(build_out.contains("Graph database built"));
    assert!(db_path.exists());

    // 手动持有辅助锁文件，阻止 CLI 进程获取
    let lock_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)
        .expect("should create lock file in test");
    lock_file
        .try_lock_exclusive()
        .expect("should hold graphdb lock in test");

    // CLI 进程在 100ms 超时后应返回 GRAPH_DB_LOCKED
    let output = run_cli(&[
        "--project-dir",
        "tests/fixtures/test_project",
        "--query-model",
        "model1",
        "--graph-db-path",
        db_path.to_str().unwrap(),
        "--graph-lock-timeout-ms",
        "100",
    ]);

    let ai: AiOutput = serde_json::from_str(&output).expect("must be AiOutput");
    assert_eq!(ai.kind, OutputKind::GraphDbCheck);
    let has_locked = ai.diagnostics.iter().any(|d| d.code == "GRAPH_DB_LOCKED");
    assert!(
        has_locked,
        "expected GRAPH_DB_LOCKED diagnostic when lock is held, got: {:?}",
        ai.diagnostics
    );

    // suggestion 必须包含等待、换路径或提高超时
    let suggestion = ai
        .diagnostics
        .iter()
        .find(|d| d.code == "GRAPH_DB_LOCKED")
        .and_then(|d| d.suggestion.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("");
    assert!(
        suggestion.contains("wait")
            || suggestion.contains("graph-db-path")
            || suggestion.contains("lock-timeout"),
        "GRAPH_DB_LOCKED suggestion should mention wait, graph-db-path, or lock-timeout, got: {}",
        suggestion
    );

    // next_queries 应该给出可执行的替代命令
    assert!(
        !ai.next_queries.is_empty(),
        "GRAPH_DB_LOCKED output should provide next_queries, got none"
    );

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&lock_path);
}
