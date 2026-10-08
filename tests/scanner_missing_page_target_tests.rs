#![cfg(all(feature = "cli-local", feature = "grafeo-store"))]

//! L0b：路径解析正确、但目标文件不存在的页面引用（语料 autocrm 上 7 个页面、75 处引用）。
//!
//! 固定的行为：
//! - 目标文件不在本次发现的文件里：不建边、不造 Page 节点、不造挂在目标页名下的
//!   link 参数节点，每处引用各记一条 `SCANNER_UNRESOLVED_REFERENCE`；
//! - 判定用本次发现的文件集合，所以目标页面出现、消失或被改写后，**未改动**的引用者
//!   也会被重解析，增量扫描的结果与全量扫描逐节点、逐边、逐诊断记录相等；
//! - 旧语义版本留下的无文件占位，在升级后的第一次扫描里被清掉。
//!
//! 每个场景在 redb 与 grafeo 两个后端上各跑一遍（两者共用同一套扫描编排）。

use metadata_checker::graph::{FileState, GraphDB, Node, NodeType};
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_store::{
    GraphReadStore, GraphWriteStore, IndexCommit, IndexStateStore,
};
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CODE_UNRESOLVED: &str = "SCANNER_UNRESOLVED_REFERENCE";
const BACKENDS: [&str; 2] = ["graph.grafeo", "graph.graphdb"];

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("l0b-{tag}-{nanos}-{}", std::process::id()));
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

/// 页 A：内嵌 `ghost.spg`，并有一个带参数的 link 动作指向同一个 `ghost.spg`，
/// 另有一个 link 动作指向 `b.spg`。`ghost.spg` 是否存在由各用例决定。
fn page_a() -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "referenceResources": ["ghost.spg", "b.spg"],
        "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "embed_x", "type": "embedsuperpage", "resPath": "0"},
            {"id": "embed_b", "type": "embedsuperpage", "resPath": "1"},
            {"id": "btn", "type": "button", "actions": [
                {"id": "go_x", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 0,
                 "data": [{"name": "order_id", "value": "1"}]},
                {"id": "go_b", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 1}
            ]}
        ]}
    })
    .to_string()
}

/// 内嵌 `ref_target` 的页面（`referenceResources[0]`）。
fn page_embedding(ref_target: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "referenceResources": [ref_target],
        "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "embed1", "type": "embedsuperpage", "resPath": "0"}
        ]}
    })
    .to_string()
}

fn plain_page(title: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "label", "type": "text", "value": title}
        ]}
    })
    .to_string()
}

// ------------------------------------------------------------------ 快照比对

/// 规范化快照：节点带全部属性，边按四元组加元数据，另附未解析引用的逐次记录。
/// 比对内容而不是计数——计数挡不住「丢一条边同时多一条占位边」。
fn snapshot(db: &Path) -> Vec<String> {
    let mut lines = if db.extension().is_some_and(|ext| ext == "grafeo") {
        graph_lines(&GrafeoGraphStore::open(db).expect("open grafeo"))
    } else {
        graph_lines(&GraphDB::open(db).expect("open redb"))
    };
    let entries = if db.extension().is_some_and(|ext| ext == "grafeo") {
        GrafeoGraphStore::open(db)
            .expect("open grafeo")
            .load_scanner_diagnostic_entries()
            .expect("entries")
    } else {
        GraphDB::open(db)
            .expect("open redb")
            .load_scanner_diagnostic_entries()
            .expect("entries")
    };
    lines.extend(unresolved_records(&entries));
    lines.sort();
    lines
}

fn graph_lines(store: &dyn GraphReadStore) -> Vec<String> {
    let mut lines = Vec::new();
    let nodes: Vec<Node> = store.iter_nodes().expect("iter_nodes").collect();
    for node in &nodes {
        lines.push(format!(
            "N|{}|{:?}|{}|{}|{}",
            node.id,
            node.node_type,
            node.path,
            node.name,
            serde_json::to_string(&node.meta).expect("meta"),
        ));
        let neighbors = store
            .get_node_edges(&node.id)
            .expect("edges")
            .expect("node exists");
        for view in neighbors.outgoing {
            let edge = &view.edge;
            lines.push(format!(
                "E|{}->{}|{:?}|{}|{}",
                edge.from,
                edge.to,
                edge.edge_type,
                edge.field_path.as_deref().unwrap_or("<none>"),
                serde_json::to_string(&edge.meta).expect("edge meta"),
            ));
        }
    }
    lines
}

fn unresolved_records(entries: &[(String, Vec<u8>)]) -> Vec<String> {
    ProjectIndexer::merge_scanner_occurrence_entries(entries)
        .expect("merge occurrences")
        .occurrences
        .iter()
        .filter(|record| record.code == CODE_UNRESOLVED)
        .map(|record| {
            format!(
                "R|{}|{}|{}|{}",
                record.location.source_file.clone().unwrap_or_default(),
                record.location.node_id.clone().unwrap_or_default(),
                record.location.json_path.clone().unwrap_or_default(),
                record.detail.clone().unwrap_or_default(),
            )
        })
        .collect()
}

fn page_paths(db: &Path) -> Vec<String> {
    let mut paths: Vec<String> = snapshot(db)
        .iter()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            (parts.first() == Some(&"N") && parts.get(2) == Some(&"Page"))
                .then(|| parts[3].to_string())
        })
        .collect();
    paths.sort();
    paths
}

/// 在 `project` 上从零全量扫一遍，返回快照（用 `tag` 区分库文件）。
fn full_snapshot(project: &Path, dir: &Path, backend: &str, tag: &str) -> Vec<String> {
    let db = dir.join(format!("{tag}-{backend}"));
    ProjectIndexer::scan_with_diagnostics(project, &db).expect("full scan");
    snapshot(&db)
}

fn assert_same(label: &str, full: &[String], incremental: &[String]) {
    let only_full: Vec<&String> = full.iter().filter(|l| !incremental.contains(l)).collect();
    let only_inc: Vec<&String> = incremental.iter().filter(|l| !full.contains(l)).collect();
    assert_eq!(
        full, incremental,
        "{label}：增量与全量必须逐项相等\n仅全量有：{only_full:#?}\n仅增量有：{only_inc:#?}"
    );
}

// ------------------------------------------------------------------ 用例

/// 目标文件不存在：没有 Page、没有边、没有目标页名下的参数节点；每处引用各一条记录。
#[test]
fn missing_target_leaves_no_page_edge_or_param_node_and_one_record_per_site() {
    for backend in BACKENDS {
        let dir = unique_dir("missing");
        let project = dir.join("proj");
        write(&project, "app/a.spg", &page_a());
        write(&project, "app/b.spg", &plain_page("B"));
        let db = dir.join(backend);
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan");

        assert_eq!(
            page_paths(&db),
            vec!["app/a.spg".to_string(), "app/b.spg".to_string()],
            "{backend}：只有磁盘上有文件的页面；ghost.spg 不得留下节点"
        );
        let lines = snapshot(&db);
        assert_eq!(
            lines
                .iter()
                .any(|line| (line.starts_with("N|") || line.starts_with("E|"))
                    && line.contains("ghost")),
            false,
            "{backend}：不得有指向或挂在 ghost 名下的节点或边：{lines:#?}"
        );
        assert_eq!(
            lines.iter().any(|line| line.starts_with("N|param:ghost/")),
            false,
            "{backend}：目标页不存在，link 参数节点也不该有"
        );
        let embeds = lines.iter().filter(|l| l.contains("|EmbedsPage|")).count();
        let navigates = lines
            .iter()
            .filter(|l| l.contains("|ActionNavigates|"))
            .count();
        assert_eq!(
            (embeds, navigates),
            (1, 1),
            "{backend}：只有指向 b.spg 的两条边"
        );

        let records: Vec<&String> = lines.iter().filter(|l| l.starts_with("R|")).collect();
        assert_eq!(
            records.len(),
            2,
            "{backend}：内嵌与 link 各一条：{records:#?}"
        );
        assert_eq!(
            records
                .iter()
                .all(|r| r.contains("reference target page does not exist: app/ghost.spg")),
            true,
            "{backend}：记录要说明目标页面不存在，并带上解析出的路径：{records:#?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 目标页面后来出现：未改动的引用者被重解析，诊断换成边；结果等于全量。
#[test]
fn target_appearing_later_turns_the_diagnostic_into_an_edge() {
    for backend in BACKENDS {
        let dir = unique_dir("appear");
        let project = dir.join("proj");
        write(&project, "app/a.spg", &page_a());
        write(&project, "app/b.spg", &plain_page("B"));
        let db = dir.join(backend);
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("initial scan");

        write(&project, "app/ghost.spg", &plain_page("now real"));
        let report = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("incremental");
        assert_eq!(
            report.report.dirty, 2,
            "{backend}：新文件与它的引用者 a.spg"
        );

        let full = full_snapshot(&project, &dir, backend, "appear-full");
        let incremental = snapshot(&db);
        assert_same(&format!("{backend} 目标出现"), &full, &incremental);
        assert_eq!(
            incremental.iter().any(|l| l.starts_with("R|")),
            false,
            "{backend}：目标已存在，不应再有未解析记录"
        );
        assert_eq!(
            incremental
                .iter()
                .any(|l| l.starts_with("N|param:ghost/order_id")),
            true,
            "{backend}：目标存在时 link 参数节点照旧"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 目标页面后来被删：边换成诊断，页面与参数节点不残留；结果等于全量。
#[test]
fn target_disappearing_turns_the_edge_into_a_diagnostic() {
    for backend in BACKENDS {
        let dir = unique_dir("disappear");
        let project = dir.join("proj");
        write(&project, "app/a.spg", &page_a());
        write(&project, "app/b.spg", &plain_page("B"));
        write(&project, "app/ghost.spg", &plain_page("real"));
        let db = dir.join(backend);
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("initial scan");
        assert_eq!(
            snapshot(&db).iter().any(|l| l.starts_with("R|")),
            false,
            "{backend}：起点没有未解析记录"
        );

        std::fs::remove_file(project.join("app/ghost.spg")).expect("remove target");
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("incremental");

        let full = full_snapshot(&project, &dir, backend, "disappear-full");
        let incremental = snapshot(&db);
        assert_same(&format!("{backend} 目标消失"), &full, &incremental);
        assert_eq!(
            incremental.iter().filter(|l| l.starts_with("R|")).count(),
            2,
            "{backend}：内嵌与 link 各记一条"
        );
        assert_eq!(
            page_paths(&db).contains(&"app/ghost.spg".to_string()),
            false
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 目标页面被改写：批删它的节点会连带删掉引用者指向它的边，引用者必须被重解析把边补回。
#[test]
fn rewriting_the_target_keeps_the_referrers_edges() {
    for backend in BACKENDS {
        let dir = unique_dir("rewrite");
        let project = dir.join("proj");
        write(&project, "app/a.spg", &page_a());
        write(&project, "app/b.spg", &plain_page("B 旧"));
        let db = dir.join(backend);
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("initial scan");

        write(&project, "app/b.spg", &plain_page("B 新"));
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("incremental");

        let full = full_snapshot(&project, &dir, backend, "rewrite-full");
        let incremental = snapshot(&db);
        assert_same(&format!("{backend} 目标改写"), &full, &incremental);
        assert_eq!(
            incremental
                .iter()
                .any(|l| l.contains("|EmbedsPage|") && l.contains("page:app/b.spg")),
            true,
            "{backend}：指向 b.spg 的内嵌边必须还在"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 没有人引用的页面变化，不牵动其他文件。
#[test]
fn unrelated_page_changes_do_not_reparse_other_files() {
    let dir = unique_dir("unrelated");
    let project = dir.join("proj");
    write(&project, "app/a.spg", &page_a());
    write(&project, "app/b.spg", &plain_page("B"));
    write(&project, "app/lonely.spg", &plain_page("没人引用"));
    let db = dir.join("graph.grafeo");
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("initial scan");

    write(&project, "app/lonely.spg", &plain_page("改了"));
    let report = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("incremental");
    assert_eq!(report.report.dirty, 1, "只有被改的文件重解析");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 引用链 C → B → A：改 A 时，B 因 A 重解析，B 的 Page 节点随之被批删，指向 B 的 C 的边
/// 也要靠重解析 C 补回（传递闭包）。
#[test]
fn rewriting_a_page_reparses_referrers_of_referrers() {
    for backend in BACKENDS {
        let dir = unique_dir("chain");
        let project = dir.join("proj");
        write(&project, "app/a.spg", &plain_page("A 旧"));
        write(&project, "app/b.spg", &page_embedding("a.spg"));
        write(&project, "app/c.spg", &page_embedding("b.spg"));
        let db = dir.join(backend);
        ProjectIndexer::scan_with_diagnostics(&project, &db).expect("initial scan");

        write(&project, "app/a.spg", &plain_page("A 新"));
        let report = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("incremental");
        assert_eq!(
            report.report.dirty, 3,
            "{backend}：a 与它的引用者 b、b 的引用者 c"
        );

        let full = full_snapshot(&project, &dir, backend, "chain-full");
        assert_same(&format!("{backend} 引用链"), &full, &snapshot(&db));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 把库改成旧语义版本留下的样子：指纹换成旧版本，并补回旧版本为 `ghost.spg` 留下的占位。
fn plant_legacy_stubs<S: GraphWriteStore + IndexStateStore>(store: &mut S) {
    let mut states: HashMap<String, FileState> =
        IndexStateStore::load_file_states(store).expect("states");
    for state in states.values_mut() {
        state.file_hash = state.file_hash.replacen("s3-", "s2-", 1);
    }
    let stub = |id: &str, node_type: NodeType, name: &str| Node {
        id: id.to_string(),
        node_type,
        path: "app/ghost.spg".to_string(),
        name: name.to_string(),
        meta: None,
        origin_file: None,
    };
    let stubs = [
        stub("page:app/ghost.spg", NodeType::Page, "ghost"),
        stub("param:ghost/order_id", NodeType::Field, "order_id"),
    ];
    for node in &stubs {
        store.upsert_node(node.clone()).expect("upsert stub");
    }
    let commit = IndexCommit {
        file_states: states,
        dirty_nodes: stubs.iter().map(|node| node.id.clone()).collect(),
        deleted_nodes: Vec::new(),
        checkpoint: None,
        delta: None,
        scanner_entries: Vec::new(),
        scanner_deleted_paths: Vec::new(),
    };
    IndexStateStore::persist_index(store, commit).expect("persist legacy state");
}

/// 旧语义版本留下的无文件 Page 与目标页名下的参数节点，在升级后的第一次扫描里被清掉，
/// 结果等于直接全量扫描；之后的扫描不再动它们。
#[test]
fn upgrade_scan_sweeps_stubs_left_by_the_previous_scanner_version() {
    let dir = unique_dir("upgrade");
    let project = dir.join("proj");
    write(&project, "app/a.spg", &page_a());
    write(&project, "app/b.spg", &plain_page("B"));
    let db = dir.join("graph.grafeo");
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan");
    let expected = snapshot(&db);

    plant_legacy_stubs(&mut GrafeoGraphStore::open(&db).expect("open"));
    assert_eq!(
        snapshot(&db)
            .iter()
            .any(|l| l.starts_with("N|page:app/ghost.spg|")),
        true,
        "前置：旧版本的占位已就位"
    );

    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("upgrade scan");
    assert_same("升级扫描", &expected, &snapshot(&db));

    let again = ProjectIndexer::scan_with_diagnostics(&project, &db).expect("no-op scan");
    assert_eq!(again.report.dirty, 0, "升级之后的扫描是空操作");
    let _ = std::fs::remove_dir_all(&dir);
}

/// diff-refresh 的候选 prepare 路径（redb）同样在升级时清掉旧占位。
#[test]
fn upgrade_prepare_sweeps_stubs_left_by_the_previous_scanner_version() {
    let dir = unique_dir("upgrade-prepare");
    let project = dir.join("proj");
    write(&project, "app/a.spg", &page_a());
    write(&project, "app/b.spg", &plain_page("B"));
    let db = dir.join("graph.graphdb");
    ProjectIndexer::scan_with_diagnostics(&project, &db).expect("scan");
    plant_legacy_stubs(&mut GraphDB::open(&db).expect("open"));

    let prepared = ProjectIndexer::prepare(&project, &db).expect("prepare");
    let stubs: Vec<String> = prepared
        .graph
        .iter_nodes()
        .expect("iter_nodes")
        .filter(|node| node.path == "app/ghost.spg")
        .map(|node| node.id)
        .collect();
    assert_eq!(stubs, Vec::<String>::new(), "候选图里不得有旧占位");
    let _ = std::fs::remove_dir_all(&dir);
}
