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
