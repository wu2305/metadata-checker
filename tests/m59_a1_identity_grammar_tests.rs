//! M59-1（A1/A2）精确回归：
//!
//! 1. 跨页同名隔离——两个页面各自的 `model1` 编码出不同节点 id；
//! 2. 中文与分隔符——页面路径含中文、`\` 分隔符时归一化与编码一致；
//! 3. 路径点段与越界——`.` / `..` 消解，越出项目根报错而非静默截断；
//! 4. 旧 target 歧义——裸 `model:<local>` 在跨页同名（含旧全局节点混存）时
//!    交回全部候选；scoped id 精确命中；物理表裸名唯一命中保持旧行为。
//!
//! 身份写入与引用解析接线在 M59-2 的 schema 版本开关就绪后才启用；
//! 本文件只验收未启用的文法与解析实现。

use metadata_checker::graph::{EdgeType, Node, NodeType};
use metadata_checker::graph_identity::{
    IdentityError, ModelTargetResolution, NodeIdKind, normalize_project_path, page_local_node_id,
    resolve_model_target, resolve_relative_reference,
};
use metadata_checker::graph_store::GraphWriteStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;

fn model_node(id: &str, path: &str, name: &str) -> Node {
    Node {
        id: id.to_string(),
        node_type: NodeType::Model,
        path: path.to_string(),
        name: name.to_string(),
        meta: None,
    }
}

fn scoped_model(page: &str, local: &str) -> Node {
    let id = page_local_node_id(NodeIdKind::Model, page, local).unwrap();
    model_node(&id, page, local)
}

/// 回归 1：跨页同名隔离——同一局部名在两个页面必须解析为不同节点，
/// scoped id 各自精确命中，互不塌陷。
#[test]
fn scoped_ids_isolate_same_local_name_across_pages() {
    let page_a = scoped_model("app/绑定车辆.spg", "model6");
    let page_b = scoped_model("app/新增车辆.spg", "model6");

    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(page_a.clone()).unwrap();
    graph.upsert_node(page_b.clone()).unwrap();

    assert_ne!(page_a.id, page_b.id, "跨页同名局部模型必须得到不同节点 id");

    let target_a = page_local_node_id(NodeIdKind::Model, "app/绑定车辆.spg", "model6").unwrap();
    match resolve_model_target(&graph, &target_a).unwrap() {
        ModelTargetResolution::Unique(node) => assert_eq!(node.id, page_a.id),
        other => panic!("scoped target 应精确命中，实际 {:?}", other),
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
/// 候选按 id 排序，诊断顺序确定。
#[test]
fn bare_target_with_same_name_on_multiple_pages_answers_every_candidate() {
    let page_a = scoped_model("app/页面一.spg", "model1");
    let page_b = scoped_model("app/页面二.spg", "model1");
    // 过渡期混存：旧 schema 写入的全局 `model:model1` 仍在图中
    let legacy_global = model_node("model:model1", "legacy.spg", "model1");
    // 干扰项：不同局部名的 scoped 节点与物理表，不得进入候选
    let other_local = scoped_model("app/页面一.spg", "model11");
    let physical = model_node(
        "model:fact_testDrive",
        "data/fact_testDrive.tbl",
        "fact_testDrive",
    );

    let mut graph = MemoryGraphStore::new();
    for node in [&page_a, &page_b, &legacy_global, &other_local, &physical] {
        graph.upsert_node(node.clone()).unwrap();
    }

    match resolve_model_target(&graph, "model:model1").unwrap() {
        ModelTargetResolution::Ambiguous(candidates) => {
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

/// 回归 4b：物理表裸名唯一命中——物理表模型保持全局语义，旧行为不变。
#[test]
fn bare_physical_table_target_resolves_unique() {
    let physical = model_node(
        "model:fact_testDrive",
        "data/fact_testDrive.tbl",
        "fact_testDrive",
    );
    let mut graph = MemoryGraphStore::new();
    graph.upsert_node(physical.clone()).unwrap();
    graph
        .upsert_node(scoped_model("app/页面一.spg", "model1"))
        .unwrap();

    match resolve_model_target(&graph, "model:fact_testDrive").unwrap() {
        ModelTargetResolution::Unique(node) => {
            assert_eq!(node.id, "model:fact_testDrive");
            assert!(!node.id.contains('|'), "物理表节点不得携带页面段");
        }
        other => panic!("物理表裸名应唯一命中，实际 {:?}", other),
    }
}

/// 回归 4c：无候选与非法 target 返回 Missing，不构造幽灵节点。
#[test]
fn missing_and_invalid_targets_resolve_missing() {
    let graph = MemoryGraphStore::new();
    assert_eq!(
        resolve_model_target(&graph, "model:不存在").unwrap(),
        ModelTargetResolution::Missing
    );
    assert_eq!(
        resolve_model_target(&graph, "model:app/a.spg|model1").unwrap(),
        ModelTargetResolution::Missing,
        "scoped id 精确查表，查不到就是 Missing，不退化为裸名搜索"
    );
    assert_eq!(
        resolve_model_target(&graph, "field:model1.x").unwrap(),
        ModelTargetResolution::Missing,
        "非 model 前缀的 target 不进入模型解析"
    );
}

/// 共享目标归属规则（A1 侧的文法判据）：物理表节点保持全局、
/// 页面局部节点携带页面段；局部模型与物理表通过 DataflowInput 关联，
/// 删除牵连按 origin 的处理在 M59-2/A3 落地。
#[test]
fn shared_physical_target_stays_global_while_local_models_own_their_page() {
    let physical = model_node("model:fact_车辆", "主数据/fact_车辆.tbl", "fact_车辆");
    let local_a = scoped_model("app/绑定车辆.spg", "model6");
    let local_b = scoped_model("app/新增车辆.spg", "model7");

    let mut graph = MemoryGraphStore::new();
    for node in [&physical, &local_a, &local_b] {
        graph.upsert_node(node.clone()).unwrap();
    }
    // 同一物理表被两个页面的局部模型消费：共享目标，全局一份
    for local in [&local_a, &local_b] {
        graph
            .add_edge(metadata_checker::graph::Edge {
                from: local.id.clone(),
                to: physical.id.clone(),
                edge_type: EdgeType::DataflowInput,
                field_path: None,
                meta: None,
            })
            .unwrap();
    }

    // 裸物理名唯一命中全局节点；两个局部名各自唯一命中本页节点
    for (target, expected) in [
        ("model:fact_车辆", physical.id.as_str()),
        ("model:app/绑定车辆.spg|model6", local_a.id.as_str()),
        ("model:app/新增车辆.spg|model7", local_b.id.as_str()),
    ] {
        match resolve_model_target(&graph, target).unwrap() {
            ModelTargetResolution::Unique(node) => assert_eq!(node.id, expected),
            other => panic!("{} 应唯一命中，实际 {:?}", target, other),
        }
    }
}
