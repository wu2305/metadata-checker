//! M53：load-time 物化 availability condition facts，供 page logic 批量复用。

use crate::explain::ConditionFact;
use crate::graph_store::{GraphReadStore, GraphStoreResult};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
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

    /// 基于脏节点列表尝试做条件事实的局部增量更新。
    ///
    /// 调用方负责先确认新旧图的 node set 稳定；本方法只重算 dirty 节点的
    /// condition facts。没有旧 facts 的 dirty 节点也必须参与重算，因为它可能
    /// 在本轮首次获得 condition；删除节点则返回 `Ok(None)` 交给 full fallback。
    pub fn try_update_incremental(
        &self,
        graph: &dyn GraphReadStore,
        dirty_node_ids: &[String],
    ) -> GraphStoreResult<Option<Self>> {
        if dirty_node_ids.is_empty() {
            return Ok(Some(self.clone()));
        }

        let mut dirty_nodes = HashSet::new();
        let mut next_conditions: HashMap<String, Vec<ConditionFact>> =
            self.conditions_by_node.as_ref().clone();

        for node_id in dirty_node_ids {
            if !dirty_nodes.insert(node_id.as_str()) {
                continue;
            }

            if graph.get_node(node_id)?.is_none() {
                return Ok(None);
            }

            let facts = crate::explain::collect_condition_facts_for_node(
                graph,
                node_id,
                &mut HashSet::new(),
            );

            if facts.is_empty() {
                next_conditions.remove(node_id);
            } else {
                next_conditions.insert(node_id.to_string(), facts);
            }
        }

        let node_count = next_conditions.len();
        let condition_count = next_conditions.values().map(|items| items.len()).sum();
        Ok(Some(Self {
            conditions_by_node: Arc::new(next_conditions),
            node_count,
            condition_count,
        }))
    }
}
