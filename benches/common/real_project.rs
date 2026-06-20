use std::path::PathBuf;

/// 真实项目 benchmark 默认目录；可通过环境变量覆盖。
pub const DEFAULT_REAL_PROJECT_DIR: &str =
    "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi";

/// 真实项目 bench 缺失时的统一跳过提示。
pub fn require_real_project_dir(bench_name: &str) -> Option<PathBuf> {
    let configured = std::env::var("METADATA_CHECKER_REAL_PROJECT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_REAL_PROJECT_DIR));
    if configured.exists() {
        Some(configured)
    } else {
        eprintln!(
            "skip {bench_name}: set METADATA_CHECKER_REAL_PROJECT_DIR or create {}",
            DEFAULT_REAL_PROJECT_DIR
        );
        None
    }
}
