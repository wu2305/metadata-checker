#![cfg(feature = "cli-local")]

//! M58.3 复核返修（第二轮）回归测试：runtime/输出契约面。
//!
//! 覆盖：
//! - P1-1：加载期 scanner 诊断缓存读取/合并失败映射为结构化
//!   `SCANNER_DIAGNOSTICS_LOAD_FAILED`，--status 与查询响应可见（原死 vec 被吞）；
//! - P1-2：查询响应合并不按 code 去重——load_diagnostics 非空时查询侧同 code
//!   多条与跨来源同 code 条目全保留（原 seen_codes 去重静默丢弃）；
//! - P1-3：携带 answer_impact=partial 诊断的响应，confidence 块 level 保持 full
//!   但 statement 不得声称「没有降低置信度的诊断」。

use metadata_checker::diagnostics::{
    CODE_RUNTIME_READ_MODEL_DEGRADED, CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED, answer_impact_for,
    envelope_diagnostic, severity_for,
};
use metadata_checker::output::{DiagnosticSeverity, Location};
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::scanner::scan_project;
use redb::{Database, TableDefinition};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 与 graph_redb 内部表定义同名，用于测试注入/删除 scanner 诊断 entry
const SCANNER_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("scanner_diagnostics");

fn unique_db_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-refix-{tag}-{}-{nanos}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    db_path
}

fn cleanup_db(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
}

/// 用测试 fixture 构建正常图库（scanner 诊断表存在、无坏行）
fn fixture_graph(tag: &str) -> anyhow::Result<PathBuf> {
    let db_path = unique_db_path(tag);
    scan_project(Path::new("tests/fixtures/test_project"), &db_path)?;
    Ok(db_path)
}

fn query_page_logic_request() -> metadata_checker::runtime::RuntimeQueryRequest {
    metadata_checker::runtime::RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryPageLogic,
        target: "page:app/actions_test.spg".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    }
}

fn result_diagnostics(result: &Value) -> Vec<Value> {
    result
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn codes_of(entries: &[Value]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| entry.get("code").and_then(Value::as_str))
        .collect()
}

/// 信封六字段断言（与 m58_3_pr1_diagnostics_tests 的口径一致）
fn assert_envelope(value: &Value) {
    for field in [
        "code",
        "severity",
        "count",
        "sample_location",
        "answer_impact",
        "first_seen_phase",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}: {value}");
    }
}

// ---------------------------------------------------------------------------
// P1-1：加载期 scanner 诊断失败 → 结构化诊断可见
// ---------------------------------------------------------------------------

/// 合并臂：durable 中混入无法解码的损坏 entry，加载时 merge 失败。
/// 不得静默吞掉——load_diagnostics / status / 查询响应三处都必须可见
/// SCANNER_DIAGNOSTICS_LOAD_FAILED，且加载本身不被阻塞。
#[test]
fn scanner_diagnostics_merge_failure_is_structured_and_visible() -> anyhow::Result<()> {
    let db_path = fixture_graph("merge-fail")?;
    // 注入损坏 entry：merge_scanner_diagnostic_entries 必然 decode 失败
    {
        let db = Database::create(&db_path)?;
        let write_txn = db.begin_write()?;
        {
            let mut table = write_txn.open_table(SCANNER_TABLE)?;
            table.insert("corrupt/bad.spg", b"not-valid-json".to_vec())?;
        }
        write_txn.commit()?;
    }

    let mut rt = GraphRuntime::load(&db_path)?;
    let hits: Vec<_> = rt
        .load_diagnostics
        .iter()
        .filter(|d| d.code == CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "merge 失败必须恰好产生一条结构化诊断: {:?}",
        rt.load_diagnostics
    );
    assert!(
        hits[0].message.contains("merge failed"),
        "message 须区分 merge 失败与 load 失败: {}",
        hits[0].message
    );
    // 信封字段与 impact 注册表口径
    assert_eq!(hits[0].severity, DiagnosticSeverity::Warning);
    assert_eq!(hits[0].answer_impact.as_deref(), Some("partial"));

    // status 透出
    assert!(
        rt.status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED),
        "status 必须透出加载失败诊断"
    );

    // 查询响应透出（经合并闸门置顶）且为完整信封
    let resp = rt.query(query_page_logic_request())?;
    let diags = result_diagnostics(&resp.result);
    let response_hits: Vec<&Value> = diags
        .iter()
        .filter(|d| {
            d.get("code").and_then(Value::as_str) == Some(CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED)
        })
        .collect();
    assert_eq!(
        response_hits.len(),
        1,
        "查询响应必须可见加载失败诊断: {diags:?}"
    );
    assert_envelope(response_hits[0]);
    assert_eq!(
        response_hits[0]
            .get("answer_impact")
            .and_then(Value::as_str),
        Some("partial"),
        "{:?}",
        response_hits[0]
    );

    cleanup_db(&db_path);
    Ok(())
}

/// 读取臂：老库缺 SCANNER_DIAGNOSTICS_TABLE（open_readonly 从不建表）。
/// 与「无 scanner 问题」必须可区分。
#[test]
fn scanner_diagnostics_missing_table_is_structured_and_visible() -> anyhow::Result<()> {
    let db_path = fixture_graph("missing-table")?;
    // 删除 scanner 诊断表，模拟 M58.3 之前构建的老库
    {
        let db = Database::create(&db_path)?;
        let write_txn = db.begin_write()?;
        let deleted = write_txn.delete_table(SCANNER_TABLE)?;
        assert!(deleted, "fixture 库应存在 scanner 诊断表");
        write_txn.commit()?;
    }

    let mut rt = GraphRuntime::load(&db_path)?;
    let hits: Vec<_> = rt
        .load_diagnostics
        .iter()
        .filter(|d| d.code == CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED)
        .collect();
    assert_eq!(
        hits.len(),
        1,
        "缺表必须恰好产生一条结构化诊断: {:?}",
        rt.load_diagnostics
    );
    assert!(
        hits[0].message.contains("load failed"),
        "message 须区分 load 失败与 merge 失败: {}",
        hits[0].message
    );
    assert!(
        rt.status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED),
        "status 必须透出缺表诊断"
    );
    let resp = rt.query(query_page_logic_request())?;
    assert!(
        codes_of(&result_diagnostics(&resp.result)).contains(&CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED),
        "查询响应必须可见缺表诊断"
    );

    cleanup_db(&db_path);
    Ok(())
}

/// 新 code 注册表口径：severity / answer_impact 显式登记。
#[test]
fn new_codes_registered_in_severity_and_impact_tables() {
    assert_eq!(
        severity_for(CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED),
        DiagnosticSeverity::Warning
    );
    assert_eq!(
        answer_impact_for(CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED),
        "partial"
    );
    // read model 降级是性能层事件：Warning 但不影响答案置信度
    assert_eq!(
        severity_for(CODE_RUNTIME_READ_MODEL_DEGRADED),
        DiagnosticSeverity::Warning
    );
    assert_eq!(answer_impact_for(CODE_RUNTIME_READ_MODEL_DEGRADED), "none");
    let diag = envelope_diagnostic(
        CODE_RUNTIME_READ_MODEL_DEGRADED,
        1,
        Location::default(),
        "x",
    );
    assert_envelope(&serde_json::to_value(&diag).expect("信封序列化"));
}

// ---------------------------------------------------------------------------
// P1-2：查询响应合并不按 code 去重
// ---------------------------------------------------------------------------

/// load_diagnostics 非空时，查询侧自带诊断一条不丢、跨来源同 code 不去重。
///
/// 旧实现的 seen_codes 去重会把查询结果里同 code 的条目丢到只剩第一条
/// （page_logic 对 UNRESOLVED_PAGE_NAVIGATION / UNKNOWN_ACTION_TYPE 等按对象/类型
/// 各产一条）；spec「传播与归属」：同一 code 只在一处产生，跨来源不去重。
#[test]
fn query_side_diagnostics_survive_merge_with_load_diagnostics() -> anyhow::Result<()> {
    let db_path = fixture_graph("merge-keep")?;
    let mut rt = GraphRuntime::load(&db_path)?;
    assert!(
        rt.load_diagnostics.is_empty(),
        "fixture 正常路径 load_diagnostics 应为空（pr1 基线同口径）: {:?}",
        rt.load_diagnostics
    );

    // 基线：无 load 诊断时查询侧自带的 diagnostics
    let baseline = result_diagnostics(&rt.query(query_page_logic_request())?.result);
    assert!(!baseline.is_empty(), "actions_test 查询侧应有诊断");
    // actions_test.spg 只有 someCustomAction 一种未知 action 类型 → 恰好一条聚合
    assert_eq!(
        codes_of(&baseline)
            .iter()
            .filter(|code| **code == "UNKNOWN_ACTION_TYPE")
            .count(),
        1,
        "基线断言依赖 fixture 仅一种未知 action 类型: {baseline:?}"
    );

    // 注入跨来源同 code 的 load 诊断（契约上同 code 单源，这里人为构造违例场景
    // 钉住「不去重」语义：违例时保留双份是 fail-visible）
    rt.load_diagnostics.push(envelope_diagnostic(
        "UNKNOWN_ACTION_TYPE",
        1,
        Location::default(),
        "load-side probe: cross-source same code",
    ));
    let resp = rt.query(query_page_logic_request())?;
    let after = result_diagnostics(&resp.result);

    assert_eq!(
        after.len(),
        baseline.len() + 1,
        "合并后 = load 侧 1 条 + 查询侧原样全保留: {after:?}"
    );
    assert_eq!(
        after[0].get("message").and_then(Value::as_str),
        Some("load-side probe: cross-source same code"),
        "load 侧置顶: {after:?}"
    );
    assert_eq!(
        &after[1..],
        baseline.as_slice(),
        "查询侧条目必须原序全保留（含同 code 条目）"
    );

    cleanup_db(&db_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// P1-3：confidence 块不再说谎
// ---------------------------------------------------------------------------

/// 混合形态数组（部分元素缺 id/type）：PR2 形态感知递归下判非组件，
/// 计 1 次 SCANNER_UNRECOGNIZED_CONTAINER_KEY
fn spg_with_unrecognized_key() -> Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "panel_a", "type": "panel", "myContainer": [{"id": "child1", "type": "button"}, {"label": "no-id"}]}
            ]
        }
    })
}

/// CLI 端到端：响应携带 answer_impact=partial 的 SCANNER_* 诊断时，
/// confidence 块 level 保持 full（spec：只有 PARTIAL_HYDRATE 降 level），
/// 但 statement 不得再声称「没有降低置信度的诊断」。
#[test]
fn confidence_statement_stays_honest_with_partial_impact_diagnostics() -> anyhow::Result<()> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let project_dir = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-refix-confidence-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&project_dir);
    std::fs::create_dir_all(&project_dir)?;
    // 一个可查询的干净页面 + 一个带未识别容器键的页面（给每次响应挂上 SCANNER_*）
    std::fs::write(
        project_dir.join("clean.spg"),
        serde_json::to_string(&serde_json::json!({
            "canvas": {"components": [{"id": "button1", "type": "button", "value": "go"}]}
        }))?,
    )?;
    std::fs::write(
        project_dir.join("bad.spg"),
        serde_json::to_string(&spg_with_unrecognized_key())?,
    )?;
    let db_path = project_dir.join("graph.db");

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_metadata-checker"));
    let status = Command::new(&bin)
        .args([
            "--project-dir",
            project_dir.to_str().expect("utf8"),
            "--graph-db-path",
            db_path.to_str().expect("utf8"),
            "--build-graph",
        ])
        .output()
        .expect("build graph");
    assert!(
        status.status.success(),
        "graphdb 构建失败: {}",
        String::from_utf8_lossy(&status.stderr)
    );

    let output = Command::new(&bin)
        .args([
            "--non-human",
            "--project-dir",
            project_dir.to_str().expect("utf8"),
            "--graph-db-path",
            db_path.to_str().expect("utf8"),
            "--budget",
            "compact",
            "--relations",
            "page:clean",
        ])
        .output()
        .expect("run cli");
    let stdout = String::from_utf8(output.stdout)?;
    let response: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "输出不是 JSON: {error}\nstdout: {stdout}\nstderr: {}",
            String::from_utf8_lossy(&output.stderr)
        )
    });

    // 前提：响应确实携带 answer_impact=partial 的 SCANNER_* 诊断
    let diags = response
        .get("diagnostics")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let scanner_hit = diags
        .iter()
        .find(|d| {
            d.get("code").and_then(Value::as_str) == Some("SCANNER_UNRECOGNIZED_CONTAINER_KEY")
        })
        .unwrap_or_else(|| panic!("响应必须携带 SCANNER_* 诊断: {diags:?}"));
    assert_eq!(
        scanner_hit.get("answer_impact").and_then(Value::as_str),
        Some("partial"),
        "{scanner_hit}"
    );

    // confidence：level 不降级（SCANNER_* 属基线登记计数，spec 只让 PARTIAL_HYDRATE
    // 降 level），但 statement 不得声称「没有降低置信度的诊断」
    let confidence = response
        .get("summary")
        .and_then(|s| s.get("confidence"))
        .cloned()
        .unwrap_or_else(|| panic!("summary.confidence 必须存在: {response}"));
    assert_eq!(
        confidence.get("level").and_then(Value::as_str),
        Some("full"),
        "SCANNER_* 不降 confidence level: {confidence}"
    );
    let statement = confidence
        .get("statement")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        !statement.contains("没有降低置信度的诊断"),
        "携带 answer_impact=partial 诊断时 statement 不得说谎: {statement}"
    );
    assert!(
        statement.contains("SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        "statement 须点名相关 code 便于回查: {statement}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    cleanup_db(&db_path);
    Ok(())
}
