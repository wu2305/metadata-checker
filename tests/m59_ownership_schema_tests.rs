#![cfg(feature = "cli-local")]

use metadata_checker::graph::{GraphDB, NodeType};
use metadata_checker::ownership::ProjectBinding;
use std::collections::HashMap;
use std::path::PathBuf;

fn temp_db(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "metadata-checker-m59-ownership-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn empty_store_initializes_ownership_binding_and_reopens() {
    let db_path = temp_db("init");
    let binding = ProjectBinding::new("project-a").expect("valid binding");

    let mut graph = GraphDB::open_for_project(&db_path, &binding).expect("initialize store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph.persist(&HashMap::new()).expect("persist graph");

    GraphDB::open_for_project(&db_path, &binding).expect("reopen with original binding");
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn populated_legacy_store_cannot_receive_ownership_marker() {
    let db_path = temp_db("legacy");
    let binding = ProjectBinding::new("project-a").expect("valid binding");

    let mut graph = GraphDB::open(&db_path).expect("create legacy store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph
        .persist(&HashMap::new())
        .expect("persist legacy graph");

    // 只读绑定入口也不能把无绑定的旧图认作当前项目。
    assert_eq!(
        GraphDB::open_readonly_for_project(&db_path, &binding)
            .err()
            .unwrap()
            .to_string()
            .contains("GRAPH_OWNERSHIP_SCHEMA_STALE"),
        true
    );
    let error = match GraphDB::open_for_project(&db_path, &binding) {
        Ok(_) => panic!("legacy populated store must require rebuild"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("GRAPH_OWNERSHIP_SCHEMA_STALE"));
    let _ = std::fs::remove_file(db_path);
}

#[test]
fn project_binding_mismatch_is_rejected_after_restart() {
    let db_path = temp_db("mismatch");
    let first = ProjectBinding::new("project-a").expect("valid binding");
    let second = ProjectBinding::new("project-b").expect("valid binding");

    let mut graph = GraphDB::open_for_project(&db_path, &first).expect("initialize store");
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph.persist(&HashMap::new()).expect("persist graph");

    let error = match GraphDB::open_for_project(&db_path, &second) {
        Ok(_) => panic!("different project must not open the store"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("GRAPH_PROJECT_BINDING_MISMATCH"));
    let _ = std::fs::remove_file(db_path);
}

/// 读写公共入口都不得把绑定当作可选提示，旧扫描入口同样拒绝。
#[test]
fn unbound_entry_points_reject_bound_database() {
    use metadata_checker::graph_redb_v2::{build_v2_layout, read_v2_layout, write_v2_shadow};
    let path = temp_db("unbound-entry");
    let binding = ProjectBinding::new("project-a").unwrap();
    let mut graph = GraphDB::open_for_project(&path, &binding).unwrap();
    graph.add_node(
        "page:a".into(),
        NodeType::Page,
        "a.spg".into(),
        "a".into(),
        None,
    );
    graph.persist(&HashMap::new()).unwrap();
    let layout = build_v2_layout(&graph, &HashMap::new()).unwrap();
    assert_eq!(GraphDB::open(&path).is_err(), true);
    assert_eq!(GraphDB::open_readonly(&path).is_err(), true);
    assert_eq!(read_v2_layout(&path).is_err(), true);
    assert_eq!(write_v2_shadow(&path, &layout).is_err(), true);
    assert_eq!(
        metadata_checker::scanner::indexer::ProjectIndexer::scan(path.parent().unwrap(), &path)
            .is_err(),
        true
    );
    let check = GraphDB::check_graph_db(&path);
    assert_eq!(check.summary["readable"], false);
    assert_eq!(
        check
            .diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_PROJECT_BINDING_REQUIRED"),
        true
    );
    let reopened = GraphDB::open_readonly_for_project(&path, &binding).unwrap();
    assert_eq!(reopened.node_indices.len(), 1);
    assert_eq!(
        GraphDB::open_readonly_for_project(&path, &ProjectBinding::new("project-b").unwrap())
            .is_err(),
        true
    );
}

/// 模拟打开后库被绑定，旧句柄无论有无图改动都不能继续提交。
#[test]
fn persist_revalidates_open_handle_binding() {
    let path = temp_db("stale-handle");
    let mut stale = GraphDB::open(&path).unwrap();
    let binding = ProjectBinding::new("project-a").unwrap();
    let mut bound = GraphDB::open_for_project(&path, &binding).unwrap();
    assert_eq!(stale.persist(&HashMap::new()).is_err(), true);
    assert_eq!(stale.load_file_states().is_err(), true);
    assert_eq!(stale.load_scanner_diagnostic_entries().is_err(), true);
    assert_eq!(stale.load_diff_refresh_checkpoint().is_err(), true);
    stale.add_node(
        "page:wrong".into(),
        NodeType::Page,
        "wrong.spg".into(),
        "wrong".into(),
        None,
    );
    assert_eq!(stale.persist(&HashMap::new()).is_err(), true);
    bound.add_node(
        "page:right".into(),
        NodeType::Page,
        "right.spg".into(),
        "right".into(),
        None,
    );
    bound.persist(&HashMap::new()).unwrap();
    let reopened = GraphDB::open_for_project(&path, &binding).unwrap();
    assert_eq!(reopened.node_indices.contains_key("page:wrong"), false);
    assert_eq!(reopened.node_indices.contains_key("page:right"), true);
}

/// 直接构造磁盘状态，验证所有不完整组合都拒载且不被修补。
#[test]
fn incomplete_unknown_and_legacy_markers_are_never_repaired() {
    use redb::{ReadableDatabase, TableDefinition};
    let states: &[(&str, &[(&str, &str)])] = &[
        ("binding-only", &[("project_binding", "project-a")]),
        ("version-only", &[("project_binding_schema_version", "1")]),
        ("unknown-only", &[("project_binding_schema_version", "999")]),
        (
            "unknown-pair",
            &[
                ("project_binding_schema_version", "999"),
                ("project_binding", "project-a"),
            ],
        ),
        (
            "legacy-ownership",
            &[
                ("ownership_schema_version", "1"),
                ("project_binding", "project-a"),
            ],
        ),
        ("ownership-only", &[("ownership_schema_version", "2")]),
        (
            "ownership-and-binding-only",
            &[
                ("ownership_schema_version", "2"),
                ("project_binding", "project-a"),
            ],
        ),
        (
            "ownership-unknown-version",
            &[
                ("ownership_schema_version", "999"),
                ("ownership_ledger_version", "1"),
                ("project_binding_schema_version", "1"),
                ("project_binding", "project-a"),
            ],
        ),
        (
            "ledger-unknown-version",
            &[
                ("ownership_schema_version", "2"),
                ("ownership_ledger_version", "999"),
                ("project_binding_schema_version", "1"),
                ("project_binding", "project-a"),
            ],
        ),
        (
            "ownership-missing-ledger",
            &[
                ("ownership_schema_version", "2"),
                ("project_binding_schema_version", "1"),
                ("project_binding", "project-a"),
            ],
        ),
    ];
    for (label, entries) in states {
        let path = temp_db(label);
        GraphDB::open(&path).unwrap();
        {
            let db = redb::Database::open(&path).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut meta = tx
                    .open_table(TableDefinition::<&str, Vec<u8>>::new("meta"))
                    .unwrap();
                for (key, value) in *entries {
                    meta.insert(*key, value.as_bytes().to_vec()).unwrap();
                }
            }
            tx.commit().unwrap();
        }
        let matching_binding = ProjectBinding::new("project-a").unwrap();
        let other_binding = ProjectBinding::new("project-b").unwrap();
        // 无论是匹配绑定还是其他绑定，不完整或未知状态均拒绝
        assert_eq!(
            GraphDB::open_for_project(&path, &other_binding).is_err(),
            true,
            "{label} open_for_project other"
        );
        assert_eq!(
            GraphDB::open_with_ownership(&path, &matching_binding).is_err(),
            true,
            "{label} open_with_ownership matching"
        );
        assert_eq!(GraphDB::open(&path).is_err(), true, "{label} open legacy");
        let check = GraphDB::check_graph_db(&path);
        assert_eq!(
            check.summary["needs_rebuild"], true,
            "{label} check rebuild"
        );
        assert_eq!(
            check
                .diagnostics
                .iter()
                .any(|d| d.code == "GRAPH_OWNERSHIP_SCHEMA_STALE"
                    || d.code == "GRAPH_PROJECT_BINDING_REQUIRED"),
            true,
            "{label} diagnostic"
        );
        // 关键门槛断言：磁盘原库数据必须保持不变，绝不补写覆盖！
        let db = redb::Database::open(&path).unwrap();
        let tx = db.begin_read().unwrap();
        let meta = tx
            .open_table(TableDefinition::<&str, Vec<u8>>::new("meta"))
            .unwrap();
        for key in [
            "project_binding",
            "project_binding_schema_version",
            "ownership_schema_version",
            "ownership_ledger_version",
        ] {
            let expected = entries
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, v)| v.as_bytes().to_vec());
            assert_eq!(
                meta.get(key).unwrap().map(|v| v.value()),
                expected,
                "{label}: {key}"
            );
        }
        let _ = std::fs::remove_file(path);
    }
}

/// 准备格式不得伪装为已启用来源账本。
#[test]
fn prepared_binding_does_not_write_ownership_marker() {
    use redb::{ReadableDatabase, TableDefinition};
    let path = temp_db("prepared-marker");
    GraphDB::open_for_project(&path, &ProjectBinding::new("project-a").unwrap()).unwrap();
    let db = redb::Database::open(&path).unwrap();
    let tx = db.begin_read().unwrap();
    let meta = tx
        .open_table(TableDefinition::<&str, Vec<u8>>::new("meta"))
        .unwrap();
    assert_eq!(
        meta.get("project_binding_schema_version")
            .unwrap()
            .unwrap()
            .value(),
        b"1".to_vec()
    );
    assert_eq!(
        meta.get("ownership_schema_version").unwrap().is_none(),
        true
    );
}

/// 验证 check_graph_db_for_project 与 CLI 访问在缺失、错误及匹配绑定时的诊断表现。
#[test]
fn check_graph_db_for_project_reports_exact_diagnostics() {
    let path = temp_db("diag-checks");
    let binding = ProjectBinding::new("project-a").unwrap();
    let wrong_binding = ProjectBinding::new("project-b").unwrap();

    let mut graph = GraphDB::open_with_ownership(&path, &binding).unwrap();
    graph.add_node(
        "page:app/a.spg".to_string(),
        NodeType::Page,
        "app/a.spg".to_string(),
        "a".to_string(),
        None,
    );
    graph.persist(&HashMap::new()).unwrap();

    // 1. 无 binding 访问 bound db -> 报 GRAPH_PROJECT_BINDING_REQUIRED
    let check_no_binding = GraphDB::check_graph_db_for_project(&path, None);
    assert_eq!(check_no_binding.summary["readable"], false);
    assert!(
        check_no_binding
            .diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_PROJECT_BINDING_REQUIRED"),
        "缺失 binding 必须报 GRAPH_PROJECT_BINDING_REQUIRED"
    );

    // 2. 错误 binding 访问 bound db -> 报 GRAPH_PROJECT_BINDING_MISMATCH
    let check_wrong = GraphDB::check_graph_db_for_project(&path, Some(&wrong_binding));
    assert_eq!(check_wrong.summary["readable"], false);
    assert!(
        check_wrong
            .diagnostics
            .iter()
            .any(|d| d.code == "GRAPH_PROJECT_BINDING_MISMATCH"),
        "错误 binding 必须报 GRAPH_PROJECT_BINDING_MISMATCH"
    );

    // 3. 正确 binding 访问 bound db -> readable: true，无报错
    let check_correct = GraphDB::check_graph_db_for_project(&path, Some(&binding));
    assert_eq!(check_correct.summary["readable"], true);
    assert_eq!(check_correct.summary["needs_rebuild"], false);
    assert!(check_correct.diagnostics.is_empty());

    let _ = std::fs::remove_file(path);
}

/// JSON 输入不能绕过绑定构造器；阻断诊断必须传到消费方。
#[test]
fn binding_deserialization_and_diagnostic_contract() {
    assert_eq!(
        serde_json::from_str::<ProjectBinding>(r#""/tmp/project""#).is_err(),
        true
    );
    for code in [
        "GRAPH_OWNERSHIP_SCHEMA_STALE",
        "GRAPH_PROJECT_BINDING_REQUIRED",
        "GRAPH_PROJECT_BINDING_MISMATCH",
    ] {
        assert_eq!(
            metadata_checker::diagnostics::answer_impact_for(code),
            "blocking"
        );
    }
}
