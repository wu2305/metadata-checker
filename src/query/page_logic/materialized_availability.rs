//! M53：load-time 物化 availability condition facts，供 page logic 批量复用。

use crate::explain::ConditionFact;
use crate::graph_store::GraphReadStore;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

/// 全图预收集的 typed condition 索引（输出边界再转 JSON）。
#[derive(Clone)]
pub struct MaterializedAvailabilityFactsIndex {
    conditions_by_node: Arc<HashMap<String, Vec<ConditionFact>>>,
    /// 带 condition 的节点数。
    pub node_count: usize,
    /// 预收集 condition 对象总数。
    pub condition_count: usize,
}

impl MaterializedAvailabilityFactsIndex {
    /// 空索引，供 runtime 在 build 失败时降级使用。
    pub fn empty() -> Self {
        Self {
            conditions_by_node: Arc::new(HashMap::new()),
            node_count: 0,
            condition_count: 0,
        }
    }

    /// 在 runtime load 阶段扫描全图，一次性收集各节点 typed condition。
    pub fn build(graph: &dyn GraphReadStore) -> Result<Self> {
        let map = crate::explain::precollect_all_node_conditions(graph)?;
        let node_count = map.len();
        let condition_count = map.values().map(|items| items.len()).sum();
        Ok(Self {
            conditions_by_node: Arc::new(map),
            node_count,
            condition_count,
        })
    }

    /// 为单次 page logic 批量构造种子化 condition 缓存。
    pub(crate) fn seed_condition_cache(&self) -> crate::explain::ConditionCollectorCache {
        crate::explain::ConditionCollectorCache::from_prefilled(Arc::clone(
            &self.conditions_by_node,
        ))
    }
}
