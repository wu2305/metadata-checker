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
use metadata_checker::runtime::{GraphRuntime, RuntimeMode, RuntimeQueryRequest};
use metadata_checker::ownership::ProjectBinding;
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};

/// 建一个启用 ownership（页面局部身份）的图。
///
/// 必须用 `scan_for_project`（带 `ProjectBinding`）而不是 legacy `scan`：
/// 页面局部身份在 ownership schema 下才写入，legacy 扫描只产出全局 id。
fn build_bound_graph(tag: &str, files: &[(&str, String)]) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "m59-2-target-{tag}-{nanos}-{}",
        std::process::id()
    ));
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
#[test]
fn bare_target_with_single_page_local_model_resolves() {
    let db_path = build_bound_graph(
        "single-local",
        &[
            (
                "app/a.spg",
                page_with_dwtable("orders", "tables/orders.tbl"),
            ),
            ("tables/orders.tbl", table_json(&["order_id"])),
        ],
    );
    let mut runtime = open_runtime(&db_path);
    let locals = model_ids(&runtime, ModelKind::PageLocal);
    assert!(
        !locals.is_empty(),
        "前置失败：图中必须有页面局部模型，实际模型节点：{:?}",
        model_ids(&runtime, ModelKind::Physical)
    );

    let result = query_model(&mut runtime, "model:orders");
    let codes = diagnostic_codes(&result);
    assert!(
        !codes.contains(&"TARGET_NOT_FOUND".to_string()),
        "唯一局部命中时不得报 TARGET_NOT_FOUND：{codes:?}"
    );
    assert_eq!(
        result.get("query_target").and_then(|v| v.as_str()),
        Some(locals[0].as_str()),
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
    let locals = model_ids(&runtime, ModelKind::PageLocal);
    assert!(
        locals.len() >= 2,
        "前置失败：需要两个页面局部同名模型，实际 {locals:?}"
    );

    let result = query_model(&mut runtime, "model:orders");
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
