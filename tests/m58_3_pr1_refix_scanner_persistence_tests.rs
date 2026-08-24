#![cfg(feature = "cli-local")]

//! M58.3 PR1 refix（F2）回归测试：scanner 诊断（SCANNER_UNRECOGNIZED_CONTAINER_KEY /
//! SCANNER_DUPLICATE_COMPONENT_ID）持久化到 redb。
//!
//! 覆盖：
//! - 首次构建 report.diagnostics 有计数；
//! - 立即二次 no-op 构建诊断不消失（回归核心）；
//! - `GraphRuntime::load` 的 load_diagnostics 与 status().load_diagnostics 含 SCANNER_* code；
//! - 修复文件后其计数消失、另一文件保留；
//! - 删除文件后其计数消失。

use metadata_checker::output::Diagnostic;
use metadata_checker::runtime::GraphRuntime;
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CODE_UNRECOGNIZED: &str = "SCANNER_UNRECOGNIZED_CONTAINER_KEY";
const CODE_DUPLICATE: &str = "SCANNER_DUPLICATE_COMPONENT_ID";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-scanner-persist-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

/// 含未识别容器键的页面：混合形态数组判非组件，计 1 次 SCANNER_UNRECOGNIZED_CONTAINER_KEY
fn spg_with_unrecognized_key() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "panel_a", "type": "panel", "myContainer": [{"id": "child1", "type": "button"}, {"label": "no-id"}]}
            ]
        }
    })
}

/// 修复后的页面：合法的组件数组，不产生 scanner 诊断
fn spg_fixed() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "panel_a", "type": "panel", "children": [{"id": "child1", "type": "button"}]}
            ]
        }
    })
}

/// 含重复组件 id 的页面：两个 dup1，计 1 次 SCANNER_DUPLICATE_COMPONENT_ID
fn spg_with_duplicate_id() -> serde_json::Value {
    serde_json::json!({
        "canvas": {
            "components": [
                {"id": "dup1", "type": "button"},
                {"id": "dup1", "type": "input"}
            ]
        }
    })
}

fn write_spg(path: &Path, value: &serde_json::Value) -> anyhow::Result<()> {
    std::fs::write(path, serde_json::to_string(value)?)?;
    Ok(())
}

/// 按 code 过滤诊断
fn code_hits<'a>(diags: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.code == code).collect()
}

/// 搭建两文件项目（a.spg 未识别容器键、b.spg 重复 id）并完成首次构建，
/// 返回 (project_dir, db_path, 首次构建诊断)。
fn build_two_file_project(tag: &str) -> anyhow::Result<(PathBuf, PathBuf, Vec<Diagnostic>)> {
    let project_dir = unique_temp_dir(tag);
    std::fs::create_dir_all(&project_dir)?;
    write_spg(&project_dir.join("a.spg"), &spg_with_unrecognized_key())?;
    write_spg(&project_dir.join("b.spg"), &spg_with_duplicate_id())?;
    let db_path = project_dir.join("graph.db");

    let first = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    Ok((project_dir, db_path, first.diagnostics))
}

/// 首次构建 + 立即二次 no-op 构建：SCANNER_* 计数必须保持一致（回归核心——
/// 旧实现 no-op 路径返回空 Vec，首次构建的警告第二次即消失）。
#[test]
fn scanner_diagnostics_survive_noop_rebuild() -> anyhow::Result<()> {
    let (project_dir, db_path, first_diags) = build_two_file_project("noop")?;

    let first_unrec = code_hits(&first_diags, CODE_UNRECOGNIZED);
    assert_eq!(first_unrec.len(), 1, "{first_diags:?}");
    assert_eq!(first_unrec[0].count, Some(1), "{first_diags:?}");
    let first_dup = code_hits(&first_diags, CODE_DUPLICATE);
    assert_eq!(first_dup.len(), 1, "{first_diags:?}");
    assert_eq!(first_dup[0].count, Some(1), "{first_diags:?}");

    // 立即二次 no-op 构建（不改任何文件）
    let second = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert_eq!(
        second.report.dirty, 0,
        "无变更应为 no-op: {:?}",
        second.report
    );
    let second_unrec = code_hits(&second.diagnostics, CODE_UNRECOGNIZED);
    assert_eq!(
        second_unrec.len(),
        1,
        "no-op 构建后未识别容器键诊断不得消失: {:?}",
        second.diagnostics
    );
    assert_eq!(second_unrec[0].count, Some(1), "{:?}", second.diagnostics);
    let second_dup = code_hits(&second.diagnostics, CODE_DUPLICATE);
    assert_eq!(
        second_dup.len(),
        1,
        "no-op 构建后重复组件 id 诊断不得消失: {:?}",
        second.diagnostics
    );
    assert_eq!(second_dup[0].count, Some(1), "{:?}", second.diagnostics);

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// `GraphRuntime::load` 后 load_diagnostics 与 status().load_diagnostics
/// 都必须含 SCANNER_* code（落点：--status / 查询响应共用同一 load_diagnostics）。
#[test]
fn runtime_load_diagnostics_include_scanner_codes() -> anyhow::Result<()> {
    let (project_dir, db_path, _) = build_two_file_project("runtime")?;

    let runtime = GraphRuntime::load(&db_path)?;
    assert_eq!(
        code_hits(&runtime.load_diagnostics, CODE_UNRECOGNIZED).len(),
        1,
        "{:?}",
        runtime.load_diagnostics
    );
    assert_eq!(
        code_hits(&runtime.load_diagnostics, CODE_DUPLICATE).len(),
        1,
        "{:?}",
        runtime.load_diagnostics
    );

    let status = runtime.status();
    assert_eq!(
        code_hits(&status.load_diagnostics, CODE_UNRECOGNIZED).len(),
        1,
        "{:?}",
        status.load_diagnostics
    );
    assert_eq!(
        code_hits(&status.load_diagnostics, CODE_DUPLICATE).len(),
        1,
        "{:?}",
        status.load_diagnostics
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 修复一个文件（去掉未识别容器键）再三扫：该文件计数消失（entry 被零计数
/// 覆盖），另一文件的重复 id 计数保留。
#[test]
fn scanner_diagnostics_entry_evicted_after_fix() -> anyhow::Result<()> {
    let (project_dir, db_path, _) = build_two_file_project("fix")?;

    write_spg(&project_dir.join("a.spg"), &spg_fixed())?;
    let third = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert_eq!(third.report.dirty, 1, "只有 a.spg 脏: {:?}", third.report);
    assert_eq!(
        code_hits(&third.diagnostics, CODE_UNRECOGNIZED).len(),
        0,
        "修复后未识别容器键计数必须消失: {:?}",
        third.diagnostics
    );
    let dup = code_hits(&third.diagnostics, CODE_DUPLICATE);
    assert_eq!(
        dup.len(),
        1,
        "未变更文件的重复 id 计数必须保留: {:?}",
        third.diagnostics
    );
    assert_eq!(dup[0].count, Some(1), "{:?}", third.diagnostics);

    // runtime 加载口径与构建报告一致
    let runtime = GraphRuntime::load(&db_path)?;
    assert_eq!(
        code_hits(&runtime.load_diagnostics, CODE_UNRECOGNIZED).len(),
        0,
        "{:?}",
        runtime.load_diagnostics
    );
    assert_eq!(
        code_hits(&runtime.load_diagnostics, CODE_DUPLICATE).len(),
        1,
        "{:?}",
        runtime.load_diagnostics
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 删除文件后再扫：其 entry 被移除，对应计数消失；另一文件计数保留。
#[test]
fn scanner_diagnostics_entry_removed_after_delete() -> anyhow::Result<()> {
    let (project_dir, db_path, _) = build_two_file_project("delete")?;

    std::fs::remove_file(project_dir.join("b.spg"))?;
    let report = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert_eq!(report.report.deleted, 1, "{:?}", report.report);
    assert_eq!(
        code_hits(&report.diagnostics, CODE_DUPLICATE).len(),
        0,
        "删除 b.spg 后重复 id 计数必须消失: {:?}",
        report.diagnostics
    );
    let unrec = code_hits(&report.diagnostics, CODE_UNRECOGNIZED);
    assert_eq!(
        unrec.len(),
        1,
        "未变更文件的未识别容器键计数必须保留: {:?}",
        report.diagnostics
    );
    assert_eq!(unrec[0].count, Some(1), "{:?}", report.diagnostics);

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}

/// 守卫测试（验收缺口补录）：`GraphRuntime::load` 不得触碰 db 文件指纹。
///
/// redb 连接 drop 时会写 clean-close 标记；加载期若有 db I/O 排在指纹采集
/// 之后，`reload_if_changed` 会误判文件已变更（regression_tests 的
/// check-reload 契约曾因此变红）。此处钉住行为：load 后立即检查必须为
/// Unchanged，且 load 前后 mtime/size 不变。
#[test]
fn runtime_load_does_not_touch_db_fingerprint() -> anyhow::Result<()> {
    let (project_dir, db_path, _) = build_two_file_project("fingerprint")?;

    let before = std::fs::metadata(&db_path)?;
    let mut runtime = GraphRuntime::load(&db_path)?;
    let after = std::fs::metadata(&db_path)?;
    assert_eq!(
        before.modified().ok(),
        after.modified().ok(),
        "load 不得改变 db 文件 mtime"
    );
    assert_eq!(before.len(), after.len(), "load 不得改变 db 文件大小");
    assert!(
        matches!(
            runtime.reload_if_changed()?,
            metadata_checker::runtime::ReloadResult::Unchanged
        ),
        "load 后立即 check-reload 必须判定未变更"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
    Ok(())
}
