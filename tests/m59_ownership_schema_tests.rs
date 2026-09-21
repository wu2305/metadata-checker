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
    graph.add_node("page:a".into(), NodeType::Page, "a.spg".into(), "a".into(), None);
    graph.persist(&HashMap::new()).unwrap();
    let layout = build_v2_layout(&graph, &HashMap::new()).unwrap();
    assert_eq!(GraphDB::open(&path).is_err(), true);
    assert_eq!(GraphDB::open_readonly(&path).is_err(), true);
    assert_eq!(read_v2_layout(&path).is_err(), true);
    assert_eq!(write_v2_shadow(&path, &layout).is_err(), true);
    assert_eq!(metadata_checker::scanner::indexer::ProjectIndexer::scan(path.parent().unwrap(), &path).is_err(), true);
    let check = GraphDB::check_graph_db(&path);
    assert_eq!(check.summary["readable"], false);
    assert_eq!(check.diagnostics.iter().any(|d| d.code == "GRAPH_PROJECT_BINDING_REQUIRED"), true);
    let reopened = GraphDB::open_readonly_for_project(&path, &binding).unwrap();
    assert_eq!(reopened.node_indices.len(), 1);
    assert_eq!(GraphDB::open_readonly_for_project(&path, &ProjectBinding::new("project-b").unwrap()).is_err(), true);
}

/// 模拟打开后库被绑定，旧句柄无论有无图改动都不能继续提交。
#[test]
fn persist_revalidates_open_handle_binding() {
    let path = temp_db("stale-handle");
    let mut stale = GraphDB::open(&path).unwrap();
    let binding = ProjectBinding::new("project-a").unwrap();
    let mut bound = GraphDB::open_for_project(&path, &binding).unwrap();
    assert_eq!(stale.persist(&HashMap::new()).is_err(), true);
    stale.add_node("page:wrong".into(), NodeType::Page, "wrong.spg".into(), "wrong".into(), None);
    assert_eq!(stale.persist(&HashMap::new()).is_err(), true);
    bound.add_node("page:right".into(), NodeType::Page, "right.spg".into(), "right".into(), None);
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
        ("unknown-pair", &[("project_binding_schema_version", "999"), ("project_binding", "project-a")]),
        ("legacy-ownership", &[("ownership_schema_version", "1"), ("project_binding", "project-a")]),
    ];
    for (label, entries) in states {
        let path = temp_db(label);
        GraphDB::open(&path).unwrap();
        {
            let db = redb::Database::open(&path).unwrap();
            let tx = db.begin_write().unwrap();
            {
                let mut meta = tx.open_table(TableDefinition::<&str, Vec<u8>>::new("meta")).unwrap();
                for (key, value) in *entries { meta.insert(*key, value.as_bytes().to_vec()).unwrap(); }
            }
            tx.commit().unwrap();
        }
        assert_eq!(GraphDB::open_for_project(&path, &ProjectBinding::new("project-b").unwrap()).is_err(), true, "{label}");
        assert_eq!(GraphDB::open(&path).is_err(), true, "{label}");
        let db = redb::Database::open(&path).unwrap();
        let tx = db.begin_read().unwrap();
        let meta = tx.open_table(TableDefinition::<&str, Vec<u8>>::new("meta")).unwrap();
        for key in ["project_binding", "project_binding_schema_version", "ownership_schema_version"] {
            let expected = entries.iter().find(|(name, _)| *name == key).map(|(_, v)| v.as_bytes().to_vec());
            assert_eq!(meta.get(key).unwrap().map(|v| v.value()), expected, "{label}: {key}");
        }
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
    let meta = tx.open_table(TableDefinition::<&str, Vec<u8>>::new("meta")).unwrap();
    assert_eq!(meta.get("project_binding_schema_version").unwrap().unwrap().value(), b"1".to_vec());
    assert_eq!(meta.get("ownership_schema_version").unwrap().is_none(), true);
}
