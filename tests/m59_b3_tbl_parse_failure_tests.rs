#![cfg(feature = "cli-local")]

//! M59 阶段 B3：损坏的 `.tbl` 不再被当成「成功的空结果」提交。
//!
//! 旧行为（`tbl.rs:14-21`）：`content.is_empty()` 与 `serde_json::from_str` 的
//! `Err(_)` 都 `return Ok(空集)`。配合增量路径「先删旧节点、再重建」，一个写坏
//! 的 `.tbl` 会让旧模型图被删干净、新图为空，而 file hash 照常记录——下一轮内容
//! 不变直接跳过。于是「读不出来」被永久固化成「模型不存在」，且没有任何诊断。
//!
//! 新契约（三条，缺一不可）：
//! 1. 解析失败的文件**不进候选图**，上一份有效图原样保留（陈旧，不是缺失）；
//! 2. 发出 `SCANNER_FILE_PARSE_FAILED`，让查询方能区分陈旧与缺失；
//! 3. file hash **不**记录，文件保持脏，修好后下一轮自动重新入图、诊断消失。

use metadata_checker::graph::GraphDB;
use metadata_checker::graph_store::GraphReadStore;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::output::Diagnostic;
use metadata_checker::scanner::indexer::ProjectIndexer;
use metadata_checker::scanner::process_tbl_file_from_string;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CODE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";
const MODEL_ID: &str = "model:orders";
/// `valid_tbl(true)` 才有的第三个维度所对应的字段节点。
const FIELD_CUSTOMER: &str = "field:orders.客户";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "metadata-checker-m59-b3-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

/// 合法表定义：两个维度。第三个维度用于验证「修好之后确实重新解析了」。
fn valid_tbl(extra_dimension: bool) -> String {
    let mut dims = vec![
        serde_json::json!({"name": "订单号", "dbfield": "orderNo", "dataType": "C"}),
        serde_json::json!({"name": "金额", "dbfield": "amount", "dataType": "N"}),
    ];
    if extra_dimension {
        dims.push(serde_json::json!({"name": "客户", "dbfield": "customer", "dataType": "C"}));
    }
    serde_json::json!({"version": "1.0", "dimensions": dims}).to_string()
}

fn code_hits<'a>(diags: &'a [Diagnostic], code: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.code == code).collect()
}

fn model_present(db_path: &Path) -> bool {
    let graph = GraphDB::open(db_path).expect("graph should open");
    graph.get_node(MODEL_ID).is_some()
}

/// 整图的**逐属性**快照：节点按 id 展开全部字段，边展开四元组连同 `meta`。
///
/// codex 复审（`2db8af9` 之后）：这里原先比的是 `model:orders` 的**邻边条数**。
/// 条数相等证明不了「旧图逐边保留」——把一条字段边换成另一条、改坏边上的
/// `field_path` / `meta`、或者把边掉个方向，条数都纹丝不动。同理
/// `edge_count > baseline` 也证明不了「新增的正是那个 customer 字段」，
/// 只证明了「多了点什么」。改成逐属性快照 + 完整差异，这些都会当场显形。
#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    /// (节点 id, 该节点的完整规范化表示)，按 id 排序。
    nodes: Vec<(String, String)>,
    /// 边的完整规范化表示，排序后比对。
    edges: Vec<String>,
}

/// 两份快照之间的**完整**差异。空 ⇔ 两张图逐属性一致。
#[derive(Debug, Default, PartialEq)]
struct SnapshotDiff {
    added_nodes: Vec<String>,
    removed_nodes: Vec<String>,
    /// id 两侧都在、但内容不同（典型：模型 meta 里的 dimensions 变了）。
    changed_nodes: Vec<String>,
    added_edges: Vec<String>,
    removed_edges: Vec<String>,
}

impl Snapshot {
    fn node_repr(&self, id: &str) -> Option<&str> {
        self.nodes
            .iter()
            .find(|(nid, _)| nid == id)
            .map(|(_, repr)| repr.as_str())
    }

    /// 本快照相对 `base` 的差异。
    fn diff_from(&self, base: &Snapshot) -> SnapshotDiff {
        let mut d = SnapshotDiff::default();
        for (id, repr) in &self.nodes {
            match base.node_repr(id) {
                None => d.added_nodes.push(id.clone()),
                Some(old) if old != repr => d.changed_nodes.push(id.clone()),
                Some(_) => {}
            }
        }
        for (id, _) in &base.nodes {
            if self.node_repr(id).is_none() {
                d.removed_nodes.push(id.clone());
            }
        }
        d.added_edges = self
            .edges
            .iter()
            .filter(|e| !base.edges.contains(e))
            .cloned()
            .collect();
        d.removed_edges = base
            .edges
            .iter()
            .filter(|e| !self.edges.contains(e))
            .cloned()
            .collect();
        d
    }
}

fn snapshot(db_path: &Path) -> Snapshot {
    let graph = GraphDB::open(db_path).expect("graph should open");

    let mut nodes: Vec<(String, String)> = graph
        .iter_nodes()
        .expect("iter_nodes")
        .map(|n| {
            let repr = format!(
                "{}\ttype={:?}\tpath={}\tname={}\tmeta={}",
                n.id,
                n.node_type,
                n.path,
                n.name,
                n.meta
                    .as_ref()
                    .map(|m| serde_json::to_string(m).expect("meta"))
                    .unwrap_or_else(|| "null".to_string())
            );
            (n.id, repr)
        })
        .collect();
    nodes.sort();

    let ids: Vec<String> = nodes.iter().map(|(id, _)| id.clone()).collect();
    let mut edges: Vec<String> = Vec::new();
    for id in &ids {
        // 走 trait 方法而非 `GraphDB` 的 inherent 同名方法：比对的是**契约**层面
        // 的图内容，将来换成 Grafeo 实现时这段一行不用改。
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

    Snapshot { nodes, edges }
}

/// 单文件层：非法 JSON 必须**报错**，不能返回 Ok(空集)。
/// 这是整条链路的源头——只要它还能悄悄成功，上层无论怎么写都分不出陈旧与缺失。
#[test]
fn corrupt_tbl_content_is_an_error_not_an_empty_success() {
    let mut store = MemoryGraphStore::new();
    let err = process_tbl_file_from_string(&mut store, "data/orders.tbl", "{\"dimensions\": [")
        .expect_err("非法 JSON 必须返回 Err");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("data/orders.tbl"),
        "报错应点名出问题的文件，实际得到 {msg}"
    );
}

/// 空文件同样不是「没有模型」，而是没读出来。
#[test]
fn empty_tbl_content_is_an_error_not_an_empty_success() {
    let mut store = MemoryGraphStore::new();
    let err = process_tbl_file_from_string(&mut store, "data/orders.tbl", "   \n")
        .expect_err("空内容必须返回 Err");
    assert!(
        format!("{err:#}").contains("empty"),
        "报错应说明文件为空，实际得到 {err:#}"
    );
}

/// 端到端：坏 `.tbl` → 旧图保留 + 陈旧诊断 + 保持脏 → 修好后自动恢复。
#[test]
fn corrupt_tbl_preserves_previous_graph_and_reports_stale() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("corrupt-tbl");
    std::fs::create_dir_all(project_dir.join("data"))?;
    let tbl_path = project_dir.join("data").join("orders.tbl");
    std::fs::write(&tbl_path, valid_tbl(false))?;
    let db_path = project_dir.join("graph.db");

    // ---- 第 1 轮：合法内容，模型入图，无解析失败诊断 ----
    let first = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&first.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "合法内容不该有解析失败诊断: {:?}",
        first.diagnostics
    );
    assert!(model_present(&db_path), "合法 .tbl 应产出 {MODEL_ID}");
    let baseline = snapshot(&db_path);
    assert!(
        !baseline.edges.is_empty(),
        "模型应带字段边，否则后面的「逐边保留」无从验起：{baseline:?}"
    );

    // ---- 第 2 轮：写坏文件 ----
    std::fs::write(&tbl_path, "{\"version\": \"1.0\", \"dimensions\": [")?;
    let second = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;

    let hits = code_hits(&second.diagnostics, CODE_PARSE_FAILED);
    assert_eq!(
        hits.len(),
        1,
        "损坏文件应产出恰好一条解析失败诊断: {:?}",
        second.diagnostics
    );
    assert_eq!(hits[0].count, Some(1), "{:?}", second.diagnostics);
    assert!(
        hits[0]
            .location
            .source_file
            .as_deref()
            .is_some_and(|f| f.contains("orders.tbl")),
        "诊断应定位到损坏的文件: {:?}",
        hits[0]
    );

    // 核心：旧图**原样保留**。旧实现在这里会把 model:orders 连同字段边一起删掉。
    assert!(
        model_present(&db_path),
        "解析失败不得删除上一份有效图——那会把「读不出来」说成「模型不存在」"
    );
    // 逐属性比对，不是比条数：换掉一条字段边、改坏边上的 meta、把边掉个方向，
    // 条数都不变，只有完整快照能看出来。
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "旧图应逐属性原样保留，而不只是「条数没变」"
    );

    // ---- 第 3 轮：内容未变，文件必须仍是脏的（hash 没被记录） ----
    let third = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        third.report.dirty > 0,
        "解析失败的文件不得记录 file hash，否则下轮会被当成「已处理」永久跳过: {:?}",
        third.report
    );
    assert_eq!(
        code_hits(&third.diagnostics, CODE_PARSE_FAILED).len(),
        1,
        "陈旧标记应持续存在直到文件修好: {:?}",
        third.diagnostics
    );
    assert!(model_present(&db_path), "重试一轮也不该丢图");
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "重试一轮同样不得动到图里的任何一个属性"
    );

    // ---- 第 4 轮：修好（并加一个维度，证明确实重新解析了） ----
    std::fs::write(&tbl_path, valid_tbl(true))?;
    let fourth = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&fourth.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "文件修好后陈旧标记必须当场消失: {:?}",
        fourth.diagnostics
    );
    assert!(model_present(&db_path));

    // 修好之后的差异必须**恰好**是那个新增维度：多一个字段节点、多一条
    // `model -Contains-> field` 边、模型自身的 meta 里多一条 dimension。
    // 「边数变多了」不构成证据——多出来的可能是任何东西。
    let repaired = snapshot(&db_path).diff_from(&baseline);
    assert_eq!(
        repaired.added_nodes,
        vec![FIELD_CUSTOMER.to_string()],
        "应当恰好多出 customer 字段节点，实际 {repaired:?}"
    );
    assert!(
        repaired.removed_nodes.is_empty() && repaired.removed_edges.is_empty(),
        "重新解析不得让原有节点/边消失，实际 {repaired:?}"
    );
    assert_eq!(
        repaired.changed_nodes,
        vec![MODEL_ID.to_string()],
        "只有模型节点自身的 meta（dimensions）应当变化，实际 {repaired:?}"
    );
    assert_eq!(
        repaired.added_edges,
        vec![format!(
            "{MODEL_ID} -Contains-> {FIELD_CUSTOMER}\tfield_path=-\tmeta=null"
        )],
        "新增的边必须正是模型到 customer 字段的 Contains 边，实际 {repaired:?}"
    );

    std::fs::remove_dir_all(&project_dir).ok();
    Ok(())
}

/// 把内容**改回原样**（而不是改成另一份合法内容）之后，陈旧标记也必须消失。
///
/// codex 复审发现（`6612a8b..5d7d973`）：上面那条用例修复时写的是**另一份**合法
/// 内容，hash 与保留下来的 `FileState` 不同，于是文件是脏的、会被重新解析、
/// 覆盖机制生效。而恢复成**上一次成功解析时的字节**时 hash 恰好相同 ⇒ 文件不脏
/// ⇒ 不解析 ⇒ 覆盖机制根本不触发，`SCANNER_FILE_PARSE_FAILED` 在文件已经完好
/// 的情况下继续挂着。这条用例专钉这个「修得太干净反而清不掉警告」的角落。
#[test]
fn restoring_original_bytes_clears_the_stale_warning() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("restore-original");
    std::fs::create_dir_all(project_dir.join("data"))?;
    let tbl_path = project_dir.join("data").join("orders.tbl");
    let original = valid_tbl(false);
    std::fs::write(&tbl_path, &original)?;
    let db_path = project_dir.join("graph.db");

    let first = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(code_hits(&first.diagnostics, CODE_PARSE_FAILED).is_empty());
    let baseline = snapshot(&db_path);
    assert!(!baseline.edges.is_empty(), "前置条件：模型应带字段边");

    std::fs::write(&tbl_path, "{\"version\": \"1.0\", \"dimensions\": [")?;
    let second = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert_eq!(
        code_hits(&second.diagnostics, CODE_PARSE_FAILED).len(),
        1,
        "前置条件：损坏这轮应当有诊断: {:?}",
        second.diagnostics
    );
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "损坏这轮旧图必须逐属性保留"
    );

    // 关键：写回**一模一样**的字节。hash 与保留下来的 FileState 相同。
    std::fs::write(&tbl_path, &original)?;
    let third = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&third.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "内容已复原，陈旧标记必须消失（此时文件不脏，靠的是诊断对账而非重解析）: {:?}",
        third.diagnostics
    );
    assert!(model_present(&db_path), "复原不得动到图");
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "内容与上一次成功解析时一致，图应逐属性不变——清诊断不得顺手动图"
    );

    // 再扫一轮：不能反复横跳，也不能把 entry 删了又冒出来。
    let fourth = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&fourth.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "清理必须是稳定的: {:?}",
        fourth.diagnostics
    );
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "再扫一轮同样不得动图"
    );

    std::fs::remove_dir_all(&project_dir).ok();
    Ok(())
}

/// 从未解析成功过的文件被删除后，它的诊断不得变成永久孤儿。
///
/// codex 复审发现（`6612a8b..5d7d973`）：首次解析就失败的文件只写了诊断、
/// **没写 `FileState`**（失败文件整个跳过 apply）。删掉它之后，它既不在
/// discovered 里、也进不了 `plan.deleted`（后者是从 `prev_states` 推出来的），
/// 于是没有任何路径会去碰它的 entry——警告指向一个已经不存在的文件，永久。
#[test]
fn deleting_a_never_valid_tbl_removes_its_orphaned_warning() -> anyhow::Result<()> {
    let project_dir = unique_temp_dir("orphan-warning");
    std::fs::create_dir_all(project_dir.join("data"))?;
    // 另有一个合法文件，保证图与 file states 非空——否则「诊断消失」可能只是
    // 因为整个库是空的，验不到对账逻辑。
    std::fs::write(
        project_dir.join("data").join("orders.tbl"),
        valid_tbl(false),
    )?;
    let broken = project_dir.join("data").join("broken.tbl");
    std::fs::write(&broken, "{\"dimensions\": [")?;
    let db_path = project_dir.join("graph.db");

    let first = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    let hits = code_hits(&first.diagnostics, CODE_PARSE_FAILED);
    assert_eq!(
        hits.len(),
        1,
        "前置条件：首轮就失败的文件应有诊断: {:?}",
        first.diagnostics
    );
    assert!(model_present(&db_path), "合法文件应正常入图");
    let baseline = snapshot(&db_path);
    assert!(
        !baseline.edges.is_empty(),
        "前置条件：合法文件应带字段边，否则「图没被牵连」验不到东西"
    );

    std::fs::remove_file(&broken)?;
    let second = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&second.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "文件已删除，指向它的诊断必须一并清除: {:?}",
        second.diagnostics
    );
    assert_eq!(
        second.report.deleted, 0,
        "该文件从未有 FileState，本就不该出现在 plan.deleted 里——\
         这正是既有清理路径够不着它的原因: {:?}",
        second.report
    );
    assert_eq!(
        snapshot(&db_path).diff_from(&baseline),
        SnapshotDiff::default(),
        "清理孤儿诊断不得牵连到图里的任何一个节点或属性"
    );

    // 稳定性：再扫一轮不该把它变回来。
    let third = ProjectIndexer::scan_with_diagnostics(&project_dir, &db_path)?;
    assert!(
        code_hits(&third.diagnostics, CODE_PARSE_FAILED).is_empty(),
        "{:?}",
        third.diagnostics
    );

    std::fs::remove_dir_all(&project_dir).ok();
    Ok(())
}
