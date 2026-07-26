use crate::graph::NodeType;
use crate::graph_store::{GraphReadStore, GraphStoreResult};
use anyhow::Result;
use std::collections::{HashMap, HashSet};

use super::graph_collect;

/// PageDependencyIndex 的覆盖度标记。
///
/// `Full`：索引按 page logic 收集范围完整构建；
/// `Partial`：构建失败降级为空索引，覆盖无法证明，
/// 调用方必须保守扩大 invalidation 并输出该标记。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageDependencyIndexCoverage {
    Full,
    Partial,
}

/// 从 node_id 反查其归属/引用它的所有 page_id。
///
/// 覆盖范围与 `graph_collect::collect_page_logic_nodes` 一致：页面自身、
/// 递归 Contains 子组件、组件 Triggers 的动作节点，以及 M55 起的数据面
/// 闭包——这些节点沿出边可达的 Model/Field（含共享物理模型、
/// DataFlow 模型与其输出字段）。
#[derive(Clone)]
pub struct PageDependencyIndex {
    node_to_pages: HashMap<String, HashSet<String>>,
    coverage: PageDependencyIndexCoverage,
}

impl PageDependencyIndex {
    /// 构造一个空的反向依赖索引（降级路径使用，覆盖度为 Partial）。
    pub fn empty() -> Self {
        Self {
            node_to_pages: HashMap::new(),
            coverage: PageDependencyIndexCoverage::Partial,
        }
    }

    /// 从图数据库构建反向依赖索引（成功时覆盖度为 Full）。
    pub fn build(graph: &dyn GraphReadStore) -> Result<Self> {
        let mut node_to_pages: HashMap<String, HashSet<String>> = HashMap::new();
        for node in graph.iter_nodes()? {
            if !matches!(node.node_type, NodeType::Page) {
                continue;
            }
            let page_id = node.id;
            register_node_page(&mut node_to_pages, &page_id, &page_id);
            let collected =
                graph_collect::collect_page_logic_nodes(graph, &page_id).map_err(|e| {
                    crate::graph_store::GraphStoreError::ReadFailed {
                        reason: e.to_string(),
                    }
                })?;
            for component in collected.child_components {
                register_node_page(&mut node_to_pages, &component.id, &page_id);
            }
            for action in collected.child_actions {
                register_node_page(&mut node_to_pages, &action.id, &page_id);
            }
            for model in collected.related_models {
                register_node_page(&mut node_to_pages, &model.id, &page_id);
            }
            for field in collected.related_fields {
                register_node_page(&mut node_to_pages, &field.id, &page_id);
            }
        }
        Ok(Self {
            node_to_pages,
            coverage: PageDependencyIndexCoverage::Full,
        })
    }

    /// 索引覆盖度标记（Full/Partial）。
    pub fn coverage(&self) -> PageDependencyIndexCoverage {
        self.coverage
    }

    /// 根据 dirty node 列表返回需要失效 warm cache 的 page_id 集合。
    pub fn affected_pages(&self, dirty_node_ids: &[String]) -> HashSet<String> {
        let mut pages = HashSet::new();
        for node_id in dirty_node_ids {
            if let Some(page_set) = self.node_to_pages.get(node_id) {
                pages.extend(page_set.iter().cloned());
            }
        }
        pages
    }

    /// 查询单个 node 关联的 page 集合（测试与交叉验证用）。
    #[cfg(feature = "cli-local")]
    pub fn pages_for_node(&self, node_id: &str) -> HashSet<String> {
        self.node_to_pages.get(node_id).cloned().unwrap_or_default()
    }

    /// 返回 node -> pages 映射条目数（profile 观测用）。
    #[cfg(feature = "cli-local")]
    pub fn indexed_node_count(&self) -> usize {
        self.node_to_pages.len()
    }

    /// 基于脏节点列表，尝试局部重建受影响页面的反向依赖映射。
    ///
    /// 调用方负责先确认新旧图的 node set 稳定。本方法允许未被旧索引登记的
    /// dirty 节点成为 no-op；只要它属于受影响页面，页面节点本身会触发该页重建。
    /// 删除节点仍返回 `Ok(None)`，交给 runtime full fallback。
    pub fn try_update_incremental(
        &self,
        graph: &dyn GraphReadStore,
        dirty_node_ids: &[String],
    ) -> GraphStoreResult<Option<Self>> {
        if self.coverage != PageDependencyIndexCoverage::Full {
            return Ok(None);
        }

        if dirty_node_ids.is_empty() {
            return Ok(Some(self.clone()));
        }

        let mut dirty_nodes = HashSet::new();
        let mut affected_pages: HashSet<String> = HashSet::new();

        for node_id in dirty_node_ids {
            if !dirty_nodes.insert(node_id.as_str()) {
                continue;
            }

            if graph.get_node(node_id)?.is_none() {
                return Ok(None);
            }

            if let Some(pages) = self.node_to_pages.get(node_id.as_str()) {
                affected_pages.extend(pages.iter().cloned());
            }
        }

        if affected_pages.is_empty() {
            return Ok(Some(self.clone()));
        }

        let mut next_node_to_pages = self.node_to_pages.clone();
        for pages in next_node_to_pages.values_mut() {
            pages.retain(|page_id| !affected_pages.contains(page_id));
        }
        next_node_to_pages.retain(|_, pages| !pages.is_empty());

        for page_id in affected_pages {
            let Some(page_node) = graph.get_node(&page_id)? else {
                return Ok(None);
            };

            if !matches!(page_node.node_type, NodeType::Page) {
                return Ok(None);
            }

            let collected =
                graph_collect::collect_page_logic_nodes(graph, &page_id).map_err(|e| {
                    crate::graph_store::GraphStoreError::ReadFailed {
                        reason: e.to_string(),
                    }
                })?;

            register_node_page(&mut next_node_to_pages, &page_id, &page_id);
            for component in collected.child_components {
                register_node_page(&mut next_node_to_pages, &component.id, &page_id);
            }
            for action in collected.child_actions {
                register_node_page(&mut next_node_to_pages, &action.id, &page_id);
            }
            for model in collected.related_models {
                register_node_page(&mut next_node_to_pages, &model.id, &page_id);
            }
            for field in collected.related_fields {
                register_node_page(&mut next_node_to_pages, &field.id, &page_id);
            }
        }

        Ok(Some(Self {
            node_to_pages: next_node_to_pages,
            coverage: self.coverage,
        }))
    }
}

fn register_node_page(
    node_to_pages: &mut HashMap<String, HashSet<String>>,
    node_id: &str,
    page_id: &str,
) {
    node_to_pages
        .entry(node_id.to_string())
        .or_default()
        .insert(page_id.to_string());
}
