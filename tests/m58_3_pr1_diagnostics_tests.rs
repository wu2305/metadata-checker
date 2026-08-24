#![cfg(feature = "cli-local")]

use metadata_checker::graph::EdgeType;
use metadata_checker::graph_redb::GraphDB;
use metadata_checker::graph_store::V2ShadowState;
use metadata_checker::scanner::scan_project;
use redb::{Database, TableDefinition};
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

fn has_code_in_value(value: &serde_json::Value, code: &str) -> bool {
    serde_json::to_string(value)
        .unwrap_or_default()
        .contains(code)
}

fn assert_envelope_serializes(value: &serde_json::Value) {
    for field in [
        "code",
        "severity",
        "count",
        "sample_location",
        "answer_impact",
        "first_seen_phase",
    ] {
        assert!(value.get(field).is_some(), "missing field {field}: {value}");
    }
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
            let mut meta =
                write_txn.open_table::<&str, Vec<u8>>(redb::TableDefinition::new("v2_meta"))?;
            let bytes = serde_json::to_vec(&V2ShadowState::Stale).unwrap();
            meta.insert("shadow_state", bytes)?;
        }
        write_txn.commit()?;
    }

    let reopened = GraphDB::open(&db_path)?;
    let diags = reopened.hydrate_diagnostics().to_diagnostics();
    assert!(has_code(&diags, "GRAPH_DB_NODE_DECODE_FAILED"), "{diags:?}");
    assert!(has_code(&diags, "GRAPH_DB_EDGE_DECODE_FAILED"), "{diags:?}");
    assert!(
        has_code(&diags, "GRAPH_DB_EDGE_DANGLING_ENDPOINT"),
        "{diags:?}"
    );
    assert!(has_code(&diags, "GRAPH_DB_PARTIAL_HYDRATE"), "{diags:?}");

    // status 透出
    let rt = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    assert!(has_code(&rt.load_diagnostics, "GRAPH_DB_PARTIAL_HYDRATE"));
    assert!(has_code(
        &rt.status().load_diagnostics,
        "GRAPH_DB_PARTIAL_HYDRATE"
    ));

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
    assert!(
        result_str.contains("GRAPH_DB_PARTIAL_HYDRATE"),
        "{result_str}"
    );

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_normal_path_has_no_partial() -> anyhow::Result<()> {
    let (graph, db_path) = fixture_graph("normal-zero")?;
    let diags = graph.hydrate_diagnostics().to_diagnostics();
    // 正常路径：pin 语料（xiaoshouyi-corpus @ 6920ac51）全量构建实测 GRAPH_DB_* 均为 0
    //（已登记在 spec 验收 1，2026-08-24）；fixture 同样为 0
    assert!(!has_code(&diags, "GRAPH_DB_PARTIAL_HYDRATE"), "{diags:?}");
    assert!(
        !has_code(&diags, "GRAPH_DB_NODE_DECODE_FAILED"),
        "{diags:?}"
    );
    assert!(
        !has_code(&diags, "GRAPH_DB_EDGE_DECODE_FAILED"),
        "{diags:?}"
    );
    assert!(
        !has_code(&diags, "GRAPH_DB_EDGE_DANGLING_ENDPOINT"),
        "{diags:?}"
    );
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_build_output_exposes_diagnostics_via_status() -> anyhow::Result<()> {
    let (_graph, db_path) = fixture_graph("build-status")?;
    let rt = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    let status_json = serde_json::to_value(rt.status())?;
    assert!(status_json.get("load_diagnostics").is_some());
    // 直接断言 build 输出本体（scan_project_with_report / scan_with_diagnostics）
    let db_path2 = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-build-output-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path2);
    let report = metadata_checker::scanner::scan_project_with_report(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path2,
    )?;
    let report_json = serde_json::to_value(&report)?;
    assert!(report_json.get("diagnostics").is_some());
    let inner = metadata_checker::scanner::indexer::ProjectIndexer::scan_with_diagnostics(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path2,
    )?;
    assert!(serde_json::to_value(&inner.diagnostics).is_ok());
    let _ = std::fs::remove_file(&db_path2);
    let _ = std::fs::remove_file(db_path2.with_extension("graphdb.lock"));

    // CLI 级断言：真实二进制的 --build-graph 输出统计行与 built-at 行（非库函数代理）
    let db_path3 = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-cli-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path3);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_metadata-checker"))
        .args([
            "--project-dir",
            "tests/fixtures/test_project",
            "--build-graph",
            "--graph-db-path",
            db_path3.to_str().expect("utf8 path"),
        ])
        .output()
        .expect("run cli --build-graph");
    assert!(
        output.status.success(),
        "cli exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Indexed "), "{stdout}");
    assert!(stdout.contains("Graph database built at"), "{stdout}");
    let _ = std::fs::remove_file(&db_path3);
    let _ = std::fs::remove_file(db_path3.with_extension("graphdb.lock"));
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_scanner_diagnostics_trigger_and_count() -> anyhow::Result<()> {
    let raw_unrec = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "a", "type": "panel", "myContainer": [{"id": "b", "type": "button"}]}
            ]
        }
    });
    let diags_unrec = metadata_checker::scanner::scan_raw_diagnostics(&raw_unrec);
    assert!(
        has_code(&diags_unrec, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        "{diags_unrec:?}"
    );
    for d in &diags_unrec {
        let v = serde_json::to_value(d)?;
        assert_envelope_serializes(&v);
    }
    let raw_dup = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "dup1", "type": "button"},
                {"id": "dup1", "type": "input"}
            ]
        }
    });
    let diags_dup = metadata_checker::scanner::scan_raw_diagnostics(&raw_dup);
    assert!(
        has_code(&diags_dup, "SCANNER_DUPLICATE_COMPONENT_ID"),
        "{diags_dup:?}"
    );
    for d in &diags_dup {
        let v = serde_json::to_value(d)?;
        assert_envelope_serializes(&v);
    }
    // 排除列表不吞合法信号：含未识别容器键的临时项目在 ScanReport.diagnostics 中可见
    let bad_project = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-badproj-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    std::fs::create_dir_all(&bad_project)?;
    std::fs::write(
        bad_project.join("bad.spg"),
        serde_json::to_string(&raw_unrec)?,
    )?;
    let bad_db = bad_project.join("graph.db");
    let bad_report = metadata_checker::scanner::scan_project_with_report(&bad_project, &bad_db)?;
    assert!(
        has_code(
            &bad_report.diagnostics,
            "SCANNER_UNRECOGNIZED_CONTAINER_KEY"
        ),
        "{:?}",
        bad_report.diagnostics
    );
    let _ = std::fs::remove_dir_all(&bad_project);

    // 正常 fixture 语料：SCANNER_* 计数必须为 0（排除列表生效，无误报）
    let db_path = std::env::temp_dir().join(format!(
        "metadata-checker-m58-3-pr1-scanner-prod-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&db_path);
    let report = metadata_checker::scanner::scan_project_with_report(
        std::path::Path::new("tests/fixtures/test_project"),
        &db_path,
    )?;
    assert!(
        !has_code(&report.diagnostics, "SCANNER_UNRECOGNIZED_CONTAINER_KEY"),
        "{:?}",
        report.diagnostics
    );
    assert!(
        !has_code(&report.diagnostics, "SCANNER_DUPLICATE_COMPONENT_ID"),
        "{:?}",
        report.diagnostics
    );
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_page_scoped_fallback_single_diagnostic() -> anyhow::Result<()> {
    let (_graph, db_path) = fixture_graph("fallback-single")?;
    let page_id = "page:app/actions_test.spg";
    let page_path = "app/actions_test.spg";
    // 构造点归属：聚合诊断由唯一构造函数产生
    let diag =
        metadata_checker::query::page_scoped_fallback_diagnostic_for_test(page_id, page_path, 2);
    assert_eq!(diag.code, "PAGE_SCOPED_TARGET_FALLBACK");
    assert_eq!(diag.count, Some(2));
    assert!(diag.answer_impact.is_some());

    // 真实查询路径不变式：key_model_availability 中带 scope_warning 的条目数
    // 与该诊断的存在性/计数严格一致，且同一 code 至多一条（聚合唯一产生点）。
    //
    // 注意：当前 scanner 会为页面引用的任何模型物化节点，page-scoped 解析在实践中
    // 总能成功——6 个 fixture 页加 2 个合成幽灵模型页（无 sources、条件引用不存在模型）
    // 实测全部 resolved scoped、无 fallback，即该诊断在 PR3 页面段 id 落地前事实不可达。
    // 此处钉住的是接线不变式（触发时恰好一条聚合、计数等于回退数），而非触发本身。
    let mut rt = metadata_checker::runtime::GraphRuntime::load(&db_path)?;
    let resp = rt.query(metadata_checker::runtime::RuntimeQueryRequest {
        command: metadata_checker::tool_contract::ToolCommand::QueryPageLogic,
        target: page_id.to_string(),
        budget: "compact".to_string(),
        human: false,
        intent: None,
        page_scope: None,
        depth: None,
        check_reload: false,
    })?;
    let diags = resp
        .result
        .get("diagnostics")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    let fallback_hits: Vec<&serde_json::Value> = diags
        .iter()
        .filter(|d| d.get("code").and_then(|c| c.as_str()) == Some("PAGE_SCOPED_TARGET_FALLBACK"))
        .collect();
    assert!(
        fallback_hits.len() <= 1,
        "同一 code 至多一条聚合诊断: {diags:?}"
    );
    let warned_items = resp
        .result
        .get("details")
        .and_then(|d| d.get("key_model_availability"))
        .and_then(|k| k.get("items"))
        .and_then(|i| i.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("scope_warning").and_then(|v| v.as_str()).is_some())
                .count()
        })
        .unwrap_or(0);
    if warned_items > 0 {
        assert_eq!(
            fallback_hits.len(),
            1,
            "有回退必须恰好一条聚合诊断: {diags:?}"
        );
        let count = fallback_hits[0].get("count").and_then(|c| c.as_u64());
        assert!(count.is_some_and(|c| c >= 1), "{diags:?}");
    } else {
        assert!(fallback_hits.is_empty(), "无回退不得产生该诊断: {diags:?}");
    }
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("graphdb.lock"));
    Ok(())
}

#[test]
fn pr1_envelope_contract_six_fields() -> anyhow::Result<()> {
    let diag = metadata_checker::diagnostics::envelope_diagnostic(
        metadata_checker::diagnostics::CODE_GRAPH_DB_NODE_DECODE_FAILED,
        2,
        metadata_checker::output::Location {
            source_file: Some("a.spg".to_string()),
            node_id: Some("n1".to_string()),
            json_path: Some("$.a".to_string()),
        },
        "test",
    );
    let value = serde_json::to_value(&diag)?;
    assert_envelope_serializes(&value);
    let expected_codes = [
        "SCANNER_UNRECOGNIZED_CONTAINER_KEY",
        "SCANNER_DUPLICATE_COMPONENT_ID",
        "GRAPH_DB_NODE_DECODE_FAILED",
        "GRAPH_DB_EDGE_DECODE_FAILED",
        "GRAPH_DB_EDGE_DANGLING_ENDPOINT",
        "GRAPH_DB_V2_LAYOUT_UNREADABLE",
        "GRAPH_DB_PARTIAL_HYDRATE",
        "PAGE_SCOPED_TARGET_FALLBACK",
    ];
    let code_set: std::collections::HashSet<&str> = expected_codes.iter().copied().collect();
    assert_eq!(code_set.len(), expected_codes.len());
    assert!(code_set.contains("PAGE_SCOPED_TARGET_FALLBACK"));
    assert!(has_code_in_value(&value, "GRAPH_DB_NODE_DECODE_FAILED"));
    // PARTIAL_HYDRATE confidence 应为 partial（非 reduced）
    let conf = metadata_checker::output::answer_effect::confidence_value(
        ["GRAPH_DB_PARTIAL_HYDRATE"].iter().copied(),
    );
    assert_eq!(
        conf.get("level").and_then(|v| v.as_str()),
        Some("partial"),
        "{conf}"
    );
    Ok(())
}
