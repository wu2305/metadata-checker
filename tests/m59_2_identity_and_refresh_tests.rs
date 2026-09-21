#![cfg(feature = "cli-local")]

//! M59-2 失败回归：身份必须在有损合并之前区分页面局部实体与物理实体。
//!
//! 已发现的反例（`codex/m59-2-ownership@2a65d7b`）：
//! SPG source `id=orders`、`path=$DATA:/tables/orders.tbl` 时，建图“成功”，但
//! - 期望的 `model:app/a.spg|orders` **不存在**；
//! - `model:orders` 上出现 `DataflowInput` **自环**。
//!
//! 根因：scanner 先按旧全局 ID 建临时图（局部 source 与同名物理表塌成一个节点，
//! `path`/`meta` 后写覆盖前写），再事后把 ID 转成页面局部——信息在转换前已丢失。
//!
//! 本文件先钉住**独立预期**（精确断言节点类型、身份、来源与边端点），再要求
//! full == incremental。不得用两条路径一起算错来证明正确。

use metadata_checker::graph::{GraphDB, NodeType};
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::ownership::ProjectBinding;
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};

const CODE_CONFLICT: &str = "GRAPH_OWNERSHIP_CONFLICT";

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-2-{tag}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    dir
}

fn write(project: &Path, rel: &str, content: &str) {
    let path = project.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&path, content).expect("write fixture");
}

/// `dwtable` source：`id` 是页面局部名，`path` 指向物理表。
fn page_with_dwtable(title: &str, source_id: &str, tbl_path: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [{"id": source_id, "modelType": "dwtable", "path": tbl_path}],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [{"id": "label", "type": "text", "value": title}]
        }
    })
    .to_string()
}

fn table_json(name: &str, fields: &[&str]) -> String {
    let dims: Vec<_> = fields
        .iter()
        .map(|f| serde_json::json!({"name": f, "dataType": "C"}))
        .collect();
    serde_json::json!({"version": "1.0", "name": name, "dimensions": dims}).to_string()
}

/// 逐属性快照：节点按 id 排序并展开类型/路径/meta/来源，边展开端点与类型。
#[derive(Debug, PartialEq)]
struct Snapshot {
    nodes: Vec<String>,
    edges: Vec<String>,
}

fn snapshot(db_path: &Path, binding: &ProjectBinding) -> Snapshot {
    let graph = GraphDB::open_readonly_with_ownership(db_path, binding).expect("reopen graph db");
    let mut nodes: Vec<String> = graph
        .iter_nodes()
        .expect("iter_nodes")
        .map(|n| {
            format!(
                "{}\ttype={:?}\tpath={}\tname={}\tmeta={}\torigin={}",
                n.id,
                n.node_type,
                n.path,
                n.name,
                n.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string()),
                n.origin_file.as_deref().unwrap_or("-")
            )
        })
        .collect();
    nodes.sort();
    let node_ids: Vec<String> = nodes
        .iter()
        .map(|line| line.split('\t').next().unwrap_or("").to_string())
        .collect();
    let mut edges: Vec<String> = Vec::new();
    for id in &node_ids {
        let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("edges") else {
            continue;
        };
        for view in neighbors.outgoing {
            let e = view.edge;
            edges.push(format!(
                "{} -{:?}-> {}\torigin={}",
                e.from,
                e.edge_type,
                e.to,
                e.origin_file.as_deref().unwrap_or("-")
            ));
        }
    }
    edges.sort();
    Snapshot { nodes, edges }
}

fn describe_diff(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
    let only_a: Vec<&String> = a.iter().filter(|x| !b.contains(x)).collect();
    let only_b: Vec<&String> = b.iter().filter(|x| !a.contains(x)).collect();
    let mut out = String::new();
    for line in &only_a {
        out.push_str(&format!("\n  仅在 {label_a}: {line}"));
    }
    for line in &only_b {
        out.push_str(&format!("\n  仅在 {label_b}: {line}"));
    }
    if out.is_empty() {
        out.push_str(" (无差异)");
    }
    out
}

/// 反例 1：局部 source 与物理表同名。
///
/// 独立预期（先于任何 full==incremental 比较）：
/// - 局部实体 `model:app/a.spg|orders` 存在，node_type=Model，来源 app/a.spg；
/// - 物理实体 `model:orders` 存在，来源 app/a.spg（Reference）；
/// - `DataflowInput` 边的端点是这两个**不同**节点，**不得自环**。
#[test]
fn local_source_sharing_name_with_physical_table_stays_distinct() {
    let tag = "local-same-name";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    write(
        &project,
        "app/a.spg",
        &page_with_dwtable("Page A", "orders", "$DATA:/tables/orders.tbl"),
    );
    write(
        &project,
        "tables/orders.tbl",
        &table_json("orders", &["order_id"]),
    );
    let db = dir.join("graph.db");
    ProjectIndexer::scan_for_project(&project, &db, &binding).expect("scan");

    let graph = GraphDB::open_readonly_with_ownership(&db, &binding).expect("reopen");

    // 独立预期 1：页面局部实体存在且身份带页面段
    let local_id = "model:app/a.spg|orders";
    let local = graph
        .get_node(local_id)
        .unwrap_or_else(|| panic!("页面局部模型 {local_id} 必须存在"));
    assert_eq!(local.node_type, NodeType::Model, "局部实体类型必须是 Model");
    assert_eq!(local.name, "orders", "局部实体的 name 是 source id");
    assert_eq!(
        local.origin_file.as_deref(),
        Some("app/a.spg"),
        "局部实体的 origin 必须是其所在页面"
    );
    assert_eq!(
        local
            .meta
            .as_ref()
            .and_then(|m| m.get("modelType"))
            .and_then(|v| v.as_str()),
        Some("dwtable"),
        "局部 dwtable 的 modelType 不得被物理表覆盖：{:?}",
        local.meta
    );

    // 独立预期 2：物理实体存在且身份为全局
    let physical = graph
        .get_node("model:orders")
        .expect("物理表 model:orders 必须存在");
    assert_eq!(physical.node_type, NodeType::Model);

    // 独立预期 3：DataflowInput 边端点不同，禁止自环
    let mut inputs = Vec::new();
    let mut self_loops = Vec::new();
    if let Some(neighbors) = GraphReadStore::get_node_edges(&graph, local_id).expect("edges") {
        for view in neighbors.outgoing {
            if format!("{:?}", view.edge.edge_type) == "DataflowInput" {
                inputs.push(format!("{} -> {}", view.edge.from, view.edge.to));
                if view.edge.from == view.edge.to {
                    self_loops.push(view.edge.to.clone());
                }
            }
        }
    }
    assert_eq!(
        inputs,
        vec![format!("{local_id} -> model:orders")],
        "DataflowInput 必须且只能从局部实体指向物理表"
    );
    assert!(self_loops.is_empty(), "不得出现 DataflowInput 自环");

    // 全图范围内也不得有任何自环的 DataflowInput
    let snapshot = snapshot(&db, &binding);
    for edge in &snapshot.edges {
        if let Some(rest) = edge.strip_prefix("model:") {
            let (from, _) = rest.split_once(" -DataflowInput-> ").unwrap_or(("", ""));
            let to = edge
                .split(" -DataflowInput-> ")
                .nth(1)
                .unwrap_or("")
                .split('\t')
                .next()
                .unwrap_or("");
            assert_ne!(from, to, "禁止 DataflowInput 自环：{edge}");
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 反例 1b：跨页同名 source 必须隔离，且共享物理表不得被页面作用域改写。
#[test]
fn cross_page_same_source_name_isolated_and_physical_table_shared() {
    let tag = "cross-page-same-source";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    write(
        &project,
        "app/a.spg",
        &page_with_dwtable("Page A", "src1", "$DATA:/tables/shared.tbl"),
    );
    write(
        &project,
        "app/b.spg",
        &page_with_dwtable("Page B", "src1", "$DATA:/tables/shared.tbl"),
    );
    write(
        &project,
        "tables/shared.tbl",
        &table_json("shared", &["col1"]),
    );
    let db = dir.join("graph.db");
    ProjectIndexer::scan_for_project(&project, &db, &binding).expect("scan");
    let graph = GraphDB::open_readonly_with_ownership(&db, &binding).expect("reopen");

    let a = graph
        .get_node("model:app/a.spg|src1")
        .expect("页面 A 的局部 src1 必须存在");
    let b = graph
        .get_node("model:app/b.spg|src1")
        .expect("页面 B 的局部 src1 必须存在");
    assert_ne!(a.id, b.id, "跨页同名 source 必须是两个节点");
    assert_eq!(a.origin_file.as_deref(), Some("app/a.spg"));
    assert_eq!(b.origin_file.as_deref(), Some("app/b.spg"));

    // 物理表恒全局：两个页面的 DataflowInput 都指向同一个 model:shared
    let physical = graph
        .get_node("model:shared")
        .expect("物理表 model:shared 必须存在");
    assert_eq!(physical.node_type, NodeType::Model);
    let edges = GraphReadStore::get_node_edges(&graph, "model:shared")
        .expect("edges")
        .expect("物理表节点必须有邻居视图");
    let incoming: Vec<String> = edges.incoming.iter().map(|v| v.edge.from.clone()).collect();
    assert!(
        incoming.contains(&"model:app/a.spg|src1".to_string()),
        "页面 A 的局部模型必须指向共享物理表：{incoming:?}"
    );
    assert!(
        incoming.contains(&"model:app/b.spg|src1".to_string()),
        "页面 B 的局部模型必须指向共享物理表：{incoming:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 反例 1c：一个 source 的名字撞上**另一个 source 的物理表名**。
///
/// 页面 A：source `beta` → `$DATA:/tables/alpha.tbl`
/// 页面 B：source `alpha` → `$DATA:/tables/beta.tbl`
///
/// 独立预期：`model:app/b.spg|alpha`（局部）与 `model:alpha`（物理，来自 A 的引用）
/// 是两个节点；`model:app/a.spg|beta`（局部）与 `model:beta`（物理）同理。
/// 四个 id 两两不同，且没有任何一个局部实体被误判成物理占位。
#[test]
fn source_name_colliding_with_another_sources_physical_table_stays_distinct() {
    let tag = "cross-collision";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    write(
        &project,
        "app/a.spg",
        &page_with_dwtable("Page A", "beta", "$DATA:/tables/alpha.tbl"),
    );
    write(
        &project,
        "app/b.spg",
        &page_with_dwtable("Page B", "alpha", "$DATA:/tables/beta.tbl"),
    );
    write(&project, "tables/alpha.tbl", &table_json("alpha", &["f_a"]));
    write(&project, "tables/beta.tbl", &table_json("beta", &["f_b"]));
    let db = dir.join("graph.db");
    ProjectIndexer::scan_for_project(&project, &db, &binding).expect("scan");
    let graph = GraphDB::open_readonly_with_ownership(&db, &binding).expect("reopen");

    for expected in [
        "model:app/a.spg|beta",
        "model:app/b.spg|alpha",
        "model:alpha",
        "model:beta",
    ] {
        let node = graph
            .get_node(expected)
            .unwrap_or_else(|| panic!("{expected} 必须存在"));
        assert_eq!(node.node_type, NodeType::Model, "{expected} 类型应为 Model");
    }
    // 局部实体不被物理表占位降级
    for local in ["model:app/a.spg|beta", "model:app/b.spg|alpha"] {
        let node = graph.get_node(local).expect("local");
        assert_eq!(
            node.meta
                .as_ref()
                .and_then(|m| m.get("modelType"))
                .and_then(|v| v.as_str()),
            Some("dwtable"),
            "{local} 的 modelType 应为 dwtable，不得被物理表覆盖：{:?}",
            node.meta
        );
    }
    // 端点精确：A 的 beta 读 alpha 表；B 的 alpha 读 beta 表
    let out_a = GraphReadStore::get_node_edges(&graph, "model:app/a.spg|beta")
        .expect("edges")
        .expect("neighbors");
    let a_targets: Vec<String> = out_a
        .outgoing
        .iter()
        .filter(|v| format!("{:?}", v.edge.edge_type) == "DataflowInput")
        .map(|v| v.edge.to.clone())
        .collect();
    assert_eq!(
        a_targets,
        vec!["model:alpha".to_string()],
        "页面 A 的局部 beta 必须指向物理表 alpha"
    );
    let out_b = GraphReadStore::get_node_edges(&graph, "model:app/b.spg|alpha")
        .expect("edges")
        .expect("neighbors");
    let b_targets: Vec<String> = out_b
        .outgoing
        .iter()
        .filter(|v| format!("{:?}", v.edge.edge_type) == "DataflowInput")
        .map(|v| v.edge.to.clone())
        .collect();
    assert_eq!(
        b_targets,
        vec!["model:beta".to_string()],
        "页面 B 的局部 alpha 必须指向物理表 beta"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 同一场景的 full 与 incremental 必须逐属性一致（先独立预期，再比两条路径）。
#[test]
fn same_name_scenarios_full_and_incremental_are_identical() {
    let tag = "same-name-parity";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");

    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(
        &full_project,
        "app/a.spg",
        &page_with_dwtable("Page A", "orders", "$DATA:/tables/orders.tbl"),
    );
    write(
        &full_project,
        "app/b.spg",
        &page_with_dwtable("Page B", "src1", "$DATA:/tables/shared.tbl"),
    );
    write(
        &full_project,
        "tables/orders.tbl",
        &table_json("orders", &["order_id"]),
    );
    write(
        &full_project,
        "tables/shared.tbl",
        &table_json("shared", &["col1"]),
    );
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full");

    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(
        &inc_project,
        "app/a.spg",
        &page_with_dwtable("Page A", "orders", "$DATA:/tables/orders.tbl"),
    );
    write(
        &inc_project,
        "tables/orders.tbl",
        &table_json("orders", &["order_id"]),
    );
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc init");
    write(
        &inc_project,
        "app/b.spg",
        &page_with_dwtable("Page B", "src1", "$DATA:/tables/shared.tbl"),
    );
    write(
        &inc_project,
        "tables/shared.tbl",
        &table_json("shared", &["col1"]),
    );
    let report =
        ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc second");
    assert!(report.dirty > 0, "必须真的走增量路径");

    let full_snap = snapshot(&full_db, &binding);
    let inc_snap = snapshot(&inc_db, &binding);
    assert_eq!(
        full_snap.nodes,
        inc_snap.nodes,
        "节点逐属性必须一致：{}",
        describe_diff("全量", &full_snap.nodes, "增量", &inc_snap.nodes)
    );
    assert_eq!(
        full_snap.edges,
        inc_snap.edges,
        "边逐属性必须一致：{}",
        describe_diff("全量", &full_snap.edges, "增量", &inc_snap.edges)
    );

    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
}

/// 反例 3（C）：ownership 冲突必须是持久状态 —— 查询可见、重启可见、修复后消失。
///
/// 两个不同目录的同 stem `a.tbl` 撞同一个全局 `model:a`。
#[test]
fn ownership_conflict_is_queryable_persistent_and_recoverable() {
    let tag = "conflict-lifecycle";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    write(&project, "tables/a.tbl", &table_json("a", &["f_a"]));
    write(&project, "tables/other/a.tbl", &table_json("a", &["f_b"]));
    let db = dir.join("graph.db");

    // 1. scan 报告冲突
    let outcome =
        ProjectIndexer::scan_with_diagnostics_for_project(&project, &db, &binding).expect("scan");
    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT && d.message.contains("model:a")),
        "scan 必须报告 model:a 冲突：{:?}",
        outcome.diagnostics
    );

    // 2. 查询侧可见：runtime status.load_diagnostics 必须带同一冲突
    let runtime =
        metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
            &db,
            None::<&std::path::Path>,
            metadata_checker::runtime::RuntimeMode::OneShot,
            &binding,
        )
        .expect("runtime load");
    let status = runtime.status();
    assert!(
        status
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "status.load_diagnostics 必须可见冲突（不是只出现在扫描报告里）：{:?}",
        status.load_diagnostics
    );
    assert_eq!(
        runtime.ownership_conflict_node_ids(),
        vec!["model:a".to_string()],
        "runtime 必须登记冲突节点 id"
    );

    // 3. 重启后仍可见（新句柄重新 load）
    drop(runtime);
    let restarted =
        metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
            &db,
            None::<&std::path::Path>,
            metadata_checker::runtime::RuntimeMode::OneShot,
            &binding,
        )
        .expect("restart runtime load");
    assert!(
        restarted
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "重启后冲突必须仍然可见"
    );

    // 4. no-op 重扫继续报告
    let again =
        ProjectIndexer::scan_with_diagnostics_for_project(&project, &db, &binding).expect("rescan");
    assert!(
        again.diagnostics.iter().any(|d| d.code == CODE_CONFLICT),
        "no-op 重扫必须继续报告存量冲突"
    );

    // 5. 修复后冲突消失：移除冲突文件并删除其贡献
    std::fs::remove_file(project.join("tables/other/a.tbl")).expect("remove conflict file");
    let fixed =
        ProjectIndexer::scan_with_diagnostics_for_project(&project, &db, &binding).expect("fix");
    assert!(
        !fixed.diagnostics.iter().any(|d| d.code == CODE_CONFLICT),
        "修复后冲突必须消失：{:?}",
        fixed.diagnostics
    );
    let after =
        metadata_checker::runtime::GraphRuntime::load_with_project_dir_and_mode_for_project(
            &db,
            None::<&std::path::Path>,
            metadata_checker::runtime::RuntimeMode::OneShot,
            &binding,
        )
        .expect("runtime after fix");
    assert!(
        !after
            .status()
            .load_diagnostics
            .iter()
            .any(|d| d.code == CODE_CONFLICT),
        "修复后查询侧也必须不再有冲突：{:?}",
        after.status().load_diagnostics
    );
    assert!(after.ownership_conflict_node_ids().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// 反例 3（C）：prepare 入口也必须把冲突交给调用方，不能丢弃。
#[test]
fn prepare_exposes_ownership_conflicts_to_caller() {
    let tag = "prepare-conflict";
    let binding = ProjectBinding::new(format!("m59-2-{tag}")).expect("binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    write(&project, "tables/a.tbl", &table_json("a", &["f_a"]));
    write(&project, "tables/other/a.tbl", &table_json("a", &["f_b"]));
    let db = dir.join("graph.db");
    ProjectIndexer::scan_for_project(&project, &db, &binding).expect("seed scan");

    // 改动其中一个文件使其变脏，走 prepare 的 dirty 路径
    write(&project, "tables/a.tbl", &table_json("a", &["f_a", "f_c"]));
    let prepared = ProjectIndexer::prepare_for_project(&project, &db, &binding).expect("prepare");
    assert!(
        prepared
            .ownership_conflicts
            .iter()
            .any(|conflict| conflict.node_id == "model:a"),
        "prepare 必须透出来源冲突，不能丢弃：{:?}",
        prepared.ownership_conflicts
    );
    assert!(
        !prepared.has_parse_failure(),
        "本场景不应有解析失败：{:?}",
        prepared.parse_failures
    );

    let _ = std::fs::remove_dir_all(&dir);
}
