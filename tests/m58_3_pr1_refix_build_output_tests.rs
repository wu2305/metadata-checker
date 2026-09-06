#![cfg(feature = "cli-local")]

//! M58.3 PR1 refix：`--build-graph` 输出契约测试。
//!
//! 背景：cli.rs 声明 non-human 单 JSON 是默认输出模式，但旧实现无条件先打一行
//! 人类统计（"Indexed N files | ..."），导致默认模式 stdout 无法整体解析为单个
//! JSON 文档。修复后默认模式必须恰好输出一个完整 ScanReport JSON；human 模式
//! 保留原有人类可读行。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 为每次运行生成独立临时图库路径，避免并行测试互相踩库。
fn temp_db_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-refix-{}-{}-{nanos}.db",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    db_path
}

/// 清理临时图库文件及其锁文件。
fn cleanup_db(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
}

/// 运行真实 CLI 二进制的 --build-graph 并返回 stdout（进程必须成功退出）。
fn run_build_graph(extra_args: &[&str], db_path: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_metadata-checker"))
        .args([
            "--project-dir",
            "tests/fixtures/test_project",
            "--build-graph",
            "--graph-db-path",
            db_path.to_str().expect("utf8 path"),
        ])
        .args(extra_args)
        .output()
        .expect("run cli --build-graph");
    assert!(
        output.status.success(),
        "cli exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout must be utf8")
}

/// 默认（非 human）模式：stdout 必须整体解析为恰好一个 JSON 文档，
/// 且包含 ScanReport 全部统计字段与 diagnostics。
#[test]
fn refix_default_output_is_single_json_document() -> anyhow::Result<()> {
    let db_path = temp_db_path("default-json");
    let stdout = run_build_graph(&[], &db_path);

    // 不再先打人类统计行
    assert!(!stdout.contains("Indexed "), "{stdout}");
    assert!(!stdout.contains("Graph database built at"), "{stdout}");

    // 整体（含行尾换行）必须解析为单个 JSON 文档：
    // 若后面还拼接了第二段输出，from_str 会因 trailing characters 失败。
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("stdout must be a single JSON document: {err}\n{stdout}"));

    for field in [
        "indexed",
        "unchanged",
        "dirty",
        "deleted",
        "node_count",
        "edge_count",
        "diagnostics",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}: {value}");
    }
    assert!(value["diagnostics"].is_array(), "{value}");
    assert_eq!(value["indexed"].is_u64(), true, "{value}");

    cleanup_db(&db_path);
    Ok(())
}

/// 默认模式在有诊断时同样只输出一个 JSON 文档（无前置人类统计行）。
#[test]
fn refix_default_output_with_diagnostics_stays_single_json() -> anyhow::Result<()> {
    // 构造含未识别容器键的临时项目，确保 ScanReport.diagnostics 非空
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    let project_dir = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-refix-diagproj-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&project_dir)?;
    std::fs::write(
        project_dir.join("bad.spg"),
        serde_json::to_string(&serde_json::json!({
            "canvas": {
                "components": [
                    // 混合形态数组（部分元素缺 id/type）：PR2 形态感知递归下判非组件，计入未识别容器键
                    {"id": "a", "type": "panel", "myContainer": [{"id": "b", "type": "button"}, {"label": "no-id"}]}
                ]
            }
        }))?,
    )?;
    let db_path = project_dir.join("graph.db");

    let output = Command::new(env!("CARGO_BIN_EXE_metadata-checker"))
        .args([
            "--project-dir",
            project_dir.to_str().expect("utf8 path"),
            "--build-graph",
            "--graph-db-path",
            db_path.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run cli --build-graph");
    assert!(
        output.status.success(),
        "cli exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout must be utf8");

    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("stdout must be a single JSON document: {err}\n{stdout}"));
    let diagnostics = value["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.get("code").and_then(|c| c.as_str())
                == Some("SCANNER_UNRECOGNIZED_CONTAINER_KEY")),
        "{value}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// human 模式（--human / --interactive）：保留原有人类可读统计行。
#[test]
fn refix_human_output_keeps_human_stats_line() -> anyhow::Result<()> {
    let db_path = temp_db_path("human-line");
    let stdout = run_build_graph(&["--human"], &db_path);

    assert!(stdout.contains("Indexed "), "{stdout}");
    assert!(stdout.contains("Graph database built at"), "{stdout}");

    cleanup_db(&db_path);
    Ok(())
}
