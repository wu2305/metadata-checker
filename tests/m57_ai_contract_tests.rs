#![cfg(feature = "cli-local")]
//! M57-0：为后续 Small Language Model 测评固定 diff-refresh 机器契约。
//!
//! 这些测试先于实现提交，用来锁定报告版本、报告类型与 persist 成本报告
//! 必须同时出现在 one-shot/stdio 可消费结果中的边界。

use metadata_checker::diff_refresh::{DiffRefreshReport, DiffRefreshTiming};
use metadata_checker::graph_store::{PersistReport, V2ShadowState};
use metadata_checker::query::PageDependencyIndexCoverage;

/// 报告必须携带稳定版本、类型和真实 PersistReport，不能只靠 change_count 判断刷新是否落盘。
#[test]
fn m57_diff_refresh_report_has_machine_contract_fields() {
    let report = DiffRefreshReport {
        schema_version: "1.0".to_string(),
        kind: "DiffRefresh".to_string(),
        change_count: 1,
        invalidated_pages: vec!["page:app/page.spg".to_string()],
        warm_failures: Vec::new(),
        checkpoint: None,
        last_poll_at: 1000,
        page_dep_index_coverage: PageDependencyIndexCoverage::Full,
        persist_report: Some(PersistReport {
            dirty_nodes: 1,
            dirty_edges: 2,
            changed_file_states: 1,
            bytes_written: 128,
            commit_ms: 3,
            full_rewrite: false,
            v2_shadow_state: V2ShadowState::Stale,
        }),
        timing: DiffRefreshTiming::default(),
    };

    let value = serde_json::to_value(report).expect("serialize diff refresh report");
    assert_eq!(value["schema_version"].as_str(), Some("1.0"));
    assert_eq!(value["kind"].as_str(), Some("DiffRefresh"));
    assert_eq!(value["persist_report"]["dirty_nodes"].as_u64(), Some(1));
    assert_eq!(
        value["persist_report"]["full_rewrite"].as_bool(),
        Some(false)
    );
}
