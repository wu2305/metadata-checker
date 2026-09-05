#![cfg(feature = "cli-local")]

//! M58.3 相邻缺口修复回归（评审外新发现，2026-09-05 立项）。
//!
//! 覆盖四个缺口：
//! - A：动作条件 / source filter 的 `referenced_symbols` 未经页面上下文归一
//!   （`${txtB.txt}` 产出 `model:txtB.txt`，指向不存在的节点）；
//! - B：裸 `${modelN}` 的空字段名产出 `model:modelN.` 尾点符号与 `field:modelN.`
//!   垃圾节点；
//! - C：cond→param / user / system 三类目标节点从未创建，边被存储层整类丢弃；
//! - D：`canvas.panels` 等非 `canvas.components` 分支的组件 json_path 退回
//!   `canvas.components[id='...']` 合成串，违反 raw JSON locator 契约。

use metadata_checker::conditions::{ConditionRecord, scan_conditions};
use metadata_checker::graph::NodeType;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::superpage::parse_superpage_from_value;

const REL_PATH: &str = "app/gap.spg";

fn build_store(raw: serde_json::Value) -> MemoryGraphStore {
    let mut store = MemoryGraphStore::new();
    metadata_checker::scanner::process_spg_file_from_value(&mut store, REL_PATH, raw)
        .expect("process_spg_file_from_value should succeed");
    store
}

/// canvas 直挂 panels，动作条件引用组件属性 / 模型 / 参数 / 用户变量。
fn fixture_page() -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "params": [{ "id": "param1", "name": "订单号" }],
        "sources": [{ "id": "model1", "modelType": "table", "path": "data/order.tbl" }],
        "canvas": {
            "id": "canvas1",
            "type": "Canvas",
            "panels": [{
                "id": "panel1",
                "type": "Panel",
                "components": [
                    { "id": "txtB", "type": "text" },
                    {
                        "id": "btn1",
                        "type": "button",
                        "actions": [
                            { "id": "act1", "actionType": "Submit", "conditionExp": "${txtB.txt} == '1'" },
                            { "id": "act2", "actionType": "Submit",
                              "conditionExp": "${model1} != null && ${param1} != null && ${$user.deptId} != null" }
                        ]
                    }
                ]
            }]
        }
    })
}

fn record<'a>(records: &'a [ConditionRecord], condition_id: &str) -> &'a ConditionRecord {
    records
        .iter()
        .find(|r| r.condition_id == condition_id)
        .unwrap_or_else(|| {
            panic!(
                "未找到条件记录 {condition_id}: {:?}",
                records.iter().map(|r| &r.condition_id).collect::<Vec<_>>()
            )
        })
}

/// 缺口 A：动作条件里的 `${txtB.txt}` 必须归一为 component 符号，而不是 model。
/// 归一前它是 `model:txtB.txt`，在图里指向一个从不存在的节点。
#[test]
fn action_condition_symbols_are_normalized_with_page_context() {
    let spg = parse_superpage_from_value(fixture_page()).expect("解析页面");
    let records = scan_conditions(&spg, Some(REL_PATH));
    assert_eq!(
        record(&records, "btn1#act1#conditionExp").referenced_symbols,
        vec!["component:txtB.txt".to_string()],
        "txtB 是已知组件，条件符号必须归一为 component 而不是 model"
    );
}

/// 缺口 A + B：模型 / 参数 / 用户变量三类符号各自归位，且没有任何尾点符号。
#[test]
fn action_condition_symbols_cover_model_param_user_without_trailing_dot() {
    let spg = parse_superpage_from_value(fixture_page()).expect("解析页面");
    let records = scan_conditions(&spg, Some(REL_PATH));
    let symbols = &record(&records, "btn1#act2#conditionExp").referenced_symbols;
    assert!(
        symbols.contains(&"model:model1".to_string()),
        "裸 ${{model1}} 必须是 model:model1，不能带尾点: {symbols:?}"
    );
    assert!(
        !symbols.iter().any(|s| s.ends_with('.')),
        "任何符号都不得以 '.' 结尾: {symbols:?}"
    );
    assert!(
        symbols.contains(&"param:param1".to_string()),
        "已知页面参数必须归一为 param 符号: {symbols:?}"
    );
    assert!(
        symbols.iter().any(|s| s.starts_with("user:")),
        "$user.deptId 必须归一为 user 符号: {symbols:?}"
    );
}

/// 缺口 D：canvas.panels 下的组件必须拿到真实 json_path，不得退回合成串。
#[test]
fn action_condition_json_path_is_a_real_raw_json_locator() {
    let spg = parse_superpage_from_value(fixture_page()).expect("解析页面");
    let records = scan_conditions(&spg, Some(REL_PATH));
    assert_eq!(
        record(&records, "btn1#act1#conditionExp").json_path,
        "canvas.panels[0].components[1].actions[0].conditionExp",
        "json_path 必须能在原始 JSON 里逐段定位到"
    );
    let synthetic: Vec<&String> = records
        .iter()
        .map(|r| &r.json_path)
        .filter(|p| p.contains("canvas.components[id="))
        .collect();
    assert!(
        synthetic.is_empty(),
        "不得出现 canvas.components[id='...'] 合成兜底: {synthetic:?}"
    );
}

/// 缺口 B：裸 `${modelN}` 不得再生成 `field:modelN.` 尾点垃圾节点，模型级节点保留。
#[test]
fn bare_model_reference_creates_no_trailing_dot_field_node() {
    let store = build_store(serde_json::json!({
        "version": "1.0",
        "sources": [{ "id": "model1", "modelType": "table", "path": "data/order.tbl" }],
        "canvas": {
            "id": "canvas1",
            "type": "Canvas",
            "components": [{ "id": "txt1", "type": "text", "exp": "${model1}" }]
        }
    }));
    let dotted: Vec<String> = store
        .iter_nodes()
        .expect("iter_nodes")
        .map(|n| n.id)
        .filter(|id| id.starts_with("field:") && id.ends_with('.'))
        .collect();
    assert!(dotted.is_empty(), "不得存在尾点 field 节点: {dotted:?}");
    assert!(
        store.get_node("model:model1").expect("get_node").is_some(),
        "模型级节点仍必须存在"
    );
}

/// 缺口 C：只出现在条件里的 param / user / system 符号，其目标节点必须被创建，
/// 否则边会被存储层静默丢弃（三个类别整类悬挂）。
#[test]
fn condition_only_param_user_system_targets_get_nodes_and_edges() {
    let store = build_store(serde_json::json!({
        "version": "1.0",
        "params": [{ "id": "param1", "name": "订单号" }],
        "canvas": {
            "id": "canvas1",
            "type": "Canvas",
            "components": [{
                "id": "btn1",
                "type": "button",
                "actions": [{
                    "id": "act1",
                    "actionType": "Submit",
                    "conditionExp": "${param1} != null && ${$user.deptId} != null"
                }]
            }]
        }
    }));

    let param_node = format!("param:{}|param1", REL_PATH);
    let param = store
        .get_node(&param_node)
        .expect("get_node")
        .unwrap_or_else(|| panic!("只在条件里出现的 param 也必须有节点: {param_node}"));
    assert_eq!(param.node_type, NodeType::Field, "与组件表达式路径同型");

    let user_nodes: Vec<String> = store
        .iter_nodes()
        .expect("iter_nodes")
        .map(|n| n.id)
        .filter(|id| id.starts_with("user:"))
        .collect();
    assert!(!user_nodes.is_empty(), "user 目标节点必须存在");

    let cond_id = format!("cond:{}|btn1#act1#conditionExp", REL_PATH);
    let neighbors = store
        .get_node_edges(&cond_id)
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("条件节点应存在: {cond_id}"));
    let targets: Vec<&str> = neighbors
        .outgoing
        .iter()
        .map(|v| v.edge.to.as_str())
        .collect();
    assert!(
        targets.contains(&param_node.as_str()),
        "cond→param 边必须落库（修复前目标节点不存在，边被丢弃）: {targets:?}"
    );
    assert!(
        targets.iter().any(|t| t.starts_with("user:")),
        "cond→user 边必须落库: {targets:?}"
    );
}
