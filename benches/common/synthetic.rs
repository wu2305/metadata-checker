use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::memory_graph_store::MemoryGraphStore;

/// 构造深层组件树，用于 page logic fanout 微基准。
pub fn build_deep_tree_page_graph(leaf_count: usize, max_depth: usize) -> MemoryGraphStore {
    let mut store = MemoryGraphStore::new();
    let page_id = "page:app/home.spg";
    store.add_test_node(page_id, "首页", NodeType::Page, "app/home.spg");
    store.add_test_node(
        "model:shared",
        "共享模型",
        NodeType::Model,
        "tables/shared.tbl",
    );

    let root_id = "comp:app/home.spg|root";
    store.add_test_node(root_id, "root", NodeType::Component, "app/home.spg");
    store.add_test_edge(page_id, root_id, EdgeType::Contains, None);

    let mut frontier = vec![(root_id.to_string(), 1_usize)];
    let mut created = 1_usize;
    let mut next_id = 0_usize;

    while created < leaf_count {
        let Some((parent_id, parent_depth)) = frontier.first().cloned() else {
            break;
        };
        frontier.remove(0);

        let child_depth = parent_depth + 1;
        let branch = 4_usize.min(leaf_count - created);
        for _ in 0..branch {
            next_id += 1;
            let comp_id = format!("comp:app/home.spg|node{next_id}");
            let field = format!("shared.field{next_id}");
            store.add_test_node(
                &comp_id,
                &format!("node{next_id}"),
                NodeType::Component,
                "app/home.spg",
            );
            store.add_test_edge(&parent_id, &comp_id, EdgeType::Contains, None);
            store.add_test_edge(&comp_id, "model:shared", EdgeType::Reads, Some(&field));
            created += 1;

            if child_depth < max_depth && created < leaf_count {
                frontier.push((comp_id, child_depth));
            }
        }
    }

    store
}

/// 构造高密度模型扇出图，用于 context 微基准。
pub fn build_dense_model_context_graph(fanout: usize) -> (MemoryGraphStore, String) {
    let mut store = MemoryGraphStore::new();
    let page_id = "page:app/home.spg";
    store.add_test_node(page_id, "首页", NodeType::Page, "app/home.spg");
    store.add_test_node("model:hub", "中心模型", NodeType::Model, "tables/hub.tbl");

    let hub_comp_id = "comp:app/home.spg|hub";
    store.add_test_node(hub_comp_id, "hub", NodeType::Component, "app/home.spg");
    store.add_test_edge(page_id, hub_comp_id, EdgeType::Contains, None);
    store.add_test_edge(
        hub_comp_id,
        "model:hub",
        EdgeType::Reads,
        Some("hub.center"),
    );

    for index in 0..fanout {
        let comp_id = format!("comp:app/home.spg|leaf{index}");
        let field = format!("hub.field{index}");
        store.add_test_node(
            &comp_id,
            &format!("leaf{index}"),
            NodeType::Component,
            "app/home.spg",
        );
        store.add_test_edge(page_id, &comp_id, EdgeType::Contains, None);
        store.add_test_edge(hub_comp_id, &comp_id, EdgeType::Triggers, None);
        store.add_test_edge(&comp_id, "model:hub", EdgeType::Reads, Some(&field));
        store.add_test_edge(&comp_id, "model:hub", EdgeType::ActionWrites, Some(&field));
    }

    (store, hub_comp_id.to_string())
}

/// 构造高 fanout 的内存页图，用于 query 微基准。
pub fn build_fanout_page_graph(fanout: usize) -> MemoryGraphStore {
    let mut store = MemoryGraphStore::new();
    store.add_test_node("page:app/home.spg", "首页", NodeType::Page, "app/home.spg");
    store.add_test_node(
        "model:shared",
        "共享模型",
        NodeType::Model,
        "tables/shared.tbl",
    );

    for index in 0..fanout {
        let comp_id = format!("comp:app/home.spg|input{index}");
        let action_id = format!("action:app/home.spg|input{index}|action1");
        let field = format!("shared.field{index}");
        store.add_test_node(
            &comp_id,
            &format!("input{index}"),
            NodeType::Component,
            "app/home.spg",
        );
        store.add_test_node(
            &action_id,
            &format!("setParamValue:action{index}"),
            NodeType::Action,
            "app/home.spg",
        );
        store.add_test_edge("page:app/home.spg", &comp_id, EdgeType::Contains, None);
        store.add_test_edge(&comp_id, "model:shared", EdgeType::Reads, Some(&field));
        store.add_test_edge(&comp_id, &action_id, EdgeType::Triggers, None);
        store.add_test_edge(
            &action_id,
            "model:shared",
            EdgeType::ActionWrites,
            Some(&field),
        );
    }

    store
}
