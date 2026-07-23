//! 进程 RSS 采样（bench 观测用，不设 CI 阈值）。

/// 当前进程常驻集大小（KB）。不可用时返回 None。
pub fn current_rss_kb() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        macos_rss_kb()
    }
    #[cfg(target_os = "linux")]
    {
        linux_rss_kb()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// 打印阶段 RSS，便于 `make perf-*` / 手工 criterion 日志对照。
pub fn log_rss(stage: &str) {
    if let Some(kb) = current_rss_kb() {
        eprintln!("[rss] {stage}: {kb} KB");
    }
}

#[cfg(target_os = "macos")]
fn macos_rss_kb() -> Option<u64> {
    // ponytail: 用 `ps` 读 RSS，避免拉 libc/mach 依赖；上限=子进程开销；升级=mach_task_basic_info。
    let pid = std::process::id();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim().parse::<u64>().ok()
}

#[cfg(target_os = "linux")]
fn linux_rss_kb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
            return Some(kb);
        }
    }
    None
}
