//! M59 阶段 B1：**共享 `GraphStore` 契约测试套件**。
//!
//! spec §2.6 把这套测试定为迁移的**硬前置**，理由是：
//!
//! > 迁移会引入第三个 `GraphStore` 实现。现在两个实现就已经不一致，再加一个
//! > 而没有共享契约测试，等于保证三方分叉——**而且我们将无法判断 Grafeo 实现
//! > 是对的还是错的**，因为没有基准。
//!
//! 所以本文件的用法是：**同一组用例，跑每一个实现**。M59-3 C1 起
//! `GrafeoGraphStore` 已接入同一组用例；通过只证明已覆盖的契约，
//! 持久化重启与查询行为仍需各自验收（见 `m59_c1_grafeo_store_tests.rs`）。
//!
//! ## 基准取谁
//!
//! 以已批准语义契约及独立手工期望为准；redb 的历史行为不是正确性标准。
//! 2026-09-21 修正：同关系的不同 metadata 必须保留，不能继续钉住首次写入胜出。
//! `MemoryGraphStore` 原有三处偏离，已在本次一并纠正（见 `src/memory_graph_store.rs`
//! 的 M59-B1 注释）：不去重边、邻接表存节点副本导致更新不可见、`upsert_node`
//! 无条件覆盖 meta。规则本身（meta 合并、边去重键）已提到
//! `graph_store.rs`，两个实现共用一份实现，避免规则分头演化。
//!
//! ## 覆盖范围（spec §3 B1 点名的五项）
//!
//! 重复边 / 节点更新 / 占位节点升级 / 删除 / 邻接一致性，另加计数自洽、
//! 「删后重建」这个去重键泄漏的经典坑，以及 `edge_type` 的原样存取与去重参与。
//!
//! ## 断言强度（codex 复审返修）
//!
//! 一套**能被错误实现骗过**的契约测试，比没有更糟——它给的是虚假的验收信号。
//! 复审用变异实现实测：一律返回 `Contains`、丢掉 `field_path`、邻接里回填
//! 错误的节点 `path`、删任一存在的节点就清空所有边——四种都能全绿通过。
//! 因此本文件的断言一律遵守三条：
//!
//! 1. 比**完整**内容（`edge_repr` / `node_repr`），不比条数、不比单个字段；
//! 2. 与**写死的期望值**比，不只比「两个输出彼此一致」——两侧可以一起错；
//! 3. 破坏性操作必须留一个**不该被波及的幸存者**，否则整体塌方也能过。

#![cfg(feature = "cli-local")]

use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
#[cfg(feature = "grafeo-store")]
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_store::{GraphEdgeView, GraphStore};
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
        origin_file: None,
    }
}

fn node_with(id: &str, name: &str, meta: Option<serde_json::Value>) -> Node {
    Node {
        name: name.to_string(),
        meta,
        ..node(id)
    }
}

fn edge_typed(from: &str, to: &str, edge_type: EdgeType, field_path: Option<&str>) -> Edge {
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type,
        field_path: field_path.map(|s| s.to_string()),
        meta: None,
        origin_file: None,
    }
}

fn edge(from: &str, to: &str, field_path: Option<&str>) -> Edge {
    edge_typed(from, to, EdgeType::DependsOn, field_path)
}

// ------------------------------------------------------- 完整比对（而非计数）
//
// codex 复审（`2db8af9` 之后）：本套件原先的断言多半只看**条数**、只看
// **名字**、或者只看「两个输出彼此一致」。这三种都挡不住实现整体偏斜——
// 实测把一个存内存实现改成「一律返回 `Contains`」「丢掉 `field_path`」
// 「邻接里回填错误的 `path`」「删任一存在的节点就清空所有边」，
// 十二条用例**全绿**。契约测试若能被这样的实现骗过，它就无法充当
// C1（Grafeo 实现）的验收判据——而这正是 spec §2.6 把它定为硬前置的理由。
//
// 现在一律比对**完整规范化表示**，并与显式写死的期望值比。

/// 边的完整表示：`from` / `edge_type` / `to` / `field_path` / `meta` 一个不落。
fn edge_repr(e: &Edge) -> String {
    format!(
        "{} -{:?}-> {}\tfield_path={}\tmeta={}",
        e.from,
        e.edge_type,
        e.to,
        e.field_path.clone().unwrap_or_else(|| "-".to_string()),
        e.meta
            .as_ref()
            .map(|m| serde_json::to_string(m).expect("meta"))
            .unwrap_or_else(|| "null".to_string())
    )
}

/// 节点的完整表示：`path` 也在内——邻接视图回填错节点时只有它看得出来。
fn node_repr(n: &Node) -> String {
    format!(
        "{}\ttype={:?}\tpath={}\tname={}\tmeta={}",
        n.id,
        n.node_type,
        n.path,
        n.name,
        n.meta
            .as_ref()
            .map(|m| serde_json::to_string(m).expect("meta"))
            .unwrap_or_else(|| "null".to_string())
    )
}

/// 全图所有出边的完整表示，排序后可与字面量逐行比对。
fn all_edge_reprs(store: &dyn GraphStore) -> Vec<String> {
    let mut ids: Vec<String> = store.iter_nodes().expect("iter").map(|n| n.id).collect();
    ids.sort();
    let mut out: Vec<String> = Vec::new();
    for id in &ids {
        for view in out_edges(store, id) {
            out.push(edge_repr(&view.edge));
        }
    }
    out.sort();
    out
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
        // 不止「只剩一条」：剩下的那条必须原样是写进去的那条。
        assert_eq!(
            all_edge_reprs(store),
            vec!["a -DependsOn-> b\tfield_path=value\tmeta=null".to_string()],
            "留下的边必须逐属性等于写入的那条"
        );
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
        // 条数对不代表读回来的 `field_path` 还在：把它一律读成 `None`，
        // 条数依旧是 2。所以这里比的是两条边的完整内容。
        assert_eq!(
            all_edge_reprs(store),
            vec![
                "a -DependsOn-> b\tfield_path=value\tmeta=null".to_string(),
                "a -DependsOn-> b\tfield_path=visible\tmeta=null".to_string(),
            ],
            "两条边必须各自带着自己的 field_path 读回来"
        );
    }

    /// `edge_type` 必须原样存取，且**参与去重键**。
    ///
    /// 全套用例原先只用 `DependsOn` 一种类型，也从不断言读回来的类型——
    /// 一个「一律返回 `Contains`」的实现能骗过所有十二条。边的类型正是查询
    /// 语义的全部：把 `EmbedsPage` 读成 `Contains`，「谁内嵌了谁」直接错答。
    pub fn edge_type_is_stored_verbatim_and_participates_in_dedup(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        store
            .add_edge(edge_typed("a", "b", EdgeType::EmbedsPage, None))
            .expect("embeds");
        store
            .add_edge(edge_typed("a", "b", EdgeType::Contains, None))
            .expect("contains");
        store
            .add_edge(edge_typed("a", "b", EdgeType::EmbedsPage, None))
            .expect("embeds again");

        assert_eq!(
            all_edge_reprs(store),
            vec![
                "a -Contains-> b\tfield_path=-\tmeta=null".to_string(),
                "a -EmbedsPage-> b\tfield_path=-\tmeta=null".to_string(),
            ],
            "同一对端点上不同类型是两条不同的边，同类型重复写只留一条，\
             且类型必须原样读回"
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

        // 比整个节点，不只比 name：邻接视图若回填了错误的 `path` / `node_type` /
        // `meta`，只看 name 是发现不了的。
        let via_edge = &out_edges(store, "a")[0].node;
        assert_eq!(
            node_repr(via_edge),
            node_repr(&direct),
            "邻接表读到的节点必须与 get_node **逐属性**一致"
        );
        assert_eq!(
            node_repr(&direct),
            "b\ttype=Component\tpath=app/b.spg\tname=renamed\tmeta=null",
            "顺带钉住节点的完整形状，避免两侧一起错还互相印证"
        );
    }

    /// 不同证据是不同事实；相同证据重复写入仍幂等，双向邻接逐属性保留。
    pub fn edge_metadata_survives_storage_and_duplicate_write(store: &mut dyn GraphStore) {
        seed(store, &["a", "b"]);
        let original = Edge {
            meta: Some(json!({"condition": "=a.value > 0", "origin_file": "app/a.spg"})),
            ..edge("a", "b", Some("value"))
        };
        let other = Edge {
            meta: Some(json!({"condition": "=false"})),
            ..original.clone()
        };
        store.add_edge(original.clone()).expect("initial edge");
        store.add_edge(other.clone()).expect("distinct fact");
        store.add_edge(original.clone()).expect("duplicate fact");
        let mut expected = vec![edge_repr(&original), edge_repr(&other)];
        expected.sort();
        let mut outgoing: Vec<_> = out_edges(store, "a")
            .iter()
            .map(|view| edge_repr(&view.edge))
            .collect();
        let mut incoming: Vec<_> = store
            .get_node_edges("b")
            .expect("neighbors")
            .expect("b")
            .incoming
            .iter()
            .map(|view| edge_repr(&view.edge))
            .collect();
        outgoing.sort();
        incoming.sort();
        assert_eq!(outgoing, expected);
        assert_eq!(incoming, expected);
    }

    /// 建边后更新两端的全部属性，双向邻接必须读取最新的完整节点。
    pub fn updated_metadata_is_visible_in_both_adjacency_directions(store: &mut dyn GraphStore) {
        for id in ["a", "b"] {
            store
                .upsert_node(node_with(id, id, Some(json!({"revision": 1}))))
                .expect("initial node");
        }
        store.add_edge(edge("a", "b", Some("value"))).expect("edge");
        let expected_a = Node {
            id: "a".to_string(),
            node_type: NodeType::Model,
            path: "data/source.tbl".to_string(),
            name: "source".to_string(),
            meta: Some(json!({"revision": 2, "fields": ["amount"]})),
            origin_file: None,
        };
        let expected_b = Node {
            id: "b".to_string(),
            node_type: NodeType::Field,
            path: "data/target.tbl".to_string(),
            name: "amount".to_string(),
            meta: Some(json!({"revision": 3, "dataType": "N"})),
            origin_file: None,
        };
        store.upsert_node(expected_a.clone()).expect("update a");
        store.upsert_node(expected_b.clone()).expect("update b");
        let outgoing = out_edges(store, "a");
        let incoming = store
            .get_node_edges("b")
            .expect("neighbors")
            .expect("b")
            .incoming;
        assert_eq!(outgoing.len(), 1);
        assert_eq!(incoming.len(), 1);
        for (expected, via_edge) in [
            (&expected_a, &incoming[0].node),
            (&expected_b, &outgoing[0].node),
        ] {
            let expected_value = serde_json::to_value(expected).expect("expected node");
            let direct = store
                .get_node(&expected.id)
                .expect("direct node")
                .expect("node exists");
            assert_eq!(
                serde_json::to_value(direct).expect("direct"),
                expected_value
            );
            assert_eq!(
                serde_json::to_value(via_edge).expect("neighbor"),
                expected_value
            );
        }
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
        seed(store, &["a", "b", "c", "x", "y"]);
        store.add_edge(edge("a", "b", None)).expect("a->b");
        store.add_edge(edge("b", "c", None)).expect("b->c");
        // 与 b 无关的一条边。删除必须**只**牵连 b 的关联边——
        // 原先这个用例里每一条边都挨着 b，于是「删任一节点就清空所有边」的
        // 实现也能全绿。留一条幸存者，这种整体塌方才会当场暴露。
        store.add_edge(edge("x", "y", Some("keep"))).expect("x->y");
        assert_eq!(store.edge_count().expect("count"), 3);

        store
            .remove_nodes_by_ids(&["b".to_string()])
            .expect("remove b");

        assert!(store.get_node("b").expect("get").is_none());
        assert_eq!(store.node_count().expect("count"), 4);
        assert_eq!(
            all_edge_reprs(store),
            vec!["x -DependsOn-> y\tfield_path=keep\tmeta=null".to_string()],
            "只有 b 的出边与入边该消失，无关的 x→y 必须原样幸存"
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
            all_edge_reprs(store),
            vec!["a -DependsOn-> b\tfield_path=value\tmeta=null".to_string()],
            "端点删后重建，同一条边必须原样重新写入（去重键须随删除一起清）"
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
        assert_eq!(
            all_edge_reprs(store),
            vec!["a -DependsOn-> b\tfield_path=-\tmeta=null".to_string()],
            "no-op 不得改动既有的边"
        );
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
        // 「两个输出彼此一致」可以两边一起错。再钉一次完整内容。
        assert_eq!(
            all_edge_reprs(store),
            vec![
                "a -DependsOn-> b\tfield_path=-\tmeta=null".to_string(),
                "a -DependsOn-> c\tfield_path=-\tmeta=null".to_string(),
                "b -DependsOn-> c\tfield_path=-\tmeta=null".to_string(),
            ],
            "邻接遍历出来的边必须逐属性等于写入的三条"
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

        // 两侧内容一致还不够——两侧可以一起错（都丢 field_path、都把类型说成
        // Contains）。所以分别与**写死的期望**比。
        let expected_edge = "a -DependsOn-> b\tfield_path=value\tmeta=null";
        assert_eq!(edge_repr(&out[0].edge), expected_edge, "出边内容");
        assert_eq!(edge_repr(&incoming[0].edge), expected_edge, "入边内容");

        // 视图挂的是**对端**节点，且是完整的对端节点。
        assert_eq!(
            node_repr(&out[0].node),
            "b\ttype=Component\tpath=app/b.spg\tname=b\tmeta=null",
            "出边视图应挂完整的对端节点 b"
        );
        assert_eq!(
            node_repr(&incoming[0].node),
            "a\ttype=Component\tpath=app/a.spg\tname=a\tmeta=null",
            "入边视图应挂完整的对端节点 a"
        );
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

        #[cfg(feature = "grafeo-store")]
        mod grafeo_store {
            use super::*;
            $(
                #[test]
                fn $case() {
                    // 用内存库：本套用例验的是图语义，不是持久化。
                    // 重开/重启由 C1 的适配器专项测试覆盖。
                    let mut store = GrafeoGraphStore::in_memory().expect("open GrafeoGraphStore");
                    cases::$case(&mut store);
                }
            )*
        }
    };
}

contract_suite!(
    edge_metadata_survives_storage_and_duplicate_write,
    updated_metadata_is_visible_in_both_adjacency_directions,
    duplicate_edge_is_stored_once,
    edges_differing_only_by_field_path_are_distinct,
    edge_type_is_stored_verbatim_and_participates_in_dedup,
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
