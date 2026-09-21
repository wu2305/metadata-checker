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
