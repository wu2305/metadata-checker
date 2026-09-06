#![cfg(feature = "cli-local")]

//! M58.3 PR2（F2 祖先链/继承链）测试电池，覆盖 spec 说明 A 的硬要求：
//! 唯一父断言、无环、近到远序稳定、page_logic 遍历路径不受影响、
//! 增量重建后祖先链结论不变。链级不变式的细粒度用例在
//! `src/explain/condition_facts/conditions.rs` 的单元测试模块中。

use metadata_checker::graph::{Edge, EdgeType, GraphDB, Node, NodeType};
use metadata_checker::graph_store::{GraphReadStore, GraphWriteStore};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::scanner::scan_project;
use serde_json::json;
use std::path::{Path, PathBuf};

const PAGE_PATH: &str = "app/chain.spg";
const PAGE_ID: &str = "page:app/chain.spg";

/// 构造带 json_path meta 的组件节点
fn comp_node(comp_id: &str, json_path: &str) -> Node {
    Node {
        id: format!("comp:{}|{}", PAGE_PATH, comp_id),
        node_type: NodeType::Component,
        path: PAGE_PATH.to_string(),
        name: comp_id.to_string(),
        meta: Some(json!({
            "component_type": "panel",
            "json_path": json_path,
        })),
    }
}

/// 构造组件 visibleCondition 条件节点（与 scanner 产出的 cond id/meta 同构）
fn visible_condition_node(comp_id: &str, json_path: &str, raw_expr: &str) -> Node {
    Node {
        id: format!("cond:{}|{}#visibleCondition", PAGE_PATH, comp_id),
        node_type: NodeType::Condition,
        path: PAGE_PATH.to_string(),
        name: format!("{}#visibleCondition", comp_id),
        meta: Some(json!({
            "condition_type": "VisibleCondition",
            "effect_type": "",
            "raw_expr": raw_expr,
            "normalized_expr": raw_expr,
            "json_path": json_path,
            "owner_type": "Component",
            "subject_type": "",
            "referenced_symbols": [],
        })),
    }
}

fn add_edge(store: &mut MemoryGraphStore, from: &str, to: &str, edge_type: EdgeType) {
    store
        .add_edge(Edge {
            from: from.to_string(),
            to: to.to_string(),
            edge_type,
            field_path: None,
            meta: None,
        })
        .expect("添加图边必须成功");
}

fn add_contains(store: &mut MemoryGraphStore, from: &str, to: &str) {
    add_edge(store, from, to, EdgeType::Contains);
}

/// 条件节点 -> 归属组件 的 DependsOn 边（与 scanner 产出同构）
fn add_condition_owner(store: &mut MemoryGraphStore, cond_id: &str, comp_id: &str) {
    add_edge(
        store,
        cond_id,
        &format!("comp:{}|{}", PAGE_PATH, comp_id),
        EdgeType::DependsOn,
    );
}

/// 从 explain-condition 输出提取 inherited 条件
/// （condition_id, inherited_from, ancestor_distance），保持输出顺序
fn explain_inherited_conditions(
    graph: &dyn GraphReadStore,
    target_comp_id: &str,
) -> Vec<(String, String, Option<u64>)> {
    let target_id = format!("comp:{}|{}", PAGE_PATH, target_comp_id);
    let result =
        metadata_checker::explain::build_explain_condition_output(graph, &target_id, "normal")
            .expect("explain-condition 必须成功");
    let blocking = result
        .get("details")
        .and_then(|d| d.get("blocking_conditions"))
        .and_then(|v| v.as_array())
        .expect("details.blocking_conditions 必须是数组");
    blocking
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited"))
        .map(|c| {
            (
                c.get("condition_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                c.get("inherited_from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                c.get("ancestor_distance").and_then(|v| v.as_u64()),
            )
        })
        .collect()
}

/// 唯一父断言：嵌套组件同时有 page→comp 与 comp→comp 两条 Contains 入边时，
/// 必须确定性地选 Component 父；每个层级恰好一个父，祖先链距离从 1 起。
#[test]
fn test_pr2_unique_component_parent_assertion() {
    for page_edge_first in [true, false] {
        let mut store = MemoryGraphStore::new();
        store
            .upsert_node(Node {
                id: PAGE_ID.to_string(),
                node_type: NodeType::Page,
                path: PAGE_PATH.to_string(),
                name: "chain".to_string(),
                meta: None,
            })
            .expect("upsert page");
        store
            .upsert_node(comp_node("panel", "canvas.components[0]"))
            .expect("upsert panel");
        store
            .upsert_node(comp_node("child", "canvas.components[0].components[0]"))
            .expect("upsert child");
        let panel_cond = visible_condition_node(
            "panel",
            "canvas.components[0].visibleCondition",
            "model1.totalRowCount__ > 0",
        );
        let panel_cond_id = panel_cond.id.clone();
        store.upsert_node(panel_cond).expect("upsert cond");
        add_condition_owner(&mut store, &panel_cond_id, "panel");
        // 按两种相反顺序插入 page→child 与 panel→child，验证与迭代序无关
        if page_edge_first {
            add_contains(&mut store, PAGE_ID, &format!("comp:{}|child", PAGE_PATH));
            add_contains(
                &mut store,
                &format!("comp:{}|panel", PAGE_PATH),
                &format!("comp:{}|child", PAGE_PATH),
            );
        } else {
            add_contains(
                &mut store,
                &format!("comp:{}|panel", PAGE_PATH),
                &format!("comp:{}|child", PAGE_PATH),
            );
            add_contains(&mut store, PAGE_ID, &format!("comp:{}|child", PAGE_PATH));
        }
        add_contains(&mut store, PAGE_ID, &format!("comp:{}|panel", PAGE_PATH));

        let inherited = explain_inherited_conditions(&store, "child");
        assert_eq!(
            inherited,
            vec![(
                panel_cond_id.clone(),
                format!("comp:{}|panel", PAGE_PATH),
                Some(1u64)
            )],
            "page_edge_first={} 时必须唯一继承 panel 的条件且距离为 1",
            page_edge_first
        );
    }
}

/// 近到远序稳定：三级嵌套，两种相反建边顺序的图必须产出完全一致的
/// inherited 序列（近祖先在前，距离 1、2 递增）。
#[test]
fn test_pr2_near_to_far_order_stable_across_insertion_orders() {
    let mut results = Vec::new();
    for reversed in [false, true] {
        let mut store = MemoryGraphStore::new();
        store
            .upsert_node(Node {
                id: PAGE_ID.to_string(),
                node_type: NodeType::Page,
                path: PAGE_PATH.to_string(),
                name: "chain".to_string(),
                meta: None,
            })
            .expect("upsert page");
        store
            .upsert_node(comp_node("top", "canvas.components[0]"))
            .expect("upsert top");
        store
            .upsert_node(comp_node("mid", "canvas.components[0].components[0]"))
            .expect("upsert mid");
        store
            .upsert_node(comp_node(
                "leaf",
                "canvas.components[0].components[0].components[0]",
            ))
            .expect("upsert leaf");
        for (owner, path, expr) in [
            (
                "top",
                "canvas.components[0].visibleCondition",
                "model1.a > 0",
            ),
            (
                "mid",
                "canvas.components[0].components[0].visibleCondition",
                "model1.b > 0",
            ),
        ] {
            let cond = visible_condition_node(owner, path, expr);
            let cond_id = cond.id.clone();
            store.upsert_node(cond).expect("upsert cond");
            add_condition_owner(&mut store, &cond_id, owner);
        }
        let mut edges = vec![
            (PAGE_ID.to_string(), format!("comp:{}|top", PAGE_PATH)),
            (PAGE_ID.to_string(), format!("comp:{}|mid", PAGE_PATH)),
            (PAGE_ID.to_string(), format!("comp:{}|leaf", PAGE_PATH)),
            (
                format!("comp:{}|top", PAGE_PATH),
                format!("comp:{}|mid", PAGE_PATH),
            ),
            (
                format!("comp:{}|mid", PAGE_PATH),
                format!("comp:{}|leaf", PAGE_PATH),
            ),
        ];
        if reversed {
            edges.reverse();
        }
        for (from, to) in edges {
            add_contains(&mut store, &from, &to);
        }
        results.push(explain_inherited_conditions(&store, "leaf"));
    }
    assert_eq!(results[0], results[1], "两种建边顺序的继承结论必须一致");
    assert_eq!(
        results[0],
        vec![
            (
                format!("cond:{}|mid#visibleCondition", PAGE_PATH),
                format!("comp:{}|mid", PAGE_PATH),
                Some(1u64)
            ),
            (
                format!("cond:{}|top#visibleCondition", PAGE_PATH),
                format!("comp:{}|top", PAGE_PATH),
                Some(2u64)
            ),
        ],
        "继承条件必须按近到远排列且距离递增"
    );
}

/// 无环：组件互含成环时 explain 必须终止，且环上条件不重复出现。
#[test]
fn test_pr2_cycle_terminates_without_duplicate_inheritance() {
    let mut store = MemoryGraphStore::new();
    store
        .upsert_node(Node {
            id: PAGE_ID.to_string(),
            node_type: NodeType::Page,
            path: PAGE_PATH.to_string(),
            name: "chain".to_string(),
            meta: None,
        })
        .expect("upsert page");
    store
        .upsert_node(comp_node("ring_a", "canvas.components[0]"))
        .expect("upsert ring_a");
    store
        .upsert_node(comp_node("ring_b", "canvas.components[0].components[0]"))
        .expect("upsert ring_b");
    let cond = visible_condition_node(
        "ring_b",
        "canvas.components[0].components[0].visibleCondition",
        "model1.c > 0",
    );
    let cond_id = cond.id.clone();
    store.upsert_node(cond).expect("upsert cond");
    add_condition_owner(&mut store, &cond_id, "ring_b");
    add_contains(&mut store, PAGE_ID, &format!("comp:{}|ring_a", PAGE_PATH));
    add_contains(&mut store, PAGE_ID, &format!("comp:{}|ring_b", PAGE_PATH));
    // 人为构造环：ring_a <-> ring_b
    add_contains(
        &mut store,
        &format!("comp:{}|ring_a", PAGE_PATH),
        &format!("comp:{}|ring_b", PAGE_PATH),
    );
    add_contains(
        &mut store,
        &format!("comp:{}|ring_b", PAGE_PATH),
        &format!("comp:{}|ring_a", PAGE_PATH),
    );

    let inherited = explain_inherited_conditions(&store, "ring_a");
    assert_eq!(
        inherited,
        vec![(cond_id, format!("comp:{}|ring_b", PAGE_PATH), Some(1u64))],
        "成环时必须终止且环上条件恰好继承一次"
    );
}

/// 无表达式组件（button13 场景）：没有 comp→comp Contains 边、组件自身
/// 无任何表达式时，必须通过节点 meta json_path + 放宽后的容器子键前缀
/// 继承到 `columns` 容器下祖先的条件；兄弟容器的条件不得混入。
#[test]
fn test_pr2_no_expression_component_inherits_via_relaxed_json_path() {
    let mut store = MemoryGraphStore::new();
    store
        .upsert_node(Node {
            id: PAGE_ID.to_string(),
            node_type: NodeType::Page,
            path: PAGE_PATH.to_string(),
            name: "chain".to_string(),
            meta: None,
        })
        .expect("upsert page");
    store
        .upsert_node(comp_node("panel", "canvas.components[0]"))
        .expect("upsert panel");
    // 无表达式按钮：只有节点 meta json_path，位于非 components 容器键 columns 下
    store
        .upsert_node(comp_node("button13", "canvas.components[0].columns[1]"))
        .expect("upsert button13");
    store
        .upsert_node(comp_node("sibling_panel", "canvas.components[1]"))
        .expect("upsert sibling_panel");
    let panel_cond = visible_condition_node(
        "panel",
        "canvas.components[0].visibleCondition",
        "model1.totalRowCount__ > 0",
    );
    let panel_cond_id = panel_cond.id.clone();
    store.upsert_node(panel_cond).expect("upsert panel cond");
    add_condition_owner(&mut store, &panel_cond_id, "panel");
    let sibling_cond = visible_condition_node(
        "sibling_panel",
        "canvas.components[1].visibleCondition",
        "model1.other > 1",
    );
    let sibling_cond_id = sibling_cond.id.clone();
    store
        .upsert_node(sibling_cond)
        .expect("upsert sibling cond");
    add_condition_owner(&mut store, &sibling_cond_id, "sibling_panel");
    // 只建 page→comp 边：模拟 scanner 尚未补 comp→comp Contains 的图形态
    for comp in ["panel", "button13", "sibling_panel"] {
        add_contains(&mut store, PAGE_ID, &format!("comp:{}|{}", PAGE_PATH, comp));
    }

    let inherited = explain_inherited_conditions(&store, "button13");
    assert_eq!(
        inherited,
        vec![(panel_cond_id, format!("comp:{}|panel", PAGE_PATH), None)],
        "无表达式组件必须经 json_path 兜底继承 columns 容器下祖先的条件"
    );
}

/// page_logic 遍历路径不受影响（只读调用方断言）：page_logic 从页面文件
/// 递归收集的组件 json_path 必须继续覆盖深层嵌套组件。
#[test]
fn test_pr2_page_logic_traversal_paths_unchanged() {
    let db_path = unique_db_path("page-logic-traversal");
    let project_dir = Path::new("tests/fixtures/m58_3_ancestor_chain");
    scan_project(project_dir, &db_path).expect("fixture graphdb 必须构建成功");
    let graph = GraphDB::open(&db_path).expect("打开 graphdb 必须成功");

    let result = metadata_checker::query::build_query_page_logic_output(
        &graph,
        "page:app/chain_page.spg",
        Some(project_dir),
        "normal",
    )
    .expect("page_logic 必须成功");
    let serialized = serde_json::to_string(&result).expect("输出必须可序列化");
    assert!(
        serialized.contains("canvas.components[0].components[0].visible"),
        "page_logic 遍历路径必须覆盖嵌套组件 child_text 的 visible 规则"
    );
    assert!(
        result
            .get("details")
            .and_then(|d| d.get("key_model_availability"))
            .is_some(),
        "page_logic 输出必须保留 key_model_availability 字段"
    );
}

/// 增量重建后祖先链结论不变：修改页面文件触发增量重建，
/// 目标组件的继承条件集合必须保持稳定。
#[test]
fn test_pr2_incremental_rebuild_keeps_ancestor_chain_conclusions() {
    let work_dir = copy_fixture_to_temp();
    let db_path = work_dir.join("graph.db");

    let report1 = scan_project(&work_dir, &db_path).expect("首次扫描必须成功");
    assert_eq!(report1.indexed, 2, "首次全量扫描必须索引两个页面文件");

    let inherited_before = {
        let graph = GraphDB::open(&db_path).expect("打开 graphdb 必须成功");
        explain_inherited_conditions_for_file(&graph, "app/chain_page.spg", "leaf_btn")
    };
    assert_eq!(
        inherited_before.len(),
        2,
        "leaf_btn 必须继承 grand_panel 与 panel_gate 两层祖先条件，实际: {:?}",
        inherited_before
    );

    // 修改页面文件（追加无关顶层组件），触发增量重建
    let page_file = work_dir.join("app/chain_page.spg");
    let mut page_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&page_file).expect("读取页面文件必须成功"))
            .expect("页面 JSON 必须合法");
    page_json["canvas"]["components"]
        .as_array_mut()
        .expect("canvas.components 必须是数组")
        .push(json!({"id": "extra_btn", "type": "button"}));
    std::fs::write(
        &page_file,
        serde_json::to_string_pretty(&page_json).unwrap(),
    )
    .expect("写回页面文件必须成功");

    let report2 = scan_project(&work_dir, &db_path).expect("增量扫描必须成功");
    assert_eq!(report2.dirty, 1, "增量重建必须只重处理变更文件");
    assert_eq!(report2.unchanged, 1, "未变更文件必须走增量跳过");

    let inherited_after = {
        let graph = GraphDB::open(&db_path).expect("重新打开 graphdb 必须成功");
        explain_inherited_conditions_for_file(&graph, "app/chain_page.spg", "leaf_btn")
    };
    assert_eq!(
        inherited_after, inherited_before,
        "增量重建后祖先链结论必须不变"
    );

    let _ = std::fs::remove_dir_all(&work_dir);
}

/// 对真实扫描产出的 graphdb 提取指定页面组件的 inherited 条件（排序后比较）
fn explain_inherited_conditions_for_file(
    graph: &dyn GraphReadStore,
    page_path: &str,
    target_comp_id: &str,
) -> Vec<(String, String, Option<u64>)> {
    let target_id = format!("comp:{}|{}", page_path, target_comp_id);
    let result =
        metadata_checker::explain::build_explain_condition_output(graph, &target_id, "normal")
            .expect("explain-condition 必须成功");
    let blocking = result
        .get("details")
        .and_then(|d| d.get("blocking_conditions"))
        .and_then(|v| v.as_array())
        .expect("details.blocking_conditions 必须是数组");
    let mut inherited: Vec<(String, String, Option<u64>)> = blocking
        .iter()
        .filter(|c| c.get("condition_scope").and_then(|v| v.as_str()) == Some("inherited"))
        .map(|c| {
            (
                c.get("condition_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                c.get("inherited_from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                c.get("ancestor_distance").and_then(|v| v.as_u64()),
            )
        })
        .collect();
    inherited.sort();
    inherited
}

fn unique_db_path(suffix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("m58-3-pr2-ancestor-chain-{}-{}.db", suffix, nanos))
}

/// 把 fixture 项目复制到临时目录，避免测试改动仓库内 fixture
fn copy_fixture_to_temp() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work_dir = std::env::temp_dir().join(format!("m58-3-pr2-ancestor-chain-proj-{}", nanos));
    let src = Path::new("tests/fixtures/m58_3_ancestor_chain");
    std::fs::create_dir_all(work_dir.join("app")).expect("创建临时目录必须成功");
    for entry in std::fs::read_dir(src.join("app")).expect("读取 fixture 目录必须成功") {
        let entry = entry.expect("读取目录项必须成功");
        std::fs::copy(entry.path(), work_dir.join("app").join(entry.file_name()))
            .expect("复制 fixture 文件必须成功");
    }
    work_dir
}
