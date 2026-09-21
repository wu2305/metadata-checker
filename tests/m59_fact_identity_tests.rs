use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_redb::{GraphDB, edge_storage_key};
use metadata_checker::graph_store::{GraphReadStore, GraphStore};
use metadata_checker::memory_graph_store::MemoryGraphStore;
use serde_json::json;

/// 使用进程与单调计数隔离测试目录，不增加测试依赖。
fn unique_temp_dir() -> std::path::PathBuf {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("m59-facts-{}-{stamp}-{sequence}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

/// 手工语义契约：两份来源必须各自保留，完全重复写入必须幂等。
fn seed_and_assert(store: &mut dyn GraphStore, reverse: bool) {
    for id in ["a", "b"] {
        store
            .upsert_node(Node {
                id: id.into(),
                node_type: NodeType::Component,
                path: "page.spg".into(),
                name: id.into(),
                meta: None,
            })
            .unwrap();
    }
    let fields = if reverse {
        ["visible", "value"]
    } else {
        ["value", "visible"]
    };
    for field in fields {
        let edge = Edge {
            from: "a".into(),
            to: "b".into(),
            edge_type: EdgeType::DependsOn,
            field_path: Some("comp:b.value".into()),
            meta: Some(
                json!({"source_file":"page.spg","json_path":format!("canvas.components[0].{field}")}),
            ),
        };
        store.add_edge(edge.clone()).unwrap();
        store.add_edge(edge).unwrap();
    }
    assert_facts(store);
}

/// 只对无序事实集合排序；逐个断言证据位置，不删除属性。
fn assert_facts(store: &dyn GraphReadStore) {
    let mut paths: Vec<_> = store
        .get_node_edges("a")
        .unwrap()
        .unwrap()
        .outgoing
        .into_iter()
        .map(|view| {
            view.edge.meta.unwrap()["json_path"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        vec!["canvas.components[0].value", "canvas.components[0].visible"]
    );
}

#[test]
fn facts_are_idempotent_and_independent_of_insertion_order() {
    for reverse in [false, true] {
        seed_and_assert(&mut MemoryGraphStore::new(), reverse);
    }
}

#[test]
fn redb_preserves_both_evidence_records_across_restart() {
    let directory = unique_temp_dir();
    let path = directory.join("facts.redb");
    let mut store = GraphDB::open(&path).unwrap();
    seed_and_assert(&mut store, false);
    store.persist(&Default::default()).unwrap();
    drop(store);
    assert_facts(&GraphDB::open(&path).unwrap());
    assert_facts(&GraphDB::open_readonly(&path).unwrap());
}

#[test]
fn storage_key_distinguishes_absent_and_empty_field_paths() {
    let mut edge = Edge {
        from: "a|b".into(),
        to: "c".into(),
        edge_type: EdgeType::Reads,
        field_path: None,
        meta: None,
    };
    let absent = edge_storage_key(&edge);
    edge.field_path = Some(String::new());
    assert_eq!(absent == edge_storage_key(&edge), false);
}

#[test]
fn missing_version_on_nonempty_database_is_rejected_without_upgrade() {
    use redb::TableDefinition;
    let directory = unique_temp_dir();
    let path = directory.join("legacy.redb");
    let mut graph = GraphDB::open(&path).unwrap();
    seed_and_assert(&mut graph, false);
    graph.persist(&Default::default()).unwrap();
    drop(graph);
    {
        let database = redb::Database::open(&path).unwrap();
        let transaction = database.begin_write().unwrap();
        transaction
            .open_table(TableDefinition::<&str, Vec<u8>>::new("meta"))
            .unwrap()
            .remove("fact_schema_version")
            .unwrap();
        transaction.commit().unwrap();
    }
    assert_eq!(
        GraphDB::open(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("GRAPH_SCHEMA_STALE"),
        true
    );
    assert_eq!(
        GraphDB::open_readonly(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("GRAPH_SCHEMA_STALE"),
        true
    );
}

// 从真实 scanner 入口验证属性语义与同关系的不同来源，而非只测 store。
#[test]
fn scanner_preserves_property_targets_and_each_source_field() {
    let mut graph = MemoryGraphStore::new();
    metadata_checker::scanner::process_spg_file_from_value(&mut graph, "page.spg", json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=b.value", "visibleCondition":"=b.value", "enable":"=b.step"},
        {"id":"b","type":"input","value":"=1"}
    ]}})).unwrap();
    let edges = graph
        .get_node_edges("comp:page.spg|a")
        .unwrap()
        .unwrap()
        .outgoing;
    let mut facts: Vec<_> = edges
        .iter()
        .filter(|view| {
            view.edge.edge_type == EdgeType::DependsOn && view.edge.to == "comp:page.spg|b"
        })
        .map(|view| {
            (
                view.edge.field_path.clone().unwrap(),
                view.edge.meta.as_ref().unwrap()["source_field"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            )
        })
        .collect();
    facts.sort();
    assert_eq!(
        facts,
        vec![
            ("comp:b.step".into(), "enable".into()),
            ("comp:b.value".into(), "value".into()),
            ("comp:b.value".into(), "visibleCondition".into())
        ]
    );
}
