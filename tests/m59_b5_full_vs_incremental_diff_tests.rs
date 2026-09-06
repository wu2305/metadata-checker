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

fn snapshot(db_path: &Path) -> GraphSnapshot {
    let graph = GraphDB::open(db_path).expect("reopen graph db");

    let mut nodes: Vec<String> = graph
        .iter_nodes()
        .expect("iter_nodes")
        .map(|n| {
            format!(
                "{}\ttype={:?}\tpath={}\tname={}\tmeta={}",
                n.id,
                n.node_type,
                n.path,
                n.name,
                // meta 也参与比对：占位节点升级、meta 合并这类问题只体现在这里。
                n.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string())
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
                "{} -{:?}-> {}\tfield_path={}\tmeta={}",
                e.from,
                e.edge_type,
                e.to,
                e.field_path.unwrap_or_else(|| "-".to_string()),
                e.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string())
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
    // 全量
    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(&full_project, "app/a.spg", page_a);
    write(&full_project, "app/b.spg", b_after);
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan(&full_project, &full_db).expect("full scan");

    // 增量
    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(&inc_project, "app/a.spg", page_a);
    write(&inc_project, "app/b.spg", b_before);
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan(&inc_project, &inc_db).expect("initial scan");

    write(&inc_project, "app/b.spg", b_after);
    let report = ProjectIndexer::scan(&inc_project, &inc_db).expect("incremental scan");
    assert!(
        report.dirty > 0,
        "第二次扫描必须真的走了增量更新路径（dirty > 0），否则本测试什么都没验到"
    );

    let out = (snapshot(&full_db), snapshot(&inc_db));
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

/// **钉住 spec §2.1 的 P1 缺陷**：增量路径丢失未变更文件（A）贡献的跨文件边。
///
/// 机制（源码逐行核对，`scanner/spg.rs`）：
/// - A 扫描时为内嵌目标 `add_node(page:app/b.spg)`，但**不**把它写进 A 的
///   `node_ids`（`spg.rs:1067-1075`——对比 `:584` / `:757` 普通节点都显式
///   `node_ids.insert`）。所以这个节点不归 A 所有。
/// - B 扫描时把 `page:app/b.spg` 作为自身页节点写进 B 的 `node_ids`（`:584`）。
///
/// 于是 B 变脏时，`previous_node_ids` 含 `page:app/b.spg` → `remove_nodes_by_ids`
/// 删掉它 → 删节点连带删边，A→B 的 `EmbedsPage` 边一并消失。随后**只**重跑 B，
/// A 不会被重扫，边**永久丢失且无任何诊断**。
///
/// 本测试如实钉住这个分歧。**A3（边带 `origin_file`、删除按 origin 而非按端点牵连）
/// 落地后这条会立刻变红**——届时把断言改成 `full.edges == inc.edges` 即可，
/// 那就是 A3 的验收判据。
#[test]
fn cross_file_edge_is_lost_by_incremental_update() {
    let (full, inc) = run_both_paths(
        "cross-file",
        &page_a_embedding_b(),
        &page_b("旧标题"),
        &page_b("新标题"),
    );

    let embeds = |snap: &GraphSnapshot| -> Vec<String> {
        snap.edges
            .iter()
            .filter(|e| e.contains("EmbedsPage"))
            .cloned()
            .collect()
    };

    assert_eq!(
        embeds(&full).len(),
        1,
        "全量路径必须建出 A→B 的 EmbedsPage 边，实际 {:?}",
        embeds(&full)
    );
    assert!(
        embeds(&full)[0].contains("page:app/b.spg"),
        "边应指向 B 的页节点，实际 {}",
        embeds(&full)[0]
    );

    // 从全量快照只移除一次已知缺边，保留其他边的每一次出现。
    let known_missing = "comp:app/a.spg|embed1 -EmbedsPage-> page:app/b.spg\tfield_path=app/b.spg\tmeta=null";
    assert_eq!(embeds(&full), vec![known_missing.to_string()]);
    let mut expected_edges = full.edges.clone();
    let missing_index = expected_edges.iter().position(|edge| edge == known_missing)
        .expect("expected embedding edge");
    expected_edges.remove(missing_index);
    assert_eq!(inc.edges, expected_edges, "只能缺少指定边，不能新增或丢失重复边");

    // 节点集不受影响：丢的是边，不是节点。这一条把缺陷的范围钉死，
    // 避免将来有人把「节点也少了」这种更严重的退化误当成同一个已知问题。
    assert_eq!(
        full.nodes,
        inc.nodes,
        "缺陷只涉及边；节点集若也不一致，那是**另一个**问题：{}",
        describe_diff("全量", &full.nodes, "增量", &inc.nodes)
    );
}

/// 最终文件集只有 `b.spg`（`a.spg` 被删）时的两条路径。
fn run_both_paths_with_deletion(
    tag: &str,
    page_a: &str,
    page_b_src: &str,
) -> (GraphSnapshot, GraphSnapshot) {
    // 全量：a.spg 从未存在过。
    let full_dir = unique_dir(&format!("{tag}-full"));
    let full_project = full_dir.join("project");
    write(&full_project, "app/b.spg", page_b_src);
    let full_db = full_dir.join("graph.db");
    ProjectIndexer::scan(&full_project, &full_db).expect("full scan");

    // 增量：先有 a.spg，再删掉。
    let inc_dir = unique_dir(&format!("{tag}-inc"));
    let inc_project = inc_dir.join("project");
    write(&inc_project, "app/a.spg", page_a);
    write(&inc_project, "app/b.spg", page_b_src);
    let inc_db = inc_dir.join("graph.db");
    ProjectIndexer::scan(&inc_project, &inc_db).expect("initial scan");

    std::fs::remove_file(inc_project.join("app/a.spg")).expect("remove a.spg");
    let report =
        ProjectIndexer::scan(&inc_project, &inc_db).expect("incremental scan after delete");
    assert_eq!(
        report.deleted, 1,
        "第二次扫描必须真的把 a.spg 当成已删除处理，否则本测试什么都没验到"
    );

    let out = (snapshot(&full_db), snapshot(&inc_db));
    let _ = std::fs::remove_dir_all(&full_dir);
    let _ = std::fs::remove_dir_all(&inc_dir);
    out
}

/// **本轮新发现（本文件首次记录，codex 未报、spec §2.1 原文未涵盖）**：
/// 内嵌目标的**占位页节点**不归产生它的文件所有，因此引用方被删除后**永久泄漏**。
///
/// `spg.rs:1067` 为内嵌目标 `add_node` 却不 `node_ids.insert`。A 引用一个
/// **不存在的**页面 `ghost.spg` 时，占位节点 `page:app/ghost.spg` 只由 A 产生，
/// 却不在 A 的 `previous_node_ids` 里——删掉 A，没有任何文件会声明它，
/// `remove_nodes_by_ids` 永远碰不到它。图里从此多一个悬空页节点。
///
/// 这与 §2.1 是**同一个根因的两面**：节点的归属（ownership）没有被记账。
/// §2.1 是「别人的边被我的删除牵连」，这条是「我的节点没人来删」。
/// A3 引入 `origin_file` 时必须同时覆盖**节点**归属，只做边是不够的——
/// 这条测试就是那半边的判据。
#[test]
fn placeholder_page_node_leaks_after_its_only_referrer_is_deleted() {
    let (full, inc) =
        run_both_paths_with_deletion("ghost-embed", &page_a_embedding_ghost(), &page_b("标题"));

    let ghost = |snap: &GraphSnapshot| -> Vec<String> {
        snap.nodes
            .iter()
            .filter(|n| n.contains("ghost"))
            .cloned()
            .collect()
    };

    assert!(
        ghost(&full).is_empty(),
        "全量路径下 a.spg 从不存在，不该有 ghost 占位节点，实际 {:?}",
        ghost(&full)
    );

    // 全量节点加上唯一允许的占位节点；直接比较向量保留每行的出现次数。
    let mut expected_nodes = full.nodes.clone();
    expected_nodes.push("page:app/ghost.spg\ttype=Page\tpath=app/ghost.spg\tname=ghost\tmeta=null".to_string());
    expected_nodes.sort();
    assert_eq!(inc.nodes, expected_nodes, "只能增加指定占位节点，其他节点必须原样保留");

    // 边：a.spg 自身的节点被删时，它指向 ghost 的那条 EmbedsPage 边也随之消失，
    // 因此两侧的边集应当**完全一致**。泄漏的是一个悬空节点，不是一条悬空边。
    assert_eq!(
        full.edges,
        inc.edges,
        "边集应完全一致；这条缺陷泄漏的只有节点：{}",
        describe_diff("全量", &full.edges, "增量", &inc.edges)
    );
    assert!(!inc.nodes.is_empty(), "前置健全性：增量侧不该只剩空图");

    // 同时确认这不是「增量删除整体失灵」：a.spg 自己的节点确实被删干净了。
    assert!(
        !inc.nodes.iter().any(|n| n.contains("app/a.spg")),
        "a.spg 自身的节点应当已被删除，实际残留 {:?}",
        inc.nodes
            .iter()
            .filter(|n| n.contains("app/a.spg"))
            .collect::<Vec<_>>()
    );
}
