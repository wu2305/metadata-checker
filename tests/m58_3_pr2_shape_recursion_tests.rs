#![cfg(feature = "cli-local")]

//! M58.3 PR2 / spec F1「形态感知递归」+ 说明 D（ComponentProperty 图契约）测试。
//!
//! 判定规则（spec F1，与 `superpage::is_component_array` 同一份）：未知子键的 value
//! 是非空数组、且每个元素都是带字符串 `id` 和字符串 `type` 的对象 → 子组件数组，
//! 递归提取；混合形态数组整体判非组件并计入 `SCANNER_UNRECOGNIZED_CONTAINER_KEY`；
//! 排除列表四键（effectStyles/conditionStyles/labelFields/stateFields）不提取也不计数。

use metadata_checker::graph::EdgeType;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::superpage::{RefType, parse_superpage_from_value};

const REL_PATH: &str = "app/shape_test.spg";

/// 把一份原始 SPG JSON 扫进内存图存储
fn build_store(raw: serde_json::Value) -> MemoryGraphStore {
    let mut store = MemoryGraphStore::new();
    metadata_checker::scanner::process_spg_file_from_value(&mut store, REL_PATH, raw)
        .expect("process_spg_file_from_value should succeed");
    store
}

fn comp_node_id(component_id: &str) -> String {
    format!("comp:{}|{}", REL_PATH, component_id)
}

/// 按 code 查找诊断并返回其计数
fn diag_count(diags: &[metadata_checker::output::Diagnostic], code: &str) -> Option<usize> {
    diags.iter().find(|d| d.code == code).and_then(|d| d.count)
}

/// 形态吻合的未知键（自造 customSlots）→ 组件被提取，父子关系逐层正确
#[test]
fn shape_matched_unknown_key_extracts_components() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "customSlots": [
                        {"id": "slotBtn", "type": "button", "text": "=1+1"},
                        {"id": "slotPanel", "type": "panel", "components": [
                            {"id": "innerInput", "type": "input"}
                        ]}
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw).expect("parse should succeed");

    let find = |id: &str| meta.components.iter().find(|c| c.id == id);
    assert!(find("grid1").is_some(), "grid1 应被提取");
    let slot_btn = find("slotBtn").expect("customSlots 下的 slotBtn 应被提取");
    assert_eq!(slot_btn.parent_id.as_deref(), Some("grid1"));
    let slot_panel = find("slotPanel").expect("customSlots 下的 slotPanel 应被提取");
    assert_eq!(slot_panel.parent_id.as_deref(), Some("grid1"));
    let inner = find("innerInput").expect("新容器内的白名单子键仍应递归");
    assert_eq!(inner.parent_id.as_deref(), Some("slotPanel"));
    // 新提取组件的表达式照常进入表达式列表
    assert!(
        meta.expressions
            .iter()
            .any(|e| e.component_id == "slotBtn" && e.field == "text"),
        "slotBtn.text 表达式应被提取: {:?}",
        meta.expressions
    );
}

/// 形态吻合的未知键 → page→comp 与 comp→comp Contains 边都存在，且不计入 unrecognized
#[test]
fn shape_matched_unknown_key_builds_contains_edges() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "customSlots": [{"id": "slotBtn", "type": "button"}]
                }
            ]
        }
    });
    let store = build_store(raw);

    let slot_id = comp_node_id("slotBtn");
    let node = store
        .get_node(&slot_id)
        .expect("get_node")
        .expect("slotBtn 组件节点应存在");

    let neighbors = store
        .get_node_edges(&slot_id)
        .expect("get_node_edges")
        .expect("slotBtn 应有邻居");
    let contains_from: Vec<&str> = neighbors
        .incoming
        .iter()
        .filter(|v| v.edge.edge_type == EdgeType::Contains)
        .map(|v| v.edge.from.as_str())
        .collect();
    // page→comp 保留 + comp→comp 新边（说明 A：嵌套组件恰好一个 Component 父）
    let page_id = format!("page:{}", REL_PATH);
    assert!(
        contains_from.contains(&page_id.as_str()),
        "缺 page→comp Contains: {contains_from:?}"
    );
    assert!(
        contains_from.contains(&comp_node_id("grid1").as_str()),
        "缺 comp→comp Contains: {contains_from:?}"
    );
    // json_path 逐层保留到未知容器键
    assert_eq!(
        node.meta
            .as_ref()
            .and_then(|m| m.get("json_path"))
            .and_then(|v| v.as_str()),
        Some("canvas.components[0].customSlots[0]")
    );

    // 形态吻合的未知键不再误计 SCANNER_UNRECOGNIZED_CONTAINER_KEY
    let raw2 = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "customSlots": [{"id": "slotBtn", "type": "button"}]
                }
            ]
        }
    });
    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw2);
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        None,
        "形态吻合键不得计入 unrecognized: {diags:?}"
    );
}

/// 排除列表四键（附录 A 初值）+ actions：即使形态吻合也不提取、不计 unrecognized
#[test]
fn excluded_keys_are_neither_extracted_nor_counted() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "form1",
                    "type": "form",
                    "effectStyles": [{"id": "effectStyle1", "type": "hover"}],
                    "conditionStyles": [{"id": "condStyle1", "type": "active"}],
                    "labelFields": [{"id": "labelField1", "type": "label"}],
                    "stateFields": [{"id": "stateField1", "type": "state"}],
                    "actions": [{"id": "act1", "actionType": "link", "panel": "nextPage"}]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    for excluded_id in [
        "effectStyle1",
        "condStyle1",
        "labelField1",
        "stateField1",
        "act1",
    ] {
        assert!(
            !meta.components.iter().any(|c| c.id == excluded_id),
            "排除列表/动作容器元素不得提取为组件: {excluded_id}"
        );
    }
    assert_eq!(
        meta.components.len(),
        1,
        "只剩 form1: {:?}",
        meta.components
    );

    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw);
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        None,
        "排除列表键不得计入 unrecognized: {diags:?}"
    );
}

/// 混合形态数组（附录 A 真实样本形态：conditionStyles 有 15 处「部分元素缺 id/type」）
/// → 整体判非组件，不计组件、计入 unrecognized 一次
#[test]
fn mixed_shape_array_counts_unrecognized_once() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "mixedWidgets": [
                        {"id": "ok1", "type": "button"},
                        {"style": {"color": "red"}}
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    assert!(
        !meta.components.iter().any(|c| c.id == "ok1"),
        "混合形态数组整体判非组件，ok1 不得提取"
    );

    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw);
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        Some(1),
        "混合形态数组应计入 unrecognized 一次: {diags:?}"
    );
}

/// 字符串形态的多态键（如 action 的 panel: "nextPage" / 组件级 panel 字符串）
/// 天然不匹配组件形态，不产生任何副作用
#[test]
fn string_shaped_polymorphic_keys_have_no_side_effects() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "btn1",
                    "type": "button",
                    "panel": "nextPage",
                    "actions": [
                        {"id": "act1", "actionType": "switchPanel", "panel": "nextPage"}
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    assert_eq!(
        meta.components.len(),
        1,
        "不应提取新组件: {:?}",
        meta.components
    );

    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw);
    assert!(
        diags.is_empty(),
        "字符串形态键不得产生任何扫描诊断: {diags:?}"
    );
}

/// json_path 逐层保留：未知容器键的每一层都进入 json_path
#[test]
fn json_path_preserved_layer_by_layer() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "customSlots": [
                        {
                            "id": "slotPanel",
                            "type": "panel",
                            "components": [
                                {"id": "innerA", "type": "input"},
                                {"id": "innerB", "type": "input"}
                            ]
                        }
                    ]
                }
            ]
        }
    });
    let store = build_store(raw);
    let json_path_of = |id: &str| {
        store
            .get_node(&comp_node_id(id))
            .expect("get_node")
            .unwrap_or_else(|| panic!("节点应存在: {id}"))
            .meta
            .and_then(|m| m.get("json_path")?.as_str().map(|s| s.to_string()))
    };
    assert_eq!(
        json_path_of("slotPanel").as_deref(),
        Some("canvas.components[0].customSlots[0]")
    );
    assert_eq!(
        json_path_of("innerB").as_deref(),
        Some("canvas.components[0].customSlots[0].components[1]")
    );
}

/// 说明 D 贯通：parse → 上下文解析（ModelField 改写为 ComponentProperty）→ scanner 建
/// comp→comp DependsOn 边并带属性名，与 dependency.rs:48 语义对齐。
/// 同时覆盖 F1 交互：引用目标是形态感知递归新发现的组件 id。
#[test]
fn component_property_builds_depends_on_edge_end_to_end() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "txtB",
                    "type": "text",
                    "customSlots": [{"id": "slotX", "type": "input"}]
                },
                {"id": "exprA", "type": "text", "value": "${txtB.txt}"},
                {"id": "exprC", "type": "text", "value": "${slotX.seconds}"}
            ]
        }
    });

    // 1) parse → 上下文解析：head 命中已抽取组件 id 的 ModelField 被改写为 ComponentProperty
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    let resolved_of = |component_id: &str| -> Vec<RefType> {
        meta.expressions
            .iter()
            .filter(|e| e.component_id == component_id)
            .flat_map(|e| e.resolved_refs.iter().map(|r| r.ref_type.clone()))
            .collect()
    };
    assert!(
        resolved_of("exprA").iter().any(
            |r| matches!(r, RefType::ComponentProperty(id, prop) if id == "txtB" && prop == "txt")
        ),
        "exprA 应解析出 ComponentProperty(txtB, txt): {:?}",
        resolved_of("exprA")
    );
    assert!(
        resolved_of("exprC")
            .iter()
            .any(|r| matches!(r, RefType::ComponentProperty(id, prop) if id == "slotX" && prop == "seconds")),
        "F1 新提取的 slotX 应进入改写集合: {:?}",
        resolved_of("exprC")
    );

    // 2) scanner 建边：comp→comp DependsOn，field_path 带属性名
    let store = build_store(raw);
    let assert_depends_on = |from: &str, to: &str, expected_field_path: &str| {
        let neighbors = store
            .get_node_edges(&comp_node_id(from))
            .expect("get_node_edges")
            .unwrap_or_else(|| panic!("{from} 应有邻居"));
        let hit = neighbors.outgoing.iter().any(|v| {
            v.edge.edge_type == EdgeType::DependsOn
                && v.edge.to == comp_node_id(to)
                && v.edge.field_path.as_deref() == Some(expected_field_path)
        });
        assert!(
            hit,
            "缺 DependsOn 边 {from} -> {to} ({expected_field_path}): {:?}",
            neighbors
                .outgoing
                .iter()
                .map(|v| (&v.edge.to, &v.edge.field_path))
                .collect::<Vec<_>>()
        );
    };
    assert_depends_on("exprA", "txtB", "comp:txtB.txt");
    assert_depends_on("exprC", "slotX", "comp:slotX.seconds");
}

/// 重复组件 id 诊断不回归：白名单内重复照常计数；形态感知递归新遍历到的
/// 容器里的重复 id 也进入同一诊断（spg.rs 诊断分支不变，只是遍历面变大）
#[test]
fn duplicate_component_id_diagnostic_unchanged() {
    // 白名单内重复：既有行为不回归
    let raw_whitelist = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "dup1", "type": "button"},
                {"id": "dup1", "type": "input"}
            ]
        }
    });
    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw_whitelist);
    assert_eq!(
        diag_count(&diags, "SCANNER_DUPLICATE_COMPONENT_ID"),
        Some(1),
        "白名单内重复 id 应计数一次: {diags:?}"
    );

    // 跨容器重复（第二处位于形态吻合的未知键下）：递归后同样被诊断捕获
    let raw_cross = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "dup2", "type": "button"},
                {"id": "wrap", "type": "panel", "customSlots": [
                    {"id": "dup2", "type": "input"}
                ]}
            ]
        }
    });
    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw_cross);
    assert_eq!(
        diag_count(&diags, "SCANNER_DUPLICATE_COMPONENT_ID"),
        Some(1),
        "新容器内的重复 id 应被同一诊断捕获: {diags:?}"
    );
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        None,
        "形态吻合键不得误计 unrecognized: {diags:?}"
    );
}

/// 前向引用回归（ComponentProperty）：引用方组件排在被引用组件之前时，
/// comp→comp DependsOn 边不得丢失。修复前 scanner 单遍顺序处理组件，
/// 目标节点尚未注册时 add_edge 会静默丢边（见 spg.rs 两遍拆分注释）。
#[test]
fn component_property_forward_reference_builds_depends_on_edge() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "exprA", "type": "text", "value": "${txtB.txt}"},
                {"id": "txtB", "type": "text"}
            ]
        }
    });
    let store = build_store(raw);

    // 目标组件节点本身应正常注册
    assert!(
        store
            .get_node(&comp_node_id("txtB"))
            .expect("get_node")
            .is_some(),
        "txtB 组件节点应存在"
    );
    let neighbors = store
        .get_node_edges(&comp_node_id("exprA"))
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("exprA 应有邻居"));
    let hit = neighbors.outgoing.iter().any(|v| {
        v.edge.edge_type == EdgeType::DependsOn
            && v.edge.to == comp_node_id("txtB")
            && v.edge.field_path.as_deref() == Some("comp:txtB.txt")
    });
    assert!(
        hit,
        "前向引用不得丢边: 缺 DependsOn exprA -> txtB (comp:txtB.txt): {:?}",
        neighbors
            .outgoing
            .iter()
            .map(|v| (&v.edge.to, &v.edge.field_path))
            .collect::<Vec<_>>()
    );
}

/// 前向引用回归（裸 `${id}` 全组件引用）：解析层把裸 `${txtB}` 先判为
/// ModelField("txtB", "")，因 txtB 是已知组件 id 且 field 为空，在 parse 层
/// 已归一为 ComponentValue("txtB")（superpage/mod.rs resolve_ref_type），
/// scanner 侧走 ComponentValue 臂建 field_path comp:txtB.value。
/// 引用方排在被引用组件之前时，DependsOn 边不得丢失。
#[test]
fn component_value_forward_reference_builds_depends_on_edge() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "exprA", "type": "text", "value": "${txtB}"},
                {"id": "txtB", "type": "text"}
            ]
        }
    });
    let store = build_store(raw);

    let neighbors = store
        .get_node_edges(&comp_node_id("exprA"))
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("exprA 应有邻居"));
    let hit = neighbors.outgoing.iter().any(|v| {
        v.edge.edge_type == EdgeType::DependsOn
            && v.edge.to == comp_node_id("txtB")
            && v.edge.field_path.as_deref() == Some("comp:txtB.value")
    });
    assert!(
        hit,
        "前向引用不得丢边: 缺 DependsOn exprA -> txtB (comp:txtB.value): {:?}",
        neighbors
            .outgoing
            .iter()
            .map(|v| (&v.edge.to, &v.edge.field_path))
            .collect::<Vec<_>>()
    );
}
