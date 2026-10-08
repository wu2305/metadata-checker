//! 写入校验（计划步 L0，spec S2）：扫描器写图时对照 `graph_schema.rs` 契约表拒收违规项。
//!
//! 三类违规各有拒收测试：
//! 1. 边的端点节点类型不是该边类型登记的组合；
//! 2. 节点或边缺契约声明的必需 `meta` 键；
//! 3. 节点类型或边类型没有在契约表里登记。
//!
//! 每类都用两种方式验证：对生产契约表直接构造畸形写入（证明真实的表会拦），以及把
//! 契约表改成「少一行 / 多一个必需键」后扫描一个正常页面（证明整条扫描路径都经过校验、
//! 且错误带着文件、元素与原因一路传回）。夹具项目的扫描零违规也在这里直接验证。
use metadata_checker::graph::{Edge, EdgeType, Node, NodeType};
use metadata_checker::graph_schema::{
    self, EdgeTypeSchema, GraphSchema, MetaKeySchema, MetaValueType, NodeTypeSchema,
};
use metadata_checker::graph_store::{GraphReadStore, GraphStoreError, GraphWriteStore};
use metadata_checker::graph_write_guard::SchemaGuard;
use metadata_checker::memory_graph_store::MemoryGraphStore;
use metadata_checker::scanner::{
    PageIdentityMode, process_spg_file_from_value, process_spg_file_with_schema,
    process_tbl_file_from_string, process_tbl_file_with_schema,
};
use std::path::{Path, PathBuf};

const PAGE: &str = "app/a.spg";

/// 一个会产生 Page / Component / Action 三类节点和 Contains / Triggers 边的最小页面。
fn simple_page() -> serde_json::Value {
    serde_json::json!({
        "version": "1",
        "params": [],
        "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "btn", "type": "button",
             "actions": [{"id": "a1", "type": "showComponent", "target": "ghost"}]}
        ]}
    })
}

fn simple_table() -> String {
    serde_json::json!({
        "properties": {"dbTableName": "fact_orders"},
        "dimensions": [
            {"name": "plain", "dbfield": "plain", "dataType": "varchar", "isDimension": true}
        ]
    })
    .to_string()
}

/// 以生产契约表为底，改出一张测试用的表。泄漏的内存只存活到测试进程结束。
fn schema_with(
    edit: impl FnOnce(&mut Vec<NodeTypeSchema>, &mut Vec<EdgeTypeSchema>),
) -> &'static GraphSchema {
    let base = graph_schema::schema();
    let mut nodes = base.node_types.to_vec();
    let mut edges = base.edge_types.to_vec();
    edit(&mut nodes, &mut edges);
    Box::leak(Box::new(GraphSchema {
        node_types: Box::leak(nodes.into_boxed_slice()),
        edge_types: Box::leak(edges.into_boxed_slice()),
        ..*base
    }))
}

/// 给一组键末尾追加一个必需键（契约里本来没有，扫描器必然不会写）。
fn with_extra_required_key(keys: &'static [MetaKeySchema]) -> &'static [MetaKeySchema] {
    let mut extended = keys.to_vec();
    extended.push(MetaKeySchema {
        name: "l0_required_probe",
        value_type: MetaValueType::String,
        nullable: false,
        required: true,
        description: "测试用必需键",
    });
    Box::leak(extended.into_boxed_slice())
}

fn node(id: &str, node_type: NodeType, meta: Option<serde_json::Value>) -> Node {
    Node {
        id: id.to_string(),
        node_type,
        path: PAGE.to_string(),
        name: id.to_string(),
        meta,
        origin_file: None,
    }
}

fn edge(from: &str, to: &str, edge_type: EdgeType) -> Edge {
    Edge {
        from: from.to_string(),
        to: to.to_string(),
        edge_type,
        field_path: None,
        meta: None,
        origin_file: None,
    }
}

fn violation_message(error: GraphStoreError) -> String {
    match error {
        GraphStoreError::SchemaViolation { message } => message,
        other => panic!("应为 SchemaViolation，实际 {other}"),
    }
}

fn scan_spg(schema: &'static GraphSchema) -> (MemoryGraphStore, anyhow::Result<Vec<String>>) {
    let mut store = MemoryGraphStore::new();
    let result = process_spg_file_with_schema(
        &mut store,
        PAGE,
        simple_page(),
        PageIdentityMode::LegacyGlobal,
        schema,
    );
    (store, result)
}

/// 错误链全文（含各层 context）：扫描调用方看到的就是这段。
fn chain(error: &anyhow::Error) -> String {
    format!("{error:#}")
}

// ------------------------------------------------------------ (a) 端点类型不符

/// 生产契约表：`Contains` 只允许 Page→Component、Component→Component、Model→Field，
/// Model→Page 这种畸形边必须被拒，且拒收原因点明文件、边与两端类型，边不落库。
#[test]
fn edge_with_unregistered_endpoint_types_is_rejected() {
    let mut store = MemoryGraphStore::new();
    let mut guard = SchemaGuard::new(&mut store, "app/m.tbl");
    guard
        .upsert_node(node("model:m", NodeType::Model, None))
        .expect("Model 节点合规");
    guard
        .upsert_node(node("page:app/a.spg", NodeType::Page, None))
        .expect("Page 节点合规");
    let message = violation_message(
        guard
            .add_edge(edge("model:m", "page:app/a.spg", EdgeType::Contains))
            .expect_err("Model -> Page 的 Contains 不在契约内"),
    );
    for expected in [
        "app/m.tbl",
        "Contains",
        "model:m -> page:app/a.spg",
        "端点组合不在契约内",
        "Model -> Page",
    ] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
    assert_eq!(store.edge_count().expect("edge_count"), 0, "违规边不得落库");
}

/// 端点类型取自底层存储里早已存在的节点（增量扫描时边可以指向别的文件写下的节点）：
/// 本守卫没写过这两个节点，仍要查出类型并拒收。
#[test]
fn endpoint_types_of_nodes_written_earlier_are_resolved_from_the_store() {
    let mut store = MemoryGraphStore::new();
    store
        .upsert_node(node("model:m", NodeType::Model, None))
        .expect("预置 Model");
    store
        .upsert_node(node("page:app/a.spg", NodeType::Page, None))
        .expect("预置 Page");
    let mut guard = SchemaGuard::new(&mut store, "app/b.spg");
    let error = guard
        .add_edge(edge("model:m", "page:app/a.spg", EdgeType::Contains))
        .expect_err("预置节点的类型也要核对");
    assert_eq!(
        violation_message(error).contains("Model -> Page"),
        true,
        "应点明两端类型"
    );
}

/// 整条扫描路径：把 `Contains` 的合法端点收窄到只剩 Model→Field，扫描正常页面必须失败，
/// 错误链带着文件、边与原因，而不是静默丢边后「成功」。
#[test]
fn scan_fails_when_an_edge_endpoint_combination_is_not_registered() {
    let schema = schema_with(|_, edges| {
        let row = edges
            .iter_mut()
            .find(|row| row.edge_type == EdgeType::Contains)
            .expect("Contains 行");
        row.endpoints = &[];
    });
    let (_, result) = scan_spg(schema);
    let message = chain(&result.expect_err("端点组合未登记时扫描必须报错"));
    for expected in [PAGE, "Contains", "端点组合不在契约内", "Page -> Component"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

// ------------------------------------------------------------ id 文法

/// 每个 id 家族带空段（或段数不对）的 id 都被拒：前缀合法不够，还要符合 `id_formats`。
/// 缺 id 的组件 / 动作会拼出 `comp:page.spg|` 这类 id，并让多个缺 id 的元素塌成同一个节点。
#[test]
fn node_ids_with_an_empty_or_miscounted_segment_are_rejected() {
    let bad_ids = [
        (NodeType::Page, "page:"),
        (NodeType::Component, "comp:app/a.spg|"),
        (NodeType::Component, "comp:|btn"),
        (NodeType::Component, "comp:app/a.spg"),
        (NodeType::Component, "comp:app/a.spg|btn|extra"),
        (NodeType::Action, "action:app/a.spg|btn|"),
        (NodeType::Action, "action:app/a.spg||a1"),
        (NodeType::Action, "action:|btn|a1"),
        (NodeType::Action, "action:app/a.spg|btn"),
        (NodeType::Model, "model:"),
        (NodeType::Model, "model:app/a.spg|"),
        (NodeType::Model, "model:|m"),
        (NodeType::Model, "model:a|b|c"),
        (NodeType::Field, "field:"),
        (NodeType::Field, "field:|m.f"),
        (NodeType::Field, "field:app/a.spg|"),
        (NodeType::Field, "param:app/a.spg|"),
        // 内部结构：field 要 `<模型>.<字段>`，param 要 `|` 或 `/`，user 要 `<命名空间>.<名字>`
        (NodeType::Field, "field:orders."),
        (NodeType::Field, "field:.total"),
        (NodeType::Field, "field:orders"),
        (NodeType::Field, "field:app/a.spg|flow1."),
        (NodeType::Field, "field:app/a.spg|flow1"),
        (NodeType::Field, "param:name"),
        (NodeType::Field, "param:page/"),
        (NodeType::Field, "user:name"),
        (NodeType::Field, "user:project."),
        (NodeType::Field, "user:.name"),
        (NodeType::Field, "param:|name"),
        (NodeType::Field, "user:"),
        (NodeType::Field, "system:"),
        (NodeType::Condition, "cond:app/a.spg|"),
        (NodeType::Condition, "cond:app/a.spg|btn#exp"),
        (NodeType::Condition, "cond:app/a.spg|#exp#0"),
        (NodeType::Condition, "cond:app/a.spg|btn##0"),
        (NodeType::Condition, "cond:app/a.spg|btn#exp#0#x#y"),
        // 含 `#` 的属主也要认固定后缀：后缀形态错、序号非数字、属主为空仍然拒收
        (NodeType::Condition, "cond:app/a.spg|m#1#filter#x#exp"),
        (NodeType::Condition, "cond:app/a.spg|#filter#0#exp"),
        (NodeType::Condition, "cond:app/a.spg|m#1#other#0#exp"),
        (NodeType::Condition, "cond:app/a.spg|btn#a1#nope"),
        (NodeType::Condition, "cond:app/a.spg|btn#a1#"),
        (NodeType::Condition, "cond:app/a.spg|#a1#conditionExp"),
    ];
    for (node_type, id) in bad_ids {
        let violations = graph_schema::node_violations(
            graph_schema::schema(),
            &node(id, node_type.clone(), None),
            true,
        );
        assert_eq!(
            violations
                .iter()
                .any(|v| v.contains("id 不符合契约文法") && v.contains(id)),
            true,
            "{id} 应因 id 文法被拒：{violations:?}"
        );
    }
}

/// 合规 id（各家族的全部登记形态）不被文法检查误拒。
#[test]
fn well_formed_node_ids_pass_the_grammar_check() {
    let good_ids = [
        (NodeType::Page, "page:app/a.spg"),
        (NodeType::Component, "comp:app/a.spg|btn"),
        (NodeType::Action, "action:app/a.spg|btn|a1"),
        (NodeType::Model, "model:orders"),
        (NodeType::Model, "model:app/a.spg|flow1"),
        (NodeType::Field, "field:orders.total"),
        (NodeType::Field, "field:orders.*"),
        (NodeType::Field, "field:app/a.spg|flow1.x"),
        (NodeType::Field, "param:app/a.spg|p1"),
        (NodeType::Field, "param:page/p1"),
        (NodeType::Field, "user:project.name"),
        (NodeType::Field, "system:time"),
        // 模型名本身可以带点：只要求某个 `.` 两侧都非空
        (NodeType::Field, "field:a.b.c"),
        (NodeType::Field, "user:user.id"),
        (NodeType::Condition, "cond:app/a.spg|btn#exp#0"),
        (NodeType::Condition, "cond:app/a.spg|btn#a1#conditionExp"),
        (NodeType::Condition, "cond:app/a.spg|flow1#filter#0#exp"),
        (NodeType::Condition, "cond:app/a.spg|flow1#filter#2#clause"),
        // source / 组件 / 动作 id 本身含 `#`
        (NodeType::Condition, "cond:app/a.spg|m#1#filter#0#exp"),
        (NodeType::Condition, "cond:app/a.spg|b#1#a#1#conditionExp"),
        (NodeType::Condition, "cond:app/a.spg|b#1#a#1#condition"),
        (NodeType::Condition, "cond:app/a.spg|b#1#visibleCondition#0"),
    ];
    for (node_type, id) in good_ids {
        let violations =
            graph_schema::node_violations(graph_schema::schema(), &node(id, node_type, None), true);
        assert_eq!(
            violations.iter().any(|v| v.contains("id 不符合契约文法")),
            false,
            "{id} 不该被文法检查拒绝：{violations:?}"
        );
    }
}

/// 整条扫描路径：SPG 里省略动作 id（解析时默认空串）会拼出 `action:page|comp|`，扫描必须报错，
/// 错误点明文件、节点 id 与「含空段」，而不是写出这个 id 并让同一组件下缺 id 的动作塌成一个节点。
#[test]
fn scan_rejects_an_action_without_an_id() {
    let page = serde_json::json!({
        "version": "1", "params": [], "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "btn", "type": "button", "actions": [{"type": "showComponent", "target": "x"}]}
        ]}
    });
    let mut store = MemoryGraphStore::new();
    let error = process_spg_file_from_value(&mut store, PAGE, page)
        .expect_err("缺 id 的动作必须让扫描报错");
    let message = chain(&error);
    for expected in [PAGE, "action:app/a.spg|btn|", "id 不符合契约文法", "含空段"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

/// 缺 id 的组件不会走到写入口：`superpage` 解析阶段就跳过它（子组件上提给父级），所以扫描
/// 产不出 `comp:page|`；这里钉住该行为，免得上游改了之后空段 id 悄悄进图（即便进了，
/// 写入校验也会拒收，见 `node_ids_with_an_empty_or_miscounted_segment_are_rejected`）。
#[test]
fn scan_never_emits_a_component_node_without_an_id() {
    let page = serde_json::json!({
        "version": "1", "params": [], "sources": [],
        "canvas": {"id": "canvas", "type": "canvas", "components": [{"type": "button"}]}
    });
    let mut store = MemoryGraphStore::new();
    let ids =
        process_spg_file_from_value(&mut store, PAGE, page).expect("缺 id 的组件被解析阶段跳过");
    assert_eq!(ids.iter().any(|id| id.ends_with('|')), false, "{ids:?}");
}

/// `.tbl` 里 `name` 为空的维度：`validate_table_identities` 认为 `<模型>.` 是非空局部名而放行，
/// 此前会写出 `field:orders.`，所有无名维度塌成同一个节点。现在写入校验拒收，错误点明
/// 文件、节点 id 与原因。
#[test]
fn tbl_dimension_with_an_empty_name_is_rejected() {
    let table = serde_json::json!({
        "properties": {"dbTableName": "fact_orders"},
        "dimensions": [
            {"name": "plain", "dbfield": "plain", "dataType": "varchar"},
            {"name": "", "dbfield": "x", "dataType": "varchar"},
            {"name": "", "dbfield": "y", "dataType": "varchar"}
        ]
    })
    .to_string();
    let mut store = MemoryGraphStore::new();
    let error = process_tbl_file_from_string(&mut store, "app/orders.tbl", &table)
        .expect_err("无名维度必须让扫描报错");
    let message = chain(&error);
    for expected in [
        "app/orders.tbl",
        "field:orders.",
        "id 不符合契约文法",
        "`<模型>.<字段>`",
    ] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

/// SPG 的 source id / 组件 id 本身可以含 `#`（与条件 id 的分隔符同一个字符）。条件 id 是
/// `<属主>#filter#<n>#exp` 之类的固定后缀接在属主之后，所以必须从右往左认后缀，属主部分
/// 不设限；按 `#` 切成固定段数会把这种合法页面整个拒掉、让扫描中止。
#[test]
fn scan_accepts_hash_in_source_and_component_ids() {
    let page = serde_json::json!({
        "version": "1", "params": [],
        "sources": [
            {"id": "m#1", "modelType": "dwtable", "path": "data/t.tbl",
             "filter": {"scope": "self", "matchAll": true,
                        "clauses": [{"exp": "m#1.status = 'active'"}]}}
        ],
        "canvas": {"id": "canvas", "type": "canvas", "components": [
            {"id": "b#1", "type": "button", "visibleCondition": "=1 = 1",
             "actions": [{"id": "a#1", "type": "showComponent", "target": "x",
                          "conditionExp": "=1 = 1"}]}
        ]}
    });
    let mut store = MemoryGraphStore::new();
    let ids = process_spg_file_from_value(&mut store, PAGE, page)
        .unwrap_or_else(|error| panic!("含 # 的 id 是合法的：{error:#}"));
    for expected in [
        "cond:app/a.spg|m#1#filter#0#exp",
        "cond:app/a.spg|b#1#a#1#conditionExp",
    ] {
        assert_eq!(
            ids.iter().any(|id| id == expected),
            true,
            "缺少 {expected}：{ids:?}"
        );
    }
}

// ------------------------------------------------------------ (b) 缺必需 meta

/// 生产契约表：Action 的 `triggerType` / `condition` / `conditionExp` / `waitPrev` 都是
/// 必需键。缺键的 Action 被拒，并逐个点明缺哪个键；节点不落库。
#[test]
fn node_missing_required_meta_keys_is_rejected() {
    let mut store = MemoryGraphStore::new();
    let mut guard = SchemaGuard::new(&mut store, PAGE);
    let message = violation_message(
        guard
            .upsert_node(node(
                "action:app/a.spg|btn|a1",
                NodeType::Action,
                Some(serde_json::json!({"triggerType": "click"})),
            ))
            .expect_err("缺 condition / conditionExp / waitPrev"),
    );
    for expected in [
        PAGE,
        "action:app/a.spg|btn|a1",
        "缺少必需 meta 键：condition",
        "缺少必需 meta 键：conditionExp",
        "缺少必需 meta 键：waitPrev",
    ] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
    assert_eq!(
        store
            .get_node("action:app/a.spg|btn|a1")
            .expect("get_node")
            .is_none(),
        true,
        "违规节点不得落库"
    );
}

/// 没带 meta 的写入：节点已存在时存储沿用既有 meta（占位后补定义的常见顺序），不算缺键；
/// 节点还不存在时就是一个没有必需键的新节点，必须拒收。
#[test]
fn write_without_meta_is_accepted_only_for_an_existing_node() {
    let full_meta = serde_json::json!({
        "triggerType": "click", "condition": null, "conditionExp": null, "waitPrev": null
    });
    let mut store = MemoryGraphStore::new();
    let mut guard = SchemaGuard::new(&mut store, PAGE);
    guard
        .upsert_node(node("action:app/a.spg|btn|a1", NodeType::Action, None))
        .expect_err("新 Action 节点没有 meta：缺必需键");
    guard
        .upsert_node(node(
            "action:app/a.spg|btn|a1",
            NodeType::Action,
            Some(full_meta),
        ))
        .expect("带齐必需键");
    guard
        .upsert_node(node("action:app/a.spg|btn|a1", NodeType::Action, None))
        .expect("已存在的节点，无 meta 写入沿用既有 meta");
}

/// meta 键存在但值违反契约（类型不符、不允许 null）同样拒收。
#[test]
fn node_meta_with_wrong_value_type_is_rejected() {
    let mut store = MemoryGraphStore::new();
    let mut guard = SchemaGuard::new(&mut store, PAGE);
    let message = violation_message(
        guard
            .upsert_node(node(
                "action:app/a.spg|btn|a1",
                NodeType::Action,
                Some(serde_json::json!({
                    "triggerType": null, "condition": null, "conditionExp": null, "waitPrev": null
                })),
            ))
            .expect_err("triggerType 不允许 null"),
    );
    assert_eq!(
        message.contains("meta 键 triggerType 为 null"),
        true,
        "{message}"
    );
}

/// 整条扫描路径：给 Action 行加一个必需键，扫描正常页面必须失败并点明缺的键。
#[test]
fn scan_fails_when_a_node_lacks_a_required_meta_key() {
    let schema = schema_with(|nodes, _| {
        let row = nodes
            .iter_mut()
            .find(|row| row.node_type == NodeType::Action)
            .expect("Action 行");
        row.meta_keys = with_extra_required_key(row.meta_keys);
    });
    let (_, result) = scan_spg(schema);
    let message = chain(&result.expect_err("缺必需 meta 键时扫描必须报错"));
    for expected in [PAGE, "action:app/a.spg|btn|a1", "l0_required_probe"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

/// 边同样有必需键：给 `Triggers` 行加一个必需键，扫描必须失败并点明是哪条边缺什么。
#[test]
fn scan_fails_when_an_edge_lacks_a_required_meta_key() {
    let schema = schema_with(|_, edges| {
        let row = edges
            .iter_mut()
            .find(|row| row.edge_type == EdgeType::Triggers)
            .expect("Triggers 行");
        row.meta_keys = with_extra_required_key(row.meta_keys);
    });
    let (_, result) = scan_spg(schema);
    let message = chain(&result.expect_err("边缺必需 meta 键时扫描必须报错"));
    for expected in [PAGE, "Triggers", "l0_required_probe"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

// ------------------------------------------------------------ (c) 未登记的类型

/// 契约表里没有 Action 这一行：Action 节点被拒，错误点明「节点类型未登记」。
#[test]
fn scan_fails_on_an_unregistered_node_type() {
    let schema = schema_with(|nodes, _| nodes.retain(|row| row.node_type != NodeType::Action));
    let (store, result) = scan_spg(schema);
    let message = chain(&result.expect_err("节点类型未登记时扫描必须报错"));
    for expected in [PAGE, "节点类型未登记：Action", "action:app/a.spg|btn|a1"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
    assert_eq!(
        store
            .get_node("action:app/a.spg|btn|a1")
            .expect("get_node")
            .is_none(),
        true,
        "未登记类型的节点不得落库"
    );
}

/// 契约表里没有 Triggers 这一行：该边被拒，错误点明「边类型未登记」与边的两端。
#[test]
fn scan_fails_on_an_unregistered_edge_type() {
    let schema = schema_with(|_, edges| edges.retain(|row| row.edge_type != EdgeType::Triggers));
    let (_, result) = scan_spg(schema);
    let message = chain(&result.expect_err("边类型未登记时扫描必须报错"));
    for expected in [
        PAGE,
        "边类型未登记：Triggers",
        "comp:app/a.spg|btn -> action:app/a.spg|btn|a1",
    ] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

/// `.tbl` 路径同样经过校验（两个扫描入口都装了守卫）。
#[test]
fn tbl_scan_fails_on_an_unregistered_node_type() {
    let schema = schema_with(|nodes, _| nodes.retain(|row| row.node_type != NodeType::Field));
    let mut store = MemoryGraphStore::new();
    let error =
        process_tbl_file_with_schema(&mut store, "app/fact_orders.tbl", &simple_table(), schema)
            .expect_err("Field 未登记时 .tbl 扫描必须报错");
    let message = chain(&error);
    for expected in ["app/fact_orders.tbl", "节点类型未登记：Field"] {
        assert_eq!(
            message.contains(expected),
            true,
            "缺少 {expected}：{message}"
        );
    }
}

/// 错误必须能被调用方程序化识别（不是只有一段文字）：根因是 `GraphStoreError::SchemaViolation`。
#[test]
fn rejection_is_a_typed_error_with_a_stable_code() {
    let schema = schema_with(|nodes, _| nodes.retain(|row| row.node_type != NodeType::Action));
    let (_, result) = scan_spg(schema);
    let error = result.expect_err("应报错");
    let root = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<GraphStoreError>())
        .expect("错误链里应有 GraphStoreError");
    assert_eq!(root.code(), "GRAPH_SCHEMA_VIOLATION");
}

// ------------------------------------------------------------ 合规路径不受影响

/// 生产契约表下，夹具项目里的每个 `.spg` / `.tbl` 都能写入且零违规。
/// （夹具图经 CLI 建库后整图合规，另见 `graph_schema_tests.rs`。）
#[test]
fn fixture_projects_scan_without_violations() {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read_dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                collect(&path, out);
            } else if path
                .extension()
                .is_some_and(|ext| ext == "spg" || ext == "tbl")
            {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    for project in ["test_project", "cross_page_project"] {
        collect(&root.join(project), &mut files);
    }
    assert_eq!(files.is_empty(), false, "夹具里应有 .spg / .tbl");
    let mut store = MemoryGraphStore::new();
    for path in files {
        let logical = path
            .strip_prefix(&root)
            .expect("夹具路径")
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path).expect("read fixture");
        if logical.ends_with(".spg") {
            let value: serde_json::Value = serde_json::from_str(&text).expect("fixture json");
            process_spg_file_from_value(&mut store, &logical, value)
                .unwrap_or_else(|error| panic!("{logical} 应零违规：{error:#}"));
        } else {
            process_tbl_file_from_string(&mut store, &logical, &text)
                .unwrap_or_else(|error| panic!("{logical} 应零违规：{error:#}"));
        }
    }
}

// ------------------------------------------------------------ node_type_of 契约

/// 各存储的 `node_type_of`：写入后可查，未知 id 与删除后都是 `None`。
fn assert_node_type_of_contract(store: &mut dyn GraphWriteStore) {
    assert_eq!(store.node_type_of("page:p").expect("查询"), None);
    store
        .upsert_node(node("page:p", NodeType::Page, None))
        .expect("upsert");
    assert_eq!(
        store.node_type_of("page:p").expect("查询"),
        Some(NodeType::Page)
    );
    store
        .remove_nodes_by_ids(&["page:p".to_string()])
        .expect("remove");
    assert_eq!(store.node_type_of("page:p").expect("查询"), None);
}

#[test]
fn memory_store_reports_node_types() {
    assert_node_type_of_contract(&mut MemoryGraphStore::new());
}

#[cfg(feature = "grafeo-store")]
#[test]
fn grafeo_store_reports_node_types() {
    let dir = std::env::temp_dir().join(format!("l0-grafeo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let mut store = metadata_checker::graph_grafeo::GrafeoGraphStore::open(&dir.join("g.grafeo"))
        .expect("open grafeo");
    assert_node_type_of_contract(&mut store);
    drop(store);
    std::fs::remove_dir_all(&dir).expect("remove dir");
}

#[cfg(feature = "cli-local")]
#[test]
fn redb_store_reports_node_types() {
    let dir = std::env::temp_dir().join(format!("l0-redb-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let mut store =
        metadata_checker::graph_redb::GraphDB::open(&dir.join("g.graphdb")).expect("open redb");
    assert_node_type_of_contract(&mut store);
    drop(store);
    std::fs::remove_dir_all(&dir).expect("remove dir");
}

// ------------------------------------------------------------ 失败的扫描不得改动图

/// Grafeo 直写、没有事务：违规若在批删与真实写入之后才暴露，图就留下半份、FileState 又没
/// 更新，重试清不干净。所以校验必须在任何改动之前完成（`apply_incremental_changes` 先干跑）。
/// 这里用真实的 `.grafeo` 增量扫描证明：失败的扫描不改动图与文件状态。
#[cfg(all(feature = "cli-local", feature = "grafeo-store"))]
mod failed_scan_leaves_graph_untouched {
    use metadata_checker::graph::Node;
    use metadata_checker::graph_grafeo::GrafeoGraphStore;
    use metadata_checker::graph_store::{GraphReadStore, IndexStateStore};
    use metadata_checker::scanner::indexer::ProjectIndexer;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// 带内嵌 dataflow 的页面；`length` 决定该页面合规（数字）还是违规（字符串，契约要求 number）。
    fn page_with_length(length: serde_json::Value, extra_component: &str) -> String {
        serde_json::json!({
            "version": "1",
            "params": [],
            "sources": [{"id": "flow1", "modelType": "dataflow", "content": {
                "dimensions": [{"name": "x", "length": length}]
            }}],
            "canvas": {"id": "canvas", "type": "canvas", "components": [
                {"id": extra_component, "type": "button"}
            ]}
        })
        .to_string()
    }

    fn good(extra_component: &str) -> String {
        page_with_length(serde_json::json!(10), extra_component)
    }

    fn bad(extra_component: &str) -> String {
        page_with_length(serde_json::json!("not-a-number"), extra_component)
    }

    struct Project {
        dir: PathBuf,
    }

    impl Project {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "l0-failed-scan-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).expect("create dir");
            Self { dir }
        }
        fn write(&self, name: &str, text: &str) {
            std::fs::write(self.dir.join(name), text).expect("write");
        }
        fn db(&self) -> PathBuf {
            self.dir.join("db").join("g.grafeo")
        }
        fn project(&self) -> PathBuf {
            self.dir.join("src")
        }
        fn write_src(&self, name: &str, text: &str) {
            std::fs::create_dir_all(self.project()).expect("src dir");
            std::fs::write(self.project().join(name), text).expect("write src");
        }
        fn scan(&self) -> anyhow::Result<()> {
            std::fs::create_dir_all(self.dir.join("db")).expect("db dir");
            ProjectIndexer::scan(&self.project(), &self.db()).map(|_| ())
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// 图的完整快照：节点、每条出边、文件状态。字符串化后可逐字节比较。
    #[derive(Debug, PartialEq, Eq)]
    struct Snapshot {
        nodes: Vec<String>,
        edges: Vec<String>,
        file_states: BTreeMap<String, String>,
    }

    fn snapshot(db: &Path) -> Snapshot {
        let store = GrafeoGraphStore::open(db).expect("open grafeo");
        let nodes: Vec<Node> = store.iter_nodes().expect("iter nodes").collect();
        let mut edges = Vec::new();
        for node in &nodes {
            let neighbors = store
                .get_node_edges(&node.id)
                .expect("edges")
                .expect("node exists");
            for view in neighbors.outgoing {
                edges.push(serde_json::to_string(&view.edge).expect("edge json"));
            }
        }
        edges.sort();
        let file_states = store
            .load_file_states()
            .expect("file states")
            .into_iter()
            .map(|(path, state)| (path, serde_json::to_string(&state).expect("state json")))
            .collect();
        Snapshot {
            nodes: nodes
                .iter()
                .map(|node| serde_json::to_string(node).expect("node json"))
                .collect(),
            edges,
            file_states,
        }
    }

    /// 节点与边（不含 FileState：mtime 会随写入时间不同）。
    fn graph_only(snapshot: &Snapshot) -> (&Vec<String>, &Vec<String>) {
        (&snapshot.nodes, &snapshot.edges)
    }

    fn assert_rejected_naming(result: anyhow::Result<()>, file: &str) {
        let message = format!("{:#}", result.expect_err("违规的扫描必须报错"));
        for expected in [file, "meta 键 length 类型不符", "写入校验未通过"] {
            assert_eq!(
                message.contains(expected),
                true,
                "缺少 {expected}：{message}"
            );
        }
    }

    /// (1) 同一批里先有一个合规的新文件、再有一个违规的新文件：整批不落，图与 FileState 原样。
    #[test]
    fn violating_new_file_after_a_valid_one_changes_nothing() {
        let project = Project::new("new-file");
        project.write_src("m.spg", &good("m_btn"));
        project.scan().expect("首次扫描合规");
        let before = snapshot(&project.db());

        // a_ok.spg 排在 z_bad.spg 之前，会先被处理
        project.write_src("a_ok.spg", &good("ok_btn"));
        project.write_src("z_bad.spg", &bad("bad_btn"));
        assert_rejected_naming(project.scan(), "z_bad.spg");
        assert_eq!(
            snapshot(&project.db()),
            before,
            "失败的扫描不得改动图与文件状态"
        );
    }

    /// (2) 违规文件此前已存在（批删本会触发）：旧节点必须还在，同批里合规文件的新内容也不落。
    #[test]
    fn violating_edit_of_an_existing_file_keeps_its_old_nodes() {
        let project = Project::new("existing-file");
        project.write_src("a.spg", &good("a_btn"));
        project.write_src("m.spg", &good("m_btn"));
        project.scan().expect("首次扫描合规");
        let before = snapshot(&project.db());

        // a.spg 先处理（合规的改动：换了组件），m.spg 随后违规
        project.write_src("a.spg", &good("a_btn_renamed"));
        project.write_src("m.spg", &bad("m_btn_renamed"));
        assert_rejected_naming(project.scan(), "m.spg");
        let after = snapshot(&project.db());
        assert_eq!(after, before, "失败的扫描不得改动图与文件状态");
        assert_eq!(
            after
                .nodes
                .iter()
                .any(|node| node.contains("comp:m.spg|m_btn\"")),
            true,
            "违规文件的旧节点必须还在"
        );
        assert_eq!(
            after
                .nodes
                .iter()
                .any(|node| node.contains("a_btn_renamed")),
            false,
            "同批合规文件的新内容也不得落库"
        );
    }

    /// 无名维度的 `.tbl` 经真实的 `.grafeo` 增量扫描：整批被拒，图与 FileState 原样不动，
    /// 同批里排在前面的合规文件也不落库；错误点明文件、节点与原因。
    #[test]
    fn tbl_with_an_unnamed_dimension_changes_nothing() {
        let project = Project::new("tbl-empty-name");
        project.write_src("m.spg", &good("m_btn"));
        project.scan().expect("首次扫描合规");
        let before = snapshot(&project.db());

        project.write_src("a_ok.spg", &good("ok_btn"));
        project.write_src(
            "orders.tbl",
            &serde_json::json!({
                "properties": {"dbTableName": "fact_orders"},
                "dimensions": [{"name": "", "dbfield": "x", "dataType": "varchar"}]
            })
            .to_string(),
        );
        let message = format!("{:#}", project.scan().expect_err("无名维度必须让扫描报错"));
        for expected in [
            "orders.tbl",
            "field:orders.",
            "id 不符合契约文法",
            "写入校验未通过",
        ] {
            assert_eq!(
                message.contains(expected),
                true,
                "缺少 {expected}：{message}"
            );
        }
        assert_eq!(
            snapshot(&project.db()),
            before,
            "失败的扫描不得改动图与文件状态"
        );
    }

    /// 空图上的首次扫描不干跑（省耗时），失败后把整张图清回空：失败的首次扫描不留任何节点、
    /// 边与文件状态；修好后下一次扫描与干净扫描相同（不会残留失败那次写下的节点）。
    #[test]
    fn violating_first_scan_rolls_back_to_an_empty_graph() {
        let project = Project::new("first-scan");
        project.write_src("a_ok.spg", &good("ok_btn"));
        project.write_src("z_bad.spg", &bad("bad_btn"));
        let message = format!("{:#}", project.scan().expect_err("违规的首次扫描必须报错"));
        for expected in ["z_bad.spg", "meta 键 length 类型不符", "已回滚"] {
            assert_eq!(
                message.contains(expected),
                true,
                "缺少 {expected}：{message}"
            );
        }
        let after = snapshot(&project.db());
        assert_eq!(
            after.nodes,
            Vec::<String>::new(),
            "失败的首次扫描不得留下节点"
        );
        assert_eq!(
            after.edges,
            Vec::<String>::new(),
            "失败的首次扫描不得留下边"
        );
        assert_eq!(
            after.file_states.is_empty(),
            true,
            "失败的首次扫描不得记录文件状态"
        );

        // 修复后产出的节点比失败那次更少：a_ok.spg 去掉一个组件，z_bad.spg 修好
        project.write_src("z_bad.spg", &good("bad_btn_fixed"));
        project.scan().expect("修复后扫描成功");
        let recovered = snapshot(&project.db());
        let clean = Project::new("first-scan-clean");
        clean.write_src("a_ok.spg", &good("ok_btn"));
        clean.write_src("z_bad.spg", &good("bad_btn_fixed"));
        clean.scan().expect("干净全量扫描");
        assert_eq!(graph_only(&recovered), graph_only(&snapshot(&clean.db())));
    }

    /// (3) 修好文件后下一次扫描成功，结果与对最终内容做一次干净全量扫描相同。
    #[test]
    fn fixing_the_file_makes_the_next_scan_match_a_clean_scan() {
        let project = Project::new("fix");
        project.write_src("a.spg", &good("a_btn"));
        project.write_src("m.spg", &good("m_btn"));
        project.scan().expect("首次扫描合规");

        project.write_src("a.spg", &good("a_btn_renamed"));
        project.write_src("m.spg", &bad("m_btn_renamed"));
        project.scan().expect_err("违规");
        // 修复：产出的节点比失败那次更少（没有 m_btn_renamed 之外的残留可言）
        project.write_src("m.spg", &good("m_btn_fixed"));
        project.scan().expect("修复后扫描成功");
        let recovered = snapshot(&project.db());

        let clean = Project::new("fix-clean");
        clean.write_src("a.spg", &good("a_btn_renamed"));
        clean.write_src("m.spg", &good("m_btn_fixed"));
        clean.scan().expect("干净全量扫描");
        let expected = snapshot(&clean.db());
        assert_eq!(graph_only(&recovered), graph_only(&expected));
        assert_eq!(
            recovered.file_states.keys().collect::<Vec<_>>(),
            expected.file_states.keys().collect::<Vec<_>>()
        );
    }
}
