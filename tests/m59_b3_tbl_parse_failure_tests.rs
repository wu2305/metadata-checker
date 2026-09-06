#![cfg(feature = "cli-local")]

//! M59 阶段 B3：损坏的 `.tbl` 不再被当成「成功的空结果」提交。
//!
//! 旧行为（`tbl.rs:14-21`）：`content.is_empty()` 与 `serde_json::from_str` 的
//! `Err(_)` 都 `return Ok(空集)`。配合增量路径「先删旧节点、再重建」，一个写坏
//! 的 `.tbl` 会让旧模型图被删干净、新图为空，而 file hash 照常记录——下一轮内容
//! 不变直接跳过。于是「读不出来」被永久固化成「模型不存在」，且没有任何诊断。
//!
//! 新契约（三条，缺一不可）：
//! 1. 解析失败的文件**不进候选图**，上一份有效图原样保留（陈旧，不是缺失）；
//! 2. 发出 `SCANNER_FILE_PARSE_FAILED`，让查询方能区分陈旧与缺失；
//! 3. file hash **不**记录，文件保持脏，修好后下一轮自动重新入图、诊断消失。

use metadata_checker::graph::GraphDB;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::output::Diagnostic;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::process_tbl_file_from_string;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CODE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";
const MODEL_ID: &str = "model:orders";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-m59-b3-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

/// 合法表定义：两个维度。第三个维度用于验证「修好之后确实重新解析了」。
fn valid_tbl(extra_dimension: bool) -> String {
    let mut dims = vec![
        serde_json::json!({"name": "订单号", "dbfield": "orderNo", "dataType": "C"}),
        serde_json::json!({"name": "金额", "dbfield": "amount", "dataType": "N"}),
    ];
    if extra_dimension {
        dims.push(serde_json::json!({"name": "客户", "dbfield": "customer", "dataType": "C"}));
    }
    serde_json::json!({"version": "1.0", "dimensions": dims}).to_string()
}

fn code_hits<'a>(diags: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.code == code).collect()
}

/// `model:orders` 的邻边总数——字段边逐条落在这里，用来证明「旧图逐边保留」
/// 而不只是「同名节点还在」。
fn model_edge_count(db_path: &Path) -> usize {
    let graph = GraphDB::open(db_path).expect("graph should open");
    let (outgoing, incoming) =
        metadata_checker::graph::get_node_edges_as_tuples(&graph, MODEL_ID).expect("edges");
    outgoing.len() + incoming.len()
}

fn model_present(db_path: &Path) -> bool {
    let graph = GraphDB::open(db_path).expect("graph should open");
    graph.get_node(MODEL_ID).is_some()
}

/// 单文件层：非法 JSON 必须**报错**，不能返回 Ok(空集)。
/// 这是整条链路的源头——只要它还能悄悄成功，上层无论怎么写都分不出陈旧与缺失。
#[test]
fn corrupt_tbl_content_is_an_error_not_an_empty_success() {
    let mut store = MemoryGraphStore::new();
    let err = process_tbl_file_from_string(&mut store, "data/orders.tbl", "{\"dimensions\": [")
        .expect_err("非法 JSON 必须返回 Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("data/orders.tbl"),
        "报错应点名出问题的文件，实际得到 {msg}"
    );
}

/// 空文件同样不是「没有模型」，而是没读出来。
#[test]
fn empty_tbl_content_is_an_error_not_an_empty_success() {
    let mut store = MemoryGraphStore::new();
    let err = process_tbl_file_from_string(&mut store, "data/orders.tbl", "   \n")
        .expect_err("空内容必须返回 Err");
    assert!(
        format!("{err:#}").contains("empty"),
        "报错应说明文件为空，实际得到 {err:#}"
    );
}

/// 端到端：坏 `.tbl` → 旧图保留 + 陈旧诊断 + 保持脏 → 修好后自动恢复。
#[test]
fn corrupt_tbl_preserves_previous_graph_and_reports_stale() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("corrupt-tbl");
    std::fs::create_dir_all(project_dir.join("data"))?;
    let tbl_path = project_dir.join("data").join("orders.tbl");
    std::fs::write(&tbl_path, valid_tbl(false))?;
    let db_path = project_dir.join("graph.db");

    // ---- 第 1 轮：合法内容，模型入图，无解析失败诊断 ----
    let first = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&first.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "合法内容不该有解析失败诊断: {:?}",
        first.diagnostics
    );
    assert!(model_present(&db_path), "合法 .tbl 应产出 {MODEL_ID}");
    let baseline_edges = model_edge_count(&db_path);
    assert!(baseline_edges > 0, "模型应带字段边");

    // ---- 第 2 轮：写坏文件 ----
    std::fs::write(&tbl_path, "{\"version\": \"1.0\", \"dimensions\": [")?;
    let second = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let hits = code_hits(&second.diagnostics, CODE_PARSE_FAILED);
    assert_eq!(
        hits.len(),
        1,
        "损坏文件应产出恰好一条解析失败诊断: {:?}",
        second.diagnostics
    );
    assert_eq!(hits[0].count, Some(1), "{:?}", second.diagnostics);
    assert!(
        hits[0]
            .location
            .source_file
            .as_deref()
            .is_some_and(|f| f.contains("orders.tbl")),
        "诊断应定位到损坏的文件: {:?}",
        hits[0]
    );

    // 核心：旧图**原样保留**。旧实现在这里会把 model:orders 连同字段边一起删掉。
    assert!(
        model_present(&db_path),
        "解析失败不得删除上一份有效图——那会把「读不出来」说成「模型不存在」"
    );
    assert_eq!(
        model_edge_count(&db_path),
        baseline_edges,
        "旧图应逐边保留，而不是被替换成空结果"
    );

    // ---- 第 3 轮：内容未变，文件必须仍是脏的（hash 没被记录） ----
    let third = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        third.report.dirty > 0,
        "解析失败的文件不得记录 file hash，否则下轮会被当成「已处理」永久跳过: {:?}",
        third.report
    );
    assert_eq!(
        code_hits(&third.diagnostics, CODE_PARSE_FAILED).len(),
        1,
        "陈旧标记应持续存在直到文件修好: {:?}",
        third.diagnostics
    );
    assert!(model_present(&db_path), "重试一轮也不该丢图");

    // ---- 第 4 轮：修好（并加一个维度，证明确实重新解析了） ----
    std::fs::write(&tbl_path, valid_tbl(true))?;
    let fourth = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&fourth.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "文件修好后陈旧标记必须当场消失: {:?}",
        fourth.diagnostics
    );
    assert!(model_present(&db_path));
    assert!(
        model_edge_count(&db_path) > baseline_edges,
        "新增的维度应体现在图里，说明文件确实被重新解析而非沿用旧图"
    );

    std::fs::remove_dir_all(&project_dir).ok();
    Ok(())
}
