//! 图 Schema 契约（`src/graph_schema.rs`）的一致性测试。
//!
//! 契约是唯一真源，这里证明三件事：
//! 1. 表覆盖了每一个 `NodeType` / `EdgeType` 变体（新增变体不改表就编译不过 / 测试变红）；
//! 2. 夹具项目扫描出的**每一个**节点和边都符合表（id 前缀、`meta` 键、端点组合、
//!    `field_path` 约定、`emitted = false` 的边确实不出现）；
//! 3. `docs/reference/graph-schema.md` 与 `--graph-schema --human` 一字不差。
#![cfg(feature = "grafeo-store")]

use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_schema::{self, FieldPathUse, variant_name};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_metadata-checker"))
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// 穷举匹配：新增 `NodeType` 变体会让这里编译失败，迫使同时更新契约表。
fn all_node_types() -> Vec<NodeType> {
    let probe = NodeType::Page;
    match probe {
        NodeType::Page
        | NodeType::Component
        | NodeType::Model
        | NodeType::Field
        | NodeType::Action
        | NodeType::Condition => {}
    }
    vec![
        NodeType::Page,
        NodeType::Component,
        NodeType::Model,
        NodeType::Field,
        NodeType::Action,
        NodeType::Condition,
    ]
}

/// 穷举匹配：新增 `EdgeType` 变体会让这里编译失败，迫使同时更新契约表。
fn all_edge_types() -> Vec<EdgeType> {
    let probe = EdgeType::Reads;
    match probe {
        EdgeType::Reads
        | EdgeType::Writes
        | EdgeType::Triggers
        | EdgeType::Contains
        | EdgeType::DataflowInput
        | EdgeType::ActionWrites
        | EdgeType::EmbedsPage
        | EdgeType::OpensPage
        | EdgeType::PassesParam
        | EdgeType::SetsParam
        | EdgeType::OutputsTo
        | EdgeType::DataflowInternal
        | EdgeType::DataflowOutput
        | EdgeType::FieldAlias
        | EdgeType::FieldWrite
        | EdgeType::ActionReads
        | EdgeType::ActionNavigates
        | EdgeType::ActionSetsParam
        | EdgeType::ActionControlsComponent
        | EdgeType::ActionValidates
        | EdgeType::ActionLoadsData
        | EdgeType::DependsOn => {}
    }
    vec![
        EdgeType::Reads,
        EdgeType::Writes,
        EdgeType::Triggers,
        EdgeType::Contains,
        EdgeType::DataflowInput,
        EdgeType::ActionWrites,
        EdgeType::EmbedsPage,
        EdgeType::OpensPage,
        EdgeType::PassesParam,
        EdgeType::SetsParam,
        EdgeType::OutputsTo,
        EdgeType::DataflowInternal,
        EdgeType::DataflowOutput,
        EdgeType::FieldAlias,
        EdgeType::FieldWrite,
        EdgeType::ActionReads,
        EdgeType::ActionNavigates,
        EdgeType::ActionSetsParam,
        EdgeType::ActionControlsComponent,
        EdgeType::ActionValidates,
        EdgeType::ActionLoadsData,
        EdgeType::DependsOn,
    ]
}

#[test]
fn schema_has_exactly_one_row_per_node_and_edge_type() {
    let schema = graph_schema::schema();
    let node_types = all_node_types();
    let edge_types = all_edge_types();
    assert_eq!(schema.node_types.len(), node_types.len(), "节点类型行数");
    assert_eq!(schema.edge_types.len(), edge_types.len(), "边类型行数");
    for node_type in &node_types {
        assert!(
            schema.node_row(node_type).is_some(),
            "缺少节点类型行：{}",
            variant_name(node_type)
        );
    }
    for edge_type in &edge_types {
        assert!(
            schema.edge_row(edge_type).is_some(),
            "缺少边类型行：{}",
            variant_name(edge_type)
        );
    }
}

#[test]
fn schema_rows_are_internally_consistent() {
    for row in graph_schema::schema().edge_types {
        let name = variant_name(&row.edge_type);
        if row.emitted {
            assert!(
                !row.endpoints.is_empty(),
                "{name}：会产生的边必须声明端点组合"
            );
        } else {
            assert!(row.endpoints.is_empty(), "{name}：不会产生的边不应声明端点");
            assert!(
                row.meta_keys.is_empty(),
                "{name}：不会产生的边不应声明 meta 键"
            );
        }
    }
    for row in graph_schema::schema().node_types {
        assert!(
            !row.id_prefixes.is_empty() && !row.id_formats.is_empty(),
            "{}：必须声明 id 前缀与格式",
            variant_name(&row.node_type)
        );
    }
}

/// 在独占目录里把 `project_dir` 建成 `.grafeo` 图库，返回库路径。
fn build_graph(project_dir: &Path, tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir =
        std::env::temp_dir().join(format!("graph-schema-{tag}-{nanos}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db = dir.join("g.grafeo");
    let output = Command::new(bin())
        .arg("--project-dir")
        .arg(project_dir)
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
    db
}

fn gql_rows(db: &Path, query: &str) -> Vec<Vec<Value>> {
    let output = Command::new(bin())
        .arg("--graph-db-path")
        .arg(db)
        .args(["--gql-max-rows", "100000", "--gql", query])
        .output()
        .expect("run --gql");
    let result: Value = serde_json::from_slice(&output.stdout).expect("--gql 输出 JSON");
    assert_eq!(result["truncated"], Value::Bool(false), "查询不应被截断");
    result["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("--gql 应返回 rows：{result}"))
        .iter()
        .map(|row| row.as_array().expect("row 是数组").clone())
        .collect()
}

fn meta_keys(meta: &Value) -> BTreeSet<String> {
    match meta.as_str() {
        Some(text) => {
            let parsed: Value = serde_json::from_str(text).expect("meta 是 JSON 文本");
            parsed
                .as_object()
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default()
        }
        None => BTreeSet::new(),
    }
}

/// 逐元素核对一张图，返回全部违规（空表示合规）。
fn conformance_violations(db: &Path) -> Vec<String> {
    let schema = graph_schema::schema();
    let mut violations = Vec::new();
    let mut node_types_by_id = std::collections::HashMap::new();

    for row in gql_rows(db, "MATCH (n:Node) RETURN n.id, n.node_type, n.meta") {
        let id = row[0].as_str().expect("id").to_string();
        let type_name = row[1].as_str().expect("node_type").to_string();
        let Some(node_row) = all_node_types()
            .into_iter()
            .find(|t| variant_name(t) == type_name)
            .and_then(|t| schema.node_row(&t))
        else {
            violations.push(format!("未知 node_type {type_name}（{id}）"));
            continue;
        };
        let prefix = id.split(':').next().unwrap_or_default();
        if !node_row.id_prefixes.contains(&prefix) {
            violations.push(format!("{type_name} 节点的 id 前缀不在契约内：{id}"));
        }
        let present = meta_keys(&row[2]);
        let declared: BTreeSet<&str> = node_row.meta_keys.iter().map(|k| k.name).collect();
        for key in &present {
            if !declared.contains(key.as_str()) {
                violations.push(format!("{type_name} 节点 {id} 的 meta 键未声明：{key}"));
            }
        }
        for key in node_row.meta_keys.iter().filter(|k| k.required) {
            if !present.contains(key.name) {
                violations.push(format!(
                    "{type_name} 节点 {id} 缺少必需 meta 键：{}",
                    key.name
                ));
            }
        }
        node_types_by_id.insert(id, type_name);
    }

    let edges = gql_rows(
        db,
        "MATCH (a:Node)-[r]->(b:Node) RETURN type(r), a.id, b.id, r.field_path, r.meta",
    );
    for row in edges {
        let type_name = row[0].as_str().expect("edge type").to_string();
        let (from_id, to_id) = (row[1].as_str().expect("from"), row[2].as_str().expect("to"));
        let Some(edge_row) = all_edge_types()
            .into_iter()
            .find(|t| variant_name(t) == type_name)
            .and_then(|t| schema.edge_row(&t))
        else {
            violations.push(format!("未知边类型 {type_name}（{from_id} -> {to_id}）"));
            continue;
        };
        if !edge_row.emitted {
            violations.push(format!(
                "契约声明不会产生的边出现了：{type_name}（{from_id} -> {to_id}）"
            ));
            continue;
        }
        let from_type = node_types_by_id.get(from_id).cloned().unwrap_or_default();
        let to_type = node_types_by_id.get(to_id).cloned().unwrap_or_default();
        let allowed = edge_row
            .endpoints
            .iter()
            .any(|p| variant_name(&p.from) == from_type && variant_name(&p.to) == to_type);
        if !allowed {
            violations.push(format!(
                "{type_name} 的端点组合不在契约内：{from_type} -> {to_type}（{from_id} -> {to_id}）"
            ));
        }
        let has_field_path = !row[3].is_null();
        match (edge_row.field_path, has_field_path) {
            (FieldPathUse::Never, true) => violations.push(format!(
                "{type_name}（{from_id} -> {to_id}）不该有 field_path"
            )),
            (FieldPathUse::Always, false) => violations.push(format!(
                "{type_name}（{from_id} -> {to_id}）缺少 field_path"
            )),
            _ => {}
        }
        let declared: BTreeSet<&str> = edge_row.meta_keys.iter().map(|k| k.name).collect();
        for key in meta_keys(&row[4]) {
            if !declared.contains(key.as_str()) {
                violations.push(format!(
                    "{type_name}（{from_id} -> {to_id}）的 meta 键未声明：{key}"
                ));
            }
        }
    }
    violations
}

#[test]
fn test_project_graph_conforms_to_the_schema() {
    let db = build_graph(&fixtures().join("test_project"), "test-project");
    let violations = conformance_violations(&db);
    assert_eq!(violations, Vec::<String>::new(), "夹具图应完全符合契约");
}

#[test]
fn cross_page_project_graph_conforms_to_the_schema() {
    let db = build_graph(&fixtures().join("cross_page_project"), "cross-page");
    let violations = conformance_violations(&db);
    assert_eq!(violations, Vec::<String>::new(), "跨页夹具图应完全符合契约");
}

#[test]
fn reference_doc_matches_generated_markdown() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/reference/graph-schema.md");
    let generated = graph_schema::to_markdown();
    if std::env::var("UPDATE_GRAPH_SCHEMA_DOC").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &generated).expect("write reference doc");
        return;
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        current, generated,
        "docs/reference/graph-schema.md 与契约表不一致；运行 UPDATE_GRAPH_SCHEMA_DOC=1 cargo test --features cli-local --test graph_schema_tests 重新生成"
    );
}

#[test]
fn graph_schema_command_prints_json_and_markdown_without_a_project() {
    let json_output = Command::new(bin())
        .arg("--graph-schema")
        .output()
        .expect("run --graph-schema");
    assert!(json_output.status.success());
    let value: Value = serde_json::from_slice(&json_output.stdout).expect("JSON 输出");
    assert_eq!(value["contract_version"], serde_json::json!(1));
    assert_eq!(
        value["edge_types"].as_array().map(Vec::len),
        Some(all_edge_types().len())
    );
    assert_eq!(
        value["node_types"].as_array().map(Vec::len),
        Some(all_node_types().len())
    );

    let human = Command::new(bin())
        .args(["--graph-schema", "--human"])
        .output()
        .expect("run --graph-schema --human");
    assert!(human.status.success());
    assert_eq!(
        String::from_utf8_lossy(&human.stdout),
        graph_schema::to_markdown(),
        "--human 输出应与 to_markdown 一致"
    );
}
