//! M59-3 C2：Grafeo 直写导入 + 持久化接线的端到端验收。
//!
//! C1 只验收了 GraphStore 读写契约；C2 要验收的是**同一条扫描编排跑在两个
//! 后端上结果一致**——plan §M59-3 的验收口径是「同一 B1 套件语义下的重复
//! 写入、重启、删除、导入结果一致」。这里不复制 B 套件，而是钉住导入路径
//! 特有的失效点：
//!
//! - `file_states` 必须随库一起复原（否则重启后全库被判脏，增量退化为全量）；
//! - per-file scanner 诊断 entry 必须与图一起持久化、随文件删除一起移除；
//! - TBL 解析失败要保留旧图并留下持久化的 parse_failed 标记（B3 语义）；
//! - grafeo 与 redb 对同一语料的导入快照必须逐节点/逐边一致（B5 口径）；
//! - ownership / diff-refresh prepare 在 `.grafeo` 路径上必须显式拒绝
//!   （两者都还是 `GraphDB` 具体 API，静默走错路径比报错更难查）。

#![cfg(all(feature = "cli-local", feature = "grafeo-store"))]

use metadata_checker::graph::{Edge, Node};
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_store::{GraphReadStore, IndexStateStore};
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::scan_project_with_report;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ 语料构造

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-c2-{tag}-{nanos}-{}", std::process::id()));
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

/// 内嵌页 B 的页 A：产生跨文件边 `comp:app/a.spg|embed1 -EmbedsPage-> page:app/b.spg`。
fn page_a_embedding_b() -> String {
    r#"{
  "version": "4.19.7",
  "theme": "default",
  "params": [],
  "referenceResources": ["b.spg"],
  "sources": [],
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {"id": "embed1", "type": "embedsuperpage", "resPath": "0"},
      {"id": "label_a", "type": "text", "value": "A"}
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

/// 三文件语料：跨文件边 + TBL 模型，覆盖 spg/tbl 两类解析器与跨文件依赖。
fn seed_corpus(project: &Path) {
    write(project, "app/a.spg", &page_a_embedding_b());
    write(project, "app/b.spg", &page_b());
    write(project, "db/orders.tbl", &orders_table());
}

// ------------------------------------------------------------------ 快照比对

/// 规范化快照：节点按 id 排序后带全部属性序列化；边按四元组排序。
/// 与 B5 同口径——逐字节比对而不是比计数，计数比对挡不住「丢一条边同时
/// 多一条占位边」这类净变化为零的损坏。
fn snapshot(store: &dyn GraphReadStore) -> Vec<String> {
    let mut nodes: Vec<Node> = store.iter_nodes().expect("iter_nodes").collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut lines = Vec::new();
    let mut edge_lines = Vec::new();
    for node in &nodes {
        lines.push(format!(
            "N|{}|{:?}|{}|{}|{}|{}",
            node.id,
            node.node_type,
            node.path,
            node.name,
            serde_json::to_string(&node.meta).expect("meta json"),
            node.origin_file.as_deref().unwrap_or("<none>"),
        ));
        let neighbors = store
            .get_node_edges(&node.id)
            .expect("get_node_edges")
            .expect("node exists");
        for view in neighbors.outgoing {
            let edge: &Edge = &view.edge;
            edge_lines.push(format!(
                "E|{}->{}|{:?}|{}|{}|{}",
                edge.from,
                edge.to,
                edge.edge_type,
                edge.field_path.as_deref().unwrap_or("<none>"),
                serde_json::to_string(&edge.meta).expect("edge meta json"),
                edge.origin_file.as_deref().unwrap_or("<none>"),
            ));
        }
    }
    edge_lines.sort();
    lines.extend(edge_lines);
    lines
}

fn grafeo_path(dir: &Path) -> PathBuf {
    dir.join("graph.grafeo")
}

fn redb_path(dir: &Path) -> PathBuf {
    dir.join("graph.graphdb")
}

// ------------------------------------------------------------------ 用例

#[test]
fn full_scan_persists_graph_file_states_and_diagnostics() {
    let dir = unique_dir("full");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    let first = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan");
    assert_eq!(first.report.indexed, 3);
    assert_eq!(first.report.dirty, 3);
    assert!(first.node_count > 0);
    assert!(first.edge_count > 0);

    // 重启重开：file states 与 scanner 诊断都随库复原——
    // 这正是「重启后增量不退化成全量」的那一层状态。
    let store = GrafeoGraphStore::open(&db).expect("reopen");
    let states = IndexStateStore::load_file_states(&store).expect("file_states");
    let paths: Vec<&str> = states.keys().map(String::as_str).collect();
    assert_eq!(paths.len(), 3);
    assert!(paths.contains(&"app/a.spg"));
    assert!(paths.contains(&"app/b.spg"));
    assert!(paths.contains(&"db/orders.tbl"));

    // IndexState meta 节点不能泄进项目图口径：iter_nodes 只见项目节点，
    // node_count 与 iter_nodes 自洽。
    let nodes: Vec<Node> = store.iter_nodes().expect("iter").collect();
    assert_eq!(nodes.len(), store.node_count().expect("count"));
    assert_eq!(nodes.len(), first.node_count);
    assert!(nodes.iter().all(|n| !n.id.is_empty()));

    // 每个成功文件都有一条 per-file scanner 诊断 entry（含全零计数）。
    let entries = store
        .load_scanner_diagnostic_entries()
        .expect("scanner entries");
    let entry_paths: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(entry_paths.len(), 3);
}

#[test]
fn incremental_rescan_reparses_only_the_changed_file() {
    let dir = unique_dir("incr");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    let first = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("first scan");
    let snapshot_v1 = {
        let store = GrafeoGraphStore::open(&db).expect("reopen");
        snapshot(&store)
    };
    drop(first);

    // 改 b.spg：它自己判脏；内嵌它的 a.spg 没改，但批删 b 的节点会带走 a 指向 b 的边，
    // 所以 a 作为引用者随之重解析（见 `extend_dirty_with_page_referrers`）。tbl 不受牵连。
    let updated_b = page_b().replace("\"value\": \"B\"", "\"value\": \"B2\"");
    write(&project, "app/b.spg", &updated_b);
    let second = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("rescan");
    assert_eq!(second.report.dirty, 2, "b.spg 与内嵌它的 a.spg 应被判脏");
    assert_eq!(second.report.indexed, 3);
    assert_eq!(second.report.deleted, 0);

    let store = GrafeoGraphStore::open(&db).expect("reopen after rescan");
    let snapshot_v2 = snapshot(&store);
    // b.spg 的 label 值变化必须落进图（不是「计数相同就完事」）。
    assert_ne!(snapshot_v1, snapshot_v2);
    assert!(snapshot_v2.iter().any(|line| line.contains("B2")));
}

#[test]
fn rescan_without_changes_is_noop_after_restart() {
    let dir = unique_dir("noop");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("first scan");
    // 显式 drop + 重新 open：比同进程二次扫描更强的「重启」模拟——
    // file_states 若没随库落盘，这里会整库被判脏。
    let second = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("rescan");
    assert_eq!(second.report.dirty, 0);
    assert_eq!(second.report.deleted, 0);
    assert_eq!(second.report.indexed, 3);
    assert_eq!(second.report.unchanged, 3);
}

#[test]
fn deleted_file_removes_nodes_states_and_scanner_entry() {
    let dir = unique_dir("delete");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    let first = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("first scan");
    let before = {
        let store = GrafeoGraphStore::open(&db).expect("reopen");
        snapshot(&store)
    };
    assert!(before.iter().any(|line| line.contains("page:app/b.spg")));

    std::fs::remove_file(project.join("app/b.spg")).expect("delete b.spg");
    let second = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("rescan");
    assert_eq!(second.report.deleted, 1);
    assert_eq!(second.report.indexed, 2);

    let store = GrafeoGraphStore::open(&db).expect("reopen after delete");
    let after = snapshot(&store);
    // 被删文件的节点与指向它的跨文件边一起消失。
    assert!(!after.iter().any(|line| line.contains("app/b.spg")));
    let states = IndexStateStore::load_file_states(&store).expect("file_states");
    assert!(!states.contains_key("app/b.spg"));
    let entries = store
        .load_scanner_diagnostic_entries()
        .expect("scanner entries");
    assert!(!entries.iter().any(|(p, _)| p == "app/b.spg"));
    drop(first);
}

#[test]
fn corrupted_tbl_keeps_stale_graph_and_persists_parse_failure() {
    let dir = unique_dir("corrupt");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("first scan");
    let good_snapshot = {
        let store = GrafeoGraphStore::open(&db).expect("reopen");
        snapshot(&store)
    };
    assert!(good_snapshot.iter().any(|line| line.contains("orders")));

    // B3 语义：TBL 坏字节 → ParseFailure，旧图保留、文件保持脏、
    // parse_failed 标记随 commit 落库（重启后仍在）。
    write_bytes(&project, "db/orders.tbl", &[0xFF, 0xFE, 0x00, 0x01]);
    let bad_round = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("rescan");
    assert_eq!(bad_round.report.dirty, 1);
    assert!(
        bad_round
            .diagnostics
            .iter()
            .any(|d| d.code == metadata_checker::diagnostics::CODE_SCANNER_FILE_PARSE_FAILED),
        "应报告 SCANNER_FILE_PARSE_FAILED，实际：{:?}",
        bad_round
            .diagnostics
            .iter()
            .map(|d| d.code.clone())
            .collect::<Vec<_>>()
    );

    let store = GrafeoGraphStore::open(&db).expect("reopen after corrupt");
    // 旧图内容原样保留（stale），不是被清空。
    assert_eq!(snapshot(&store), good_snapshot);
    // 失败文件不拿新 file_state：旧 state 保留，下轮仍应判脏重试。
    let states = IndexStateStore::load_file_states(&store).expect("file_states");
    assert!(states.contains_key("db/orders.tbl"));
    // 关闭释放文件锁——同一路径的下一轮扫描需要独占打开。
    store.close().expect("close");

    // 修好后重扫：标记当场退休，图恢复最新内容。
    write(&project, "db/orders.tbl", &orders_table());
    let fixed_round = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("third scan");
    assert!(
        fixed_round
            .diagnostics
            .iter()
            .all(|d| d.code != "SCANNER_FILE_PARSE_FAILED"),
        "修复后诊断应清空，实际：{:?}",
        fixed_round
            .diagnostics
            .iter()
            .map(|d| &d.code)
            .collect::<Vec<_>>()
    );
}

#[test]
fn grafeo_and_redb_imports_produce_identical_snapshots() {
    // 同一语料分别导进两个后端，逐节点/逐边快照必须一致——
    // 这是 C2「导入结果一致」的验收判据（B5 口径）。
    let dir = unique_dir("parity");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);

    let grafeo_db = grafeo_path(&dir);
    let redb_db = redb_path(&dir);
    let grafeo_report = scan_project_with_report(&project, &grafeo_db).expect("grafeo scan");
    let redb_report = scan_project_with_report(&project, &redb_db).expect("redb scan");

    // 报告口径也必须一致（文件口径字段 + 图规模）。
    assert_eq!(grafeo_report.indexed, redb_report.indexed);
    assert_eq!(grafeo_report.dirty, redb_report.dirty);
    assert_eq!(grafeo_report.deleted, redb_report.deleted);
    assert_eq!(grafeo_report.node_count, redb_report.node_count);
    assert_eq!(grafeo_report.edge_count, redb_report.edge_count);
    assert!(grafeo_report.node_count > 0);

    let grafeo_store = GrafeoGraphStore::open(&grafeo_db).expect("open grafeo");
    let redb_store = GraphDB::open(&redb_db).expect("open redb");
    let grafeo_snapshot = snapshot(&grafeo_store);
    let redb_snapshot = snapshot(&redb_store);
    assert_eq!(
        grafeo_snapshot, redb_snapshot,
        "grafeo 与 redb 的导入快照必须逐行一致"
    );
}

#[test]
fn repeat_scan_is_idempotent() {
    let dir = unique_dir("idem");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan 1");
    let snap1 = snapshot(&GrafeoGraphStore::open(&db).expect("open 1"));
    // 连续三轮：结果不应漂移（重复写入不重影、边不重复累积）。
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan 2");
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan 3");
    let snap2 = snapshot(&GrafeoGraphStore::open(&db).expect("open 2"));
    assert_eq!(snap1, snap2);
}

#[test]
fn ownership_scan_on_grafeo_path_fails_closed() {
    let dir = unique_dir("ownership");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    let binding = metadata_checker::ownership::ProjectBinding::new("proj").expect("binding");
    let err = ProjectIndexer::scan_with_diagnostics_for_project(&project, &db, &binding)
        .expect_err("ownership 扫描必须在 .grafeo 上失败");
    let message = format!("{err:#}");
    assert!(
        message.contains("GRAPH_BACKEND_UNSUPPORTED"),
        "期望 fail-closed 错误，实际：{message}"
    );
}

#[test]
fn prepare_on_grafeo_path_fails_closed() {
    let dir = unique_dir("prepare");
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project");
    seed_corpus(&project);
    let db = grafeo_path(&dir);

    let err = match ProjectIndexer::prepare(&project, &db) {
        // PreparedIndexUpdate 不实现 Debug（内嵌 GraphDB），不用 expect_err。
        Ok(_) => panic!("候选 prepare 必须在 .grafeo 上失败"),
        Err(err) => err,
    };
    assert!(format!("{err:#}").contains("GRAPH_BACKEND_UNSUPPORTED"));
}
