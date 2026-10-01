//! M59-3 C3：runtime 读侧（链路查询动词）在 `.grafeo` 后端上的端到端验收。
//!
//! spec 的判据是「链路查询动词，每类边自己的投影规则」：同一语料扫描进
//! redb 与 grafeo 两个后端，同一组查询动词在两个 `GraphReadStore` 上的
//! 输出必须逐值相等——**查询等价**是 C3 的核心判据，不是「能跑通就算过」。
//!
//! 钉住的失效点：
//! - 全部链路动词（explain / context / find / query_model / query_page /
//!   query_cross / query_dataflow / query_page_logic / query via runtime）
//!   的 JSON 输出在两后端上逐值相等——每类边的投影规则一致才算等价；
//! - status() 的节点/边口径不含 IndexState meta 节点（grafeo 同库存边状态）；
//! - SCANNER_* 持久化诊断经 runtime 加载管道透出（grafeo 的 IndexState
//!   元节点 → load_diagnostics，与 redb 的 SCANNER_DIAGNOSTICS_TABLE 同信道）；
//! - project_binding / diff-refresh 持久化 / runtime reload 在 grafeo 上
//!   显式失败（fail-closed），不留「看着像支持实际半残」的路径。

#![cfg(all(feature = "cli-local", feature = "grafeo-store"))]

use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::ownership::ProjectBinding;
use metadata_checker::runtime::{GraphRuntime, RuntimeMode, RuntimeQueryRequest};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ 语料构造

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-c3-{tag}-{nanos}-{}", std::process::id()));
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

fn write_bytes(project: &Path, rel: &str, content: &[u8]) {
    let path = project.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&path, content).expect("write fixture");
}

/// 边类型尽量多的页：内嵌页 b（EmbedsPage）+ 局部模型 + submitField/value 表达式
/// （Reads / FieldAlias 类边）+ action 条件表达式（Condition 节点 + Triggers 类边）。
fn page_a() -> String {
    r#"{
  "version": "4.19.7",
  "theme": "default",
  "params": [{"id": "p1", "name": "入参A"}],
  "referenceResources": ["b.spg"],
  "sources": [{"id": "orders", "modelType": "dwtable", "path": "db/orders.tbl"}],
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {"id": "embed1", "type": "embedsuperpage", "resPath": "0"},
      {"id": "input1", "type": "input", "submitField": "orders.订单号", "value": "=orders.amount"},
      {"id": "btn1", "type": "button", "value": "提交",
       "action": {"actionType": "save", "conditionExp": "=param.p1 != ''"}}
    ]
  }
}"#
    .to_string()
}

fn page_b() -> String {
    r#"{
  "version": "4.19.7",
  "theme": "default",
  "params": [],
  "sources": [],
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [{"id": "label_b", "type": "text", "value": "B"}]
  }
}"#
    .to_string()
}

fn orders_table() -> String {
    r#"{
	"version": "1.0",
	"properties": {"name": "订单表"},
	"dimensions": [
		{"name": "订单号", "dbfield": "订单号", "dataType": "C", "length": 50, "isDimension": true},
		{"name": "金额", "dbfield": "amount", "dataType": "N", "length": 18, "isDimension": false}
	]
}"#
    .to_string()
}

/// dataflow 语料：两个外部 ModelTable 输入（目标 tbl 不存在 → 占位节点 + DataflowInput 边）。
fn dataflow_table() -> String {
    r#"{
	"version": "1.0",
	"properties": {"name": "每日生成工单"},
	"dimensions": [
		{"name": "工单号", "dbfield": "workNo", "dataType": "C", "length": 50, "isDimension": true}
	],
	"dataFlow": {
		"nodes": {
			"node1": {"id": "node1", "alias": "服务预约", "type": "ModelTable",
				"moduleTablePath": "$DATA:/售后/fact_serviceappointments.tbl"},
			"node2": {"id": "node2", "alias": "客户信息", "type": "ModelTable",
				"moduleTablePath": "$DATA:/客户/customer_info.tbl"}
		}
	}
}"#
    .to_string()
}

/// 语料：双页跨文件边 + 物理模型 + dataflow + 一份坏 tbl。
/// 坏文件让两后端都留下持久化的 SCANNER_FILE_PARSE_FAILED entry——
/// 顺带验收「索引边状态 → runtime 诊断」这条通道。
fn seed_corpus(project: &Path) {
    write(project, "app/a.spg", &page_a());
    write(project, "app/b.spg", &page_b());
    write(project, "db/orders.tbl", &orders_table());
    write(project, "flow/df.tbl", &dataflow_table());
    write_bytes(project, "db/broken.tbl", &[0xFF, 0xFE, 0x00, 0x01]);
}

/// 同一语料分别扫描进 redb / grafeo，返回两个已加载的 runtime。
fn load_pair(dir: &Path, mode: RuntimeMode) -> (GraphRuntime, GraphRuntime) {
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let redb_db = dir.join("graph.graphdb");
    let grafeo_db = dir.join("graph.grafeo");
    // scanner 写端结束即释放两后端各自的文件锁，runtime 才能随后打开。
    let redb_scan = ProjectIndexer::scan_with_diagnostics(&project, &redb_db).expect("redb scan");
    assert!(redb_scan.node_count > 0);
    let grafeo_scan =
        ProjectIndexer::scan_with_diagnostics(&project, &grafeo_db).expect("grafeo scan");
    // 项目图口径相等：meta 节点不泄进计数（grafeo 多存了 IndexState 节点）。
    assert_eq!(redb_scan.node_count, grafeo_scan.node_count);
    assert_eq!(redb_scan.edge_count, grafeo_scan.edge_count);
    let redb = GraphRuntime::load_with_project_dir_and_mode(&redb_db, Some(&project), mode)
        .expect("redb runtime");
    let grafeo = GraphRuntime::load_with_project_dir_and_mode(&grafeo_db, Some(&project), mode)
        .expect("grafeo runtime");
    (redb, grafeo)
}

/// 在两后端各自的图里收集的节点 id 集合必须相同（连占位节点都要一致）。
fn node_ids(runtime: &GraphRuntime) -> BTreeSet<String> {
    runtime
        .graph
        .iter_nodes()
        .expect("iter_nodes")
        .map(|node| node.id)
        .collect()
}

/// 全量链路动词矩阵：对语料里每个存在的目标都调一遍，返回 (用例名, 输出)。
/// 任何一边新增的投影差异都会在这里现形。
fn verb_outputs(graph: &dyn GraphReadStore, project: &Path) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    let mut nodes: Vec<_> = graph.iter_nodes().expect("iter_nodes").collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let mut pages = Vec::new();
    let mut models = Vec::new();
    for node in &nodes {
        // explain / context 覆盖全部节点——每类边的投影都从邻接视图走一遍。
        out.push((
            format!("explain|{}", node.id),
            metadata_checker::explain::build_explain_output(graph, &node.id)
                .expect("explain output"),
        ));
        out.push((
            format!("context|{}", node.id),
            metadata_checker::context::build_context_output(graph, &node.id, 2, "normal")
                .expect("context output"),
        ));
        match node.node_type {
            NodeType::Page => pages.push(node.id.clone()),
            NodeType::Model => models.push(node.id.clone()),
            _ => {}
        }
    }

    for page in &pages {
        out.push((
            format!("query_page|{page}"),
            metadata_checker::query::build_query_page_output(graph, page)
                .expect("query_page output"),
        ));
        out.push((
            format!("page_logic|{page}"),
            metadata_checker::query::build_query_page_logic_output(
                graph,
                page,
                Some(project),
                "normal",
            )
            .expect("page_logic output"),
        ));
    }
    for (a, b) in pages.iter().zip(pages.iter().skip(1)) {
        out.push((
            format!("query_cross|{a}|{b}"),
            metadata_checker::query::build_query_cross_output(graph, a, b)
                .expect("query_cross output"),
        ));
        out.push((
            format!("query_cross|{b}|{a}"),
            metadata_checker::query::build_query_cross_output(graph, b, a)
                .expect("query_cross reverse output"),
        ));
    }
    for model in &models {
        for budget in ["normal", "compact"] {
            out.push((
                format!("query_model|{model}|{budget}"),
                metadata_checker::query::build_query_model_output(graph, model, budget)
                    .expect("query_model output"),
            ));
        }
        out.push((
            format!("query_dataflow|{model}"),
            metadata_checker::query::build_query_dataflow_output(graph, model)
                .expect("query_dataflow output"),
        ));
    }

    // find 动词：关键词 × 类型过滤，命中与不命中都要一致。
    for (keyword, filter) in [
        ("订单", None),
        ("orders", Some("model")),
        ("input", Some("component")),
        ("page", Some("page")),
        ("zzz-no-such-node", None),
    ] {
        out.push((
            format!("find|{keyword}|{}", filter.unwrap_or("*")),
            serde_json::to_value(
                metadata_checker::query::find_nodes(graph, keyword, filter, 20)
                    .expect("find_nodes"),
            )
            .expect("find json"),
        ));
    }
    out
}

// ------------------------------------------------------------------ 用例

/// 核心判据：同一语料、同一动词矩阵，redb / grafeo 输出逐值相等。
/// OneShot 与 LongLived（含 dense snapshot + 物化索引）两条加载路径都验。
#[test]
fn query_verb_outputs_are_identical_across_backends() {
    for mode in [RuntimeMode::OneShot, RuntimeMode::LongLived] {
        let dir = unique_dir("parity");
        let (redb, grafeo) = load_pair(&dir, mode);
        assert_eq!(
            node_ids(&redb),
            node_ids(&grafeo),
            "节点 id 集合在两后端必须一致（{mode:?}）"
        );
        let project = dir.join("proj");
        let redb_out = verb_outputs(&redb.graph, &project);
        let grafeo_out = verb_outputs(&grafeo.graph, &project);
        assert_eq!(redb_out.len(), grafeo_out.len());
        for ((name, expected), (same_name, actual)) in redb_out.iter().zip(grafeo_out.iter()) {
            assert_eq!(name, same_name);
            assert_eq!(
                expected, actual,
                "查询动词输出在 grafeo 后端不一致：{name}（{mode:?}）"
            );
        }
        // 语料确实产出了跨文件边与多类边——否则上面的逐值相等没有证明力。
        let edge_types: BTreeSet<String> = redb
            .graph
            .iter_nodes()
            .expect("iter")
            .flat_map(|node| {
                redb.graph
                    .get_node_edges(&node.id)
                    .expect("edges")
                    .expect("node exists")
                    .outgoing
                    .into_iter()
                    .map(|view| format!("{:?}", view.edge.edge_type))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert!(edge_types.contains(&format!("{:?}", EdgeType::EmbedsPage)));
        assert!(
            edge_types.len() >= 3,
            "语料边类型过少，等价断言无效：{edge_types:?}"
        );
    }
}

/// runtime.query() 调度入口逐值相等（除 timing/diagnostics 文案外）。
/// 这是真实产品通路（stdio/CLI 经 RuntimeQueryRequest 进来）。
#[test]
fn runtime_query_dispatch_outputs_match_across_backends() {
    let dir = unique_dir("dispatch");
    let (mut redb, mut grafeo) = load_pair(&dir, RuntimeMode::LongLived);
    let model_id = {
        let ids = node_ids(&redb);
        ids.iter()
            .find(|id| id.starts_with("model:") && id.contains("orders"))
            .cloned()
            .expect("corpus should contain the orders model")
    };
    let request = |command| RuntimeQueryRequest {
        command,
        target: model_id.clone(),
        budget: "normal".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    };
    for command in [
        metadata_checker::runtime::RuntimeQueryCommand::QueryModel,
        metadata_checker::runtime::RuntimeQueryCommand::Explain,
        metadata_checker::runtime::RuntimeQueryCommand::Context,
    ] {
        let a = redb.query(request(command.clone())).expect("redb query");
        let b = grafeo.query(request(command)).expect("grafeo query");
        assert_eq!(a.result, b.result, "runtime.query {command:?} 输出不一致");
    }
}

/// status() 口径：node_count / edge_count 不含 IndexState meta 节点，
/// load_diagnostics 里的持久化 SCANNER_* 诊断两后端同信道透出。
#[test]
fn status_counts_and_scanner_diagnostics_match() {
    let dir = unique_dir("status");
    let (redb, grafeo) = load_pair(&dir, RuntimeMode::OneShot);
    let (sa, sb) = (redb.status(), grafeo.status());
    assert_eq!(sa.node_count, sb.node_count);
    assert_eq!(sa.edge_count, sb.edge_count);
    let codes = |runtime: &GraphRuntime| -> BTreeSet<String> {
        runtime
            .load_diagnostics
            .iter()
            .map(|d| d.code.clone())
            .collect()
    };
    let (ca, cb) = (codes(&redb), codes(&grafeo));
    assert_eq!(
        ca, cb,
        "load_diagnostics 在两后端必须一致（持久化 SCANNER_* 同信道）"
    );
    assert!(
        ca.iter()
            .any(|code| code == metadata_checker::diagnostics::CODE_SCANNER_FILE_PARSE_FAILED),
        "坏 tbl 的持久化诊断应透出，实际：{ca:?}"
    );
}

/// `.grafeo` + project_binding：binding 只有 ownership 语义，显式 bail，
/// 不静默忽略（与扫描入口同口径）。
#[test]
fn grafeo_runtime_with_project_binding_fails_closed() {
    let dir = unique_dir("binding");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = dir.join("graph.grafeo");
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan");

    let binding = ProjectBinding::new("m59-c3-binding").expect("binding");
    let Err(error) = GraphRuntime::load_with_project_dir_and_mode_for_project(
        &db,
        Some(&project),
        RuntimeMode::OneShot,
        &binding,
    ) else {
        panic!("grafeo + binding 应显式失败");
    };
    assert!(
        format!("{error}").contains("GRAPH_BACKEND_UNSUPPORTED"),
        "应为 GRAPH_BACKEND_UNSUPPORTED，实际：{error}"
    );
}

/// reload 在 grafeo 上显式失败且旧 runtime 仍可服务：
/// 独占文件锁下「先开新句柄再换旧图」在物理上不成立，bail 比锁错误可读。
#[test]
fn grafeo_runtime_reload_fails_closed_but_keeps_serving() {
    let dir = unique_dir("reload");
    let (mut _redb, mut grafeo) = load_pair(&dir, RuntimeMode::OneShot);
    let error = grafeo.reload().expect_err("grafeo reload 应显式失败");
    assert!(
        format!("{error}").contains("GRAPH_BACKEND_UNSUPPORTED"),
        "应为 GRAPH_BACKEND_UNSUPPORTED，实际：{error}"
    );
    // 旧 runtime 不受影响，仍能回答查询。
    let ids = node_ids(&grafeo);
    assert!(!ids.is_empty());
    let first = ids.iter().next().expect("has node").clone();
    let output = metadata_checker::explain::build_explain_output(&grafeo.graph, &first)
        .expect("explain after failed reload");
    assert!(output.is_object());
}

/// diff-refresh 持久化句柄在 grafeo 上显式失败（PersistFn 仍是 redb 具体化签名）。
#[test]
fn grafeo_runtime_diff_refresh_persist_fails_closed() {
    let dir = unique_dir("persist");
    let (_redb, mut grafeo) = load_pair(&dir, RuntimeMode::OneShot);
    let Err(error) = grafeo.graph.graph_db_mut() else {
        panic!("grafeo 上拿不到 GraphDB 写句柄");
    };
    assert!(
        format!("{error}").contains("GRAPH_BACKEND_UNSUPPORTED"),
        "应为 GRAPH_BACKEND_UNSUPPORTED，实际：{error}"
    );
}

/// 状态读通道：file_states / checkpoint / scanner entries 都能从 grafeo 的
/// IndexState 边状态读回（diff-refresh 判脏 + 诊断透出依赖这些读侧出口）。
#[test]
fn grafeo_runtime_state_reads_match_redb() {
    let dir = unique_dir("state");
    let (redb, grafeo) = load_pair(&dir, RuntimeMode::OneShot);
    let redb_states = redb.graph.load_file_states().expect("redb file_states");
    let grafeo_states = grafeo.graph.load_file_states().expect("grafeo file_states");
    assert_eq!(redb_states, grafeo_states);

    let redb_entries = redb
        .graph
        .load_scanner_diagnostic_entries()
        .expect("redb entries");
    let grafeo_entries = grafeo
        .graph
        .load_scanner_diagnostic_entries()
        .expect("grafeo entries");
    let sorted = |mut v: Vec<(String, Vec<u8>)>| {
        v.sort();
        v
    };
    assert_eq!(sorted(redb_entries), sorted(grafeo_entries));

    assert_eq!(
        redb.graph
            .load_diff_refresh_checkpoint()
            .expect("redb checkpoint"),
        grafeo
            .graph
            .load_diff_refresh_checkpoint()
            .expect("grafeo checkpoint"),
    );
}
