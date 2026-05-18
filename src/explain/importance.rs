use crate::graph::{Node, NodeType};

/// 从组件节点元数据中读取组件类型。
pub(super) fn component_type_from_meta(node: &Node) -> String {
    node.meta
        .as_ref()
        .and_then(|m| m.get("component_type"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| node.name.clone())
}

fn is_container_type(comp_type: &str) -> bool {
    matches!(comp_type, "dialog" | "panel" | "embedsuperpage" | "form")
}

fn is_form_input_type(comp_type: &str) -> bool {
    matches!(
        comp_type,
        "input" | "textarea" | "select" | "radio" | "checkbox" | "datePicker" | "number" | "search"
    )
}

/// 根据读写、导航、节点类型和组件类型给 explain 目标分类。
pub(super) fn classify_importance(
    has_nav: bool,
    has_write: bool,
    has_read: bool,
    has_action: bool,
    comp_type: &str,
    node_type: &NodeType,
) -> String {
    match node_type {
        NodeType::Page => {
            if has_nav || has_write {
                "entrypoint".to_string()
            } else {
                "container".to_string()
            }
        }
        NodeType::Component => {
            if is_container_type(comp_type) {
                "container".to_string()
            } else if is_form_input_type(comp_type) {
                "form_input".to_string()
            } else if comp_type == "text" && has_read {
                "data_source_display".to_string()
            } else if has_action {
                "entrypoint".to_string()
            } else if has_write {
                "action_target".to_string()
            } else if has_read {
                "data_source_display".to_string()
            } else {
                "static_display".to_string()
            }
        }
        NodeType::Action => {
            if has_nav {
                "entrypoint".to_string()
            } else if has_write {
                "action_target".to_string()
            } else if has_read {
                "data_source_display".to_string()
            } else {
                "static_display".to_string()
            }
        }
        NodeType::Model | NodeType::Field => {
            if has_write {
                "action_target".to_string()
            } else if has_read {
                "data_source_display".to_string()
            } else {
                "unknown".to_string()
            }
        }
        NodeType::Condition => "condition".to_string(),
    }
}
