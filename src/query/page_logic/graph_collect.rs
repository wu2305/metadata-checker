use crate::graph::Node;
use crate::graph_store::GraphReadStore;
use anyhow::Result;
use std::collections::HashSet;

/// 页面逻辑查询需要同时遍历组件节点和组件触发的动作节点
pub(super) struct PageLogicNodes {
    pub(super) child_components: Vec<Node>,
    pub(super) child_actions: Vec<Node>,
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

    Ok(PageLogicNodes {
        child_components,
        child_actions,
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
