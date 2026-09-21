use metadata_checker::graph_identity::{NodeIdKind, page_local_node_id, parse_node_id};
// 不支持竖线时应拒绝输入，支持时必须保证不同元组不碰撞。
#[test]
fn different_page_local_pairs_never_collide() {
    let first = page_local_node_id(NodeIdKind::Model, "a.spg|b.spg", "c");
    let second = page_local_node_id(NodeIdKind::Model, "a.spg", "b.spg|c");
    assert_eq!(first.is_err() || second.is_err() || first != second, true);
}
// 成功构造的身份必须能恢复输入的页面和局部名。
#[test]
fn successful_constructor_roundtrips() {
    for (page, local) in [("a.spg", "c"), ("目录/a.spg", "字段.名称")] {
        let id = page_local_node_id(NodeIdKind::Model, page, local).unwrap();
        let parsed = parse_node_id(&id).unwrap();
        assert_eq!(parsed.page.as_deref(), Some(page));
        assert_eq!(parsed.local, local);
    }
    assert_eq!(page_local_node_id(NodeIdKind::Model, "a.spg|b.spg", "c").is_err(), true);
}

// 不允许把绝对/盘符引用折叠到项目相对身份中。
#[test]
fn absolute_paths_are_rejected_before_joining() {
    use metadata_checker::graph_identity::{normalize_project_path, resolve_relative_reference};
    for path in [
        "/app/a.spg",
        "C:\\app\\a.spg",
        "C:app/a.spg",
        "\\\\host\\share\\a.spg",
    ] {
        assert_eq!(normalize_project_path(path).is_err(), true, "{path}");
        assert_eq!(
            resolve_relative_reference("app/current.spg", path).is_err(),
            true,
            "{path}"
        );
    }
}

// 验证生产 scanner 写入边界，不能只靠未启用 helper 的单测。
#[test]
fn scanner_rejects_global_names_with_reserved_separator() {
    use metadata_checker::graph_store::GraphReadStore;
    use metadata_checker::memory_graph_store::MemoryGraphStore;
    let mut graph = MemoryGraphStore::new();
    let result = metadata_checker::scanner::process_tbl_file_from_string(
        &mut graph,
        "table|name.tbl",
        "{\"dimensions\":[]}",
    );
    assert_eq!(result.is_err(), true);
    assert_eq!(graph.node_count().unwrap(), 0);
}

// 不能用全局构造器生成本应带页面作用域的节点。
#[test]
fn global_constructor_rejects_scoped_kinds() {
    use metadata_checker::graph_identity::global_node_id;
    for kind in [NodeIdKind::Comp, NodeIdKind::Param, NodeIdKind::Action, NodeIdKind::Cond, NodeIdKind::Page] {
        assert_eq!(global_node_id(kind, "x").is_err(), true);
    }
}

// 先出现合法模型/字段、后出现非法身份，也必须在首次写入前失败。
#[test]
fn invalid_table_identity_leaves_no_partial_graph() {
    use metadata_checker::graph_store::GraphReadStore;
    use metadata_checker::memory_graph_store::MemoryGraphStore;
    for value in [
        serde_json::json!({"dimensions":[{"name":"ok"},{"name":"bad|field"}]}),
        serde_json::json!({"properties":{"dbTableName":"bad|table"}}),
        serde_json::json!({"dataFlow":{},"properties":{"depends":["ok.tbl","bad|table.tbl"]}}),
        serde_json::json!({"dataFlow":{"nodes":{"n":{"moduleTablePath":"bad|input.tbl"}}}}),
    ] {
        let mut graph = MemoryGraphStore::new();
        assert_eq!(metadata_checker::scanner::process_tbl_file_from_string(&mut graph, "ok.tbl", &value.to_string()).is_err(), true);
        assert_eq!(graph.node_count().unwrap(), 0);
    }
}
