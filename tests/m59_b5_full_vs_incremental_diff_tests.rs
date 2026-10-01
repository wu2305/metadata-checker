//! M59 阶段 B5：**全量 vs 增量差分测试**。
//!
//! spec §3 明确要求「同一最终文件集，逐节点/逐边/逐属性比对（**非计数比对**）」。
//! 计数比对挡不住最要命的一类错误：丢一条边、同时多一条占位边，总数不变。
//! 所以这里比对的是**规范化快照**——节点按 id 排序、连同 `node_type`/`path`/
//! `name`/`meta` 全量序列化；边按四元组排序、连同 `meta` 序列化。任何一处属性
//! 不同都会显示在 diff 里。
//!
//! ## 为什么这套测试现在就有价值
//!
//! 它是 §2.1（增量更新丢失跨文件边）的**可执行判据**。该缺陷此前只有源码论证，
//! 没有任何测试能证明它存在、也没有任何测试能在 A3 修好后证明它消失了。
//! 本文件两件事一起做：
//!
//! 1. `no_cross_file_edges_*`：不含跨文件边时，全量与增量必须**逐字节一致**。
//!    这条现在就通过，且证明差分器不是空转——它确实在比较真实内容。
//! 2. `cross_file_edge_*`：含跨文件边时，如实钉住**当前的分歧**。
//!    这不是认可，是把缺陷变成会说话的断言。**A3 落地后这条会立刻变红**，
//!    届时把期望改成「两侧完全一致」即可——那正是 A3 的验收判据。

#![cfg(feature = "cli-local")]

use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::ownership::ProjectBinding;
use metadata_checker::scanner::indexer::ProjectIndexer;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ 语料构造

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-b5-{tag}-{nanos}-{}", std::process::id()));
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

/// 页 A：内嵌页 B（产生**跨文件**边 `comp:app/a.spg|embed1 -EmbedsPage-> page:app/b.spg`）。
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

/// 页 A 的独立版本：**不**引用 B，用作「无跨文件边」对照组。
fn page_a_standalone() -> String {
    r#"{
  "version": "4.19.7",
  "theme": "default",
  "params": [],
  "sources": [],
  "canvas": {
    "id": "canvas",
    "type": "canvas",
    "components": [
      {"id": "label_a", "type": "text", "value": "A"}
    ]
  }
}"#
    .to_string()
}

/// 页 A：内嵌一个**不存在**的页面，用于观察占位节点的归属。
fn page_a_embedding_ghost() -> String {
    r#"{
  "version": "4.19.7",
  "theme": "default",
  "params": [],
  "referenceResources": ["ghost.spg"],
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

/// 页 B。`title` 参与内容哈希，改它即可让 B 变脏而 A 不变。
fn page_b(title: &str) -> String {
    format!(
        r#"{{
  "version": "4.19.7",
  "theme": "default",
  "params": [],
  "sources": [],
  "canvas": {{
    "id": "canvas",
    "type": "canvas",
    "components": [
      {{"id": "label_b", "type": "text", "value": "{title}"}}
    ]
  }}
}}"#
    )
}

// ------------------------------------------------------------ 规范化快照与差分

/// 图的规范化快照：节点与边**各自逐属性**展开、排序，供逐行比对。
#[derive(Debug, PartialEq)]
struct GraphSnapshot {
    nodes: Vec<String>,
    edges: Vec<String>,
}

fn snapshot(db_path: &Path, binding: &ProjectBinding) -> GraphSnapshot {
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
                // meta 与 origin 都参与比对：占位升级和来源撤销不能被掩盖。
                n.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string()),
                n.origin_file.as_deref().unwrap_or("-")
            )
        })
        .collect();
    nodes.sort();

    let node_ids: Vec<String> = {
        let mut ids: Vec<String> = graph.iter_nodes().expect("iter").map(|n| n.id).collect();
        ids.sort();
        ids
    };

    let mut edges: Vec<String> = Vec::new();
    for id in &node_ids {
        // 走 trait 方法，不走 `GraphDB` 的同名 inherent 方法——后者返回借用元组，
        // 是 redb 专有形状；B5 比对的是**契约**层面的图内容，将来接 Grafeo 时
        // 这段一行不用改。
        let Some(neighbors) = GraphReadStore::get_node_edges(&graph, id).expect("edges") else {
            continue;
        };
        for view in neighbors.outgoing {
            let e = view.edge;
            edges.push(format!(
                "{} -{:?}-> {}\tfield_path={}\tmeta={}\torigin={}",
                e.from,
                e.edge_type,
                e.to,
                e.field_path.unwrap_or_else(|| "-".to_string()),
                e.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string()),
                e.origin_file.as_deref().unwrap_or("-")
            ));
        }
    }
    edges.sort();

    GraphSnapshot { nodes, edges }
}

/// 人类可读的逐行差分。只在断言失败时构造。
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

/// 跑两条路径，返回 (全量快照, 增量快照)。
///
/// - **全量**：直接以最终文件集在空库上索引一次。
/// - **增量**：先索引初版，改动 `b.spg` 后再索引一次（走增量路径）。
///
/// 两者最终的文件内容完全相同，因此**任何差异都来自索引路径本身**。
fn run_both_paths(
    tag: &str,
    page_a: &str,
    b_before: &str,
    b_after: &str,
) -> (GraphSnapshot, GraphSnapshot) {
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid project binding");
    // 全量
    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(&full_project, "app/a.spg", page_a);
    write(&full_project, "app/b.spg", b_after);
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full scan");

    // 增量
    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(&inc_project, "app/a.spg", page_a);
    write(&inc_project, "app/b.spg", b_before);
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("initial scan");

    write(&inc_project, "app/b.spg", b_after);
    let report = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding)
        .expect("incremental scan");
    assert!(
        report.dirty > 0,
        "第二次扫描必须真的走了增量更新路径（dirty > 0），否则本测试什么都没验到"
    );

    let out = (snapshot(&full_db, &binding), snapshot(&inc_db, &binding));
    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
    out
}

// ---------------------------------------------------------------------- 用例

/// 对照组：没有跨文件边时，全量与增量必须**完全一致**。
///
/// 这条同时验证差分器本身有效——它确实在比对真实内容，而不是拿两个空集互比。
#[test]
fn without_cross_file_edges_full_and_incremental_are_identical() {
    let (full, inc) = run_both_paths(
        "standalone",
        &page_a_standalone(),
        &page_b("旧标题"),
        &page_b("新标题"),
    );

    assert!(
        !full.nodes.is_empty() && !full.edges.is_empty(),
        "差分器不得空转：全量快照必须真的有内容，实际 nodes={} edges={}",
        full.nodes.len(),
        full.edges.len()
    );
    assert_eq!(
        full.nodes,
        inc.nodes,
        "节点逐属性比对应完全一致：{}",
        describe_diff("全量", &full.nodes, "增量", &inc.nodes)
    );
    assert_eq!(
        full.edges,
        inc.edges,
        "边逐属性比对应完全一致：{}",
        describe_diff("全量", &full.edges, "增量", &inc.edges)
    );
}

/// 改动的文件本身，其节点在两条路径上必须一致——B 自己的内容是被重新解析的，
/// 不受 §2.1 影响。把它单独断出来，是为了确认下一条测试里的分歧**只**出在跨文件边，
/// 而不是增量路径整体失灵。
#[test]
fn the_changed_file_itself_is_rebuilt_identically() {
    let (full, inc) = run_both_paths(
        "changed-file",
        &page_a_embedding_b(),
        &page_b("旧标题"),
        &page_b("新标题"),
    );

    let b_nodes = |snap: &GraphSnapshot| -> Vec<String> {
        snap.nodes
            .iter()
            .filter(|n| n.contains("app/b.spg"))
            .cloned()
            .collect()
    };
    assert_eq!(
        b_nodes(&full),
        b_nodes(&inc),
        "被改动文件自身的节点必须重建得一模一样：{}",
        describe_diff("全量", &b_nodes(&full), "增量", &b_nodes(&inc))
    );
    assert!(
        b_nodes(&full).iter().any(|n| n.contains("新标题")),
        "应当读到改动后的内容，实际 {:?}",
        b_nodes(&full)
    );
    // 过滤后的子集相等还不够：若增量侧整体塌了（比如 A 的节点也没了），
    // 上面的断言依旧成立。节点集在这个场景里两侧本就应当完全一致——
    // §2.1 丢的是边，不是节点。
    assert_eq!(
        full.nodes,
        inc.nodes,
        "本场景两侧节点集应完全一致：{}",
        describe_diff("全量", &full.nodes, "增量", &inc.nodes)
    );
}

/// 跨文件边必须按来源保留：改动 B 只能替换 B 的贡献，A→B 的边不能消失。
#[test]
fn cross_file_edge_survives_incremental_update() {
    let (full, inc) = run_both_paths(
        "cross-file",
        &page_a_embedding_b(),
        &page_b("旧标题"),
        &page_b("新标题"),
    );

    assert_eq!(
        full.nodes,
        inc.nodes,
        "ownership 全量与增量节点必须逐属性完全一致：{}",
        describe_diff("全量", &full.nodes, "增量", &inc.nodes)
    );
    assert_eq!(
        full.edges,
        inc.edges,
        "ownership 全量与增量边必须逐属性完全一致：{}",
        describe_diff("全量", &full.edges, "增量", &inc.edges)
    );

    let embeds: Vec<&String> = full
        .edges
        .iter()
        .filter(|edge| edge.contains("EmbedsPage"))
        .collect();
    assert_eq!(embeds.len(), 1, "独立预期：最终图恰有一条内嵌边");
    assert!(embeds[0].contains("comp:app/a.spg|embed1"));
    assert!(embeds[0].contains("page:app/b.spg"));
    assert!(embeds[0].contains("origin=app/a.spg"));

    // 独立预期（冷脸 P1 回归钉）：被嵌目标页的 Definition 只能来自它自己的
    // 文件；embedder 的 stub 是 Reference，不允许覆盖目标页的 origin_file。
    let b_page_nodes: Vec<&String> = full
        .nodes
        .iter()
        .filter(|node| node.starts_with("page:app/b.spg\t"))
        .collect();
    assert_eq!(
        b_page_nodes.len(),
        1,
        "目标页节点应恰有一个：{:?}",
        b_page_nodes
    );
    assert!(
        b_page_nodes[0].contains("origin=app/b.spg"),
        "被嵌目标页的 origin_file 必须属于其自身定义，实际 {}",
        b_page_nodes[0]
    );
}

/// 两个同 stem 的 TBL（全局 model:{stem} 撞 id）必须报告来源冲突，不得静默择一。
#[test]
fn conflicting_tbl_model_definitions_are_reported() {
    let tag = "tbl-conflict";
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    // TBL 主模型 id 取文件 stem（非 JSON name）：同 stem 不同目录才会撞 id
    write(&project, "tables/a.tbl", &table_json("dup", &["f_a"]));
    write(&project, "tables/other/a.tbl", &table_json("dup", &["f_b"]));

    let outcome = ProjectIndexer::scan_with_diagnostics_for_project(
        &project,
        &dir.join("graph.db"),
        &binding,
    )
    .expect("scan with diagnostics");

    assert!(
        outcome
            .diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_OWNERSHIP_CONFLICT"
                && d.message.contains("model:a")
                && d.message.contains("tables/a.tbl")
                && d.message.contains("tables/other/a.tbl")),
        "同 stem TBL 主模型必须报告 GRAPH_OWNERSHIP_CONFLICT：{:?}",
        outcome.diagnostics
    );

    // 冲突只报告不阻断：图仍按确定性规则择一构建，且重复扫描（no-op）继续报告
    let again = ProjectIndexer::scan_with_diagnostics_for_project(
        &project,
        &dir.join("graph.db"),
        &binding,
    )
    .expect("no-op rescan");
    assert!(
        again
            .diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_OWNERSHIP_CONFLICT"),
        "no-op 重扫必须继续报告存量冲突：{:?}",
        again.diagnostics
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// 最终文件集只有 `b.spg`（`a.spg` 被删）时的两条路径。
fn run_both_paths_with_deletion(
    tag: &str,
    page_a: &str,
    page_b_src: &str,
) -> (GraphSnapshot, GraphSnapshot) {
    let binding =
        ProjectBinding::new(format!("m59-b5-delete-{tag}")).expect("valid project binding");
    // 全量：a.spg 从未存在过。
    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(&full_project, "app/b.spg", page_b_src);
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full scan");

    // 增量：先有 a.spg，再删掉。
    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(&inc_project, "app/a.spg", page_a);
    write(&inc_project, "app/b.spg", page_b_src);
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("initial scan");

    std::fs::remove_file(inc_project.join("app/a.spg")).expect("remove a.spg");
    let report = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding)
        .expect("incremental scan after delete");
    assert_eq!(
        report.deleted, 1,
        "第二次扫描必须真的把 a.spg 当成已删除处理，否则本测试什么都没验到"
    );

    let out = (snapshot(&full_db, &binding), snapshot(&inc_db, &binding));
    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
    out
}

/// 删除唯一引用者后，占位目标必须回收，不能留下悬空节点或来源贡献。
#[test]
fn placeholder_page_node_is_reclaimed_after_its_only_referrer_is_deleted() {
    let (full, inc) =
        run_both_paths_with_deletion("ghost-embed", &page_a_embedding_ghost(), &page_b("标题"));

    assert_eq!(
        full.nodes,
        inc.nodes,
        "删除唯一引用者后全量与增量节点必须完全一致：{}",
        describe_diff("全量", &full.nodes, "增量", &inc.nodes)
    );
    assert_eq!(
        full.edges,
        inc.edges,
        "删除唯一引用者后全量与增量边必须完全一致：{}",
        describe_diff("全量", &full.edges, "增量", &inc.edges)
    );
    assert!(
        !inc.nodes.iter().any(|node| node.contains("ghost")),
        "独立预期：唯一引用者删除后不得残留 ghost 占位节点"
    );
    assert!(
        !inc.nodes.iter().any(|node| node.contains("app/a.spg")),
        "独立预期：a.spg 的实体贡献必须全部撤销"
    );
}

// ------------------------------------------------------------------ 扩展场景辅助

fn page_with_dwtable(title: &str, source_id: &str, tbl_path: &str) -> String {
    serde_json::json!({
        "version": "4.19.7",
        "theme": "default",
        "params": [],
        "sources": [
            {
                "id": source_id,
                "modelType": "dwtable",
                "path": tbl_path
            }
        ],
        "canvas": {
            "id": "canvas",
            "type": "canvas",
            "components": [
                {"id": "label", "type": "text", "value": title}
            ]
        }
    })
    .to_string()
}

fn table_json(name: &str, fields: &[&str]) -> String {
    let dims: Vec<_> = fields
        .iter()
        .map(|f| serde_json::json!({"name": f, "dataType": "C"}))
        .collect();
    serde_json::json!({
        "version": "1.0",
        "name": name,
        "dimensions": dims
    })
    .to_string()
}

/// 跨页同名 dwtable 必须局部隔离，物理表共享，且 dwtable 不得误降为 PhysicalTable。
#[test]
fn cross_page_same_name_dwtable_isolated_and_shared_physical_tables() {
    let tag = "cross-page-same-name";
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid binding");

    // 全量
    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(
        &full_project,
        "app/a.spg",
        &page_with_dwtable("Page A", "src1", "$DATA:/tables/tbl_a.tbl"),
    );
    write(
        &full_project,
        "app/b.spg",
        &page_with_dwtable("Page B", "src1", "$DATA:/tables/tbl_b.tbl"),
    );
    write(
        &full_project,
        "app/c.spg",
        &page_with_dwtable("Page C", "src_c", "$DATA:/tables/tbl_a.tbl"),
    );
    write(
        &full_project,
        "tables/tbl_a.tbl",
        &table_json("tbl_a", &["f_a"]),
    );
    write(
        &full_project,
        "tables/tbl_b.tbl",
        &table_json("tbl_b", &["f_b"]),
    );
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full scan");

    // 增量
    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(
        &inc_project,
        "app/a.spg",
        &page_with_dwtable("Page A", "src1", "$DATA:/tables/tbl_a.tbl"),
    );
    write(
        &inc_project,
        "tables/tbl_a.tbl",
        &table_json("tbl_a", &["f_a"]),
    );
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("initial scan");

    // 逐步加入 page B 和 page C
    write(
        &inc_project,
        "app/b.spg",
        &page_with_dwtable("Page B", "src1", "$DATA:/tables/tbl_b.tbl"),
    );
    write(
        &inc_project,
        "app/c.spg",
        &page_with_dwtable("Page C", "src_c", "$DATA:/tables/tbl_a.tbl"),
    );
    write(
        &inc_project,
        "tables/tbl_b.tbl",
        &table_json("tbl_b", &["f_b"]),
    );
    let report = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding)
        .expect("incremental scan");
    assert!(report.dirty > 0);

    let full_snap = snapshot(&full_db, &binding);
    let inc_snap = snapshot(&inc_db, &binding);

    assert_eq!(
        full_snap.nodes,
        inc_snap.nodes,
        "跨页同名模型节点逐属性全量与增量必须完全一致：{}",
        describe_diff("全量", &full_snap.nodes, "增量", &inc_snap.nodes)
    );
    assert_eq!(
        full_snap.edges,
        inc_snap.edges,
        "跨页同名模型边逐属性全量与增量必须完全一致：{}",
        describe_diff("全量", &full_snap.edges, "增量", &inc_snap.edges)
    );

    // 独立预期验证：局部模型隔离、物理表共享、dwtable 不被误改为 PhysicalTable
    let graph = GraphDB::open_readonly_with_ownership(&inc_db, &binding).expect("open inc db");
    let node_a = graph
        .get_node("model:app/a.spg|src1")
        .expect("local model A must exist");
    let node_b = graph
        .get_node("model:app/b.spg|src1")
        .expect("local model B must exist");
    assert_ne!(
        node_a.id, node_b.id,
        "两页同名 src1 必须拥有互不相同的局部节点 ID"
    );

    // 验证 modelType 仍为 dwtable，未被覆盖为 PhysicalTable
    let meta_a = node_a.meta.as_ref().unwrap();
    assert_eq!(
        meta_a.get("modelType").unwrap().as_str().unwrap(),
        "dwtable"
    );
    let meta_b = node_b.meta.as_ref().unwrap();
    assert_eq!(
        meta_b.get("modelType").unwrap().as_str().unwrap(),
        "dwtable"
    );

    // 验证物理表共享：model:tbl_a 应同时接收来自 app/a.spg 与 app/c.spg 的输入边
    let tbl_a_edges = GraphReadStore::get_node_edges(&graph, "model:tbl_a")
        .unwrap()
        .unwrap();
    let incoming_sources: Vec<_> = tbl_a_edges
        .incoming
        .iter()
        .map(|e| e.edge.from.clone())
        .collect();
    assert!(incoming_sources.contains(&"model:app/a.spg|src1".to_string()));
    assert!(incoming_sources.contains(&"model:app/c.spg|src_c".to_string()));

    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
}

/// 共享目标修改/删除/恢复：全量与增量保持完全一致，且有独立预期。
#[test]
fn shared_table_modified_deleted_and_restored_maintains_parity_and_independent_state() {
    let tag = "shared-tbl-lifecycle";
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid binding");

    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    let full_db = full_dir.join("graph.db");

    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    let inc_db = inc_dir.join("graph.db");

    // 阶段 1：初始状态，两页引用 shared.tbl，shared.tbl 存在
    let p_a = page_with_dwtable("Page A", "src_a", "$DATA:/tables/shared.tbl");
    let p_b = page_with_dwtable("Page B", "src_b", "$DATA:/tables/shared.tbl");
    let t_init = table_json("shared", &["col1", "col2"]);

    write(&full_project, "app/a.spg", &p_a);
    write(&full_project, "app/b.spg", &p_b);
    write(&full_project, "tables/shared.tbl", &t_init);
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full 1");

    write(&inc_project, "app/a.spg", &p_a);
    write(&inc_project, "app/b.spg", &p_b);
    write(&inc_project, "tables/shared.tbl", &t_init);
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc 1");

    let snap_full_1 = snapshot(&full_db, &binding);
    let snap_inc_1 = snapshot(&inc_db, &binding);
    assert_eq!(snap_full_1.nodes, snap_inc_1.nodes, "阶段1节点一致");
    assert_eq!(snap_full_1.edges, snap_inc_1.edges, "阶段1边一致");

    // 阶段 2：修改 shared.tbl（新增 col3）
    let t_mod = table_json("shared", &["col1", "col2", "col3"]);
    write(&full_project, "tables/shared.tbl", &t_mod);
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full 2");

    write(&inc_project, "tables/shared.tbl", &t_mod);
    let r2 = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc 2");
    assert!(r2.dirty > 0);

    let snap_full_2 = snapshot(&full_db, &binding);
    let snap_inc_2 = snapshot(&inc_db, &binding);
    assert_eq!(snap_full_2.nodes, snap_inc_2.nodes, "阶段2修改后节点一致");
    assert_eq!(snap_full_2.edges, snap_inc_2.edges, "阶段2修改后边一致");
    assert!(
        snap_inc_2
            .nodes
            .iter()
            .any(|n| n.contains("field:shared.col3")),
        "独立预期：新字段 col3 存在"
    );

    // 阶段 3：删除 shared.tbl（两页仍引用它，应降为 PhysicalTable 占位节点）
    std::fs::remove_file(full_project.join("tables/shared.tbl")).expect("rm full shared.tbl");
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full 3");

    std::fs::remove_file(inc_project.join("tables/shared.tbl")).expect("rm inc shared.tbl");
    let r3 = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc 3");
    assert_eq!(r3.deleted, 1);

    let snap_full_3 = snapshot(&full_db, &binding);
    let snap_inc_3 = snapshot(&inc_db, &binding);
    assert_eq!(snap_full_3.nodes, snap_inc_3.nodes, "阶段3删除后节点一致");
    assert_eq!(snap_full_3.edges, snap_inc_3.edges, "阶段3删除后边一致");
    // 独立预期：model:shared 依然存在（作为占位节点），但 field:shared.col* 被移除
    assert!(
        snap_inc_3
            .nodes
            .iter()
            .any(|n| n.starts_with("model:shared\t")),
        "独立预期：model:shared 占位节点保留"
    );
    assert!(
        !snap_inc_3
            .nodes
            .iter()
            .any(|n| n.contains("field:shared.col3")),
        "独立预期：定义被删除后字段移除"
    );

    // 阶段 4：恢复 shared.tbl
    write(&full_project, "tables/shared.tbl", &t_init);
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full 4");

    write(&inc_project, "tables/shared.tbl", &t_init);
    let r4 = ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc 4");
    assert!(r4.dirty > 0);

    let snap_full_4 = snapshot(&full_db, &binding);
    let snap_inc_4 = snapshot(&inc_db, &binding);
    assert_eq!(snap_full_4.nodes, snap_inc_4.nodes, "阶段4恢复后节点一致");
    assert_eq!(snap_full_4.edges, snap_inc_4.edges, "阶段4恢复后边一致");
    assert!(
        snap_inc_4
            .nodes
            .iter()
            .any(|n| n.contains("field:shared.col1")),
        "独立预期：恢复后字段重新出现"
    );

    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
}

/// 坏 TBL 在增量扫描中必须保留旧图，修复后重新入图并与全量保持一致。
#[test]
fn bad_tbl_preserves_old_graph_and_recovers_upon_repair() {
    let tag = "bad-tbl-recovery";
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid binding");

    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    let full_db = full_dir.join("graph.db");

    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    let inc_db = inc_dir.join("graph.db");

    let p_a = page_with_dwtable("Page A", "src_a", "$DATA:/tables/orders.tbl");
    let t_good = table_json("orders", &["order_id", "amount"]);

    write(&full_project, "app/a.spg", &p_a);
    write(&full_project, "tables/orders.tbl", &t_good);
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full init");

    write(&inc_project, "app/a.spg", &p_a);
    write(&inc_project, "tables/orders.tbl", &t_good);
    ProjectIndexer::scan_for_project(&inc_project, &inc_db, &binding).expect("inc init");

    let snap_init = snapshot(&inc_db, &binding);
    assert!(
        snap_init
            .nodes
            .iter()
            .any(|n| n.contains("field:orders.amount"))
    );

    // 写入语法错误的坏 TBL
    write(
        &inc_project,
        "tables/orders.tbl",
        "{ invalid json syntax -- not closed",
    );
    let with_diags =
        ProjectIndexer::scan_with_diagnostics_for_project(&inc_project, &inc_db, &binding)
            .expect("scan with error should succeed without panic");
    assert!(
        with_diags
            .diagnostics
            .iter()
            .any(|d| d.code == "SCANNER_FILE_PARSE_FAILED"),
        "必须产生 SCANNER_FILE_PARSE_FAILED 诊断"
    );

    // 独立预期：旧图被完整保留，field:orders.amount 仍在
    let snap_during_bad = snapshot(&inc_db, &binding);
    assert_eq!(
        snap_init.nodes, snap_during_bad.nodes,
        "坏 TBL 解析失败时，旧图节点必须原样保留！"
    );

    // 坏文件状态下，**增量**必须保留上一次成功解析的内容；而**从零全量重建**没有
    // 「上一次成功」可言，只能看到占位节点。两者**本来就不该相等**——这正是 B3
    // 「坏 TBL 保留旧图」的语义：增量有记忆，全量没有。
    //
    // 此前只有「坏前 == 坏后」的断言，证明不了这一点；这里把差异钉成显式预期，
    // 让「增量保留了旧图」与「全量重建拿不到」各自可判，而不是含糊地都不检查。
    let full_during_bad_dir = unique_dir(&format!("{tag}-full-bad"));
    let full_during_bad_project = full_during_bad_dir.join("project");
    let full_during_bad_db = full_during_bad_dir.join("graph.db");
    write(&full_during_bad_project, "app/a.spg", &p_a);
    write(
        &full_during_bad_project,
        "tables/orders.tbl",
        "{ invalid json syntax -- not closed",
    );
    ProjectIndexer::scan_with_diagnostics_for_project(
        &full_during_bad_project,
        &full_during_bad_db,
        &binding,
    )
    .expect("full scan with bad tbl should succeed without panic");
    let snap_full_during_bad = snapshot(&full_during_bad_db, &binding);

    // 增量侧：坏文件解析失败 ⇒ 上一次成功的 model/field 原样保留
    assert!(
        snap_during_bad
            .nodes
            .iter()
            .any(|n| n.contains("field:orders.amount")),
        "增量必须保留上一次成功解析的字段（B3 旧图保留）"
    );
    // 全量侧：从零构建、该文件从未成功解析 ⇒ 没有这些字段，只有 SPG 里的占位模型
    assert!(
        !snap_full_during_bad
            .nodes
            .iter()
            .any(|n| n.contains("field:orders.amount")),
        "从零全量重建没有「上一次成功」可保留，不应出现该字段：{}",
        describe_diff(
            "incremental(bad)",
            &snap_during_bad.nodes,
            "full(bad)",
            &snap_full_during_bad.nodes
        )
    );
    assert!(
        snap_full_during_bad
            .nodes
            .iter()
            .any(|n| n.contains("model:orders")),
        "全量重建仍应留下 SPG 声明的占位模型，而不是整页消失"
    );
    // 两侧都不得因为一个坏文件而丢掉 SPG 自身的内容
    for snap in [&snap_during_bad, &snap_full_during_bad] {
        assert!(
            snap.nodes.iter().any(|n| n.contains("page:app/a.spg")),
            "坏文件不得连带丢掉页面节点"
        );
    }
    let _ = std::fs::remove_dir_all(&full_during_bad_dir);

    // 修复坏 TBL 并增加字段
    let t_repaired = table_json("orders", &["order_id", "amount", "customer"]);
    write(&full_project, "tables/orders.tbl", &t_repaired);
    ProjectIndexer::scan_for_project(&full_project, &full_db, &binding).expect("full repaired");

    write(&inc_project, "tables/orders.tbl", &t_repaired);
    let rep_report =
        ProjectIndexer::scan_with_diagnostics_for_project(&inc_project, &inc_db, &binding)
            .expect("inc repaired");
    assert!(
        !rep_report
            .diagnostics
            .iter()
            .any(|d| d.code == "SCANNER_FILE_PARSE_FAILED"),
        "修复后解析失败诊断必须消除"
    );

    let snap_full_repaired = snapshot(&full_db, &binding);
    let snap_inc_repaired = snapshot(&inc_db, &binding);
    assert_eq!(
        snap_full_repaired.nodes, snap_inc_repaired.nodes,
        "修复后全量与增量节点完全一致"
    );
    assert_eq!(
        snap_full_repaired.edges, snap_inc_repaired.edges,
        "修复后全量与增量边完全一致"
    );
    assert!(
        snap_inc_repaired
            .nodes
            .iter()
            .any(|n| n.contains("field:orders.customer")),
        "独立预期：customer 字段入图"
    );

    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
}

/// 重启：扫描持久化后关闭，重新以 bound / ownership 打开，图与账本状态一致。
#[test]
fn ownership_graph_reopens_and_restarts_cleanly() {
    let tag = "restart";
    let binding = ProjectBinding::new(format!("m59-b5-{tag}")).expect("valid binding");
    let dir = unique_dir(tag);
    let project = dir.join("project");
    let db_path = dir.join("graph.db");

    write(
        &project,
        "app/a.spg",
        &page_with_dwtable("Page A", "src1", "$DATA:/tables/orders.tbl"),
    );
    write(
        &project,
        "tables/orders.tbl",
        &table_json("orders", &["f1", "f2"]),
    );

    ProjectIndexer::scan_for_project(&project, &db_path, &binding).expect("initial scan");

    // 重启 1：以 ownership 写入口打开
    {
        let graph = GraphDB::open_with_ownership(&db_path, &binding).expect("open_with_ownership");
        assert_eq!(graph.project_binding(), Some(&binding));
        assert!(graph.node_indices.len() > 0);
    }

    // 重启 2：以 open_readonly_with_ownership 打开
    {
        let graph = GraphDB::open_readonly_with_ownership(&db_path, &binding)
            .expect("open_readonly_with_ownership");
        assert_eq!(graph.project_binding(), Some(&binding));
        assert!(graph.ownership_enabled());
        assert_eq!(graph.ownership_ledgers().unwrap().len(), 2);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

/// 验证 session refresh → query → diff-refresh 完整链路中，ownership 与 project_binding 严格贯通。
#[test]
fn session_refresh_query_diff_refresh_cycle_preserves_ownership() {
    use metadata_checker::diff_refresh::FixtureMetaFilesChangeSource;
    use metadata_checker::diff_refresh::orchestrator::DiffRefreshOrchestrator;
    use metadata_checker::remote_metadata::MetadataContentType;
    use metadata_checker::remote_metadata::RemoteFileContent;
    use metadata_checker::runtime::{GraphRuntime, RuntimeMode};
    use metadata_checker::session::SessionManager;
    use metadata_checker::session::remote_provider::{
        InMemoryRemoteSessionProvider, RemoteMetafileEntry, RemoteProjectInfo,
    };
    use metadata_checker::session::remote_sync::{
        SessionRefreshOptions, refresh_session_from_remote,
    };

    let tag = "session-cycle";
    let dir = unique_dir(tag);
    let manager = SessionManager::new(&dir);
    let session_id = "s1";
    let project_ref = "test-proj";
    let binding = ProjectBinding::new(project_ref).expect("valid binding");

    let mut provider = InMemoryRemoteSessionProvider::new();
    provider
        .register_project(RemoteProjectInfo {
            project_ref: project_ref.to_string(),
            project_name: project_ref.to_string(),
            source_origin: "remote".to_string(),
        })
        .unwrap();

    let page_content = page_with_dwtable("Page A", "src1", "$DATA:/tables/table1.tbl");
    let table_content = table_json("table1", &["id", "name"]);

    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: project_ref.to_string(),
                source_path: "app/Page.spg".to_string(),
                file_id: Some("id1".to_string()),
                revision: Some("1".to_string()),
                etag: None,
                mtime: Some(1715000000000),
                size: Some(page_content.len() as u64),
                deleted: false,
            },
            RemoteFileContent {
                source_path: "app/Page.spg".to_string(),
                file_id: Some("id1".to_string()),
                revision: Some("1".to_string()),
                content_type: MetadataContentType::SuperPage,
                raw_text: page_content,
            },
        )
        .unwrap();

    provider
        .add_metafile(
            RemoteMetafileEntry {
                project_ref: project_ref.to_string(),
                source_path: "tables/table1.tbl".to_string(),
                file_id: Some("id2".to_string()),
                revision: Some("1".to_string()),
                etag: None,
                mtime: Some(1715000000000),
                size: Some(table_content.len() as u64),
                deleted: false,
            },
            RemoteFileContent {
                source_path: "tables/table1.tbl".to_string(),
                file_id: Some("id2".to_string()),
                revision: Some("1".to_string()),
                content_type: MetadataContentType::Table,
                raw_text: table_content,
            },
        )
        .unwrap();

    // 1. Session refresh
    let refresh_report = refresh_session_from_remote(
        &provider,
        &manager,
        SessionRefreshOptions {
            session_id: session_id.to_string(),
            remote_server: "https://bi.test".to_string(),
            project_ref: project_ref.to_string(),
            project_name: project_ref.to_string(),
            sync_mode: metadata_checker::session::sync::SessionSyncMode::Full,
            create_if_missing: true,
            filter: None,
            graph_db_path: None,
        },
    )
    .expect("session refresh");
    assert!(refresh_report.ok);
    assert_eq!(refresh_report.index.indexed, 2);

    let graph_db_path = PathBuf::from(&refresh_report.graph_db_path);

    // 2. Query runtime 加载（diff-refresh 需要长驻读模型）
    let runtime = GraphRuntime::load_with_project_dir_and_mode_for_project(
        &graph_db_path,
        None::<&std::path::Path>,
        RuntimeMode::LongLived,
        &binding,
    )
    .expect("runtime load with project binding");
    assert_eq!(runtime.project_binding.as_ref(), Some(&binding));

    // 3. Diff-refresh
    let fixture_json = r#"{
      "schema_version": 1,
      "cursor": {"updated_at_ms": 1715000000000, "boundary_event_ids": []},
      "changes": []
    }"#;
    let source = FixtureMetaFilesChangeSource::from_json_str(fixture_json).unwrap();
    let session_dir = manager.session_dir(session_id);
    let manifest = manager.read_manifest(session_id).expect("read manifest");
    let mut orchestrator = DiffRefreshOrchestrator::new(
        manager,
        session_dir,
        manifest,
        Box::new(source),
        Box::new(provider),
        runtime,
    );
    let diff_report = orchestrator.refresh_once().expect("diff refresh");
    // 首轮 refresh_once 无 checkpoint，走 bootstrap 全量同步，消费全部 2 个 metafile
    assert_eq!(diff_report.change_count, 2);
    assert_eq!(diff_report.warm_failures, Vec::<String>::new());

    // 4. 再次验证库内 ownership 状态完好
    let graph = GraphDB::open_readonly_with_ownership(&graph_db_path, &binding)
        .expect("reopen with ownership after diff-refresh");
    assert_eq!(graph.project_binding(), Some(&binding));
    assert!(graph.ownership_enabled());
    assert_eq!(graph.ownership_ledgers().unwrap().len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
