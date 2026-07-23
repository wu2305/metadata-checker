use crate::graph::{Node, NodeType};
use crate::graph_store::GraphReadStore;
use anyhow::Result;
use std::collections::HashSet;

/// 页面逻辑查询需要同时遍历组件节点和组件触发的动作节点
pub(super) struct PageLogicNodes {
    pub(super) child_components: Vec<Node>,
    pub(super) child_actions: Vec<Node>,
    /// M55：页面数据面反向关系覆盖到的 Model 节点（含共享物理模型与 DataFlow 模型）
    pub(super) related_models: Vec<Node>,
    /// M55：页面数据面反向关系覆盖到的 Field 节点（含 DataFlow 输出字段）
    pub(super) related_fields: Vec<Node>,
}

/// 递归收集页面下所有组件，以及这些组件直接触发的动作
pub(crate) fn collect_page_logic_nodes(
    graph: &dyn GraphReadStore,
    page_id: &str,
) -> Result<PageLogicNodes> {
    let mut child_components = Vec::new();
    let mut child_actions = Vec::new();
    let mut visited = HashSet::new();

    collect_components_recursive(
        graph,
        page_id,
        &mut child_components,
        &mut child_actions,
        &mut visited,
    )?;

    // M55：数据面闭包——从页面自身与已收集的组件/动作出发，沿出边遍历
    // Model/Field 目标并继续闭包（覆盖本地模型 → 物理模型/字段的
    // FieldAlias、DataFlow 的 DataflowInput/Internal/Output 结构边）。
    // 不穿越 Component/Action/Page，避免把整页依赖图拉进闭包。
    let mut related_models = Vec::new();
    let mut related_fields = Vec::new();
    let mut data_visited = HashSet::new();
    let mut stack: Vec<String> = vec![page_id.to_string()];
    stack.extend(child_components.iter().map(|node| node.id.clone()));
    stack.extend(child_actions.iter().map(|node| node.id.clone()));
    while let Some(current_id) = stack.pop() {
        if let Some(neighbors) = graph.get_node_edges(&current_id)? {
            for ev in neighbors.outgoing {
                let target = ev.node;
                match target.node_type {
                    NodeType::Model => {
                        if data_visited.insert(target.id.clone()) {
                            stack.push(target.id.clone());
                            related_models.push(target);
                        }
                    }
                    NodeType::Field => {
                        if data_visited.insert(target.id.clone()) {
                            stack.push(target.id.clone());
                            related_fields.push(target);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(PageLogicNodes {
        child_components,
        child_actions,
        related_models,
        related_fields,
    })
}

fn collect_components_recursive(
    graph: &dyn GraphReadStore,
    parent_id: &str,
    child_components: &mut Vec<Node>,
    child_actions: &mut Vec<Node>,
    visited: &mut HashSet<String>,
) -> Result<()> {
    if let Some(neighbors) = graph.get_node_edges(parent_id)? {
        for ev in neighbors.outgoing {
            let target = ev.node;
            let edge = ev.edge;

            if matches!(edge.edge_type, crate::graph::EdgeType::Contains)
                && matches!(target.node_type, crate::graph::NodeType::Component)
                && !visited.contains(&target.id)
            {
                visited.insert(target.id.clone());
                child_components.push(target.clone());

                let component_id = target.id.clone();
                if let Some(comp_neighbors) = graph.get_node_edges(&component_id)? {
                    for action_ev in comp_neighbors.outgoing {
                        let action = action_ev.node;
                        let action_edge = action_ev.edge;
                        if matches!(action_edge.edge_type, crate::graph::EdgeType::Triggers)
                            && matches!(action.node_type, crate::graph::NodeType::Action)
                            && !visited.contains(&action.id)
                        {
                            visited.insert(action.id.clone());
                            child_actions.push(action);
                        }
                    }
                }
                collect_components_recursive(
                    graph,
                    &component_id,
                    child_components,
                    child_actions,
                    visited,
                )?;
            }
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "cli-local"))]
mod tests {
    use super::*;
    use crate::graph::GraphDB;
    use crate::graph_store::GraphReadStore;
    use crate::scanner::scan_project;
    use std::path::Path;
    use std::process;

    #[test]
    fn collect_page_logic_nodes_accepts_graph_read_store_trait_object() -> anyhow::Result<()> {
        let db_path = std::env::temp_dir().join(format!(
            "metadata-checker-task39-page-logic-nodes-{}.db",
            process::id()
        ));
        let _ = std::fs::remove_file(&db_path);
        scan_project(Path::new("tests/fixtures/test_project"), &db_path)?;
        let graph = GraphDB::open(&db_path)?;
        let graph_store: &dyn GraphReadStore = &graph;
        let nodes = collect_page_logic_nodes(graph_store, "page:app/page_relations.spg")?;

        assert!(
            !nodes.child_components.is_empty() || !nodes.child_actions.is_empty(),
            "graph_collect should find components or actions for page"
        );
        Ok(())
    }
}
