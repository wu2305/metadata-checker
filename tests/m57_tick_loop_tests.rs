#![cfg(feature = "cli-local")]
//! M57-1：tick loop 与 Backoff 的 TDD 契约。
//!
//! 先固定网络错误可恢复、数据错误不可静默重试、尝试上限和零尝试边界，
//! 再实现实际的 orchestrator 驱动。

use std::collections::VecDeque;

use anyhow::anyhow;
use metadata_checker::diff_refresh::{
    DiffRefreshReport, DiffRefreshTickReport, DiffRefreshTiming, run_tick_loop_with_hooks,
};
use metadata_checker::query::PageDependencyIndexCoverage;

fn successful_report() -> DiffRefreshReport {
    DiffRefreshReport {
        schema_version: "1.0".to_string(),
        kind: "DiffRefresh".to_string(),
        change_count: 0,
        invalidated_pages: Vec::new(),
        persist_report: None,
        warm_failures: Vec::new(),
        checkpoint: None,
        last_poll_at: 1,
        page_dep_index_coverage: PageDependencyIndexCoverage::Partial,
        timing: DiffRefreshTiming::default(),
    }
}

/// 网络错误应按 1 秒退避后继续，并在下一次成功时清零退避状态。
#[test]
fn m57_tick_loop_retries_network_failure_with_backoff() {
    let mut outcomes = VecDeque::from([
        Err(anyhow!("network unavailable")),
        Ok(successful_report()),
    ]);
    let mut delays = Vec::new();

    let report = run_tick_loop_with_hooks(
        2,
        || outcomes.pop_front().expect("test outcome"),
        |delay_secs| delays.push(delay_secs),
    )
    .expect("network failure should recover on the next tick");

    assert_eq!(delays, vec![1]);
    assert_eq!(report.attempted_ticks, 2);
    assert_eq!(report.successful_ticks, 1);
    assert_eq!(report.retryable_failures, 1);
    assert_eq!(report.reports.len(), 1);
    assert_eq!(report.exhausted, false);
}

/// 数据格式错误不得被退避循环吞掉或重试。
#[test]
fn m57_tick_loop_does_not_retry_data_error() {
    let mut delays = Vec::new();
    let result: anyhow::Result<DiffRefreshTickReport> = run_tick_loop_with_hooks(
        3,
        || Err(anyhow!("INVALID_ACTIVE_CHANGE_EVENT: missing revision")),
        |delay_secs| delays.push(delay_secs),
    );

    let error = result.expect_err("data error must stop the tick loop");
    assert!(error.to_string().contains("INVALID_ACTIVE_CHANGE_EVENT"));
    assert_eq!(delays, Vec::<u64>::new());
}

/// 尝试次数为零时不得调用刷新或 sleep，返回空统计。
#[test]
fn m57_tick_loop_zero_attempts_is_empty() {
    let mut refresh_calls = 0;
    let mut sleep_calls = 0;
    let report = run_tick_loop_with_hooks(
        0,
        || {
            refresh_calls += 1;
            Ok(successful_report())
        },
        |_| sleep_calls += 1,
    )
    .expect("zero attempts should be a valid empty run");

    assert_eq!(refresh_calls, 0);
    assert_eq!(sleep_calls, 0);
    assert_eq!(report.attempted_ticks, 0);
    assert_eq!(report.successful_ticks, 0);
    assert_eq!(report.retryable_failures, 0);
    assert_eq!(report.exhausted, false);
}
