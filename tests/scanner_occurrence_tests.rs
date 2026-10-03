#![cfg(feature = "cli-local")]

//! 扫描期诊断逐次记录（每处出现一条）回归测试。
//!
//! 此前只保留「计数 + 每类一个样例位置」，无法把诊断挂到具体节点。本组测试固定：
//! - 单文件扫描逐处产出记录，位置（source_file / node_id / json_path）准确；
//! - 记录数与聚合信封的计数一致（两者来自同一次遍历，不允许漂移）；
//! - 记录随 per-file entry 落库、跨文件按路径字典序合并，修复/删除文件后随之消失；
//! - 旧版本 entry（有计数、无记录）被显式列入 `legacy_files`，而不是静默当作「无诊断」；
//! - 解析失败也有一条记录，带原因原文。

use metadata_checker::graph::GraphDB;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::{scan_raw_diagnostics, scan_raw_occurrences};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CODE_UNRECOGNIZED: &str = "SCANNER_UNRECOGNIZED_CONTAINER_KEY";
const CODE_DUPLICATE: &str = "SCANNER_DUPLICATE_COMPONENT_ID";
const CODE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-scan-occurrence-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

/// 一页里两类问题各出现多次：
/// - `loose` 有 id 无 type（未识别）；`myContainer` 是混合形态数组（未识别）；
/// - `dup1` 出现三次（重复 2 次）。
fn spg_with_several_problems() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "loose"},
                {"id": "panel_a", "type": "panel",
                 "myContainer": [{"id": "child1", "type": "button"}, {"label": "no-id"}]},
                {"id": "dup1", "type": "button"},
                {"id": "dup1", "type": "input"},
                {"id": "dup1", "type": "label"}
            ]
        }
    })
}

fn spg_clean() -> serde_json::Value {
    serde_json::json!({
        "canvas": {"components": [{"id": "ok1", "type": "button"}]}
    })
}

fn write_spg(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_string(value)?)?;
    Ok(())
}

/// 逐处出现：每个问题一条记录，位置精确，source_file 回填为传入的逻辑路径。
#[test]
fn one_record_per_occurrence_with_precise_location() {
    let occurrences = scan_raw_occurrences(&spg_with_several_problems(), "app/page.spg");

    let unrecognized: Vec<_> = occurrences
        .iter()
        .filter(|o| o.code == CODE_UNRECOGNIZED)
        .collect();
    assert_eq!(unrecognized.len(), 2, "{occurrences:?}");
    let mut unrecognized_paths: Vec<_> = unrecognized
        .iter()
        .map(|o| {
            (
                o.location.node_id.clone().unwrap_or_default(),
                o.location.json_path.clone().unwrap_or_default(),
            )
        })
        .collect();
    unrecognized_paths.sort();
    assert_eq!(
        unrecognized_paths,
        vec![
            ("loose".to_string(), "canvas.components[0]".to_string()),
            (
                "panel_a".to_string(),
                "canvas.components[1].myContainer".to_string()
            ),
        ]
    );

    let duplicates: Vec<_> = occurrences
        .iter()
        .filter(|o| o.code == CODE_DUPLICATE)
        .collect();
    assert_eq!(duplicates.len(), 2, "三次出现 = 两次重复: {occurrences:?}");
    let duplicate_paths: Vec<_> = duplicates
        .iter()
        .map(|o| o.location.json_path.clone().unwrap_or_default())
        .collect();
    assert_eq!(
        duplicate_paths,
        vec!["canvas.components[3]", "canvas.components[4]"],
        "重复记录按出现顺序，指向后出现的那一处"
    );
    for occurrence in &occurrences {
        assert_eq!(
            occurrence.location.source_file.as_deref(),
            Some("app/page.spg"),
            "每条记录都应带源文件: {occurrence:?}"
        );
        assert_eq!(
            occurrence.detail.as_deref().map(str::is_empty),
            Some(false),
            "每条记录都应说明具体问题: {occurrence:?}"
        );
    }
}

/// 干净页面没有记录。
#[test]
fn clean_page_has_no_records() {
    assert_eq!(scan_raw_occurrences(&spg_clean(), "app/clean.spg").len(), 0);
}

/// 记录数与聚合信封计数一致：二者来自同一次遍历，不允许漂移。
#[test]
fn record_counts_match_aggregate_counts() {
    let value = spg_with_several_problems();
    let occurrences = scan_raw_occurrences(&value, "app/page.spg");
    for diagnostic in scan_raw_diagnostics(&value) {
        let records = occurrences
            .iter()
            .filter(|o| o.code == diagnostic.code)
            .count();
        assert_eq!(
            Some(records),
            diagnostic.count,
            "{} 的记录数应等于聚合计数",
            diagnostic.code
        );
    }
}

/// 落库后可读：跨文件按路径字典序合并，修复文件后其记录消失，旧库不误报。
#[test]
fn records_persist_merge_in_path_order_and_retire_on_fix() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("persist");
    std::fs::create_dir_all(&project_dir)?;
    write_spg(&project_dir.join("b.spg"), &spg_with_several_problems())?;
    write_spg(&project_dir.join("a.spg"), &spg_with_several_problems())?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let entries = GraphDB::open(&db_path)?.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(report.legacy_files, Vec::<String>::new());
    assert_eq!(report.occurrences.len(), 8, "两个文件各 4 条: {report:?}");
    let files: Vec<_> = report
        .occurrences
        .iter()
        .map(|o| o.location.source_file.clone().unwrap_or_default())
        .collect();
    assert_eq!(
        files,
        vec!["a.spg"; 4]
            .into_iter()
            .chain(vec!["b.spg"; 4])
            .map(String::from)
            .collect::<Vec<_>>(),
        "按文件路径字典序合并"
    );

    // 修复 a.spg 后重建：它的记录退场，b.spg 的保留
    write_spg(&project_dir.join("a.spg"), &spg_clean())?;
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    let entries = GraphDB::open(&db_path)?.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(report.occurrences.len(), 4, "{report:?}");
    assert!(
        report
            .occurrences
            .iter()
            .all(|o| o.location.source_file.as_deref() == Some("b.spg")),
        "{report:?}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 旧版本写入的 entry（有计数、没有 occurrences 字段）：聚合信封照常可用，
/// 逐次接口把它列入 legacy_files，不把「缺记录」说成「没问题」。
#[test]
fn legacy_entry_is_reported_not_silently_empty() -> anyhow::Result<()> {
    let legacy = serde_json::to_vec(&serde_json::json!({
        "unrecognized_container_key": 2,
        "duplicate_component_id": 0,
        "sample_unrecognized_location": null,
        "sample_duplicate_location": null,
    }))?;
    let clean = serde_json::to_vec(&serde_json::json!({
        "unrecognized_container_key": 0,
        "duplicate_component_id": 0,
        "sample_unrecognized_location": null,
        "sample_duplicate_location": null,
    }))?;
    let entries = vec![
        ("old.spg".to_string(), legacy),
        ("clean.spg".to_string(), clean),
    ];

    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(report.occurrences.len(), 0);
    assert_eq!(report.legacy_files, vec!["old.spg".to_string()]);

    let aggregate = ProjectIndexer::merge_scanner_diagnostic_entries(&entries)?;
    assert_eq!(aggregate.len(), 1, "聚合信封不受影响: {aggregate:?}");
    assert_eq!(aggregate[0].count, Some(2));
    Ok(())
}

/// 损坏的 entry 必须报错带路径，不能被当作空记录吞掉。
#[test]
fn corrupt_entry_is_an_error_with_path() {
    let entries = vec![("broken.spg".to_string(), b"not json".to_vec())];
    let error = ProjectIndexer::merge_scanner_occurrence_entries(&entries)
        .expect_err("损坏的 entry 必须报错");
    assert!(
        format!("{error:#}").contains("broken.spg"),
        "错误应带上出问题的文件路径: {error:#}"
    );
}

/// 解析失败也有一条记录：位置是该文件，detail 带失败原因原文；修好后消失。
///
/// 用 `.tbl` 触发：目前只有 TBL 的解析失败走「跳过并保留旧图」路径（M59-B3），
/// 损坏的 `.spg` 仍会让整轮扫描报错，这是既有行为，本测试不改它。
#[test]
fn parse_failure_has_a_record_and_retires_when_fixed() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("parse-failed");
    std::fs::create_dir_all(&project_dir)?;
    write_spg(&project_dir.join("good.spg"), &spg_clean())?;
    std::fs::write(
        project_dir.join("bad.tbl"),
        "{\"version\": \"1.0\", \"dimensions\": [",
    )?;
    let db_path = project_dir.join("graph.db");
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let entries = GraphDB::open(&db_path)?.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    let failed: Vec<_> = report
        .occurrences
        .iter()
        .filter(|o| o.code == CODE_PARSE_FAILED)
        .collect();
    assert_eq!(failed.len(), 1, "{report:?}");
    assert_eq!(failed[0].location.source_file.as_deref(), Some("bad.tbl"));
    assert!(
        failed[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("bad.tbl")),
        "detail 应带文件名与失败原因: {:?}",
        failed[0]
    );

    let fixed = serde_json::json!({
        "version": "1.0",
        "dimensions": [{"name": "订单号", "dbfield": "orderNo", "dataType": "C"}]
    });
    std::fs::write(project_dir.join("bad.tbl"), fixed.to_string())?;
    ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    let entries = GraphDB::open(&db_path)?.load_scanner_diagnostic_entries()?;
    let report = ProjectIndexer::merge_scanner_occurrence_entries(&entries)?;
    assert_eq!(report.occurrences.len(), 0, "{report:?}");

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}
