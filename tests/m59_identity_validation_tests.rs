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
    if let Ok(id) = page_local_node_id(NodeIdKind::Model, "a.spg|b.spg", "c") {
        let parsed = parse_node_id(&id).unwrap();
        assert_eq!(parsed.page.as_deref(), Some("a.spg|b.spg"));
        assert_eq!(parsed.local, "c");
    }
}

// 不允许把绝对/盘符引用折叠到项目相对身份中。
#[test]
fn absolute_paths_are_rejected_before_joining() {
    use metadata_checker::graph_identity::{normalize_project_path, resolve_relative_reference};
    for path in ["/app/a.spg", "C:\\app\\a.spg", "C:app/a.spg", "\\\\host\\share\\a.spg"] {
        assert_eq!(normalize_project_path(path).is_err(), true, "{path}");
        assert_eq!(resolve_relative_reference("app/current.spg", path).is_err(), true, "{path}");
    }
}

// 验证生产 scanner 写入边界，不能只靠未启用 helper 的单测。
#[test]
fn scanner_rejects_global_names_with_reserved_separator() {
    use metadata_checker::memory_graph_store::MemoryGraphStore;
    use metadata_checker::graph_store::GraphReadStore;
    let mut graph = MemoryGraphStore::new();
    let result = metadata_checker::scanner::process_tbl_file_from_string(&mut graph, "table|name.tbl", "{\"dimensions\":[]}");
    assert_eq!(result.is_err(), true);
    assert_eq!(graph.node_count().unwrap(), 0);
}
