use crate::graph::{Edge, EdgeType, Node, NodeType};
use crate::graph_store::GraphWriteStore;
use anyhow::{Context, Result};
use std::path::Path;

/// 向图存储写入**已完成身份构造**的节点。
///
/// 调用方必须先经 [`crate::graph_identity`] 或 [`crate::scanner::spg::PageScope`]
/// 决定页面局部/全局身份；本函数只做写入边界的歧义拒绝，不再二次转换 id——
/// 页面局部 id 过 [`add_node`] 的全局转换会丢掉页面段，正是 M59-2 自环与身份
/// 塌陷的根因之一。
pub fn add_identified_node(
    graph: &mut dyn GraphWriteStore,
    id: String,
    node_type: NodeType,
    path: String,
    name: String,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    // 竖线是「页面局部 vs 全局」的唯一判据：全局名不得含分隔符；页面局部 id
    // 的页面段与局部名各自也不得再含分隔符（否则解析侧无法判定作用域）。
    if let Some(rest) = id
        .strip_prefix("model:")
        .or_else(|| id.strip_prefix("field:"))
    {
        crate::graph_identity::reject_reserved_separator(rest, &id)?;
    }
    graph
        .upsert_node(Node {
            id,
            node_type,
            path,
            name,
            meta,
            origin_file: None,
        })
        .with_context(|| "Failed to upsert graph node")
}

/// 向图存储写入节点（旧全局身份入口；model/field 仅限 `LegacyGlobal` 路径，
/// 页面/组件等非 model/field 节点不受身份模式影响）。
pub fn add_node(
    graph: &mut dyn GraphWriteStore,
    id: String,
    node_type: NodeType,
    path: String,
    name: String,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    // LegacyGlobal 模式下 model/field 保持全局编码；页面局部身份只允许经
    // [`add_identified_node`] 写入（调用方先用 `PageScope` 构造完整 id），
    // 此处强制走全局形态并在写入边界拒绝歧义分隔符。
    let id = if let Some(local) = id.strip_prefix("model:") {
        crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Model, local)?
    } else if let Some(local) = id.strip_prefix("field:") {
        crate::graph_identity::global_node_id(crate::graph_identity::NodeIdKind::Field, local)?
    } else {
        id
    };
    graph
        .upsert_node(Node {
            id,
            node_type,
            path,
            name,
            meta,
            origin_file: None,
        })
        .with_context(|| "Failed to upsert graph node")
}

/// 向图存储写入带元数据的边。
pub fn add_edge_with_meta(
    graph: &mut dyn GraphWriteStore,
    from: &str,
    to: &str,
    edge_type: EdgeType,
    field_path: Option<String>,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    graph
        .add_edge(Edge {
            from: from.to_string(),
            to: to.to_string(),
            edge_type,
            field_path,
            meta,
            origin_file: None,
        })
        .with_context(|| format!("Failed to add graph edge from {} to {}", from, to))
}

/// Resolve a reference path from referenceResources to an absolute path.
/// Handles relative paths (../, ./) and $TAPP: prefix.
pub fn resolve_reference_path(
    rel_path: &str,
    ref_idx: usize,
    reference_resources: &[String],
) -> Option<String> {
    let ref_path = reference_resources.get(ref_idx)?;
    if ref_path.starts_with("$TAPP:") {
        // $TAPP:/path/to/page.spg → resolve relative to project root
        let path = ref_path.strip_prefix("$TAPP:").unwrap_or(ref_path);
        Some(path.trim_start_matches('/').to_string())
    } else {
        // Relative path: resolve based on current .spg directory
        let current_dir = Path::new(rel_path).parent()?;
        let resolved = current_dir.join(ref_path);
        Some(resolved.to_string_lossy().to_string().replace('\\', "/"))
    }
}
