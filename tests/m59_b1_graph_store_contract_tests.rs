//! M59 阶段 B1：**共享 `GraphStore` 契约测试套件**。
//!
//! spec §2.6 把这套测试定为迁移的**硬前置**，理由是：
//!
//! > 迁移会引入第三个 `GraphStore` 实现。现在两个实现就已经不一致，再加一个
//! > 而没有共享契约测试，等于保证三方分叉——**而且我们将无法判断 Grafeo 实现
//! > 是对的还是错的**，因为没有基准。
//!
//! 所以本文件的用法是：**同一组用例，跑每一个实现**。`GrafeoGraphStore`（C1）
//! 落地时只需在 `contract_suite!` 里加一行，不改任何用例——用例通过即视为
//! 与现有实现语义等价，这就是 C1 的验收判据。
//!
//! ## 基准取谁
//!
//! 分歧处一律以 **redb 侧（`GraphDB`）为准**，因为它是生产查询路径实际跑的实现。
//! `MemoryGraphStore` 原有三处偏离，已在本次一并纠正（见 `src/memory_graph_store.rs`
//! 的 M59-B1 注释）：不去重边、邻接表存节点副本导致更新不可见、`upsert_node`
//! 无条件覆盖 meta。规则本身（meta 合并、边去重键）已提到
//! `graph_store.rs`，两个实现共用一份实现，避免规则分头演化。
//!
//! ## 覆盖范围（spec §3 B1 点名的五项）
//!
//! 重复边 / 节点更新 / 占位节点升级 / 删除 / 邻接一致性，另加计数自洽与
//! 「删后重建」这个去重键泄漏的经典坑。

#![cfg(feature = "cli-local")]

use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_store::{GraphEdgeView, GraphReadStore, GraphStore, GraphWriteStore};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use serde_json::json;
use std::path::PathBuf;

// ---------------------------------------------------------------- 构造 helper

fn node(id: &str) -> Node {
    Node {
        id: id.to_string(),
        node_type: NodeType::Component,
        path: format!("app/{id}.spg"),
        name: id.to_string(),
        meta: None,
    }
}

fn node_with(id: &str, name: &str, meta: Option<serde_json::Value>) -> Node {
    Node {
        name: name.to_string(),
        meta,
        ..node(id)
    }
}

fn edge(from: &str, to: &str, field_path: Option<&str>) -> Edge {
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type: EdgeType::DependsOn,
        field_path: field_path.map(|s| s.to_string()),
        meta: None,
    }
}

fn seed(store: &mut dyn GraphStore, ids: &[&str]) {
    for id in ids {
        store.upsert_node(node(id)).expect("upsert");
    }
}

fn out_edges(store: &dyn GraphStore, id: &str) -> Vec<GraphEdgeView> {
    store
        .get_node_edges(id)
        .expect("get_node_edges")
        .map(|n| n.outgoing)
        .unwrap_or_default()
}

// ------------------------------------------------------------------ 契约用例
//
// 每个用例只用 GraphStore trait 方法，不碰任何实现细节——这是它能跨实现复用的
// 前提，也是 Grafeo 实现无需改用例即可接入的前提。

mod cases {
    use super::*;

    /// 重复边：同一四元组写两次只留一条。
    pub fn duplicate_edge_is_stored_once(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", Some("value"))).expect("e1");
        store.add_edge(edge("a", "b", Some("value"))).expect("e2");

        assert_eq!(
            store.edge_count().expect("count"),
            1,
            "同一条边写两次应只留一条"
        );
        assert_eq!(out_edges(store, "a").len(), 1, "邻接表也不得重复");
    }

    /// 去重键含 `field_path`：不同字段是不同的事实，不得合并。
    pub fn edges_differing_only_by_field_path_are_distinct(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", Some("value"))).expect("e1");
        store.add_edge(edge("a", "b", Some("visible"))).expect("e2");

        assert_eq!(
            store.edge_count().expect("count"),
            2,
            "同一对端点上不同 field_path 的引用是两条不同的边，去重不得跨字段"
        );
    }

    /// 节点更新必须**经邻接表**可见——不能读到建边时的旧副本。
    pub fn node_update_is_visible_through_adjacency(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", None)).expect("edge");
        store
            .upsert_node(node_with("b", "renamed", None))
            .expect("update b");

        let direct = store.get_node("b").expect("get").expect("b exists");
        assert_eq!(direct.name, "renamed");

        let via_edge = &out_edges(store, "a")[0].node;
        assert_eq!(
            via_edge.name, direct.name,
            "邻接表读到的节点必须与 get_node 一致，实际 {} vs {}",
            via_edge.name, direct.name
        );
    }

    /// 不带 meta 的 upsert 不得抹掉既有 meta。
    pub fn upsert_without_meta_preserves_existing_meta(store: &mut dyn GraphStore) {
        store
            .upsert_node(node_with("m", "m", Some(json!({"modelType": "DataFlow"}))))
            .expect("seed meta");
        store
            .upsert_node(node_with("m", "m2", None))
            .expect("re-upsert");

        let got = store.get_node("m").expect("get").expect("m exists");
        assert_eq!(got.name, "m2", "非 meta 字段仍以最新写入为准");
        assert_eq!(
            got.meta,
            Some(json!({"modelType": "DataFlow"})),
            "写入方没带 meta 时必须保留既有 meta，实际 {:?}",
            got.meta
        );
    }

    /// 占位节点升级：`PhysicalTable` 是引用侧的保守推断，不得覆盖已确认的类型。
    pub fn placeholder_type_does_not_downgrade_confirmed_type(store: &mut dyn GraphStore) {
        store
            .upsert_node(node_with("m", "m", Some(json!({"modelType": "DataFlow"}))))
            .expect("confirmed");
        store
            .upsert_node(node_with(
                "m",
                "m",
                Some(json!({"modelType": "PhysicalTable"})),
            ))
            .expect("placeholder");

        let got = store.get_node("m").expect("get").expect("m exists");
        assert_eq!(
            got.meta,
            Some(json!({"modelType": "DataFlow"})),
            "PhysicalTable 不得降级已确认的 DataFlow，实际 {:?}",
            got.meta
        );

        // 反向：确认类型可以覆盖占位类型。
        store
            .upsert_node(node_with(
                "p",
                "p",
                Some(json!({"modelType": "PhysicalTable"})),
            ))
            .expect("placeholder first");
        store
            .upsert_node(node_with("p", "p", Some(json!({"modelType": "App"}))))
            .expect("confirmed later");
        assert_eq!(
            store.get_node("p").expect("get").expect("p").meta,
            Some(json!({"modelType": "App"})),
            "确认类型应当覆盖占位类型"
        );
    }

    /// 端点缺失的边静默忽略——扫描顺序会让引用先于定义出现，这不是错误。
    pub fn edge_with_missing_endpoint_is_ignored(store: &mut dyn GraphStore) {
        seed(store, &["a"]);
        store
            .add_edge(edge("a", "ghost", None))
            .expect("dangling out");
        store
            .add_edge(edge("ghost", "a", None))
            .expect("dangling in");

        assert_eq!(store.edge_count().expect("count"), 0, "悬挂边不得入图");
        assert!(out_edges(store, "a").is_empty());
        assert!(
            store.get_node("ghost").expect("get").is_none(),
            "不得因为建边而凭空创建端点节点"
        );
    }

    /// 删除节点连带删除其所有关联边（两个方向）。
    pub fn removing_a_node_removes_its_incident_edges(store: &mut dyn GraphStore) {
        seed(store, &["a", "b", "c"]);
        store.add_edge(edge("a", "b", None)).expect("a->b");
        store.add_edge(edge("b", "c", None)).expect("b->c");
        assert_eq!(store.edge_count().expect("count"), 2);

        store
            .remove_nodes_by_ids(&["b".to_string()])
            .expect("remove b");

        assert!(store.get_node("b").expect("get").is_none());
        assert_eq!(store.node_count().expect("count"), 2);
        assert_eq!(
            store.edge_count().expect("count"),
            0,
            "b 的出边与入边都应随之消失"
        );
        assert!(out_edges(store, "a").is_empty(), "a 的出边应已清掉");
        assert!(
            store
                .get_node_edges("c")
                .expect("edges")
                .expect("c exists")
                .incoming
                .is_empty(),
            "c 的入边应已清掉"
        );
    }

    /// 删掉再建回来，同一条边必须能重新写入。
    ///
    /// 去重键若在删除时没跟着清，这条边会被永久当成「重复」丢弃——
    /// 图里静默缺一条边，且没有任何报错。
    pub fn edge_can_be_recreated_after_endpoint_removal(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", Some("value"))).expect("e1");
        store
            .remove_nodes_by_ids(&["b".to_string()])
            .expect("remove b");
        assert_eq!(store.edge_count().expect("count"), 0);

        seed(store, &["b"]);
        store.add_edge(edge("a", "b", Some("value"))).expect("e2");
        assert_eq!(
            store.edge_count().expect("count"),
            1,
            "端点删后重建，同一条边必须能重新写入（去重键须随删除一起清）"
        );
    }

    /// 删除不存在的 id 是 no-op，不报错也不改动其它内容。
    pub fn removing_unknown_node_is_a_noop(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", None)).expect("edge");

        store
            .remove_nodes_by_ids(&["nope".to_string()])
            .expect("remove unknown");

        assert_eq!(store.node_count().expect("count"), 2);
        assert_eq!(store.edge_count().expect("count"), 1);
    }

    /// `get_node_edges`：节点不存在返回 `None`，节点存在但无边返回 `Some(空)`。
    /// 这两者对查询方含义完全不同，不能混为一谈。
    pub fn missing_node_and_isolated_node_are_distinguishable(store: &mut dyn GraphStore) {
        seed(store, &["lonely"]);

        assert!(
            store.get_node_edges("nope").expect("edges").is_none(),
            "节点不存在必须是 None"
        );
        let neighbors = store
            .get_node_edges("lonely")
            .expect("edges")
            .expect("孤立节点必须是 Some(空邻居)，不能是 None");
        assert!(neighbors.outgoing.is_empty() && neighbors.incoming.is_empty());
    }

    /// 计数自洽：`iter_nodes` 与 `node_count` 一致；逐节点出边之和与 `edge_count` 一致。
    pub fn counts_agree_with_iteration_and_adjacency(store: &mut dyn GraphStore) {
        seed(store, &["a", "b", "c"]);
        store.add_edge(edge("a", "b", None)).expect("e1");
        store.add_edge(edge("a", "c", None)).expect("e2");
        store.add_edge(edge("b", "c", None)).expect("e3");

        let ids: Vec<String> = {
            let mut v: Vec<String> = store.iter_nodes().expect("iter").map(|n| n.id).collect();
            v.sort();
            v
        };
        assert_eq!(ids, vec!["a", "b", "c"], "iter_nodes 应恰好覆盖全部节点");
        assert_eq!(store.node_count().expect("count"), ids.len());

        let adjacency_total: usize = ids.iter().map(|id| out_edges(store, id).len()).sum();
        assert_eq!(
            adjacency_total,
            store.edge_count().expect("count"),
            "逐节点出边之和必须等于 edge_count"
        );
    }

    /// 入边与出边必须互为镜像：a→b 在 a 的 outgoing 里，也必须在 b 的 incoming 里，
    /// 且两侧读到的 `Edge` 内容一致。
    pub fn incoming_and_outgoing_are_mirrors(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store.add_edge(edge("a", "b", Some("value"))).expect("edge");

        let out = out_edges(store, "a");
        let incoming = store
            .get_node_edges("b")
            .expect("edges")
            .expect("b exists")
            .incoming;

        assert_eq!(out.len(), 1);
        assert_eq!(incoming.len(), 1);
        assert_eq!(out[0].node.id, "b", "出边视图挂的是**对端**节点");
        assert_eq!(incoming[0].node.id, "a", "入边视图挂的也是对端节点");
        assert_eq!(out[0].edge.from, incoming[0].edge.from);
        assert_eq!(out[0].edge.to, incoming[0].edge.to);
        assert_eq!(out[0].edge.field_path, incoming[0].edge.field_path);
    }
}

// ------------------------------------------------------------- 实现接入与展开

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-b1-{tag}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

/// 为每个用例 × 每个实现展开一个 `#[test]`。
///
/// 新增实现（C1 的 `GrafeoGraphStore`）时只加一个 `impl_mod` 分支，用例一行不动。
macro_rules! contract_suite {
    ($($case:ident),* $(,)?) => {
        mod memory_store {
            use super::*;
            $(
                #[test]
                fn $case() {
                    let mut store = MemoryGraphStore::new();
                    cases::$case(&mut store);
                }
            )*
        }

        mod redb_store {
            use super::*;
            $(
                #[test]
                fn $case() {
                    let dir = unique_temp_dir(stringify!($case));
                    let mut store = GraphDB::open(&dir.join("graph.db")).expect("open GraphDB");
                    cases::$case(&mut store);
                    let _ = std::fs::remove_dir_all(&dir);
                }
            )*
        }
    };
}

contract_suite!(
    duplicate_edge_is_stored_once,
    edges_differing_only_by_field_path_are_distinct,
    node_update_is_visible_through_adjacency,
    upsert_without_meta_preserves_existing_meta,
    placeholder_type_does_not_downgrade_confirmed_type,
    edge_with_missing_endpoint_is_ignored,
    removing_a_node_removes_its_incident_edges,
    edge_can_be_recreated_after_endpoint_removal,
    removing_unknown_node_is_a_noop,
    missing_node_and_isolated_node_are_distinguishable,
    counts_agree_with_iteration_and_adjacency,
    incoming_and_outgoing_are_mirrors,
);
