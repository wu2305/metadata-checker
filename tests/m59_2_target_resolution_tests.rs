#![cfg(feature = "cli-local")]

//! M59-2 A1 接线回归：旧 target 必须在**生产查询链路**上显式解析。
//!
//! approved plan（2026-09-06-m59-grafeo-implementation-plan.md:25-32）要求
//! 「旧 target 显式解析，歧义诊断」与 A1/A4 配套发布。`resolve_node_target`
//! 已就绪但此前**没有任何生产调用方**：`runtime.rs` 把 `request.target` 原样
//! 交给 `build_query_model_output`，后者 `get_node` 精确查表。
//!
//! 新页面局部身份启用后：
//! - 裸 `model:orders` 在只有页面局部模型时 ⇒ 精确查表落空（missing）；
//! - 图中同时存在物理 `model:orders` 与局部 `model:app/a.spg|orders` 时 ⇒
//!   精确查表**静默命中物理模型**，跨页/跨形态同名的事实被吞掉且不报歧义。
//!
//! 本文件钉住生产入口（`GraphRuntime::query` 的 `QueryModel` 命令）的独立预期，
//! 不直接测 resolver helper。

use metadata_checker::graph::NodeType;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::ownership::ProjectBinding;
use metadata_checker::runtime::{GraphRuntime, RuntimeMode, RuntimeQueryRequest};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};

/// 内嵌 dataflow source：产出页面局部模型 + 其字段（供 field: kind 隔离用例）。
fn page_with_dataflow(source_id: &str, fields: &[&str]) -> String {
    let dims: Vec<_> = fields
        .iter()
        .map(|f| serde_json::json!({"name": f, "dataType": "C"}))
        .collect();
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [{
            "id": source_id,
            "modelType": "dataflow",
            "content": {"dimensions": dims}
        }],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [{"id": "label", "type": "text", "value": "v"}]
        }
    })
    .to_string()
}

/// 通过 `--explain` 生产入口执行查询（`field:` target 的入口）。
fn explain(runtime: &mut GraphRuntime, target: &str) -> serde_json::Value {
    let response = runtime
        .query(RuntimeQueryRequest {
            command: metadata_checker::tool_contract::ToolCommand::Explain,
            target: target.to_string(),
            budget: "normal".to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        })
        .expect("explain must not error");
    response.result
}

/// 建一个启用 ownership（页面局部身份）的图。
///
/// 必须用 `scan_for_project`（带 `ProjectBinding`）而不是 legacy `scan`：
/// 页面局部身份在 ownership schema 下才写入，legacy 扫描只产出全局 id。
fn build_bound_graph(tag: &str, files: &[(&str, String)]) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("m59-2-target-{tag}-{nanos}-{}", std::process::id()));
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("create project");
    for (rel, content) in files {
        let path = project.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&path, content).expect("write fixture");
    }
    let db_path = root.join("graph.redb");
    let binding = ProjectBinding::new("proj").expect("valid binding");
    ProjectIndexer::scan_for_project(&project, &db_path, &binding).expect("scan ownership graph");
    let _ = project;
    db_path
}

fn page_with_dwtable(source_id: &str, tbl_path: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [{"id": source_id, "modelType": "dwtable", "path": tbl_path}],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [{"id": "label", "type": "text", "value": "v"}]
        }
    })
    .to_string()
}

fn table_json(fields: &[&str]) -> String {
    let dims: Vec<_> = fields
        .iter()
        .map(|f| serde_json::json!({"name": f, "dataType": "C"}))
        .collect();
    serde_json::json!({"version": "1.0", "dimensions": dims}).to_string()
}

fn open_runtime(db_path: &Path) -> GraphRuntime {
    let binding = ProjectBinding::new("proj").expect("valid binding");
    GraphRuntime::load_with_project_dir_and_mode_for_project(
        db_path,
        None::<&Path>,
        RuntimeMode::OneShot,
        &binding,
    )
    .expect("load runtime")
}

fn model_ids(runtime: &GraphRuntime, kind: ModelKind) -> Vec<String> {
    let mut ids: Vec<String> = runtime
        .graph
        .iter_nodes()
        .expect("iter_nodes")
        .filter(|node| node.node_type == NodeType::Model)
        .filter(|node| match kind {
            ModelKind::PageLocal => node.id.contains('|'),
            ModelKind::Physical => !node.id.contains('|'),
        })
        .map(|node| node.id)
        .collect();
    ids.sort();
    ids
}

#[derive(Clone, Copy)]
enum ModelKind {
    PageLocal,
    Physical,
}

fn query_model(runtime: &mut GraphRuntime, target: &str) -> serde_json::Value {
    let response = runtime
        .query(RuntimeQueryRequest {
            command: metadata_checker::tool_contract::ToolCommand::QueryModel,
            target: target.to_string(),
            budget: "normal".to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        })
        .expect("query_model must not error");
    response.result
}

/// 通过共享 stdio 入口执行 query_model：验证 stdio 与 CLI/runtime 同一套解析。
fn stdio_query_model(runtime: &mut GraphRuntime, target: &str) -> serde_json::Value {
    let line = serde_json::json!({
        "request_id": "req-1",
        "command": "query_model",
        "target": target,
    })
    .to_string();
    let response = metadata_checker::stdio_server::dispatch_stdio_line(runtime, &line);
    serde_json::to_value(&response).expect("serialize stdio response")
}

/// 通过 `--explain-condition` 生产入口执行查询。
///
/// 这是 `--explain` 路由展开出的**补充调用**（route.rs:122，
/// `merge_key = "condition_facts"`），也是 `--explain-condition` 旧动词的入口。
fn explain_condition(runtime: &mut GraphRuntime, target: &str) -> serde_json::Value {
    let response = runtime
        .query(RuntimeQueryRequest {
            command: metadata_checker::tool_contract::ToolCommand::ExplainCondition,
            target: target.to_string(),
            budget: "normal".to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: None,
            check_reload: false,
        })
        .expect("explain_condition must not error");
    response.result
}

/// 通过 `--context` 生产入口执行查询。
fn context(runtime: &mut GraphRuntime, target: &str) -> serde_json::Value {
    let response = runtime
        .query(RuntimeQueryRequest {
            command: metadata_checker::tool_contract::ToolCommand::Context,
            target: target.to_string(),
            budget: "normal".to_string(),
            human: false,
            intent: None,
            page_scope: None,
            depth: Some(1),
            check_reload: false,
        })
        .expect("context must not error");
    response.result
}

fn diagnostic_codes(result: &serde_json::Value) -> Vec<String> {
    result
        .get("diagnostics")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("code").and_then(|code| code.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 裸旧 target 在只有页面局部模型时：必须解析到那个唯一局部节点，而不是报 missing。
///
/// source id 取 `ordersView`、物理表叫 `orders`——这样图里只有一个叫
/// `ordersView` 的模型（页面局部），没有任何同名的物理模型，正是「精确
/// get_node 必然落空」的场景。
#[test]
fn bare_target_with_single_page_local_model_resolves() {
    let db_path = build_bound_graph(
        "single-local",
        &[
            (
                "app/a.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);
    let locals = model_ids(&runtime, ModelKind::PageLocal);
    assert_eq!(
        locals,
        vec!["model:app/a.spg|ordersView".to_string()],
        "前置失败：图中应只有一个页面局部模型"
    );
    assert!(
        !model_ids(&runtime, ModelKind::Physical)
            .iter()
            .any(|id| id == "model:ordersView"),
        "前置失败：不得存在同名物理模型，否则测的不是「只有局部」场景"
    );

    let result = query_model(&mut runtime, "model:ordersView");
    let codes = diagnostic_codes(&result);
    assert!(
        !codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "唯一局部命中时不得报 TARGET_NOT_FOUND：{codes:?}"
    );
    assert_eq!(
        result.get("query_target").and_then(|v| v.as_str()),
        Some("model:app/a.spg|ordersView"),
        "唯一局部命中必须解析到该局部节点并如实回报解析后的 target：{:?}",
        result.get("query_target")
    );
}

/// 跨页同名：两个页面各有一个同名局部模型。裸 target 必须报歧义，
/// 不得静默挑一个（更不能报 missing）。
#[test]
fn bare_target_with_same_local_name_on_two_pages_is_ambiguous() {
    let db_path = build_bound_graph(
        "two-pages",
        &[
            (
                "app/a.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            (
                "app/b.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);
    let locals = model_ids(&runtime, ModelKind::PageLocal);
    assert_eq!(
        locals.len(),
        2,
        "前置失败：需要两个页面局部同名模型，实际 {locals:?}"
    );

    let result = query_model(&mut runtime, "model:ordersView");
    let codes = diagnostic_codes(&result);
    assert!(
        codes.iter().any(|code| code == "AMBIGUOUS_TARGET"),
        "跨页同名必须报 AMBIGUOUS_TARGET 而不是静默挑一个：{codes:?}"
    );
    let candidates = result
        .get("details")
        .and_then(|details| details.get("candidate_targets"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        candidates.len() >= 2,
        "歧义必须交回全部候选（≥2），实际 {}：{candidates:?}",
        candidates.len()
    );
    let ids: Vec<String> = candidates
        .iter()
        .filter_map(|item| item.get("id").and_then(|id| id.as_str()))
        .map(str::to_string)
        .collect();
    for local in &locals {
        assert!(ids.contains(local), "候选中必须包含 {local}：{ids:?}");
    }
}

/// scoped 精确命中：`<kind>:<PAGE>|<local>` 必须直接命中且无歧义诊断。
#[test]
fn scoped_target_hits_exactly_without_ambiguity() {
    let db_path = build_bound_graph(
        "scoped",
        &[
            (
                "app/a.spg",
                page_with_dwtable("orders", "tables/orders.tbl"),
            ),
            (
                "app/b.spg",
                page_with_dwtable("orders", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);
    let scoped = model_ids(&runtime, ModelKind::PageLocal);
    let target = scoped
        .iter()
        .find(|id| id.starts_with("model:app/a.spg|"))
        .cloned()
        .unwrap_or_else(|| panic!("前置失败：需要 app/a.spg 的页面局部模型，实际 {scoped:?}"));

    let result = query_model(&mut runtime, &target);
    let codes = diagnostic_codes(&result);
    assert!(
        !codes.iter().any(|code| code == "AMBIGUOUS_TARGET"),
        "scoped 精确命中不得报歧义：{codes:?}"
    );
    assert!(
        !codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "scoped 精确命中的 id 必须存在：{codes:?}"
    );
    assert_eq!(
        result.get("query_target").and_then(|v| v.as_str()),
        Some(target.as_str())
    );
}

/// 局部与物理同名：图里同时有 Physical `model:orders` 与页面局部
/// `model:app/a.spg|orders`。裸 target 必须报歧义，不得静默命中物理模型。
#[test]
fn bare_target_matching_both_local_and_physical_is_ambiguous() {
    let db_path = build_bound_graph(
        "local-and-physical",
        &[
            (
                "app/a.spg",
                page_with_dwtable("orders", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);
    let physical = model_ids(&runtime, ModelKind::Physical);
    let locals = model_ids(&runtime, ModelKind::PageLocal);

    assert!(
        physical.iter().any(|id| id == "model:orders") && !locals.is_empty(),
        "前置失败：需要同名的物理模型与页面局部模型同时存在（physical={physical:?}, local={locals:?}）"
    );

    let result = query_model(&mut runtime, "model:orders");
    let codes = diagnostic_codes(&result);
    assert!(
        codes.iter().any(|code| code == "AMBIGUOUS_TARGET"),
        "局部与物理同名必须报 AMBIGUOUS_TARGET，不得静默命中物理模型：{codes:?}"
    );
}

/// field 与 model 的 kind 隔离：同名 `ordersView` 的局部字段与局部模型互不干扰，
/// 裸 `field:` 只匹配 Field 节点，裸 `model:` 只匹配 Model 节点。
///
/// `--explain` 是 `field:` target 的生产入口，与 `--query-model` 共用同一套解析。
#[test]
fn field_and_model_kinds_do_not_cross_match() {
    let db_path = build_bound_graph(
        "kind-isolation",
        &[
            ("app/a.spg", page_with_dataflow("ordersView", &["order_id"])),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);

    let model_ids: Vec<String> = runtime
        .graph
        .iter_nodes()
        .expect("iter_nodes")
        .filter(|node| node.node_type == NodeType::Model && node.id.contains('|'))
        .map(|node| node.id)
        .collect();
    let field_ids: Vec<String> = runtime
        .graph
        .iter_nodes()
        .expect("iter_nodes")
        .filter(|node| node.node_type == NodeType::Field && node.id.contains('|'))
        .map(|node| node.id)
        .collect();
    assert!(
        model_ids
            .iter()
            .any(|id| id == "model:app/a.spg|ordersView"),
        "前置失败：需要页面局部模型，实际 {model_ids:?}"
    );
    assert!(
        field_ids
            .iter()
            .any(|id| id == "field:app/a.spg|ordersView.order_id"),
        "前置失败：需要页面局部字段，实际 {field_ids:?}"
    );

    // 裸 `field:ordersView.order_id` 必须解析到 Field 节点，不得被同名 Model 干扰
    let field_result = explain(&mut runtime, "field:ordersView.order_id");
    assert_eq!(
        field_result.get("query_target").and_then(|v| v.as_str()),
        Some("field:app/a.spg|ordersView.order_id"),
        "裸 field: 必须只匹配 Field 节点：{:?}",
        field_result.get("query_target")
    );

    // 裸 `model:ordersView` 必须解析到 Model 节点，不得落到同名字段
    let model_result = query_model(&mut runtime, "model:ordersView");
    assert_eq!(
        model_result.get("query_target").and_then(|v| v.as_str()),
        Some("model:app/a.spg|ordersView"),
        "裸 model: 必须只匹配 Model 节点：{:?}",
        model_result.get("query_target")
    );

    // 反向隔离：用 model 的裸名去问 field 前缀，必须 missing（kind 不匹配）
    let crossed = explain(&mut runtime, "field:ordersView");
    let crossed_codes = diagnostic_codes(&crossed);
    assert!(
        crossed_codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "kind 不匹配时必须报 TARGET_NOT_FOUND，不得跨 kind 命中：{crossed_codes:?}"
    );
}

/// `--explain` 路由展开出的**补充调用** `ExplainCondition` 也必须走同一套旧
/// target 解析。
///
/// 只接线 `build_explain_output` 是不够的：`route_explain` 把 `--explain`
/// 展开成 `Explain`（主）+ `ExplainCondition`（补充，merge_key=condition_facts），
/// 后者此前仍做精确 `get_node`。裸 `field:` 在页面局部身份启用后会在这里
/// 报 TARGET_NOT_FOUND，于是 `--explain` 的补充块静默缺失——主调用有答案，
/// 条件成因却永远是「查不到」。
#[test]
fn explain_condition_supplement_resolves_bare_legacy_target() {
    let db_path = build_bound_graph(
        "explain-condition",
        &[
            ("app/a.spg", page_with_dataflow("ordersView", &["order_id"])),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);

    let result = explain_condition(&mut runtime, "field:ordersView.order_id");
    let codes = diagnostic_codes(&result);
    assert!(
        !codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "ExplainCondition 必须解析裸 field: 旧 target，不得报 TARGET_NOT_FOUND：{codes:?}"
    );
    assert_eq!(
        result.get("query_target").and_then(|v| v.as_str()),
        Some("field:app/a.spg|ordersView.order_id"),
        "ExplainCondition 必须解析到唯一的页面局部字段并如实回报：{:?}",
        result.get("query_target")
    );
}

/// `--context`（`--explain --depth N` 展开出的补充调用）同样不得绕过解析。
#[test]
fn context_supplement_resolves_bare_legacy_target() {
    let db_path = build_bound_graph(
        "context-supplement",
        &[
            (
                "app/a.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);

    let result = context(&mut runtime, "model:ordersView");
    let codes = diagnostic_codes(&result);
    assert!(
        !codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "Context 必须解析裸 model: 旧 target，不得报 TARGET_NOT_FOUND：{codes:?}"
    );
    assert_eq!(
        result.get("query_target").and_then(|v| v.as_str()),
        Some("model:app/a.spg|ordersView"),
        "Context 必须解析到唯一的页面局部模型并如实回报：{:?}",
        result.get("query_target")
    );
}

/// 跨页同名时，补充调用也必须报歧义，而不是静默挑一个或报 missing。
#[test]
fn explain_condition_supplement_reports_ambiguity() {
    let db_path = build_bound_graph(
        "explain-condition-ambiguous",
        &[
            ("app/a.spg", page_with_dataflow("ordersView", &["order_id"])),
            ("app/b.spg", page_with_dataflow("ordersView", &["order_id"])),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);

    let result = explain_condition(&mut runtime, "field:ordersView.order_id");
    let codes = diagnostic_codes(&result);
    assert!(
        codes.iter().any(|code| code == "AMBIGUOUS_TARGET"),
        "ExplainCondition 遇到跨页同名必须报 AMBIGUOUS_TARGET：{codes:?}"
    );
}

/// 共享 stdio 入口：query_model 走同一套旧 target 解析，不得因为入口不同而绕过。
#[test]
fn stdio_entry_uses_same_legacy_target_resolution() {
    let db_path = build_bound_graph(
        "stdio-entry",
        &[
            (
                "app/a.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            (
                "app/b.spg",
                page_with_dwtable("ordersView", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);

    let response = stdio_query_model(&mut runtime, "model:ordersView");
    let flat = serde_json::to_string(&response).expect("flatten response");
    assert!(
        flat.contains("AMBIGUOUS_TARGET"),
        "stdio 入口也必须报 AMBIGUOUS_TARGET，不得静默挑一个：{flat}"
    );

    let scoped = stdio_query_model(&mut runtime, "model:app/a.spg|ordersView");
    let scoped_flat = serde_json::to_string(&scoped).expect("flatten response");
    assert!(
        !scoped_flat.contains("AMBIGUOUS_TARGET"),
        "scoped 精确命中在 stdio 入口也不得报歧义：{scoped_flat}"
    );
    assert!(
        !scoped_flat.contains("TARGET_NOT_FOUND"),
        "scoped 精确命中的 id 在 stdio 入口必须存在：{scoped_flat}"
    );
}
