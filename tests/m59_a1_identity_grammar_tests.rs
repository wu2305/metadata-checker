//! M59-1（A1/A2）精确回归：
//!
//! 1. 跨页同名隔离——两个页面各自的 `model1`/`field1` 编码出不同节点 id；
//! 2. 中文与分隔符——页面路径含中文、`\` 分隔符时归一化与编码一致；
//! 3. 路径点段与越界——`.` / `..` 消解，越出项目根报错而非静默截断；
//! 4. 旧 target 歧义——裸 `model:` / `field:` 在跨页同名（含旧全局节点混存）时
//!    交回全部候选；scoped id 精确命中；物理表/物理字段裸名唯一命中保持旧行为；
//!    跨 kind 同名互不干扰。
//!
//! 身份写入与引用解析接线在 M59-2 的 schema 版本开关就绪后才启用；
//! 本文件只验收未启用的文法与解析实现。

use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_identity::{
    IdentityError, NodeIdKind, TargetResolution, normalize_project_path, page_local_node_id,
    resolve_node_target, resolve_relative_reference,
};
use metadata_checker::graph_store::GraphWriteStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;

fn node_of(kind: NodeIdKind, id: &str, path: &str, name: &str) -> Node {
    Node {
        id: id.to_string(),
        node_type: match kind {
            NodeIdKind::Model => NodeType::Model,
            NodeIdKind::Field => NodeType::Field,
            _ => panic!("本文件只构造 Model/Field 节点"),
        },
        path: path.to_string(),
        name: name.to_string(),
        meta: None,
    ,     origin_file: None,
origin_file: None}
}

fn scoped_node(kind: NodeIdKind, page: &str, local: &str) -> Node {
    let id = page_local_node_id(kind, page, local).unwrap();
    node_of(kind, &id, page, local)
    origin_file: None,
}

fn resolve_model(graph: &MemoryGraphStore, target: &str) -> TargetResolution {
    resolve_node_target(graph, NodeIdKind::Model, target).unwrap()
}

fn resolve_field(graph: &MemoryGraphStore, target: &str) -> TargetResolution {
    resolve_node_target(graph, NodeIdKind::Field, target).unwrap()
}

/// 回归 1：跨页同名隔离——同一局部名在两个页面必须解析为不同节点，
/// scoped id 各自精确命中，互不塌陷。
#[test]
fn scoped_ids_isolate_same_local_name_across_pages() {
    let page_a = scoped_node(NodeIdKind::Model, "app/绑定车辆.spg", "model6");
    let page_b = scoped_node(NodeIdKind::Model, "app/新增车辆.spg", "model6");

    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(page_a.clone()).unwrap();
    graph.upsert_node(page_b.clone()).unwrap();

    assert_ne!(page_a.id, page_b.id, "跨页同名局部模型必须得到不同节点 id");

    let target_a = page_local_node_id(NodeIdKind::Model, "app/绑定车辆.spg", "model6").unwrap();
    match resolve_model(&graph, &target_a) {
        TargetResolution::Unique(node) => assert_eq!(node.id, page_a.id),
        other => panic!("scoped target 应精确命中，实际 {:?}", other),
    }
}

/// 回归 1（field 半边）：页面局部字段同样按页隔离，scoped 精确命中。
#[test]
fn scoped_field_ids_isolate_same_local_name_across_pages() {
    let field_a = scoped_node(NodeIdKind::Field, "app/页面一.spg", "model1.金额");
    let field_b = scoped_node(NodeIdKind::Field, "app/页面二.spg", "model1.金额");

    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(field_a.clone()).unwrap();
    graph.upsert_node(field_b.clone()).unwrap();

    assert_ne!(field_a.id, field_b.id, "跨页同名字段必须得到不同节点 id");

    let target = page_local_node_id(NodeIdKind::Field, "app/页面二.spg", "model1.金额").unwrap();
    match resolve_field(&graph, &target) {
        TargetResolution::Unique(node) => assert_eq!(node.id, field_b.id),
        other => panic!("scoped field target 应精确命中，实际 {:?}", other),
    }
}

/// 回归 2 + 3：中文与 `\` 分隔符、点段消解、越界报错。
#[test]
fn normalization_handles_chinese_separators_dot_segments_and_escape() {
    // 中文 + 反斜杠 + 点段收敛到同一路径
    assert_eq!(
        normalize_project_path("app\\绑定车辆\\./子页.spg").unwrap(),
        "app/绑定车辆/子页.spg"
    );
    // 相对引用跨目录上跳
    assert_eq!(
        resolve_relative_reference("app/绑定车辆/页面.spg", "../../主数据/fact_车辆.tbl").unwrap(),
        "主数据/fact_车辆.tbl"
    );
    // 越界：报错而非静默截断（旧实现 app/../b.spg ≠ 扫描产物 b.spg 的病根）
    assert_eq!(
        resolve_relative_reference("页面.spg", "../escape.spg"),
        Err(IdentityError::EscapeBeyondRoot {
            path: "../escape.spg".to_string()
        })
    );
    assert_eq!(
        normalize_project_path("app/../../escape.spg"),
        Err(IdentityError::EscapeBeyondRoot {
            path: "app/../../escape.spg".to_string()
        })
    );
}

/// 回归 4a：裸 `model:<local>` 跨页同名 → 交回全部候选（含旧全局节点混存），
/// 候选按 id 升序，诊断顺序确定。
#[test]
fn bare_target_with_same_name_on_multiple_pages_answers_every_candidate() {
    let page_a = scoped_node(NodeIdKind::Model, "app/页面一.spg", "model1");
    let page_b = scoped_node(NodeIdKind::Model, "app/页面二.spg", "model1");
    // 过渡期混存：旧 schema 写入的全局 `model:model1` 仍在图中
    let legacy_global = node_of(NodeIdKind::Model, "model:model1", "legacy.spg", "model1");
    // 干扰项：不同局部名的 scoped 节点与物理表，不得进入候选
    let other_local = scoped_node(NodeIdKind::Model, "app/页面一.spg", "model11");
    let physical = node_of(
        NodeIdKind::Model,
        "model:fact_testDrive",
        "data/fact_testDrive.tbl",
        "fact_testDrive",
    );

    let mut graph = MemoryGraphStore::new();
    for node in [&page_a, &page_b, &legacy_global, &other_local, &physical] {
        graph.upsert_node(node.clone()).unwrap();
    }

    match resolve_model(&graph, "model:model1") {
        TargetResolution::Ambiguous(candidates) => {
            let ids: Vec<&str> = candidates.iter().map(|n| n.id.as_str()).collect();
            assert_eq!(
                ids,
                vec![
                    "model:app/页面一.spg|model1",
                    "model:app/页面二.spg|model1",
                    "model:model1",
                ],
                "歧义时必须交回全部候选（旧全局 + 两个页面局部），且按 id 升序"
            );
        }
        other => panic!("跨页同名裸 target 必须报歧义，实际 {:?}", other),
    }
}

/// 回归 4a（field 半边）：裸 `field:<local>` 同样交回全部候选，
/// 与物理字段全局节点混存时同理。
#[test]
fn bare_field_target_with_same_name_answers_every_candidate() {
    let field_a = scoped_node(NodeIdKind::Field, "app/页面一.spg", "model1.金额");
    let field_b = scoped_node(NodeIdKind::Field, "app/页面二.spg", "model1.金额");
    let legacy_global = node_of(NodeIdKind::Field, "field:model1.金额", "legacy.spg", "金额");

    let mut graph = MemoryGraphStore::new();
    for node in [&field_a, &field_b, &legacy_global] {
        graph.upsert_node(node.clone()).unwrap();
    }

    match resolve_field(&graph, "field:model1.金额") {
        TargetResolution::Ambiguous(candidates) => {
            let ids: Vec<&str> = candidates.iter().map(|n| n.id.as_str()).collect();
            assert_eq!(
                ids,
                vec![
                    "field:app/页面一.spg|model1.金额",
                    "field:app/页面二.spg|model1.金额",
                    "field:model1.金额",
                ],
                "field 裸 target 歧义时必须交回全部候选，且按 id 升序"
            );
        }
        other => panic!("跨页同名字段裸 target 必须报歧义，实际 {:?}", other),
    }
}

/// 回归 4b：物理表/物理字段裸名唯一命中——全局聚合语义保持，旧行为不变。
#[test]
fn bare_physical_targets_resolve_unique() {
    let physical = node_of(
        NodeIdKind::Model,
        "model:fact_testDrive",
        "data/fact_testDrive.tbl",
        "fact_testDrive",
    );
    let physical_field = node_of(
        NodeIdKind::Field,
        "field:fact_testDrive.金额",
        "data/fact_testDrive.tbl",
        "金额",
    );
    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(physical.clone()).unwrap();
    graph.upsert_node(physical_field.clone()).unwrap();
    graph
        .upsert_node(scoped_node(NodeIdKind::Model, "app/页面一.spg", "model1"))
        .unwrap();

    match resolve_model(&graph, "model:fact_testDrive") {
        TargetResolution::Unique(node) => {
            assert_eq!(node.id, "model:fact_testDrive");
            assert!(!node.id.contains('|'), "物理表节点不得携带页面段");
        }
        other => panic!("物理表裸名应唯一命中，实际 {:?}", other),
    }
    match resolve_field(&graph, "field:fact_testDrive.金额") {
        TargetResolution::Unique(node) => assert_eq!(node.id, "field:fact_testDrive.金额"),
        other => panic!("物理字段裸名应唯一命中，实际 {:?}", other),
    }
}

/// 回归 4c：无候选与非法 target 返回 Missing，不构造幽灵节点；
/// scoped id 查表 MISS 不退化为裸名搜索。
#[test]
fn missing_and_invalid_targets_resolve_missing() {
    let graph = MemoryGraphStore::new();
    assert_eq!(
        resolve_model(&graph, "model:不存在"),
        TargetResolution::Missing
    );
    assert_eq!(
        resolve_model(&graph, "model:app/a.spg|model1"),
        TargetResolution::Missing,
        "scoped id 精确查表，查不到就是 Missing，不退化为裸名搜索"
    );
    assert_eq!(
        resolve_model(&graph, "field:model1.x"),
        TargetResolution::Missing,
        "kind 前缀不匹配的 target 不进入解析"
    );
    assert_eq!(
        resolve_node_target(&graph, NodeIdKind::Cond, "cond:some_cond").unwrap(),
        TargetResolution::Missing,
        "cond/comp/action/param 文法恒为 scoped，裸名解析未定义"
    );
}

/// 回归 4d：跨 kind 同名互不干扰——`model:x` 与 `field:x` 各自只命中本 kind。
#[test]
fn bare_targets_do_not_cross_kinds() {
    let model = node_of(NodeIdKind::Model, "model:共享名", "a.spg", "共享名");
    let field = node_of(NodeIdKind::Field, "field:共享名", "a.spg", "共享名");
    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(model.clone()).unwrap();
    graph.upsert_node(field.clone()).unwrap();

    match resolve_model(&graph, "model:共享名") {
        TargetResolution::Unique(node) => assert_eq!(node.node_type, NodeType::Model),
        other => panic!("model 裸名不得命中 field 节点，实际 {:?}", other),
    }
    match resolve_field(&graph, "field:共享名") {
        TargetResolution::Unique(node) => assert_eq!(node.node_type, NodeType::Field),
        other => panic!("field 裸名不得命中 model 节点，实际 {:?}", other),
    }
}

/// 文法不变式（评审 P1-4 登记）：竖线是「页面局部 vs 全局」的唯一判据，
/// id 段不得含 `|`；含 `|` 的全局名在文法之外——解析按第一个 `|` 切分，
/// 不得产生第三种解读。
#[test]
fn pipe_is_the_scoped_discriminator_by_grammar() {
    let parsed = metadata_checker::graph_identity::parse_node_id("model:app/a.spg|model1").unwrap();
    assert!(parsed.page.is_some());
    assert!(parsed.local.find('|').is_none(), "局部名内不得再含竖线");

    // 全局名含 `|` 属文法之外：解析仍按第一个 `|` 切分（页面段非空即 scoped），
    // 不静默 reinterpret 为全局名
    let outside = metadata_checker::graph_identity::parse_node_id("model:we|ird").unwrap();
    assert_eq!(outside.page.as_deref(), Some("we"));
    assert_eq!(outside.local, "ird");
}

/// 共享目标归属规则（A1 侧的文法判据）：物理表节点保持全局、
/// 页面局部节点携带页面段；局部模型与物理表通过 DataflowInput 关联，
/// 删除牵连按 origin 的处理在 M59-2/A3 落地。
#[test]
fn shared_physical_target_stays_global_while_local_models_own_their_page() {
    let physical = node_of(
        NodeIdKind::Model,
        "model:fact_车辆",
        "主数据/fact_车辆.tbl",
        "fact_车辆",
    );
    let local_a = scoped_node(NodeIdKind::Model, "app/绑定车辆.spg", "model6");
    let local_b = scoped_node(NodeIdKind::Model, "app/新增车辆.spg", "model7");

    let mut graph = MemoryGraphStore::new();
    for node in [&physical, &local_a, &local_b] {
        graph.upsert_node(node.clone()).unwrap();
    }
    // 同一物理表被两个页面的局部模型消费：共享目标，全局一份
    for local in [&local_a, &local_b] {
        graph
            .add_edge(Edge {
                from: local.id.clone(),
                to: physical.id.clone(),
                edge_type: EdgeType::DataflowInput,
                field_path: None,
                meta: None,
            origin_file: None})
            .unwrap();
    }

    // 裸物理名唯一命中全局节点；两个局部名各自唯一命中本页节点
    for (target, expected) in [
        ("model:fact_车辆", physical.id.as_str()),
        ("model:app/绑定车辆.spg|model6", local_a.id.as_str()),
        ("model:app/新增车辆.spg|model7", local_b.id.as_str()),
    ] {
        match resolve_model(&graph, target) {
            TargetResolution::Unique(node) => assert_eq!(node.id, expected),
            other => panic!("{} 应唯一命中，实际 {:?}", target, other),
        }
    }
}
