use crate::graph::{Edge, EdgeType, Node, NodeType};
use crate::graph_store::GraphWriteStore;
use anyhow::{Context, Result};
use std::path::Path;

/// 向图存储写入节点。
pub fn add_node(
    graph: &mut dyn GraphWriteStore,
    id: String,
    node_type: NodeType,
    path: String,
    name: String,
    meta: Option<serde_json::Value>,
) -> Result<()> {
    graph
        .upsert_node(Node {
            id,
            node_type,
            path,
            name,
            meta,
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
