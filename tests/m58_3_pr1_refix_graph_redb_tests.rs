#![cfg(feature = "cli-local")]

//! M58.3 PR1 refix 回归测试：
//! - F7+F4：`GraphDB::open` 的 v2 探针顺序与 `Ok(None)` 计数；
//! - F6：`IndexReport` 四字段统一为文件口径；
//! - F8：scanner 聚合诊断的 sample_location 回填 source_file。

use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_redb_v2::RedbV2Meta;
use metadata_checker::graph_store::V2ShadowState;
use metadata_checker::output::Diagnostic;
use metadata_checker::scanner::{scan_project, scan_project_with_report};
use redb::{Database, ReadableTable, TableDefinition};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const V2_META_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("v2_meta");
const V2_BUNDLE_KEY: &str = "bundle";
const V2_SHADOW_STATE_KEY: &str = "shadow_state";
const CODE_V2_LAYOUT_UNREADABLE: &str = "GRAPH_DB_V2_LAYOUT_UNREADABLE";

fn unique_temp_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-refix-{tag}-{}-{nanos}.{ext}",
        std::process::id()
    ))
}

fn cleanup_db(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
}

/// 用既有 fixture 构建一份正常图库并返回路径（首次打开句柄已释放）。
fn build_fixture_db(tag: &str) -> anyhow::Result<PathBuf> {
    let db_path = unique_temp_path(tag, "db");
    let _ = std::fs::remove_file(&db_path);
    scan_project(Path::new("tests/fixtures/test_project"), &db_path)?;
    Ok(db_path)
}

fn v2_layout_diagnostics(diags: &[Diagnostic]) -> Vec<&Diagnostic> {
    diags
        .iter()
        .filter(|d| d.code == CODE_V2_LAYOUT_UNREADABLE)
        .collect()
}

/// 破坏 v2 layout 的 content fingerprint（read_v2_layout 走 Ok(None) 折叠路径）。
fn corrupt_v2_fingerprint(db_path: &Path) -> anyhow::Result<()> {
    let db = Database::create(db_path)?;
    let write_txn = db.begin_write()?;
    {
        let mut meta_table = write_txn.open_table(V2_META_TABLE)?;
        let raw = meta_table
            .get(V2_BUNDLE_KEY)?
            .expect("v2 meta bundle must exist after full build")
            .value();
        let mut meta: RedbV2Meta = serde_json::from_slice(&raw)?;
        meta.content_fingerprint ^= 1;
        meta_table.insert(V2_BUNDLE_KEY, serde_json::to_vec(&meta)?)?;
    }
    write_txn.commit()?;
    Ok(())
}

/// 强制 v2 shadow 状态为 Stale。
fn force_v2_shadow_stale(db_path: &Path) -> anyhow::Result<()> {
    let db = Database::create(db_path)?;
    let write_txn = db.begin_write()?;
    {
        let mut meta_table = write_txn.open_table(V2_META_TABLE)?;
        meta_table.insert(
            V2_SHADOW_STATE_KEY,
            serde_json::to_vec(&V2ShadowState::Stale)?,
        )?;
    }
    write_txn.commit()?;
    Ok(())
}

/// F4 回归：v2 Current 的正常库重开不得产生 GRAPH_DB_V2_LAYOUT_UNREADABLE。
#[test]
fn refix_v2_current_reopen_has_no_layout_diagnostic() -> anyhow::Result<()> {
    let db_path = build_fixture_db("v2-current-ok")?;

    let reopened = GraphDB::open(&db_path)?;
    let diags = reopened.hydrate_diagnostics().to_diagnostics();
    assert_eq!(
        v2_layout_diagnostics(&diags).len(),
        0,
        "正常库重开不得产生 v2_layout 诊断: {diags:?}"
    );

    drop(reopened);
    cleanup_db(&db_path);
    Ok(())
}

/// F4 回归：破坏 v2 fingerprint 后重开（Current 路径），read_v2_layout 的
/// Ok(None) 折叠路径必须恰好产生一条 GRAPH_DB_V2_LAYOUT_UNREADABLE 且 count=1，
/// 同时 v1 fallback 仍成功加载图。
#[test]
fn refix_v2_fingerprint_mismatch_counts_exactly_one_layout_diagnostic() -> anyhow::Result<()> {
    let db_path = build_fixture_db("v2-fingerprint")?;
    corrupt_v2_fingerprint(&db_path)?;

    let reopened = GraphDB::open(&db_path)?;
    let diags = reopened.hydrate_diagnostics().to_diagnostics();
    let hits = v2_layout_diagnostics(&diags);
    assert_eq!(
        hits.len(),
        1,
        "fingerprint 不匹配应恰好一条 v2_layout 诊断: {diags:?}"
    );
    assert_eq!(hits[0].count, Some(1), "{diags:?}");

    drop(reopened);
    cleanup_db(&db_path);
    Ok(())
}

/// F7 回归：shadow=Stale 时 open 必须跳过 v2 探针——即使 v2 layout 已损坏，
/// 也不得新发 GRAPH_DB_V2_LAYOUT_UNREADABLE。
#[test]
fn refix_v2_stale_skips_probe_and_emits_no_layout_diagnostic() -> anyhow::Result<()> {
    let db_path = build_fixture_db("v2-stale-skip")?;
    corrupt_v2_fingerprint(&db_path)?;
    force_v2_shadow_stale(&db_path)?;

    let reopened = GraphDB::open(&db_path)?;
    let diags = reopened.hydrate_diagnostics().to_diagnostics();
    assert_eq!(
        v2_layout_diagnostics(&diags).len(),
        0,
        "Stale 路径不跑探针，不得新发 v2_layout 诊断: {diags:?}"
    );

    drop(reopened);
    cleanup_db(&db_path);
    Ok(())
}

fn write_spg(path: &Path, extra_component_id: Option<&str>) -> anyhow::Result<()> {
    let mut components = vec![
        serde_json::json!({"id": "root_panel", "type": "panel"}),
        serde_json::json!({"id": "btn_a", "type": "button"}),
        serde_json::json!({"id": "input_a", "type": "input"}),
    ];
    if let Some(id) = extra_component_id {
        components.push(serde_json::json!({"id": id, "type": "button"}));
    }
    let value = serde_json::json!({"canvas": {"components": components}});
    std::fs::write(path, serde_json::to_string(&value)?)?;
    Ok(())
}

/// F6 回归：IndexReport 四字段为文件口径——一个脏文件含多个节点时
/// `dirty` 必须是 1（文件数），不是节点数。
#[test]
fn refix_index_report_counts_are_file_scoped() -> anyhow::Result<()> {
    let project_dir = unique_temp_path("report-units", "proj");
    std::fs::create_dir_all(&project_dir)?;
    write_spg(&project_dir.join("a.spg"), None)?;
    write_spg(&project_dir.join("b.spg"), None)?;
    let db_path = project_dir.join("graph.db");

    // 首次全量构建：两个文件都是脏文件（文件口径 dirty=2）
    let initial = scan_project_with_report(&project_dir, &db_path)?;
    assert_eq!(initial.indexed, 2);
    assert_eq!(initial.dirty, 2);
    assert_eq!(initial.deleted, 0);
    assert_eq!(initial.unchanged, 0);

    // 修改一个文件（新增组件 => 该文件产生 4+ 个节点），二次扫描
    write_spg(&project_dir.join("a.spg"), Some("btn_extra"))?;
    let report = scan_project_with_report(&project_dir, &db_path)?;
    assert_eq!(report.indexed, 2, "indexed 为发现的文件总数");
    assert_eq!(
        report.dirty, 1,
        "单个脏文件含多个节点时 dirty 仍应为文件数 1"
    );
    assert_eq!(report.unchanged, 1, "unchanged = indexed - dirty（文件口径）");
    assert_eq!(report.deleted, 0);

    // no-op 路径口径一致：不再改动任何文件，三轮扫描全为 0/不变
    let noop = scan_project_with_report(&project_dir, &db_path)?;
    assert_eq!(noop.indexed, 2);
    assert_eq!(noop.dirty, 0);
    assert_eq!(noop.unchanged, 2);
    assert_eq!(noop.deleted, 0);

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// F8 回归：scanner 聚合诊断的 sample_location.source_file 回填为
/// 贡献样例的文件的相对路径。
#[test]
fn refix_scanner_sample_location_backfills_source_file() -> anyhow::Result<()> {
    let project_dir = unique_temp_path("scanner-src-file", "proj");
    let pages_dir = project_dir.join("pages");
    std::fs::create_dir_all(&pages_dir)?;
    // 同一文件内同时包含未识别容器键与重复组件 id，两类样例都应回填
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                // 混合形态数组：PR2 形态感知递归下判非组件，计入未识别容器键
                {"id": "dup1", "type": "panel", "myContainer": [{"id": "child1", "type": "button"}, {"label": "no-id"}]},
                {"id": "dup1", "type": "input"}
            ]
        }
    });
    std::fs::write(pages_dir.join("bad.spg"), serde_json::to_string(&raw)?)?;
    let db_path = project_dir.join("graph.db");

    let report = scan_project_with_report(&project_dir, &db_path)?;

    let unrecognized: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "SCANNER_UNRECOGNIZED_CONTAINER_KEY")
        .collect();
    assert_eq!(unrecognized.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(
        unrecognized[0].location.source_file,
        Some("pages/bad.spg".to_string()),
        "未识别容器键样例应回填来源文件: {:?}",
        unrecognized[0].location
    );

    let duplicate: Vec<&Diagnostic> = report
        .diagnostics
        .iter()
        .filter(|d| d.code == "SCANNER_DUPLICATE_COMPONENT_ID")
        .collect();
    assert_eq!(duplicate.len(), 1, "{:?}", report.diagnostics);
    assert_eq!(
        duplicate[0].location.source_file,
        Some("pages/bad.spg".to_string()),
        "重复组件 id 样例应回填来源文件: {:?}",
        duplicate[0].location
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}
