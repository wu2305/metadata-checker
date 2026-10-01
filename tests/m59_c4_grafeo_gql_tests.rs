//! M59-3 C4：`.grafeo` 图库的只读 GQL 查询。
//!
//! C4 原先是「手写 WHERE/LIMIT 迷你语言」，改为直接把引擎自带的 GQL 开给 LLM。
//! GQL 能力来自 grafeo 的 `gql` feature（`edge` 已带上，无需再开 `lpg`）。
//! 这里钉住的是**只读边界**——LLM 能写的 GQL 越多，越要证明图和本地文件都碰不到：
//!
//! - 每一种写入形态（INSERT/SET/DELETE/DETACH DELETE/REMOVE/MERGE、建索引、建图、
//!   DROP）都被拒绝，且**拒绝之后节点集合、边数、属性原样不变**——只断言 `Err`
//!   不够，那只证明报了错，不证明没写进去；
//! - 起始关键字能通过形态闸的写入（`MATCH ... SET`、`WITH ... INSERT` 等）由引擎
//!   只读会话挡下，这一组专门验引擎闸，不被形态闸掩盖；
//! - `LOAD CSV/DATA` 是读，引擎只读会话放行，但会把本地文件读进结果——必须在形态闸
//!   拒绝，错误文本里也不能带出文件内容；
//! - 结果截断、列/值的 JSON 形态、持久库重开后仍可查询。

#![cfg(feature = "grafeo-store")]

use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_grafeo::GrafeoGraphStore;
use metadata_checker::graph_store::{GqlError, GraphReadStore, GraphWriteStore};
use serde_json::json;
use std::path::PathBuf;

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("m59-c4-{tag}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn node_of(id: &str, node_type: NodeType, name: &str) -> Node {
    Node {
        id: id.to_string(),
        node_type,
        path: format!("app/{id}.spg"),
        name: name.to_string(),
        meta: Some(json!({"tag": id})),
        origin_file: Some(format!("src/{id}.spg")),
    }
}

fn edge_of(from: &str, to: &str, edge_type: EdgeType, field_path: &str) -> Edge {
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type,
        field_path: Some(field_path.to_string()),
        meta: None,
        origin_file: Some("src/page.spg".to_string()),
    }
}

/// 三个节点、两条边：页面 page 包含组件 comp，组件 comp 读模型 orders。
fn seeded_store() -> GrafeoGraphStore {
    let mut store = GrafeoGraphStore::in_memory().expect("open store");
    store
        .upsert_node(node_of("page:p1", NodeType::Page, "订单页"))
        .expect("page");
    store
        .upsert_node(node_of("comp:c1", NodeType::Component, "提交按钮"))
        .expect("component");
    store
        .upsert_node(node_of("model:orders", NodeType::Model, "订单"))
        .expect("model");
    store
        .add_edge(edge_of("page:p1", "comp:c1", EdgeType::Contains, ""))
        .expect("contains");
    store
        .add_edge(edge_of(
            "comp:c1",
            "model:orders",
            EdgeType::Reads,
            "orders.amount",
        ))
        .expect("reads");
    store
}

/// 图的完整指纹：节点（含全部字段）按 id 序 + 每个节点的邻接。写入哪怕只改一个属性、
/// 多一条边都会让指纹变化，比单看计数严格。
fn fingerprint(store: &GrafeoGraphStore) -> String {
    let nodes: Vec<Node> = store.iter_nodes().expect("iter").collect();
    let mut out = format!(
        "nodes={} edges={}\n",
        store.node_count().expect("nodes"),
        store.edge_count().expect("edges")
    );
    for node in &nodes {
        out.push_str(&format!("{node:?}\n"));
        let neighbors = store
            .get_node_edges(&node.id)
            .expect("neighbors")
            .expect("node exists");
        out.push_str(&format!(
            "  out={} in={}\n",
            neighbors.outgoing.len(),
            neighbors.incoming.len()
        ));
    }
    out
}

// ---------------------------------------------------------------- 读能力

/// 基本投影：列名取自 RETURN，值转成 JSON 标量，ORDER BY 保序。
#[test]
fn gql_projects_columns_and_rows_as_json() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only(
            "MATCH (n:Node) RETURN n.id AS id, n.name AS name ORDER BY n.id",
            100,
        )
        .expect("query");
    assert_eq!(table.columns, vec!["id", "name"]);
    assert_eq!(
        table.rows,
        vec![
            vec![json!("comp:c1"), json!("提交按钮")],
            vec![json!("model:orders"), json!("订单")],
            vec![json!("page:p1"), json!("订单页")],
        ]
    );
    assert_eq!(table.total_rows, 3);
    assert!(!table.truncated);
}

/// 边类型名就是 `EdgeType` 变体名，LLM 可以直接按类型遍历。
#[test]
fn gql_traverses_edges_by_type_name() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only(
            "MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path",
            100,
        )
        .expect("query");
    assert_eq!(
        table.rows,
        vec![vec![
            json!("comp:c1"),
            json!("model:orders"),
            json!("orders.amount")
        ]]
    );
}

/// 两跳路径：页面经组件到达模型。
#[test]
fn gql_multi_hop_traversal() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only(
            "MATCH (p:Node {id: 'page:p1'})-[:Contains]->(:Node)-[:Reads]->(m:Node) RETURN m.name",
            100,
        )
        .expect("query");
    assert_eq!(table.rows, vec![vec![json!("订单")]]);
}

/// 聚合、布尔、列表的 JSON 形态；浮点单独验。
///
/// 浮点不和 `count()` 放进同一条 RETURN：grafeo 0.5.43 在聚合之后的投影里会把
/// 浮点字面量当作 `Int64(位模式)` 返回（`count(n) AS total, 1.5 AS ratio` 得到
/// 4609434218613702656，即 1.5 的 IEEE 位模式）。这是引擎缺陷，不在本层修；
/// 图里的属性全是字符串，浮点只会来自 LLM 自己写的字面量或 `avg()` 之类的计算。
#[test]
fn gql_value_shapes_are_plain_json() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only(
            "MATCH (n:Node) RETURN count(n) AS total, true AS flag, [1, 2] AS items",
            100,
        )
        .expect("query");
    assert_eq!(table.columns, vec!["total", "flag", "items"]);
    assert_eq!(table.rows, vec![vec![json!(3), json!(true), json!([1, 2])]]);

    let floats = store
        .query_gql_read_only("RETURN 1.5 AS ratio", 100)
        .expect("float query");
    assert_eq!(floats.rows, vec![vec![json!(1.5)]]);
}

/// 结果超出 `max_rows` 时截断并显式标记，总行数照实报告。
#[test]
fn gql_truncates_to_max_rows_and_flags_it() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only("MATCH (n:Node) RETURN n.id ORDER BY n.id", 2)
        .expect("query");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.total_rows, 3, "total_rows 是截断前的行数");
    assert!(table.truncated);
    assert_eq!(table.rows[0], vec![json!("comp:c1")]);

    let exact = store
        .query_gql_read_only("MATCH (n:Node) RETURN n.id", 3)
        .expect("query");
    assert!(!exact.truncated, "恰好等于上限不算截断");
}

/// JSON / TSV 两种输出口径（human 与 JSON 同步，见 AGENTS.md output 约定）。
#[test]
fn gql_rows_render_json_and_tsv() {
    let store = seeded_store();
    let table = store
        .query_gql_read_only("MATCH (n:Node) RETURN n.id, n.name ORDER BY n.id", 2)
        .expect("query");
    assert_eq!(
        table.to_json(),
        json!({
            "columns": ["n.id", "n.name"],
            "rows": [["comp:c1", "提交按钮"], ["model:orders", "订单"]],
            "row_count": 2,
            "total_rows": 3,
            "truncated": true,
        })
    );
    assert_eq!(
        table.to_tsv(),
        "n.id\tn.name\ncomp:c1\t提交按钮\nmodel:orders\t订单\n# 已截断：返回 3 行，仅显示前 2 行"
    );
}

/// 持久库关闭重开后，GQL 读到的是落盘数据，且重开用的读写 `open()`
/// 同样受会话级只读约束（runtime 就是这样打开 `.grafeo` 的）。
#[test]
fn gql_reads_persisted_store_after_reopen_and_stays_read_only() {
    let path = unique_dir("reopen").join("graph.grafeo");
    {
        let mut store = GrafeoGraphStore::open(&path).expect("create");
        store
            .upsert_node(node_of("page:p1", NodeType::Page, "订单页"))
            .expect("page");
        store.close().expect("close");
    }
    let store = GrafeoGraphStore::open(&path).expect("reopen");
    let table = store
        .query_gql_read_only("MATCH (n:Node) RETURN n.name", 10)
        .expect("query");
    assert_eq!(table.rows, vec![vec![json!("订单页")]]);

    let before = fingerprint(&store);
    let err = store
        .query_gql_read_only("MATCH (n:Node) SET n.name = 'tampered'", 10)
        .expect_err("写入必须被拒");
    assert_eq!(err.code, "GQL_READ_ONLY", "写入被拒应带稳定错误码：{err}");
    assert_eq!(fingerprint(&store), before, "拒绝之后图必须原样不变");
}

// ---------------------------------------------------------------- 写入必须被拒

/// 起始关键字能通过形态闸的写入：只能靠引擎只读会话挡下。这一组验的是引擎闸本身，
/// 与形态闸无关；每条都断言拒绝**并且**图指纹不变。
#[test]
fn gql_engine_gate_rejects_writes_that_pass_the_shape_gate() {
    let store = seeded_store();
    let before = fingerprint(&store);
    let writes = [
        "MATCH (n:Node) SET n.name = 'tampered'",
        "MATCH (n:Node) SET n.brand_new = 1",
        "MATCH (n:Node) REMOVE n.name",
        "MATCH (n:Node {id: 'page:p1'}) DELETE n",
        "MATCH (n:Node) DETACH DELETE n",
        "MATCH (a:Node)-[r:Reads]->(b:Node) DELETE r",
        "MATCH (n:Node) INSERT (:Node {id: 'x', name: 'x'})",
        "MATCH (n:Node {id: 'page:p1'}) INSERT (n)-[:Reads]->(:Node {id: 'y'})",
        "UNWIND [1, 2] AS item MATCH (n:Node) SET n.name = 'tampered'",
        "FOR item IN [1, 2] MATCH (n:Node) DETACH DELETE n",
        "OPTIONAL MATCH (n:Node) SET n.name = 'tampered'",
        "MATCH (n:Node) CREATE (:Node {id: 'c', name: 'c'})",
    ];
    for query in writes {
        // 必须是「被只读会话拒绝」，而不是别的原因（语法不支持等）报的错——
        // 否则这条断言会在语句根本没被引擎当写入处理时也通过。
        let err = store
            .query_gql_read_only(query, 10)
            .expect_err(&format!("写入语句必须被拒：{query}"));
        assert_eq!(
            err.code, "GQL_READ_ONLY",
            "{query} 应因只读会话被拒，实际：{err}"
        );
        assert_eq!(
            fingerprint(&store),
            before,
            "语句被拒之后图必须原样不变：{query}"
        );
    }
}

/// 起始关键字不在白名单的语句（写入、DDL、会话命令、过程调用）在形态闸就被拒，
/// 并带稳定错误码 `GQL_READ_ONLY`；图同样不变。
#[test]
fn gql_shape_gate_rejects_non_read_statements() {
    let store = seeded_store();
    let before = fingerprint(&store);
    let rejected = [
        "INSERT (:Node {id: 'x'})",
        "insert (:Node {id: 'x'})",
        "MERGE (:Node {id: 'x'})",
        "DELETE (n)",
        "CREATE (:Node {id: 'x'})",
        "CREATE INDEX idx_name FOR (n:Node) ON (n.name)",
        "CREATE GRAPH extra",
        "DROP GRAPH extra",
        "ALTER NODE TYPE Node ADD PROPERTY x STRING",
        "SESSION SET GRAPH extra",
        "START TRANSACTION READ WRITE",
        "COMMIT",
        "ROLLBACK",
        "CALL grafeo.procedures()",
        "SHOW GRAPHS",
        "USE extra",
        "WITH 1 AS one RETURN one",
        "SELECT 1",
        "  ",
        "",
        "-- 注释开头\nMATCH (n) RETURN n",
        "(n) RETURN n",
    ];
    for query in rejected {
        let err = store
            .query_gql_read_only(query, 10)
            .expect_err(&format!("必须被拒：{query:?}"));
        assert_eq!(err.code, "GQL_READ_ONLY", "{query:?} 应被形态闸拒绝：{err}");
        assert_eq!(fingerprint(&store), before, "{query:?} 被拒后图必须不变");
    }
}

// ---------------------------------------------------------------- 不得读本地文件

/// `LOAD CSV/DATA` 引擎视为读，会话只读放行——不加形态闸时它能把任意本地文本文件
/// 读成行。每种写法（含大小写、反引号、夹在读子句之后）都必须被拒，错误文本
/// 不能带出文件内容。
#[test]
fn gql_rejects_load_forms_without_leaking_file_contents() {
    let dir = unique_dir("load");
    let secret_file = dir.join("secret.csv");
    std::fs::write(&secret_file, "top-secret-token,other\nrow2,row2b\n").expect("write secret");
    let path = secret_file.display().to_string();

    let store = seeded_store();
    let before = fingerprint(&store);
    let attempts = [
        format!("LOAD CSV FROM '{path}' AS row RETURN row"),
        format!("LOAD CSV WITH HEADERS FROM '{path}' AS row RETURN row"),
        format!("LOAD DATA FROM '{path}' FORMAT CSV AS row RETURN row"),
        format!("LOAD DATA FROM '{path}' FORMAT JSONL AS row RETURN row"),
        format!("load csv from '{path}' as row return row"),
        format!("MATCH (n:Node) LOAD CSV FROM '{path}' AS row RETURN row"),
        format!("WITH 1 AS one LOAD CSV FROM '{path}' AS row RETURN row"),
        format!("UNWIND [1] AS one LOAD DATA FROM '{path}' FORMAT CSV AS row RETURN row"),
        format!("MATCH (n:Node) `LOAD` CSV FROM '{path}' AS row RETURN row"),
        format!("MATCH (n:Node)\nLOAD\tCSV FROM '{path}' AS row RETURN row"),
    ];
    for query in attempts {
        let err = store
            .query_gql_read_only(&query, 10)
            .expect_err(&format!("LOAD 必须被拒：{query:?}"));
        assert_eq!(err.code, "GQL_READ_ONLY", "{query:?} 应被形态闸拒绝：{err}");
        assert!(
            !err.message.contains("top-secret-token"),
            "错误文本不能带出文件内容：{err}"
        );
    }
    assert_eq!(fingerprint(&store), before);
}

/// 保守判定的已知代价：字符串字面量里出现 `load` 一词也会被拒。
/// 这条测试把「宁可误拒」的取舍固定下来，防止有人为了放行字面量把判定放松成
/// 与引擎词法不一致的形态而漏放真正的 LOAD。
#[test]
fn gql_load_word_is_rejected_even_inside_string_literals() {
    let store = seeded_store();
    let err = store
        .query_gql_read_only("MATCH (n:Node) WHERE n.name = 'load' RETURN n.id", 10)
        .expect_err("含 load 一词的查询被保守拒绝");
    assert_eq!(err.code, "GQL_READ_ONLY");

    // `download` / `loader` / `load_time` 不是 LOAD 一词，不受影响。
    let ok = store
        .query_gql_read_only("MATCH (n:Node) WHERE n.name = 'download' RETURN n.id", 10)
        .expect("相近词不应被误拒");
    assert!(ok.rows.is_empty());
}

// ---------------------------------------------------------------- 错误分类与上限

/// 语法错误归 `GQL_QUERY_INVALID`，调用方据此改写查询。
#[test]
fn gql_syntax_error_is_reported_as_invalid_query() {
    let store = seeded_store();
    let err = store
        .query_gql_read_only("MATCH (n:Node RETURN n", 10)
        .expect_err("语法错误");
    assert_eq!(err.code, "GQL_QUERY_INVALID", "实际：{err}");
}

/// 超长查询文本在形态闸拒绝，不进引擎。
#[test]
fn gql_rejects_oversized_query_text() {
    let store = seeded_store();
    let padding = "x".repeat(17 * 1024);
    let err = store
        .query_gql_read_only(&format!("MATCH (n:Node) RETURN '{padding}'"), 10)
        .expect_err("超长查询");
    assert_eq!(err.code, "GQL_QUERY_INVALID");
}

/// 只读查询之间互不污染：被拒的写入之后，随后的读查询照常返回完整数据。
#[test]
fn gql_rejected_write_does_not_poison_later_reads() {
    let store = seeded_store();
    let _ = store.query_gql_read_only("MATCH (n:Node) DETACH DELETE n", 10);
    let table = store
        .query_gql_read_only("MATCH (n:Node) RETURN count(n)", 10)
        .expect("later read");
    assert_eq!(table.rows, vec![vec![json!(3)]]);
}

/// 错误的 Display 形态是 `码: 说明`，CLI 与日志直接用。
#[test]
fn gql_error_display_leads_with_stable_code() {
    let err = GqlError::new("GQL_READ_ONLY", "拒绝");
    assert_eq!(err.to_string(), "GQL_READ_ONLY: 拒绝");
}

// ---------------------------------------------------------------- CLI 端到端

/// `--gql` 经真实二进制在扫描产出的 `.grafeo` 图库上跑通：JSON / TSV 两种输出、
/// 写入与 LOAD 被拒、redb 图库显式失败。判据取自 `--build-graph` 自己的节点计数，
/// 不硬编码语料规模。
#[cfg(feature = "cli-local")]
mod cli {
    use serde_json::Value;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn bin() -> PathBuf {
        PathBuf::from(env!("CARGO_BIN_EXE_metadata-checker"))
    }

    fn fixture_project() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/test_project")
    }

    /// 在独占目录里用 `--build-graph` 建库，返回（库路径，扫描报告里的节点数）。
    fn build_graph(tag: &str, file_name: &str) -> (PathBuf, u64) {
        let dir = super::unique_dir(tag);
        let db = dir.join(file_name);
        let output = Command::new(bin())
            .arg("--project-dir")
            .arg(fixture_project())
            .arg("--build-graph")
            .arg("--graph-db-path")
            .arg(&db)
            .output()
            .expect("run build-graph");
        assert!(
            output.status.success(),
            "build-graph 失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value =
            serde_json::from_slice(&output.stdout).expect("build-graph 输出一个 JSON 文档");
        let nodes = report["node_count"].as_u64().expect("node_count");
        (db, nodes)
    }

    fn run_gql(db: &Path, extra: &[&str], query: &str) -> (String, bool) {
        let output = Command::new(bin())
            .arg("--graph-db-path")
            .arg(db)
            .args(extra)
            .arg("--gql")
            .arg(query)
            .output()
            .expect("run --gql");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.success(),
        )
    }

    fn run_gql_json(db: &Path, extra: &[&str], query: &str) -> Value {
        let (stdout, ok) = run_gql(db, extra, query);
        assert!(ok, "--gql 进程应正常退出");
        serde_json::from_str(stdout.trim()).expect("--gql 输出一个 JSON 文档")
    }

    #[test]
    fn gql_counts_match_the_build_report() {
        let (db, nodes) = build_graph("count", "g.grafeo");
        let result = run_gql_json(&db, &[], "MATCH (n:Node) RETURN count(n) AS total");
        assert_eq!(result["columns"], serde_json::json!(["total"]));
        assert_eq!(result["rows"], serde_json::json!([[nodes]]));
        assert_eq!(result["truncated"], serde_json::json!(false));
    }

    #[test]
    fn gql_max_rows_truncates_and_reports_it() {
        let (db, nodes) = build_graph("trunc", "g.grafeo");
        let result = run_gql_json(
            &db,
            &["--gql-max-rows", "2"],
            "MATCH (n:Node) RETURN n.id ORDER BY n.id",
        );
        assert_eq!(result["row_count"], serde_json::json!(2));
        assert_eq!(result["total_rows"], serde_json::json!(nodes));
        assert_eq!(result["truncated"], serde_json::json!(true));
    }

    #[test]
    fn gql_human_mode_prints_tsv() {
        let (db, _) = build_graph("human", "g.grafeo");
        let (stdout, ok) = run_gql(
            &db,
            &["--human"],
            "MATCH (n:Node) RETURN n.node_type AS type, n.id AS id ORDER BY n.id LIMIT 2",
        );
        assert!(ok);
        let mut lines = stdout.lines();
        assert_eq!(lines.next(), Some("type\tid"));
        assert_eq!(lines.clone().count(), 2, "两行数据：{stdout}");
        assert!(lines.all(|line| line.contains('\t')));
    }

    /// 写入被拒，随后节点数不变——进程级验证（独立的重新打开）。
    #[test]
    fn gql_write_is_rejected_and_graph_survives() {
        let (db, nodes) = build_graph("write", "g.grafeo");
        let rejected = run_gql_json(&db, &[], "MATCH (n:Node) DETACH DELETE n");
        assert_eq!(rejected["ok"], serde_json::json!(false));
        assert_eq!(
            rejected["error"]["code"],
            serde_json::json!("GQL_READ_ONLY")
        );

        let after = run_gql_json(&db, &[], "MATCH (n:Node) RETURN count(n) AS total");
        assert_eq!(after["rows"], serde_json::json!([[nodes]]));
    }

    #[test]
    fn gql_load_is_rejected_through_the_cli() {
        let (db, _) = build_graph("load", "g.grafeo");
        let rejected = run_gql_json(
            &db,
            &[],
            "MATCH (n:Node) LOAD CSV FROM '/etc/passwd' AS row RETURN row[0]",
        );
        assert_eq!(rejected["ok"], serde_json::json!(false));
        assert_eq!(
            rejected["error"]["code"],
            serde_json::json!("GQL_READ_ONLY")
        );
    }

    #[test]
    fn gql_on_redb_graph_reports_backend_unsupported() {
        let (db, _) = build_graph("redb", "g.graphdb");
        let rejected = run_gql_json(&db, &[], "MATCH (n) RETURN n");
        assert_eq!(rejected["ok"], serde_json::json!(false));
        assert_eq!(
            rejected["error"]["code"],
            serde_json::json!("GQL_BACKEND_UNSUPPORTED")
        );
    }

    /// `--help` 里写的方言说明与示例必须真的成立：有人改引擎或图结构时，这条测试先红。
    #[test]
    fn gql_long_help_claims_hold_against_a_real_graph() {
        let help = Command::new(bin())
            .arg("--help")
            .output()
            .expect("run --help");
        let help = String::from_utf8_lossy(&help.stdout).into_owned();
        for needle in [
            "[:Reads|Triggers]",
            "CONTAINS",
            "=~",
            "NOT (n)--()",
            "IndexState",
        ] {
            assert!(help.contains(needle), "--help 应提到 {needle}");
        }

        let (db, _) = build_graph("help_claims", "g.grafeo");

        // 示例查询可直接运行。
        let example = run_gql_json(
            &db,
            &[],
            "MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path",
        );
        assert!(example.get("rows").is_some(), "示例 1 应成功：{example}");
        let example = run_gql_json(
            &db,
            &[],
            "MATCH (n:Node) WHERE n.meta CONTAINS 'visibleCondition' RETURN n.id",
        );
        assert!(example.get("rows").is_some(), "示例 2 应成功：{example}");

        // 多边类型写法 [:A|B] 成立，且结果等于两种边之和。
        let count = |query: &str| -> u64 {
            run_gql_json(&db, &[], query)["rows"][0][0]
                .as_u64()
                .expect("count 为整数")
        };
        let reads = count("MATCH (a:Node)-[r:Reads]->(b:Node) RETURN count(r)");
        let triggers = count("MATCH (a:Node)-[r:Triggers]->(b:Node) RETURN count(r)");
        let both = count("MATCH (a:Node)-[r:Reads|Triggers]->(b:Node) RETURN count(r)");
        assert_eq!(both, reads + triggers);

        // 帮助里声明不支持的写法确实失败（并以 GQL_QUERY_* 报错，而不是静默返回）。
        for unsupported in [
            "MATCH (n:Node) WHERE n.id =~ 'page.*' RETURN n.id",
            "MATCH (n:Node) WHERE NOT (n)--() RETURN n.id",
        ] {
            let rejected = run_gql_json(&db, &[], unsupported);
            assert_eq!(rejected["ok"], serde_json::json!(false), "{unsupported}");
        }

        // meta 键清单：Action 的 meta 含文档里列出的键。
        let meta = run_gql_json(
            &db,
            &[],
            "MATCH (n:Node) WHERE n.node_type = 'Action' RETURN n.meta LIMIT 1",
        );
        let meta_text = meta["rows"][0][0].as_str().expect("meta 是 JSON 文本");
        let meta: Value = serde_json::from_str(meta_text).expect("meta 可解析");
        for key in ["triggerType", "condition", "conditionExp", "waitPrev"] {
            assert!(meta.get(key).is_some(), "Action.meta 应含 {key}");
        }
    }
}
