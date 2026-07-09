use crate::graph::NodeType;
use crate::graph_store::GraphReadStore;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

use super::graph_collect;

/// 从 node_id 反查其归属/引用它的所有 page_id。
///
/// 覆盖范围与 `graph_collect::collect_page_logic_nodes` 一致：页面自身、
/// 递归 Contains 子组件，以及组件 Triggers 的动作节点。
pub struct PageDependencyIndex {
    node_to_pages: HashMap<String, HashSet<String>>,
}

impl PageDependencyIndex {
    /// 构造一个空的反向依赖索引（降级路径使用）。
    pub fn empty() -> Self {
        Self {
            node_to_pages: HashMap::new(),
        }
    }

    /// 从图数据库构建反向依赖索引。
    pub fn build(graph: &dyn GraphReadStore) -> Result<Self> {
        let mut node_to_pages: HashMap<String, HashSet<String>> = HashMap::new();
        for node in graph.iter_nodes()? {
            if !matches!(node.node_type, NodeType::Page) {
                continue;
            }
            let page_id = node.id;
            register_node_page(&mut node_to_pages, &page_id, &page_id);
            let collected = graph_collect::collect_page_logic_nodes(graph, &page_id)?;
            for component in collected.child_components {
                register_node_page(&mut node_to_pages, &component.id, &page_id);
            }
            for action in collected.child_actions {
                register_node_page(&mut node_to_pages, &action.id, &page_id);
            }
        }
        Ok(Self { node_to_pages })
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
