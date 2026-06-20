use criterion::Criterion;
use std::time::Duration;

/// 真实项目 bench 的 Criterion 采样配置：较长测量窗口，较低样本数。
pub fn real_project_criterion_config() -> Criterion {
    Criterion::default()
        .sample_size(10)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(10))
}
