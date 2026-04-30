use serde_json::{Value, json};

/// actionType → action_category 映射
pub fn classify_action(action_type: &str) -> &'static str {
    match action_type {
        "submitData" | "insertData" | "updateData" | "deleteData" => "data_write",
        "loadData" => "data_read",
        "link" | "showDialog" => "navigation",
        "setParamValue" => "param_mutation",
        "showComponent" | "hideComponent" | "switchPanel" | "closeDialog" => "ui_control",
        "validateData" => "validation",
        "newData" | "resetData" | "refreshModels" => "data_refresh",
        _ => "unknown",
    }
}

/// 生成动作的自然语言语义摘要
///
/// 参数：action_type, component_name, action_id, reads, writes, navigation
/// 返回：人类可读的一句话摘要
pub fn build_semantic_summary(
    action_type: &str,
    component_name: &str,
    _action_id: &str,
    _reads: &[Value],
    writes: &[Value],
    nav: &[Value],
) -> String {
    match action_type {
        "submitData" => {
            let write_fields: Vec<&str> = writes
                .iter()
                .filter_map(|w| w.get("field_path").and_then(|v| v.as_str()))
                .collect();
            if write_fields.is_empty() {
                format!("点击 {} 后提交数据（未识别目标字段）", component_name)
            } else {
                format!(
                    "点击 {} 后提交数据到 {}",
                    component_name,
                    write_fields.join(", ")
                )
            }
        }
        "insertData" => {
            let models: Vec<&str> = writes
                .iter()
                .filter_map(|w| {
                    w.get("model")
                        .and_then(|v| v.as_str())
                        .or_else(|| w.get("name").and_then(|v| v.as_str()))
                })
                .collect();
            if models.is_empty() {
                format!("点击 {} 后插入新记录（未识别目标模型）", component_name)
            } else {
                format!(
                    "点击 {} 后向 {} 插入新记录",
                    component_name,
                    models.join(", ")
                )
            }
        }
        "updateData" => {
            let models: Vec<&str> = writes
                .iter()
                .filter_map(|w| {
                    w.get("model")
                        .and_then(|v| v.as_str())
                        .or_else(|| w.get("name").and_then(|v| v.as_str()))
                })
                .collect();
            if models.is_empty() {
                format!("点击 {} 后更新记录（未识别目标模型）", component_name)
            } else {
                format!(
                    "点击 {} 后更新 {} 的记录",
                    component_name,
                    models.join(", ")
                )
            }
        }
        "deleteData" => {
            let models: Vec<&str> = writes
                .iter()
                .filter_map(|w| {
                    w.get("model")
                        .and_then(|v| v.as_str())
                        .or_else(|| w.get("name").and_then(|v| v.as_str()))
                })
                .collect();
            if models.is_empty() {
                format!("点击 {} 后删除记录（未识别目标模型）", component_name)
            } else {
                format!(
                    "点击 {} 后删除 {} 的记录",
                    component_name,
                    models.join(", ")
                )
            }
        }
        "link" => {
            let targets: Vec<&str> = nav
                .iter()
                .filter_map(|n| n.get("to_name").and_then(|v| v.as_str()))
                .collect();
            if targets.is_empty() {
                format!("点击 {} 后跳转（未识别目标页面）", component_name)
            } else {
                format!("点击 {} 后跳转到 {}", component_name, targets.join(", "))
            }
        }
        "setParamValue" => {
            format!("点击 {} 后设置页面参数", component_name)
        }
        "showDialog" => {
            format!("点击 {} 后打开对话框", component_name)
        }
        "closeDialog" => {
            format!("点击 {} 后关闭对话框", component_name)
        }
        "switchPanel" => {
            format!("点击 {} 后切换面板", component_name)
        }
        "showComponent" => {
            format!("点击 {} 后显示组件", component_name)
        }
        "hideComponent" => {
            format!("点击 {} 后隐藏组件", component_name)
        }
        "validateData" => {
            format!("点击 {} 后校验数据", component_name)
        }
        "newData" => {
            format!("点击 {} 后新建数据", component_name)
        }
        "loadData" => {
            format!("点击 {} 后加载数据", component_name)
        }
        "resetData" => {
            format!("点击 {} 后重置数据", component_name)
        }
        "refreshModels" => {
            format!("点击 {} 后刷新模型", component_name)
        }
        _ => format!("点击 {} 后执行 {} 动作", component_name, action_type),
    }
}

/// 解析 waitPrev 为结构化 blocks_on
///
/// 输入：waitPrev 原始字符串，如 "button11.action1"
/// 输出：结构化 JSON
pub fn parse_wait_prev(raw: Option<&str>) -> Value {
    match raw {
        None => json!({"raw": null, "component_id": null, "action_id": null, "resolved": false}),
        Some(s) => {
            let parts: Vec<&str> = s.split('.').collect();
            if parts.len() == 2 {
                // 启发式判断：如果第二部分是常见组件属性，则不是 action_id
                let is_property = matches!(
                    parts[1],
                    "value"
                        | "text"
                        | "checked"
                        | "selected"
                        | "visible"
                        | "disabled"
                        | "readonly"
                        | "hidden"
                );
                if is_property {
                    json!({
                        "raw": s,
                        "component_id": parts[0],
                        "action_id": null,
                        "resolved": false,
                        "reason": "second part looks like a component property, not an action id",
                    })
                } else {
                    json!({
                        "raw": s,
                        "component_id": parts[0],
                        "action_id": parts[1],
                        "resolved": true,
                    })
                }
            } else {
                json!({
                    "raw": s,
                    "component_id": null,
                    "action_id": null,
                    "resolved": false,
                })
            }
        }
    }
}

/// 粗粒度解析 condition 表达式引用
///
/// 输入：condition 原始表达式字符串
/// 输出：结构化 JSON，包含 raw_expr、refs、confidence
pub fn parse_condition(raw: Option<&str>) -> Value {
    match raw {
        None => json!({"raw_expr": null, "refs": [], "resolved_refs": [], "confidence": "high"}),
        Some(expr) => {
            // 粗粒度提取：用简单正则提取 component/model 引用
            let refs = crate::superpage::parse_expression_refs(expr);
            let ref_ids: Vec<String> = refs
                .iter()
                .filter_map(|r| match r {
                    crate::superpage::RefType::ComponentValue(id) => Some(id.clone()),
                    crate::superpage::RefType::ComponentProperty(id, _) => Some(id.clone()),
                    crate::superpage::RefType::ModelField(model, _) => Some(model.clone()),
                    crate::superpage::RefType::Param(id) => Some(id.clone()),
                    _ => None,
                })
                .collect();
            let resolved: Vec<Value> = refs
                .iter()
                .map(|r| match r {
                    crate::superpage::RefType::ComponentValue(id) => json!({
                        "type": "ComponentValue",
                        "id": id,
                    }),
                    crate::superpage::RefType::ComponentProperty(id, field) => json!({
                        "type": "ComponentProperty",
                        "id": id,
                        "field": field,
                    }),
                    crate::superpage::RefType::ModelField(model, field) => json!({
                        "type": "ModelField",
                        "id": model,
                        "field": field,
                    }),
                    crate::superpage::RefType::Param(id) => json!({
                        "type": "Param",
                        "id": id,
                    }),
                    crate::superpage::RefType::UserProperty(prop) => json!({
                        "type": "UserProperty",
                        "id": prop,
                    }),
                    crate::superpage::RefType::SystemVar(var) => json!({
                        "type": "SystemVar",
                        "id": var,
                    }),
                    crate::superpage::RefType::Other(lit) => json!({
                        "type": "Literal",
                        "value": lit,
                    }),
                })
                .collect();
            json!({
                "raw_expr": expr,
                "refs": ref_ids,
                "resolved_refs": resolved,
                "confidence": "medium",
            })
        }
    }
}
