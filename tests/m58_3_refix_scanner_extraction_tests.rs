#![cfg(feature = "cli-local")]

//! M58.3 复核返修测试电池，覆盖 hostile review 的 4 个 P1：
//!
//! - P1-4：cond→upstream `component:{id}.{prop}` 符号边剥掉属性后缀（节点 id 只到
//!   组件 id），属性名保留在边 meta 的 `target_property`，边不再因目标节点不存在
//!   被存储层静默丢弃。
//! - P1-5：`scan_conditions` 的动作条件驱动源从第三份裸 JSON 递归改为 superpage
//!   提取结果——extra 键组件（operateButtons/columns 等）的动作条件全覆盖，
//!   json_path 取自同一形态判定的 `collect_json_paths`。
//! - P1-6：裸 `${paramN}` 归一为 Param（命名启发式 + 页面 param 上下文两条路径），
//!   不再产出 `model:paramN` 垃圾节点与 `field:paramN.` 尾点节点，也不再被误判为
//!   继承 dataSet 的裸字段 Reads。
//! - P1-7：scanner 组件上下文注册对齐提取侧口径（id 与 type 同时非空才是组件）；
//!   id 有 type 无的对象不计为组件、其子组件 parent 透传祖父，并按「对象形态
//!   未识别」计入既有 SCANNER_UNRECOGNIZED_CONTAINER_KEY 安全网。

use metadata_checker::conditions::{ConditionType, OwnerType, scan_conditions};
use metadata_checker::graph::EdgeType;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::superpage::{
    RefType, parse_expression_refs, parse_superpage, parse_superpage_from_value,
};

const REL_PATH: &str = "app/refix.spg";

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

fn cond_node_id(condition_id: &str) -> String {
    format!("cond:{}|{}", REL_PATH, condition_id)
}

/// 按 code 查找诊断并返回其计数
fn diag_count(diags: &[metadata_checker::output::Diagnostic], code: &str) -> Option<usize> {
    diags.iter().find(|d| d.code == code).and_then(|d| d.count)
}

/// P1-4：cond→upstream 的 `component:{id}.{prop}` 符号边落到真实组件节点，
/// 边 meta 携带 `target_property`；裸 `component:{id}` 符号不带属性名。
#[test]
fn cond_to_component_symbol_edge_lands_with_target_property() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "txtB", "type": "text"},
                {"id": "exprA", "type": "text", "value": "${txtB.txt}"},
                {"id": "exprC", "type": "text", "value": "${txtB}"}
            ]
        }
    });
    let store = build_store(raw);

    // 带属性后缀：cond(exprA#value#0) → comp txtB，field_path 保留完整符号串，
    // meta.target_property 记录属性名
    let cond_a = cond_node_id("exprA#value#0");
    assert!(
        store
            .get_node(&cond_a)
            .expect("get_node")
            .is_some(),
        "cond 节点应存在: {cond_a}"
    );
    let neighbors = store
        .get_node_edges(&cond_a)
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("{cond_a} 应有邻居"));
    let prop_edge = neighbors.outgoing.iter().find(|v| {
        v.edge.edge_type == EdgeType::DependsOn && v.edge.to == comp_node_id("txtB")
    });
    let prop_edge = prop_edge.unwrap_or_else(|| {
        panic!(
            "缺 cond→comp DependsOn 边（P1-4 修复前目标节点 id 带 .txt 后缀不存在，边被静默丢弃）: {:?}",
            neighbors
                .outgoing
                .iter()
                .map(|v| (&v.edge.to, &v.edge.field_path))
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(
        prop_edge.edge.field_path.as_deref(),
        Some("component:txtB.txt"),
        "field_path 仍携带完整符号串"
    );
    assert_eq!(
        prop_edge
            .edge
            .meta
            .as_ref()
            .and_then(|m| m.get("target_property"))
            .and_then(|v| v.as_str()),
        Some("txt"),
        "属性名应保留在边 meta.target_property"
    );

    // 裸组件符号：cond(exprC#value#1) → comp txtB，无 target_property
    let cond_c = cond_node_id("exprC#value#1");
    let neighbors_c = store
        .get_node_edges(&cond_c)
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("{cond_c} 应有邻居"));
    let bare_edge = neighbors_c.outgoing.iter().find(|v| {
        v.edge.edge_type == EdgeType::DependsOn && v.edge.to == comp_node_id("txtB")
    });
    let bare_edge = bare_edge.unwrap_or_else(|| {
        panic!(
            "裸 component 符号边应落图: {:?}",
            neighbors_c
                .outgoing
                .iter()
                .map(|v| (&v.edge.to, &v.edge.field_path))
                .collect::<Vec<_>>()
        )
    });
    assert_eq!(
        bare_edge.edge.field_path.as_deref(),
        Some("component:txtB")
    );
    assert!(
        bare_edge
            .edge
            .meta
            .as_ref()
            .and_then(|m| m.get("target_property"))
            .is_none(),
        "裸组件符号不得携带 target_property"
    );
}

/// P1-5：extra 键（operateButtons）内组件的动作条件进入 scan_conditions 结果，
/// json_path 是形态感知递归得到的真实路径；图侧 cond 节点与 cond→action owner
/// 边都落图（修复前 scan_actions_recursive 只走白名单四键，根本到不了 opBtn）。
#[test]
fn extra_key_component_action_condition_recorded_with_real_json_path() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "operateButtons": [
                        {
                            "id": "opBtn",
                            "type": "button",
                            "actions": [
                                {
                                    "id": "act1",
                                    "actionType": "updateData",
                                    "conditionExp": "=model1.totalRowCount__ > 0"
                                }
                            ]
                        }
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    // F1 形态感知递归应已把 opBtn 提取为组件
    assert!(
        meta.components.iter().any(|c| c.id == "opBtn"),
        "opBtn 应被提取: {:?}",
        meta.components
    );

    let records = scan_conditions(&meta, Some(REL_PATH));
    let record = records
        .iter()
        .find(|r| r.condition_id == "opBtn#act1#conditionExp")
        .unwrap_or_else(|| {
            panic!(
                "extra 键组件的动作条件应被记录: {:?}",
                records.iter().map(|r| &r.condition_id).collect::<Vec<_>>()
            )
        });
    assert_eq!(record.condition_type, ConditionType::ActionConditionExp);
    assert_eq!(record.owner_type, OwnerType::Action);
    assert_eq!(record.owner_id, "opBtn:act1");
    assert_eq!(
        record.json_path, "canvas.components[0].operateButtons[0].actions[0].conditionExp",
        "json_path 应是真实路径而非 canvas.components[id=...] 合成兜底"
    );
    assert_eq!(record.raw_expr, "=model1.totalRowCount__ > 0");

    // 图侧：cond 节点存在，cond→action owner 边落图
    let store = build_store(raw);
    let cond_id = cond_node_id("opBtn#act1#conditionExp");
    assert!(
        store.get_node(&cond_id).expect("get_node").is_some(),
        "cond 节点应存在: {cond_id}"
    );
    let action_node = format!("action:{}|opBtn|act1", REL_PATH);
    assert!(
        store.get_node(&action_node).expect("get_node").is_some(),
        "extra 键组件的 action 节点应存在: {action_node}"
    );
    let neighbors = store
        .get_node_edges(&cond_id)
        .expect("get_node_edges")
        .unwrap_or_else(|| panic!("{cond_id} 应有邻居"));
    assert!(
        neighbors.outgoing.iter().any(|v| {
            v.edge.edge_type == EdgeType::DependsOn && v.edge.to == action_node
        }),
        "缺 cond→action owner 边: {:?}",
        neighbors
            .outgoing
            .iter()
            .map(|v| (&v.edge.to, &v.edge.field_path))
            .collect::<Vec<_>>()
    );
}

/// P1-5 回归：白名单嵌套路径下的动作条件 json_path 与旧驱动（裸 JSON 递归）
/// 完全一致——驱动源切换不得改变既有可达组件的输出形态。
#[test]
fn whitelist_nested_action_condition_json_path_unchanged() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "panel1",
                    "type": "panel",
                    "components": [
                        {
                            "id": "btn1",
                            "type": "button",
                            "actions": [
                                {"id": "act1", "actionType": "link", "conditionExp": "=1 > 0"}
                            ]
                        }
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw).expect("parse should succeed");
    let records = scan_conditions(&meta, Some(REL_PATH));
    let record = records
        .iter()
        .find(|r| r.condition_id == "btn1#act1#conditionExp")
        .expect("白名单嵌套组件的动作条件应被记录");
    assert_eq!(
        record.json_path, "canvas.components[0].components[0].actions[0].conditionExp",
        "白名单嵌套的 json_path 不得因驱动源切换而变化"
    );
}

/// P1-5 行为变化锁定：canvas 自身带 id+type 时会被提取为组件，其动作条件因此
/// 进入扫描结果（旧驱动从 canvas.components 起递归，永远漏掉 canvas 自身的
/// actions）。json_path 退回合成兜底是既有机制，不在本次修复范围。
#[test]
fn canvas_level_action_condition_now_covered() {
    let raw = serde_json::json!({
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "actions": [
                {"id": "actC", "actionType": "link", "conditionExp": "=1 > 0"}
            ],
            "components": []
        }
    });
    let meta = parse_superpage_from_value(raw).expect("parse should succeed");
    assert!(
        meta.components.iter().any(|c| c.id == "canvas"),
        "canvas 带 id+type 时应被提取为组件: {:?}",
        meta.components
    );
    let records = scan_conditions(&meta, Some(REL_PATH));
    let record = records
        .iter()
        .find(|r| r.condition_id == "canvas#actC#conditionExp")
        .unwrap_or_else(|| {
            panic!(
                "canvas 级动作条件应被记录: {:?}",
                records.iter().map(|r| &r.condition_id).collect::<Vec<_>>()
            )
        });
    assert_eq!(record.owner_id, "canvas:actC");
    assert!(
        record.json_path.ends_with(".actions[0].conditionExp"),
        "json_path 应指向动作条件字段: {}",
        record.json_path
    );
}

/// P1-6（解析层）：裸 `${paramN}` 直接判 Param；带点形态 `${paramN.x}` 维持
/// ModelField 现状不动（语料中该形态指向 params 表格模型字段）。
#[test]
fn bare_param_placeholder_classified_as_param() {
    assert_eq!(
        parse_expression_refs("${param3}"),
        vec![RefType::Param("param3".to_string())],
        "裸 ${{param3}} 应判为 Param"
    );
    assert_eq!(
        parse_expression_refs("${param3.x}"),
        vec![RefType::ModelField("param3".to_string(), "x".to_string())],
        "${{param3.x}} 应维持 ModelField"
    );
}

/// P1-6（图侧）：裸 `${paramN}` 与不带 param 前缀的裸 `${phone}`（命中页面
/// param 上下文）都产出 param 节点 + comp→param DependsOn；不再产出
/// `model:paramN` / `field:paramN.` 垃圾节点，也不再被误判为继承 dataSet
/// 的裸字段 Reads。
#[test]
fn bare_param_ref_builds_param_node_not_garbage_model() {
    let raw = serde_json::json!({
        "params": [
            {"id": "param3", "name": "param3"},
            {"id": "phone", "name": "phone"}
        ],
        "canvas": {
            "components": [
                {
                    "id": "list1",
                    "type": "list",
                    "dataSet": "model1",
                    "components": [
                        {"id": "inner1", "type": "input", "value": "${param3}"},
                        {"id": "inner2", "type": "input", "value": "${phone}"}
                    ]
                }
            ]
        }
    });
    let store = build_store(raw);

    for param in ["param3", "phone"] {
        let param_node = format!("param:{}|{}", REL_PATH, param);
        assert!(
            store.get_node(&param_node).expect("get_node").is_some(),
            "param 节点应存在: {param_node}"
        );
        // 垃圾节点不得存在
        assert!(
            store
                .get_node(&format!("model:{}", param))
                .expect("get_node")
                .is_none(),
            "不得产出 model:{} 垃圾节点",
            param
        );
        assert!(
            store
                .get_node(&format!("field:{}.", param))
                .expect("get_node")
                .is_none(),
            "不得产出 field:{}. 尾点垃圾节点",
            param
        );
        // 继承 dataSet 的裸字段 Reads 不得误建（param 守卫）
        assert!(
            store
                .get_node(&format!("field:model1.{}", param))
                .expect("get_node")
                .is_none(),
            "param 不得误判为继承 dataSet 的裸字段: field:model1.{}",
            param
        );
    }

    let assert_param_edge = |comp: &str, param: &str| {
        let expected_to = format!("param:{}|{}", REL_PATH, param);
        let expected_field_path = format!("param:{}", param);
        let neighbors = store
            .get_node_edges(&comp_node_id(comp))
            .expect("get_node_edges")
            .unwrap_or_else(|| panic!("{comp} 应有邻居"));
        assert!(
            neighbors.outgoing.iter().any(|v| {
                v.edge.edge_type == EdgeType::DependsOn
                    && v.edge.to == expected_to
                    && v.edge.field_path.as_deref() == Some(expected_field_path.as_str())
            }),
            "缺 comp→param DependsOn 边 {comp} -> {param}: {:?}",
            neighbors
                .outgoing
                .iter()
                .map(|v| (&v.edge.to, &v.edge.field_path))
                .collect::<Vec<_>>()
        );
    };
    assert_param_edge("inner1", "param3");
    // 非 param 前缀的 param id 只能靠页面 param 上下文归一（resolve_ref_type 新臂）
    assert_param_edge("inner2", "phone");
}

/// P1-6（真实 fixture）：real_world_3.spg 声明了页面 param5，组件 text1 的
/// `value: "${param5}"` 必须归一为 Param，不得残留 ModelField("param5", _)。
#[test]
fn real_world_3_text1_bare_param_resolves_to_param() {
    let path = std::path::Path::new("tests/fixtures/real_world_3.spg");
    let meta = parse_superpage(path).expect("real_world_3.spg should parse");
    let matches: Vec<_> = meta
        .expressions
        .iter()
        .filter(|e| e.component_id == "text1" && e.field == "value")
        .collect();
    assert!(!matches.is_empty(), "text1.value 表达式应存在");
    for expr in matches {
        assert!(
            expr.resolved_refs
                .iter()
                .any(|r| matches!(&r.ref_type, RefType::Param(p) if p == "param5")),
            "text1.value 应归一为 Param(param5): {:?}",
            expr.resolved_refs
        );
        assert!(
            !expr
                .resolved_refs
                .iter()
                .any(|r| matches!(&r.ref_type, RefType::ModelField(m, _) if m == "param5")),
            "text1.value 不得残留 ModelField(param5, _): {:?}",
            expr.resolved_refs
        );
        // refs 被原地改写（scan_conditions 消费的是 refs），同步断言
        assert!(
            expr.refs
                .iter()
                .any(|r| matches!(r, RefType::Param(p) if p == "param5")),
            "text1.value 的 refs 应同步改写为 Param(param5): {:?}",
            expr.refs
        );
    }
}

/// P1-7：id 有、type 无的对象不是组件——parse 侧与 scanner 侧同口径：
/// 子组件 y 的 parent 透传祖父 panel1；图侧无 comp:x 节点、y 的 Contains
/// 恰好来自 page 与 panel1；该对象计入 SCANNER_UNRECOGNIZED_CONTAINER_KEY
/// 且 sample location 指向它（不再静默丢边）。
#[test]
fn id_without_type_object_passes_through_parent_and_counts_diagnostic() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "panel1",
                    "type": "panel",
                    "components": [
                        {
                            "id": "x",
                            "components": [
                                {"id": "y", "type": "input"}
                            ]
                        }
                    ]
                }
            ]
        }
    });

    // parse 侧：x 不是组件，y 的 parent_id 透传 panel1（提取侧既有行为）
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    assert!(
        !meta.components.iter().any(|c| c.id == "x"),
        "id 有 type 无的对象不得提取为组件"
    );
    let y = meta
        .components
        .iter()
        .find(|c| c.id == "y")
        .expect("y 应被提取");
    assert_eq!(y.parent_id.as_deref(), Some("panel1"));

    // 图侧：无 comp:x 节点；y 的 incoming Contains 恰好 {page, panel1}
    let store = build_store(raw.clone());
    assert!(
        store
            .get_node(&comp_node_id("x"))
            .expect("get_node")
            .is_none(),
        "x 不得有组件节点"
    );
    let y_neighbors = store
        .get_node_edges(&comp_node_id("y"))
        .expect("get_node_edges")
        .expect("y 应有邻居");
    let contains_from: Vec<&str> = y_neighbors
        .incoming
        .iter()
        .filter(|v| v.edge.edge_type == EdgeType::Contains)
        .map(|v| v.edge.from.as_str())
        .collect();
    // P1-7 修复前 comp→comp 边指向不存在的 comp:x，被存储层静默丢弃
    let page_id = format!("page:{}", REL_PATH);
    assert_eq!(
        contains_from.len(),
        2,
        "y 的 Contains 入边应恰好两条: {contains_from:?}"
    );
    assert!(
        contains_from.contains(&page_id.as_str()),
        "缺 page→y Contains: {contains_from:?}"
    );
    assert!(
        contains_from.contains(&comp_node_id("panel1").as_str()),
        "缺 panel1→y Contains（parent 应透传祖父）: {contains_from:?}"
    );
    let y_node = store
        .get_node(&comp_node_id("y"))
        .expect("get_node")
        .expect("y 节点应存在");
    assert_eq!(
        y_node
            .meta
            .as_ref()
            .and_then(|m| m.get("parent_id"))
            .and_then(|v| v.as_str()),
        Some("panel1"),
        "y 的 meta.parent_id 应与 parse 侧同口径"
    );

    // 诊断：id 有 type 无的对象计入既有 unrecognized 安全网，sample 指向该对象
    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw);
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        Some(1),
        "id 有 type 无的对象应计入 unrecognized 一次: {diags:?}"
    );
    let diag = diags
        .iter()
        .find(|d| d.code == "SCANNER_UNRECOGNIZED_CONTAINER_KEY")
        .expect("诊断应存在");
    assert_eq!(diag.location.node_id.as_deref(), Some("x"));
    assert_eq!(
        diag.location.json_path.as_deref(),
        Some("canvas.components[0].components[0]"),
        "sample location 应指向该对象自身路径"
    );
}

/// P1-7 计数点与 F1 混合形态计数点不得重复计数：extra 键数组形态不吻合
/// （元素缺 type）时整体判非组件计一次，递归不下钻，元素自身的 id-无-type
/// 不再单独计数。
#[test]
fn mixed_shape_extra_key_counts_once_without_double_count() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {
                    "id": "grid1",
                    "type": "grid",
                    "customSlots": [
                        {
                            "id": "x",
                            "components": [
                                {"id": "y", "type": "input"}
                            ]
                        }
                    ]
                }
            ]
        }
    });
    let meta = parse_superpage_from_value(raw.clone()).expect("parse should succeed");
    assert!(
        !meta.components.iter().any(|c| c.id == "x" || c.id == "y"),
        "混合形态数组整体判非组件，x/y 均不得提取: {:?}",
        meta.components
    );
    let diags = metadata_checker::scanner::scan_raw_diagnostics(&raw);
    assert_eq!(
        diag_count(&diags, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        Some(1),
        "数组级计数一次、不重复计元素自身: {diags:?}"
    );
    let diag = diags
        .iter()
        .find(|d| d.code == "SCANNER_UNRECOGNIZED_CONTAINER_KEY")
        .expect("诊断应存在");
    assert_eq!(
        diag.location.json_path.as_deref(),
        Some("canvas.components[0].customSlots"),
        "sample 应指向数组所在键而非元素"
    );
}
