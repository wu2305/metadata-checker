#![cfg(feature = "cli-local")]
//! M57-2：RefreshScope 声明契约的 TDD。
//!
//! 重点不是重新实现旧筛选器，而是保证机器报告不会把排序提示或默认全项目
//! 路径误报成已经应用的局部范围。

use metadata_checker::session::{
    RefreshScopeKind, RefreshScopeResolution, SessionRefreshFilter, resolve_refresh_scope,
};

/// 显式 module 筛选必须声明为已应用的 module scope。
#[test]
fn m57_scope_declares_explicit_module_filter() {
    let filter = SessionRefreshFilter {
        module: Some("app/sales".to_string()),
        source_path: None,
        file_id: None,
        current_source_path: None,
    };

    let scope = resolve_refresh_scope(Some(&filter));

    assert_eq!(scope.kind, RefreshScopeKind::Module);
    assert_eq!(scope.value.as_deref(), Some("app/sales"));
    assert_eq!(scope.resolution, RefreshScopeResolution::Explicit);
    assert_eq!(scope.applied, true);
    assert_eq!(scope.selectors, vec!["module:app/sales"]);
    assert_eq!(scope.fallback_reason, None);
}

/// 多个显式条件必须声明 compound，不能只展示其中一个条件。
#[test]
fn m57_scope_declares_compound_explicit_filters() {
    let filter = SessionRefreshFilter {
        module: Some("app".to_string()),
        source_path: Some("app/sales/page.spg".to_string()),
        file_id: Some("file-1".to_string()),
        current_source_path: None,
    };

    let scope = resolve_refresh_scope(Some(&filter));

    assert_eq!(scope.kind, RefreshScopeKind::Compound);
    assert_eq!(scope.resolution, RefreshScopeResolution::Explicit);
    assert_eq!(scope.applied, true);
    assert_eq!(
        scope.selectors,
        vec![
            "module:app".to_string(),
            "source_path:app/sales/page.spg".to_string(),
            "file_id:file-1".to_string()
        ]
    );
}

/// 当前页字段目前只是排序提示，必须声明 auto 但未应用，避免误导模型。
#[test]
fn m57_scope_does_not_misreport_current_page_hint_as_filter() {
    let filter = SessionRefreshFilter {
        module: None,
        source_path: None,
        file_id: None,
        current_source_path: Some("app/sales/page.spg".to_string()),
    };

    let scope = resolve_refresh_scope(Some(&filter));

    assert_eq!(scope.kind, RefreshScopeKind::SourcePath);
    assert_eq!(scope.resolution, RefreshScopeResolution::Auto);
    assert_eq!(scope.applied, false);
    assert_eq!(scope.fallback_reason.as_deref(), Some("current_source_path is an ordering hint"));
}

/// 没有显式过滤或当前页提示时必须明确回落到 project。
#[test]
fn m57_scope_declares_project_fallback() {
    let scope = resolve_refresh_scope(None);

    assert_eq!(scope.kind, RefreshScopeKind::Project);
    assert_eq!(scope.resolution, RefreshScopeResolution::Fallback);
    assert_eq!(scope.applied, false);
    assert!(scope.fallback_reason.is_some());
    assert_eq!(scope.selectors, Vec::<String>::new());
}
