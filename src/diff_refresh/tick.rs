//! M55/57：tick 轮询与退避运行器。
//!
//! 对 `DiffRefreshOrchestrator::refresh_once` 做可复用的重试封装，保证：
//! - 可重试错误按 `BackoffSchedule` 退避；
//! - 明确错误分类（鉴权、数据错误、未分类直接失败）；
//! - 统计字段稳定可序列化，可被 AI-facing 合同消费。

use anyhow::Result;

use super::bi_meta_files_source::BackoffSchedule;
use super::orchestrator::DiffRefreshReport;

/// 稳定机器契约：Tick 报告版本。
pub const DIFF_REFRESH_TICK_SCHEMA_VERSION: &str = "1.0";
/// 稳定机器契约：Tick 报告类型名。
pub const DIFF_REFRESH_TICK_KIND: &str = "DiffRefreshTick";

/// 一次 tick loop 的可回放统计结果。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct DiffRefreshTickReport {
    /// 稳定机器契约版本号。
    pub schema_version: String,
    /// 稳定机器契约类型名。
    pub kind: String,
    /// 已尝试 tick 数（包括失败和成功）。
    pub attempted_ticks: usize,
    /// 成功 tick 数。
    pub successful_ticks: usize,
    /// 可重试失败次数（仅记录 `record_failure` 的场景）。
    pub retryable_failures: usize,
    /// 每次退避实际等待秒数（只在「仍有下一次尝试」时记录）。
    pub backoff_delays_secs: Vec<u64>,
    /// 成功 tick 的完整报告；用于后续 SLM 评测复用。
    pub reports: Vec<DiffRefreshReport>,
    /// 达到 `max_attempts` 仍未成功。
    pub exhausted: bool,
}

/// 以注入式 hook 运行 tick loop，便于 TDD 注入刷新行为与 sleep。
///
/// `refresh_fn` 返回 `DiffRefreshReport` 的单次刷新结果；
/// `sleep_fn` 按秒进行退避等待。
pub fn run_tick_loop_with_hooks<RefreshFn, SleepFn>(
    max_attempts: usize,
    mut refresh_fn: RefreshFn,
    mut sleep_fn: SleepFn,
) -> Result<DiffRefreshTickReport>
where
    RefreshFn: FnMut() -> Result<DiffRefreshReport>,
    SleepFn: FnMut(u64),
{
    let mut report = DiffRefreshTickReport {
        schema_version: DIFF_REFRESH_TICK_SCHEMA_VERSION.to_string(),
        kind: DIFF_REFRESH_TICK_KIND.to_string(),
        attempted_ticks: 0,
        successful_ticks: 0,
        retryable_failures: 0,
        backoff_delays_secs: Vec::new(),
        reports: Vec::new(),
        exhausted: false,
    };

    let mut backoff = BackoffSchedule::new();

    for attempt_index in 0..max_attempts {
        report.attempted_ticks = report.attempted_ticks.saturating_add(1);
        match refresh_fn() {
            Ok(mutation) => {
                backoff.record_success();
                report.successful_ticks = report.successful_ticks.saturating_add(1);
                report.reports.push(mutation);
                report.exhausted = false;
                return Ok(report);
            }
            Err(error) => {
                if !is_retryable_error(&error) {
                    return Err(error);
                }

                report.retryable_failures = report.retryable_failures.saturating_add(1);
                if attempt_index + 1 < max_attempts {
                    let delay_secs = backoff.record_failure();
                    report.backoff_delays_secs.push(delay_secs);
                    sleep_fn(delay_secs);
                } else {
                    report.exhausted = true;
                }
            }
        }
    }

    Ok(report)
}

/// 识别网络/传输类临时错误；其余错误立即返回。
fn is_retryable_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    let normalized = message.to_lowercase();

    if is_non_retryable_error(&normalized) {
        return false;
    }

    if normalized.contains("network")
        || normalized.contains("connection")
        || normalized.contains("timeout")
        || normalized.contains("http request failed")
    {
        return true;
    }

    has_http_5xx_status(&normalized)
}

/// 不重试错误：鉴权失败、数据错误、DIFF_REFRESH_*。
fn is_non_retryable_error(message: &str) -> bool {
    if message.contains("401")
        || message.contains("403")
        || message.contains("unauthorized")
        || message.contains("forbidden")
        || message.contains("invalid_")
        || message.contains("diff_refresh_")
    {
        return true;
    }
    false
}

/// 识别 HTTP 5xx 的可重试状态表达。
fn has_http_5xx_status(message: &str) -> bool {
    if !message.contains("http") {
        return false;
    }

    for token in
        message.split(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.' | '_'))
    {
        if token.len() == 3 {
            if let Ok(code) = token.parse::<u16>() {
                if (500..600).contains(&code) {
                    return true;
                }
            }
        }
    }
    false
}
