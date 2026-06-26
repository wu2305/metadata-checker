//! 轻量性能归因数据结构。
//!
//! M51 只把这些结构用于 benchmark / profiling runner，不写入默认产品输出。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// 单个阶段的耗时记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PerfStage {
    /// 阶段名，使用稳定 snake_case。
    pub name: String,
    /// 阶段 wall-clock 耗时毫秒。
    pub duration_ms: u128,
}

/// 单次能力调用的性能归因快照。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfProfile {
    /// 产品能力名，例如 query_page_logic。
    pub capability: String,
    /// 整次能力调用 wall-clock 耗时毫秒。
    pub wall_duration_ms: u128,
    /// 阶段耗时列表，按记录顺序保存。
    pub stages: Vec<PerfStage>,
    /// 与性能解释相关的计数维度。
    pub counters: BTreeMap<String, u64>,
    #[serde(skip)]
    started_at: Option<Instant>,
}

impl PerfProfile {
    /// 创建一个能力 profile。
    pub fn new(capability: impl Into<String>) -> Self {
        Self {
            capability: capability.into(),
            wall_duration_ms: 0,
            stages: Vec::new(),
            counters: BTreeMap::new(),
            started_at: Some(Instant::now()),
        }
    }

    /// 记录一个阶段耗时。
    pub fn record_stage(&mut self, name: impl Into<String>, duration: Duration) {
        self.stages.push(PerfStage {
            name: name.into(),
            duration_ms: duration.as_millis(),
        });
    }

    /// 设置 counter 的最终值。
    pub fn set_counter(&mut self, name: impl Into<String>, value: u64) {
        self.counters.insert(name.into(), value);
    }

    /// 增加 counter。
    pub fn add_counter(&mut self, name: impl Into<String>, value: u64) {
        let entry = self.counters.entry(name.into()).or_insert(0);
        *entry += value;
    }

    /// 结束 profile，写入 wall-clock 总耗时。
    pub fn finish(&mut self) {
        if let Some(started_at) = self.started_at.take() {
            self.wall_duration_ms = started_at.elapsed().as_millis();
        }
    }

    /// 查询指定阶段。
    pub fn stage(&self, name: &str) -> Option<&PerfStage> {
        self.stages.iter().find(|stage| stage.name == name)
    }

    /// 查询 counter，不存在时返回 0。
    pub fn counter(&self, name: &str) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    /// 已记录阶段的总耗时。
    pub fn total_duration_ms(&self) -> u128 {
        self.stages.iter().map(|stage| stage.duration_ms).sum()
    }
}
