#![cfg(feature = "cli-local")]
//! M57-4：锁定主 diff-refresh 报告的 RefreshScope 机器契约。
//!
//! one-shot、stdio 和 tick 共享 `DiffRefreshReport`，因此 scope 不能只存在
//! session-refresh 的文件报告中；无显式筛选时也必须诚实声明 project fallback。

use metadata_checker::diff_refresh::{DiffRefreshReport, DiffRefreshTiming};
use metadata_checker::query::PageDependencyIndexCoverage;
use metadata_checker::session::{RefreshScope, RefreshScopeKind, RefreshScopeResolution};

/// 主刷新报告必须暴露稳定的 project fallback scope，供 SLM runner 直接消费。
#[test]
fn m57_diff_refresh_report_declares_project_scope() {
    let report = DiffRefreshReport {
        schema_version: "1.0".to_string(),
        kind: "DiffRefresh".to_string(),
        scope: RefreshScope::default(),
        change_count: 0,
        invalidated_pages: Vec::new(),
        persist_report: None,
        warm_failures: Vec::new(),
        persisted: false,
        pending_dirty_total: 0,
        checkpoint: None,
        last_poll_at: 1,
        page_dep_index_coverage: PageDependencyIndexCoverage::Partial,
        timing: DiffRefreshTiming::default(),
    };

    let value = serde_json::to_value(report).expect("serialize diff refresh scope");
    assert_eq!(value["scope"]["kind"].as_str(), Some("project"));
    assert_eq!(value["scope"]["resolution"].as_str(), Some("fallback"));
    assert_eq!(value["scope"]["applied"].as_bool(), Some(false));
    assert_eq!(
        value["scope"]["selectors"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(
        value["scope"]["fallback_reason"].as_str(),
        Some("refresh scope fallback to project scope")
    );
}

/// scope 枚举的 JSON 名称必须与 session-refresh 文件报告一致。
#[test]
fn m57_diff_refresh_scope_enum_contract_is_stable() {
    let scope = RefreshScope {
        kind: RefreshScopeKind::SourcePath,
        value: Some("app/Page.spg".to_string()),
        resolution: RefreshScopeResolution::Auto,
        applied: false,
        selectors: vec!["source_path:app/Page.spg".to_string()],
        fallback_reason: Some("current_source_path is an ordering hint".to_string()),
    };

    let value = serde_json::to_value(scope).expect("serialize refresh scope");
    assert_eq!(value["kind"].as_str(), Some("source_path"));
    assert_eq!(value["resolution"].as_str(), Some("auto"));
}
