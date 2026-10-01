//! M59-3 C1：`GrafeoGraphStore` 适配器专项测试。
//!
//! 图语义本身由 `m59_b1_graph_store_contract_tests.rs` 的共享契约套件验收
//! （同一组 15 条用例跑 memory / redb / grafeo 三个实现）。本文件只覆盖
//! **只有 Grafeo 适配器才有的那层**：项目 `Node`/`Edge` ↔ grafeo 属性的
//! 互转，以及持久化重开后派生状态（id 映射、边去重键）的复原。
//!
//! 这两处是适配器特有的失效点：属性互转错了契约套件未必察觉（它在同一个
//! 进程里写进读出，错误可能对称抵消），而去重键不复原会让重开的库把已有边
//! 当新事实重复写入——那是一个只在「重启之后」才暴露的缺陷。

#![cfg(feature = "grafeo-store")]

use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};
use serde_json::json;
use std::path::PathBuf;

/// 每个用例一个独立库路径。扩展名必须是 `.grafeo`（单文件格式，支持只读共享打开）。
fn unique_db_path(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-c1-{tag}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("graph.grafeo")
}

fn node_of(id: &str, node_type: NodeType, meta: Option<serde_json::Value>) -> Node {
    Node {
        id: id.to_string(),
        node_type,
        path: format!("app/{id}.spg"),
        name: format!("name-{id}"),
        meta,
        origin_file: Some(format!("src/{id}.spg")),
    }
}

/// 全部 `NodeType` 变体。新增变体时这里会编译失败，提醒补齐覆盖。
fn all_node_types() -> Vec<NodeType> {
    vec![
        NodeType::Page,
        NodeType::Component,
        NodeType::Model,
        NodeType::Field,
        NodeType::Action,
        NodeType::Condition,
    ]
}

/// 全部 `EdgeType` 变体（22 个）。少一个就意味着某类关系的存取没被验过。
fn all_edge_types() -> Vec<EdgeType> {
    vec![
        EdgeType::Reads,
        EdgeType::Writes,
        EdgeType::Triggers,
        EdgeType::Contains,
        EdgeType::DataflowInput,
        EdgeType::ActionWrites,
        EdgeType::EmbedsPage,
        EdgeType::OpensPage,
        EdgeType::PassesParam,
        EdgeType::SetsParam,
        EdgeType::OutputsTo,
        EdgeType::DataflowInternal,
        EdgeType::DataflowOutput,
        EdgeType::FieldAlias,
        EdgeType::FieldWrite,
        EdgeType::ActionReads,
        EdgeType::ActionNavigates,
        EdgeType::ActionSetsParam,
        EdgeType::ActionControlsComponent,
        EdgeType::ActionValidates,
        EdgeType::ActionLoadsData,
        EdgeType::DependsOn,
    ]
}

/// 每个 `NodeType` 都能原样存取（类型只存属性、不编成标签，这条验的就是它）。
#[test]
fn every_node_type_round_trips() {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    for node_type in all_node_types() {
        let id = format!("n-{node_type:?}");
        let expected = node_of(&id, node_type.clone(), Some(json!({"k": "v"})));
        store.upsert_node(expected.clone()).expect("upsert");
        let actual = store.get_node(&id).expect("get").expect("node exists");
        assert_eq!(actual, expected, "NodeType {node_type:?} 应原样存取");
    }
    assert_eq!(store.node_count().expect("count"), all_node_types().len());
}

/// 每个 `EdgeType` 都能原样存取，且 `field_path` / `meta` / `origin_file` 一并保真。
#[test]
fn every_edge_type_round_trips_with_full_payload() {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    store
        .upsert_node(node_of("a", NodeType::Component, None))
        .expect("upsert a");
    store
        .upsert_node(node_of("b", NodeType::Model, None))
        .expect("upsert b");

    let mut expected: Vec<Edge> = Vec::new();
    for edge_type in all_edge_types() {
        let edge = Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            edge_type: edge_type.clone(),
            field_path: Some(format!("field/{edge_type:?}")),
            meta: Some(json!({"edge": format!("{edge_type:?}")})),
            origin_file: Some("src/a.spg".to_string()),
        };
        store.add_edge(edge.clone()).expect("add edge");
        expected.push(edge);
    }

    assert_eq!(
        store.edge_count().expect("count"),
        all_edge_types().len(),
        "22 种边类型应各存一条"
    );

    let neighbors = store
        .get_node_edges("a")
        .expect("edges")
        .expect("node exists");
    let mut actual: Vec<Edge> = neighbors.outgoing.into_iter().map(|v| v.edge).collect();
    actual.sort_by_key(|e| format!("{:?}", e.edge_type));
    expected.sort_by_key(|e| format!("{:?}", e.edge_type));
    assert_eq!(actual, expected, "每种边类型的完整载荷都应原样存取");
}

/// meta 的嵌套结构、Unicode、空容器、`None` 都要保真。
///
/// meta 以紧凑 JSON 文本存进 grafeo 属性，所以这里验的是序列化口径没有
/// 把 `null` / `{}` / `[]` 混成同一种东西，也没有在中文与转义字符上出错。
#[test]
fn metadata_shapes_round_trip() {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    let shapes = vec![
        ("m-none", None),
        ("m-null", Some(json!(null))),
        ("m-empty-object", Some(json!({}))),
        ("m-empty-array", Some(json!([]))),
        (
            "m-nested",
            Some(json!({"modelType": "PhysicalTable", "cols": [1, 2, {"deep": true}]})),
        ),
        (
            "m-unicode",
            Some(json!({"名称": "销售易\t\"报表\"", "emoji": "📊"})),
        ),
    ];

    for (id, meta) in &shapes {
        let expected = node_of(id, NodeType::Model, meta.clone());
        store.upsert_node(expected.clone()).expect("upsert");
        let actual = store.get_node(id).expect("get").expect("node exists");
        assert_eq!(actual, expected, "{id} 的 meta 应原样存取");
    }
}

/// 节点无 `origin_file`（旧兼容写入）时读回仍是 `None`，不是空串。
#[test]
fn absent_origin_file_reads_back_as_none() {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    let expected = Node {
        origin_file: None,
        ..node_of("a", NodeType::Page, None)
    };
    store.upsert_node(expected.clone()).expect("upsert");
    assert_eq!(
        store.get_node("a").expect("get").expect("node exists"),
        expected
    );
}

/// 重开持久化库后：节点与边完整还原，且**已有边不会被当成新事实重复写入**。
///
/// 后半句是去重键复原的验收点：若 `seen_edges` 不从库里重建，这里
/// 重复写的三条边会再入库一次，`edge_count` 就是 6 而不是 3。
#[test]
fn reopen_restores_graph_and_edge_dedup_keys() {
    let path = unique_db_path("reopen");
    let edges = vec![
        Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            edge_type: EdgeType::Reads,
            field_path: Some("f1".to_string()),
            meta: Some(json!({"n": 1})),
            origin_file: Some("src/a.spg".to_string()),
        },
        // 同关系不同 field_path：是另一条事实，必须共存
        Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            edge_type: EdgeType::Reads,
            field_path: Some("f2".to_string()),
            meta: Some(json!({"n": 1})),
            origin_file: Some("src/a.spg".to_string()),
        },
        // 同关系同 field_path 但来源不同：也是另一条事实
        Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            edge_type: EdgeType::Reads,
            field_path: Some("f1".to_string()),
            meta: Some(json!({"n": 1})),
            origin_file: Some("src/other.spg".to_string()),
        },
    ];

    {
        let mut store = GrafeoGraphStore::open(&path).expect("open store");
        store
            .upsert_node(node_of("a", NodeType::Component, Some(json!({"v": 1}))))
            .expect("upsert a");
        store
            .upsert_node(node_of("b", NodeType::Field, None))
            .expect("upsert b");
        for edge in &edges {
            store.add_edge(edge.clone()).expect("add edge");
        }
        assert_eq!(store.edge_count().expect("count"), 3);
        store.close().expect("close store");
    }

    let mut reopened = GrafeoGraphStore::open(&path).expect("reopen store");
    assert_eq!(reopened.node_count().expect("count"), 2, "重开后节点应还原");
    assert_eq!(reopened.edge_count().expect("count"), 3, "重开后边应还原");
    assert_eq!(
        reopened.get_node("a").expect("get").expect("node exists"),
        node_of("a", NodeType::Component, Some(json!({"v": 1}))),
        "重开后节点属性应完整还原"
    );

    for edge in &edges {
        reopened.add_edge(edge.clone()).expect("re-add edge");
    }
    assert_eq!(
        reopened.edge_count().expect("count"),
        3,
        "重开后重复写入已有边不应新增边（去重键已从库里重建）"
    );

    let mut ids: Vec<String> = reopened
        .iter_nodes()
        .expect("iter")
        .map(|n| n.id)
        .collect::<Vec<_>>();
    ids.sort();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    reopened.close().expect("close reopened");
}

/// 不手工 `close()`、直接 `drop` 后重开：数据与去重键同样要还原。
/// 持久化后端的失效模式不能只靠干净关库覆盖——调用方忘了 close 时，恢复
/// 走的是 `open()` 的 WAL 回放路径（`GrafeoDB` 的 `Drop` 会触发关闭/检查点，
/// 所以这里用 `drop(store)` 而不是 `store.close()` 模拟「调用方没调 close」）。
#[test]
fn drop_without_close_reopens_with_graph_and_dedup_keys() {
    let path = unique_db_path("noclose");
    {
        let mut store = GrafeoGraphStore::open(&path).expect("open store");
        store
            .upsert_node(node_of("a", NodeType::Component, Some(json!({"v": 1}))))
            .expect("upsert a");
        store
            .upsert_node(node_of("b", NodeType::Field, None))
            .expect("upsert b");
        store
            .add_edge(Edge {
                from: "a".to_string(),
                to: "b".to_string(),
                edge_type: EdgeType::Reads,
                field_path: Some("f1".to_string()),
                meta: None,
                origin_file: None,
            })
            .expect("add edge");
        // 关键：不调用 close()，让 store 走 Drop 释放文件锁。
        drop(store);
    }

    let mut reopened = GrafeoGraphStore::open(&path).expect("reopen after drop");
    assert_eq!(
        reopened.node_count().expect("count"),
        2,
        "drop 后节点应还原"
    );
    assert_eq!(reopened.edge_count().expect("count"), 1, "drop 后边应还原");
    assert_eq!(
        reopened.get_node("a").expect("get").expect("node exists"),
        node_of("a", NodeType::Component, Some(json!({"v": 1}))),
        "drop 后节点属性应完整还原"
    );
    // 去重键从库中重建：重放同一条边不应新增。
    reopened
        .add_edge(Edge {
            from: "a".to_string(),
            to: "b".to_string(),
            edge_type: EdgeType::Reads,
            field_path: Some("f1".to_string()),
            meta: None,
            origin_file: None,
        })
        .expect("re-add edge");
    assert_eq!(
        reopened.edge_count().expect("count"),
        1,
        "drop 后重放同一边不应新增（去重键已从库里重建）"
    );
    reopened.close().expect("close reopened");
}

/// 只读打开：能读到全量内容。
#[test]
fn read_only_open_sees_persisted_graph() {
    let path = unique_db_path("readonly");
    {
        let mut store = GrafeoGraphStore::open(&path).expect("open store");
        store
            .upsert_node(node_of("a", NodeType::Action, Some(json!({"x": "y"}))))
            .expect("upsert a");
        store
            .upsert_node(node_of("b", NodeType::Condition, None))
            .expect("upsert b");
        store
            .add_edge(Edge {
                from: "a".to_string(),
                to: "b".to_string(),
                edge_type: EdgeType::Triggers,
                field_path: None,
                meta: None,
                origin_file: None,
            })
            .expect("add edge");
        store.close().expect("close store");
    }

    let reader = GrafeoGraphStore::open_read_only(&path).expect("open read-only");
    assert_eq!(reader.node_count().expect("count"), 2);
    assert_eq!(reader.edge_count().expect("count"), 1);
    assert_eq!(
        reader.get_node("a").expect("get").expect("node exists"),
        node_of("a", NodeType::Action, Some(json!({"x": "y"}))),
    );
    let neighbors = reader
        .get_node_edges("a")
        .expect("edges")
        .expect("node exists");
    assert_eq!(neighbors.outgoing.len(), 1);
    assert_eq!(neighbors.outgoing[0].edge.edge_type, EdgeType::Triggers);
    reader.close().expect("close reader");
}

/// 打开不存在的只读库必须报 `GRAPH_DB_OPEN_FAILED`，而不是给一个空图。
#[test]
fn read_only_open_of_missing_file_fails() {
    let path = unique_db_path("missing").with_file_name("nope.grafeo");
    let err = GrafeoGraphStore::open_read_only(&path).expect_err("不存在的库不应打开成功");
    assert_eq!(err.code(), "GRAPH_DB_OPEN_FAILED");
}

/// 删除节点后计数以库为准：grafeo 的 `delete_node` 不级联删边，
/// 适配器必须自己先删光邻边，否则这里 `edge_count` 会残留。
#[test]
fn counts_follow_the_database_after_deletion() {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    for id in ["a", "b", "c"] {
        store
            .upsert_node(node_of(id, NodeType::Component, None))
            .expect("upsert");
    }
    for (from, to) in [("a", "b"), ("b", "c"), ("c", "a")] {
        store
            .add_edge(Edge {
                from: from.to_string(),
                to: to.to_string(),
                edge_type: EdgeType::Contains,
                field_path: None,
                meta: None,
                origin_file: None,
            })
            .expect("add edge");
    }
    assert_eq!(store.node_count().expect("count"), 3);
    assert_eq!(store.edge_count().expect("count"), 3);

    store
        .remove_nodes_by_ids(&["b".to_string()])
        .expect("remove");
    assert_eq!(store.node_count().expect("count"), 2);
    assert_eq!(
        store.edge_count().expect("count"),
        1,
        "b 的两条邻边应随 b 一起消失，只剩 c -> a"
    );
    let neighbors = store
        .get_node_edges("c")
        .expect("edges")
        .expect("node exists");
    assert_eq!(neighbors.outgoing.len(), 1);
    assert_eq!(neighbors.incoming.len(), 0, "b -> c 的入边应已消失");
}
