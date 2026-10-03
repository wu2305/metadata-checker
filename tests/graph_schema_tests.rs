//! 图 Schema 契约（`src/graph_schema.rs`）的一致性测试。
//!
//! 契约是唯一真源，这里证明：
//! 1. 表覆盖了每一个 `NodeType` / `EdgeType` 变体（新增变体不改表就编译不过 / 测试变红）；
//! 2. 夹具项目扫描出的**每一个**节点和边都符合表（id 前缀、`meta` 键与值类型、端点组合、
//!    `field_path` 约定、`emitted = false` 的边确实不出现）；有真实语料时对语料同样核对；
//! 3. 每条边的写入路径（producers）指向的文件和函数都还在，示例查询真的能跑；
//! 4. `docs/reference/graph-schema.md` 与 `--graph-schema --human` 一字不差，
//!    `--gql` 的 help 与 JSON 输出都来自同一张表。
#![cfg(feature = "grafeo-store")]

use metadata_checker::graph::{EdgeType, NodeType};
use metadata_checker::graph_schema::{self, FieldPathUse, MetaKeySchema, variant_name};
use serde_json::Value;
use std::collections::BTreeMap;
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
        assert_eq!(
            schema.node_row(node_type).is_some(),
            true,
            "缺少节点类型行：{}",
            variant_name(node_type)
        );
    }
    for edge_type in &edge_types {
        assert_eq!(
            schema.edge_row(edge_type).is_some(),
            true,
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
            assert_eq!(
                row.endpoints.is_empty(),
                false,
                "{name}：会产生的边必须声明端点组合"
            );
            assert_eq!(
                row.producers.is_empty(),
                false,
                "{name}：会产生的边必须说明由哪条写入路径、按什么规则产生"
            );
        } else {
            assert_eq!(
                row.endpoints.is_empty(),
                true,
                "{name}：不会产生的边不应声明端点"
            );
            assert_eq!(
                row.meta_keys.is_empty(),
                true,
                "{name}：不会产生的边不应声明 meta 键"
            );
            assert_eq!(
                row.producers.is_empty(),
                true,
                "{name}：不会产生的边不应声明写入路径"
            );
        }
    }
    for row in graph_schema::schema().node_types {
        let name = variant_name(&row.node_type);
        assert_eq!(
            !row.id_prefixes.is_empty() && !row.id_formats.is_empty(),
            true,
            "{name}：必须声明 id 前缀与格式"
        );
        // 没有声明任何 meta 键的节点类型不能同时声明开放 meta
        assert_eq!(
            row.meta_open && row.meta_keys.is_empty(),
            false,
            "{name}：开放 meta 至少要列出常见键"
        );
    }
}

/// 每条写入路径引用的文件与函数必须还在：函数改名或搬走时，表必须跟着改。
#[test]
fn every_producer_points_at_an_existing_function() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for row in graph_schema::schema().edge_types {
        for source in row.producers {
            let (file, function) = source
                .path
                .split_once("::")
                .unwrap_or_else(|| panic!("producer 路径应为 文件::函数：{}", source.path));
            let text = std::fs::read_to_string(src.join(file))
                .unwrap_or_else(|error| panic!("producer 文件读不到 {file}：{error}"));
            assert_eq!(
                text.contains(&format!("fn {function}")),
                true,
                "{} 的 producer {} 指向的函数不存在",
                variant_name(&row.edge_type),
                source.path
            );
            assert_eq!(
                source.rule.is_empty(),
                false,
                "{}：producer 必须写明规则",
                source.path
            );
        }
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
    assert_eq!(
        output.status.success(),
        true,
        "build-graph 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    db
}

fn gql_rows(db: &Path, query: &str) -> Vec<Vec<Value>> {
    let output = Command::new(bin())
        .arg("--graph-db-path")
        .arg(db)
        .args(["--gql-max-rows", "1000000", "--gql", query])
        .output()
        .expect("run --gql");
    let result: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "--gql 输出应为 JSON（{error}）：{} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(result["truncated"], Value::Bool(false), "查询不应被截断");
    result["rows"]
        .as_array()
        .unwrap_or_else(|| panic!("--gql 应返回 rows：{result}"))
        .iter()
        .map(|row| row.as_array().expect("row 是数组").clone())
        .collect()
}

/// `meta` 列是 JSON 文本；解析成键值对（没有 meta 时为空）。
fn meta_object(meta: &Value) -> BTreeMap<String, Value> {
    match meta.as_str() {
        Some(text) => {
            let parsed: Value = serde_json::from_str(text).expect("meta 是 JSON 文本");
            parsed
                .as_object()
                .map(|object| {
                    object
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default()
        }
        None => BTreeMap::new(),
    }
}

/// 一组 `meta` 键值对相对契约键表的违规：未声明的键（开放 meta 除外）、值类型不符、
/// 不允许 null 的键为 null。
fn meta_violations(
    owner: &str,
    declared: &[MetaKeySchema],
    open: bool,
    actual: &BTreeMap<String, Value>,
) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, value) in actual {
        let Some(key) = declared.iter().find(|candidate| candidate.name == name) else {
            if !open {
                violations.push(format!("{owner} 的 meta 键未声明：{name}"));
            }
            continue;
        };
        if value.is_null() {
            if !key.nullable {
                violations.push(format!("{owner} 的 meta 键 {name} 为 null，契约不允许"));
            }
        } else if !key.value_type.accepts(value) {
            violations.push(format!(
                "{owner} 的 meta 键 {name} 类型不符：契约 {}，实际 {value}",
                key.value_type.as_str()
            ));
        }
    }
    violations
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
            .find(|candidate| variant_name(candidate) == type_name)
            .and_then(|candidate| schema.node_row(&candidate))
        else {
            violations.push(format!("未知 node_type {type_name}（{id}）"));
            continue;
        };
        let prefix = id.split(':').next().unwrap_or_default();
        if !node_row.id_prefixes.contains(&prefix) {
            violations.push(format!("{type_name} 节点的 id 前缀不在契约内：{id}"));
        }
        let actual = meta_object(&row[2]);
        violations.extend(meta_violations(
            &format!("{type_name} 节点 {id}"),
            node_row.meta_keys,
            node_row.meta_open,
            &actual,
        ));
        for key in node_row.meta_keys.iter().filter(|key| key.required) {
            if !actual.contains_key(key.name) {
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
            .find(|candidate| variant_name(candidate) == type_name)
            .and_then(|candidate| schema.edge_row(&candidate))
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
            .any(|pair| variant_name(&pair.from) == from_type && variant_name(&pair.to) == to_type);
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
            // Sometimes：取决于端点组合，行内说明；这里不逐边判
            _ => {}
        }
        violations.extend(meta_violations(
            &format!("{type_name}（{from_id} -> {to_id}）"),
            edge_row.meta_keys,
            false,
            &meta_object(&row[4]),
        ));
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

/// 有真实语料时（`METADATA_CHECKER_REAL_PROJECT_DIR`）同样核对：夹具覆盖不到的写入分支
/// （例如只在真实项目里出现的 meta 键、端点组合）只有真实语料才能暴露。
/// 没有语料的环境跳过，不算验收通过的证据。
#[test]
fn real_project_graph_conforms_to_the_schema_when_available() {
    let Some(dir) = std::env::var_os("METADATA_CHECKER_REAL_PROJECT_DIR") else {
        eprintln!("CORPUS_UNAVAILABLE: METADATA_CHECKER_REAL_PROJECT_DIR 未设置，跳过真实语料核对");
        return;
    };
    let db = build_graph(Path::new(&dir), "real-project");
    let violations = conformance_violations(&db);
    assert_eq!(violations, Vec::<String>::new(), "真实语料图应完全符合契约");
}

/// 夹具图里没有的写入分支：带 `exp` 引用别的模型字段的 `.tbl` 维度，其字段节点的
/// `meta` 会带 `source_expr` / `source_expr_models`，契约必须声明。
#[test]
fn computed_dimension_meta_keys_are_declared() {
    use metadata_checker::graph_store::GraphReadStore;
    use metadata_checker::memory_graph_store::MemoryGraphStore;
    use metadata_checker::scanner::process_tbl_file_from_string;

    let table = serde_json::json!({
        "properties": {"dbTableName": "fact_orders"},
        "dimensions": [
            {"name": "plain", "dbfield": "plain", "dataType": "varchar", "isDimension": true, "length": 10},
            {"name": "total", "dbfield": "total", "dataType": "number", "isDimension": true,
             "exp": "=model1.amount + 1", "inputField": "amount"}
        ]
    });
    let mut store = MemoryGraphStore::new();
    process_tbl_file_from_string(&mut store, "app/fact_orders.tbl", &table.to_string())
        .expect("scan tbl");
    let field_row = graph_schema::schema()
        .node_row(&NodeType::Field)
        .expect("Field 行");
    let mut seen_models_key = false;
    for node in store.iter_nodes().expect("iter nodes") {
        if node.node_type != NodeType::Field {
            continue;
        }
        let Some(meta) = node.meta.as_ref().and_then(Value::as_object) else {
            continue;
        };
        let actual: BTreeMap<String, Value> = meta
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        seen_models_key |= actual.contains_key("source_expr_models");
        assert_eq!(
            meta_violations(
                &format!("Field 节点 {}", node.id),
                field_row.meta_keys,
                field_row.meta_open,
                &actual
            ),
            Vec::<String>::new()
        );
    }
    assert_eq!(
        seen_models_key, true,
        "这个夹具应当走到 source_expr_models 分支"
    );
}

/// 契约里写明的「缺失」与边界形态必须和扫描器的真实产出一致：
/// 裸模型引用的 field_path、非 `user` 命名空间的用户属性 id、被丢弃的 Condition 属主边、
/// 指向不存在组件的显隐控制、以及没有 canvas 的已扫描页面。
#[test]
fn documented_boundary_cases_match_the_scanner() {
    let project = std::env::temp_dir().join(format!(
        "graph-schema-boundary-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&project).expect("create project dir");
    let page = serde_json::json!({
        "version": "1",
        "params": [],
        "sources": [
            {"id": "model1", "modelType": "dwtable", "path": "data/table1.tbl",
             "filter": {"scope": "self", "matchAll": true, "clauses": [{"exp": "model1.status = 'active'"}]}},
            {"id": "model2", "modelType": "dwtable", "path": "data/table2.tbl"}
        ],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "btn", "type": "button",
             "actions": [{"id": "a1", "type": "showComponent", "target": "ghost"}]},
            {"id": "txt", "type": "input", "value": "=$project.name"},
            {"id": "txt2", "type": "input", "value": "${model2}"}
        ]}
    });
    std::fs::write(project.join("a.spg"), page.to_string()).expect("write a.spg");
    // 指向绝对路径的 link 解析不了：没有边、没有 Page 节点，缺口记在 scanner_entry 里
    let linking_page = serde_json::json!({
        "referenceResources": ["/sysdata/公共/页.spg"],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "btn", "type": "button", "actions": [
                {"id": "go", "actionType": "link", "triggerType": "click",
                 "targetType": "app", "path": 0}
            ]}
        ]}
    });
    std::fs::write(project.join("linking.spg"), linking_page.to_string())
        .expect("write linking.spg");
    // dbTableName 为空的 dataflow 是未落表的：Model 上 landed 为 false，且没有 OutputsTo
    let unlanded_flow = serde_json::json!({
        "properties": {"dbTableName": ""},
        "dataFlow": {"nodes": {}},
        "dimensions": []
    });
    std::fs::write(project.join("flow.tbl"), unlanded_flow.to_string()).expect("write flow.tbl");
    std::fs::write(
        project.join("nocanvas.spg"),
        serde_json::json!({"version": "1", "params": [], "sources": []}).to_string(),
    )
    .expect("write nocanvas.spg");
    let db = build_graph(&project, "boundary");

    // 裸 `${model2}` 只产出模型级 Reads，field_path 就是模型名，没有 `.<field>`
    let reads = gql_rows(
        &db,
        "MATCH (a:Node)-[r:Reads]->(b:Node) WHERE a.id = 'comp:a.spg|txt2' RETURN b.id, r.field_path",
    );
    assert_eq!(
        reads.contains(&vec![
            Value::String("model:model2".into()),
            Value::String("model2".into())
        ]),
        true,
        "裸模型引用应得到 field_path = 模型名：{reads:?}"
    );
    // `$project.name` 的命名空间不是 user，id 仍是 user:<namespace>.<name>
    let users = gql_rows(
        &db,
        "MATCH (a:Node)-[r:DependsOn]->(b:Node) WHERE a.id = 'comp:a.spg|txt' AND b.id STARTS WITH 'user:' RETURN b.id",
    );
    assert_eq!(users, vec![vec![Value::String("user:project.name".into())]]);
    // 没有别处引用的 dwtable 源：Condition 节点在，但属主边（连到 model:model1）被丢弃
    let owner_edges = gql_rows(
        &db,
        "MATCH (c:Node)-[r:DependsOn]->(m:Node) WHERE m.id = 'model:model1' RETURN c.id",
    );
    assert_eq!(owner_edges, Vec::<Vec<Value>>::new());
    let filter_conditions = gql_rows(
        &db,
        "MATCH (c:Node) WHERE c.id = 'cond:a.spg|model1#filter#0#exp' RETURN c.id",
    );
    assert_eq!(filter_conditions.len(), 1, "Condition 节点本身仍在");
    // 指向不存在组件的 showComponent 没有 ActionControlsComponent 边
    let controls = gql_rows(
        &db,
        "MATCH (a:Node)-[r:ActionControlsComponent]->(b:Node) RETURN a.id",
    );
    assert_eq!(controls, Vec::<Vec<Value>>::new());
    // 没有 canvas 的已扫描页面：没有 Contains 边，但有 file_state 记录
    let contains = gql_rows(
        &db,
        "MATCH (p:Node)-[r:Contains]->(c:Node) WHERE p.id = 'page:nocanvas.spg' RETURN c.id",
    );
    assert_eq!(contains, Vec::<Vec<Value>>::new());
    let file_state = gql_rows(
        &db,
        "MATCH (s:IndexState) WHERE s.key = 'file_state:nocanvas.spg' RETURN s.key",
    );
    assert_eq!(file_state.len(), 1, "已扫描的页面有 file_state 记录");
    // 解析不了的 link：没有 Page 节点，scanner_entry 里有一条 unresolved_reference
    let ghost_pages = gql_rows(
        &db,
        "MATCH (p:Node) WHERE p.node_type = 'Page' AND p.path CONTAINS 'sysdata' RETURN p.id",
    );
    assert_eq!(ghost_pages, Vec::<Vec<Value>>::new());
    let entry = gql_rows(
        &db,
        "MATCH (s:IndexState) WHERE s.key = 'scanner_entry:linking.spg' RETURN s.value",
    );
    let entry_json: Value = serde_json::from_str(
        entry[0][0]
            .as_str()
            .expect("scanner_entry 的 value 是 JSON 文本"),
    )
    .expect("scanner_entry 的 value 可解析");
    assert_eq!(entry_json["unresolved_reference"], Value::from(1));
    // 契约里写明的 occurrences 形态：code / location{source_file,node_id,json_path} / detail
    let occurrence = &entry_json["occurrences"][0];
    assert_eq!(
        occurrence["code"],
        Value::String("SCANNER_UNRESOLVED_REFERENCE".into())
    );
    assert_eq!(occurrence["location"]["source_file"].is_string(), true);
    assert_eq!(occurrence["location"]["json_path"].is_string(), true);
    assert_eq!(occurrence["detail"].is_string(), true);
    // 契约里列出的索引记录 key 形态都真实存在（file_state 在上面已查过）
    for record in graph_schema::schema().labels.internal_records {
        let prefix = record.key_form.split('<').next().expect("key 前缀");
        let rows = gql_rows(
            &db,
            &format!("MATCH (s:IndexState) WHERE s.key STARTS WITH '{prefix}' RETURN s.key"),
        );
        assert_eq!(rows.is_empty(), false, "没有 {prefix} 记录");
    }
    // 未落表 dataflow：landed = false（Bool），没有 OutputsTo
    let unlanded = gql_rows(
        &db,
        "MATCH (m:Node) WHERE m.node_type = 'Model' AND m.meta CONTAINS 'landed' RETURN m.id, m.meta",
    );
    assert_eq!(
        unlanded.len(),
        1,
        "只有未落表的 dataflow 带 landed：{unlanded:?}"
    );
    let landed_meta: Value =
        serde_json::from_str(unlanded[0][1].as_str().expect("meta 是 JSON 文本")).expect("meta");
    assert_eq!(landed_meta["landed"], Value::Bool(false));
    // 整张边界图（含 landed）也要符合契约表：表里删掉 landed 时这里必须失败
    assert_eq!(conformance_violations(&db), Vec::<String>::new());
    std::fs::remove_dir_all(&project).expect("remove project dir");
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
    assert_eq!(json_output.status.success(), true);
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
    // 类型、写入路径、示例、标签都在机器可读输出里，不只在 Markdown 里
    assert_eq!(
        value["node_types"][2]["meta_keys"][0]["value_type"],
        serde_json::json!("string")
    );
    assert_eq!(
        value["examples"].as_array().map(Vec::len),
        Some(graph_schema::schema().examples.len())
    );
    assert_eq!(
        value["labels"]["project_node"],
        serde_json::json!(graph_schema::schema().labels.project_node)
    );
    let producers_everywhere = value["edge_types"]
        .as_array()
        .expect("edge_types")
        .iter()
        .filter(|row| row["emitted"] == serde_json::json!(true))
        .all(|row| row["producers"].as_array().is_some_and(|p| !p.is_empty()));
    assert_eq!(producers_everywhere, true);

    // `--human` 与 `--interactive`（它的别名）都输出 Markdown
    for flag in ["--human", "--interactive"] {
        let human = Command::new(bin())
            .args(["--graph-schema", flag])
            .output()
            .expect("run --graph-schema in human mode");
        assert_eq!(human.status.success(), true, "{flag}");
        assert_eq!(
            String::from_utf8_lossy(&human.stdout),
            graph_schema::to_markdown(),
            "{flag} 输出应与 to_markdown 一致"
        );
    }
}

/// `--gql` 的 help 里的示例、标签与属性都来自契约表，不是手写副本。
#[test]
fn gql_help_is_generated_from_the_schema() {
    let help = graph_schema::gql_long_help();
    let schema = graph_schema::schema();
    for example in schema.examples {
        assert_eq!(
            help.contains(example.gql),
            true,
            "help 缺示例：{}",
            example.gql
        );
    }
    for property in schema.labels.node_properties {
        assert_eq!(help.contains(property), true, "help 缺节点属性：{property}");
    }
    for property in schema.labels.edge_properties {
        assert_eq!(help.contains(property), true, "help 缺边属性：{property}");
    }
    for record in schema.labels.internal_records {
        assert_eq!(
            help.contains(record.key_form),
            true,
            "help 缺索引记录：{}",
            record.key_form
        );
    }
}

/// 契约里的示例查询必须真能在夹具图上跑通（不要求有结果，但不能报错）。
#[test]
fn every_example_query_runs_on_a_fixture_graph() {
    let db = build_graph(&fixtures().join("test_project"), "examples");
    for example in graph_schema::schema().examples {
        let output = Command::new(bin())
            .arg("--graph-db-path")
            .arg(&db)
            .args(["--gql-max-rows", "100", "--gql", example.gql])
            .output()
            .expect("run example");
        assert_eq!(
            output.status.success(),
            true,
            "示例「{}」执行失败：{}",
            example.title,
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).expect("示例输出 JSON");
        assert_eq!(
            result["rows"].is_array(),
            true,
            "示例「{}」没有返回 rows：{result}",
            example.title
        );
    }
}
