use crate::graph::{GraphDB, Node};

/// 页面逻辑查询需要同时遍历组件节点和组件触发的动作节点
pub(super) struct PageLogicNodes<'a> {
    pub(super) child_components: Vec<&'a Node>,
    pub(super) child_actions: Vec<&'a Node>,
}

/// 递归收集页面下所有组件，以及这些组件直接触发的动作
pub(super) fn collect_page_logic_nodes<'a>(
    graph: &'a GraphDB,
    page_id: &str,
) -> PageLogicNodes<'a> {
    let mut child_components = Vec::new();
    let mut child_actions = Vec::new();
    let mut visited = std::collections::HashSet::new();

    collect_components_recursive(
        graph,
        page_id,
        &mut child_components,
        &mut child_actions,
        &mut visited,
    );

    PageLogicNodes {
        child_components,
        child_actions,
    }
}

fn collect_components_recursive<'a>(
    graph: &'a GraphDB,
    parent_id: &str,
    child_components: &mut Vec<&'a Node>,
    child_actions: &mut Vec<&'a Node>,
    visited: &mut std::collections::HashSet<String>,
) {
    if let Some((outgoing, _)) = graph.get_node_edges(parent_id) {
        for (target, edge) in &outgoing {
            if matches!(edge.edge_type, crate::graph::EdgeType::Contains)
                && matches!(target.node_type, crate::graph::NodeType::Component)
                && !visited.contains(&target.id)
            {
                visited.insert(target.id.clone());
                child_components.push(target);
                if let Some((comp_out, _)) = graph.get_node_edges(&target.id) {
                    for (act, e) in &comp_out {
                        if matches!(e.edge_type, crate::graph::EdgeType::Triggers)
                            && matches!(act.node_type, crate::graph::NodeType::Action)
                            && !visited.contains(&act.id)
                        {
                            visited.insert(act.id.clone());
                            child_actions.push(act);
                        }
                    }
                }
                collect_components_recursive(
                    graph,
                    &target.id,
                    child_components,
                    child_actions,
                    visited,
                );
            }
        }
    }
}
