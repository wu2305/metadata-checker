#![cfg(feature = "cli-local")]

use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_redb::GraphDB;
use metadata_checker::scanner::scan_project;
use redb::{Database, TableDefinition};
use metadata_checker::graph_store::V2ShadowState;
use std::time::{SystemTime, UNIX_EPOCH};

const NODES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("nodes");
const EDGES_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("edges");

fn fixture_graph(tag: &str) -> anyhow::Result<(GraphDB, std::path::PathBuf)> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-{}-{}-{nanos}.db",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&db_path);
    scan_project(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    let graph = GraphDB::open(&db_path)?;
    Ok((graph, db_path))
}

fn has_code(diags: &[metadata_checker::output::Diagnostic], code: &str) -> bool {
    diags.iter().any(|d| d.code == code)
}

#[test]
fn pr1_hydrate_counts_and_partial_gate() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("hydrate-partial")?;
    drop(graph);

    // 注入：一条坏节点、一条坏边、一条悬挂边
    {
        let db = Database::create(&db_path)?;
        let write_txn = db.begin_write()?;
        {
            let mut nodes = write_txn.open_table(NODES_TABLE)?;
            nodes.insert("bad_node:1", b"not-json".to_vec())?;
        }
        {
            let mut edges = write_txn.open_table(EDGES_TABLE)?;
            edges.insert("bad_edge_key", b"not-json".to_vec())?;
            // 悬挂边：from/to 均不存在
            let dangling = metadata_checker::graph::Edge {
                from: "ghost:from".to_string(),
                to: "ghost:to".to_string(),
                edge_type: EdgeType::Reads,
                field_path: None,
                meta: None,
            };
            let key = metadata_checker::graph_redb::edge_storage_key(&dangling);
            edges.insert(key.as_str(), serde_json::to_vec(&dangling)?)?;
        }
        // Force v2 to Stale so hydrate falls back to v1 tables (where bad rows live)
        {
            let mut meta = write_txn.open_table::<&str, Vec<u8>>(redb::TableDefinition::new("v2_meta"))?;
            let bytes = serde_json::to_vec(&V2ShadowState::Stale).unwrap();
            meta.insert("shadow_state", bytes)?;
        }
        write_txn.commit()?;
    }

    let reopened = GraphDB::open(&db_path)?;
    let diags = reopened.hydrate_diagnostics().to_diagnostics();
    assert!(has_code(&diags, "GRAPH_DB_NODE_DECODE_FAILED"), "{diags:?}");
    assert!(has_code(&diags, "GRAPH_DB_EDGE_DECODE_FAILED"), "{diags:?}");
    assert!(has_code(&diags, "GRAPH_DB_EDGE_DANGLING_ENDPOINT"), "{diags:?}");
    assert!(has_code(&diags, "GRAPH_DB_PARTIAL_HYDRATE"), "{diags:?}");

    // status 透出
    let rt = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    assert!(has_code(&rt.load_diagnostics, "GRAPH_DB_PARTIAL_HYDRATE"));
    assert!(has_code(&rt.status().load_diagnostics, "GRAPH_DB_PARTIAL_HYDRATE"));

    // 查询响应同样带闸门
    let mut rt2 = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    let resp = rt2.query(metadata_checker::runtime::RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryPageLogic,
        target: "page:app/actions_test.spg".to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    })?;
    let result_str = serde_json::to_string(&resp.result)?;
    assert!(result_str.contains("GRAPH_DB_PARTIAL_HYDRATE"), "{result_str}");

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_normal_path_has_no_partial() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("normal-zero")?;
    let diags = graph.hydrate_diagnostics().to_diagnostics();
    // 正常路径：partial 相关诊断必须为 0（白名单基线在 spec 登记，当前为 0）
    assert!(!has_code(&diags, "GRAPH_DB_PARTIAL_HYDRATE"), "{diags:?}");
    assert!(!has_code(&diags, "GRAPH_DB_NODE_DECODE_FAILED"), "{diags:?}");
    assert!(!has_code(&diags, "GRAPH_DB_EDGE_DECODE_FAILED"), "{diags:?}");
    assert!(!has_code(&diags, "GRAPH_DB_EDGE_DANGLING_ENDPOINT"), "{diags:?}");
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_build_output_exposes_diagnostics_via_status() -> anyhow::Result<()> {
    // 复用 normal 路径的 status 透出作为构建输出的代理验证
    let (_graph, db_path) = fixture_graph("build-status")?;
    let rt = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    let status_json = serde_json::to_value(rt.status())?;
    assert!(status_json.get("load_diagnostics").is_some());
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}
