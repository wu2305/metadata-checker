use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// SuperPage 元数据解析模块
///
/// 负责解析 .spg 文件的 JSON 结构，提取：
/// - 组件树（支持递归嵌套：components/panels/steps/comps）
/// - 表达式字段（exp、value、defaultValue、visible、enable 等）
/// - 动作列表（actions）及其参数
/// - 页面参数（params）和数据源（sources）
///
/// 表达式引用解析：识别组件值引用、模型字段引用、页面参数引用等。
mod raw_types;
use raw_types::*;
mod types;
pub use types::*;

// ============================================================

// ============================================================

/// 从已解析的 JSON Value 构建 SuperPageMetadata
pub fn parse_superpage_from_value(raw_value: serde_json::Value) -> Result<SuperPageMetadata> {
    let raw: RawSuperPage = serde_json::from_value(raw_value.clone())
        .with_context(|| "Failed to parse JSON as RawSuperPage")?;

    let mut meta = SuperPageMetadata {
        raw: raw_value,
        version: raw.version,
        theme: raw.theme,
        params: raw
            .params
            .into_iter()
            .map(|p| SpgParam {
                id: p.id,
                name: p.name,
                desc: p.desc,
                default_value: p.value,
            })
            .collect(),
        reference_resources: raw.reference_resources,
        sources: raw
            .sources
            .into_iter()
            .map(|s| SpgSource {
                id: s.id,
                model_type: s.model_type,
                path: s.path,
                content: s.content,
            })
            .collect(),
        ..Default::default()
    };
    if let Some(canvas) = raw.canvas {
        extract_components(&canvas, None, &mut meta.components, &mut meta.expressions);
    }

    // Post-process: resolve refs with known context (component_ids, source_ids, param_ids)
    resolve_expression_refs_with_context(
        &mut meta.expressions,
        &meta.components,
        &meta.sources,
        &meta.params,
    );

    Ok(meta)
}

/// 读取并解析 .spg 文件
pub fn parse_superpage(path: &Path) -> Result<SuperPageMetadata> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let raw_value: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON from: {}", path.display()))?;
    parse_superpage_from_value(raw_value)
}

/// 使用已知的组件/模型/参数上下文，重新解析表达式引用
/// 解决纯 regex 无法区分 customer.name（模型字段）和 input1.value（组件值）的问题
fn resolve_expression_refs_with_context(
    expressions: &mut [ComponentExpr],
    components: &[SpgComponent],
    sources: &[SpgSource],
    params: &[SpgParam],
) {
    let component_ids: std::collections::HashSet<&str> =
        components.iter().map(|c| c.id.as_str()).collect();
    let source_ids: std::collections::HashSet<&str> =
        sources.iter().map(|s| s.id.as_str()).collect();
    let param_ids: std::collections::HashSet<&str> = params.iter().map(|p| p.id.as_str()).collect();

    for expr in expressions {
        for ref_type in &mut expr.refs {
            *ref_type = match ref_type {
                RefType::Other(token) => {
                    resolve_ref_token(token, &component_ids, &source_ids, &param_ids)
                }
                RefType::ComponentValue(id) => {
                    // Keep ComponentValue even if not found in current page
                    // It may be a component from a parent page or external context
                    RefType::ComponentValue(id.clone())
                }
                RefType::ModelField(model, field) if component_ids.contains(model.as_str()) => {
                    // Was guessed as model but actually a component
                    RefType::ComponentProperty(model.clone(), field.clone())
                }
                _ => ref_type.clone(),
            };
        }
    }
}

fn resolve_ref_token(
    token: &str,
    component_ids: &std::collections::HashSet<&str>,
    source_ids: &std::collections::HashSet<&str>,
    param_ids: &std::collections::HashSet<&str>,
) -> RefType {
    let parts: Vec<&str> = token.split('.').collect();
    let first = parts[0];

    if parts.len() >= 2 {
        let rest = parts[1..].join(".");

        // Exact match: first part is a known component ID
        if component_ids.contains(first) {
            return RefType::ComponentProperty(first.to_string(), rest);
        }

        // Exact match: first part is a known source (model) ID
        if source_ids.contains(first) {
            return RefType::ModelField(first.to_string(), rest);
        }

        // Exact match: first part is a known param
        if param_ids.contains(first) {
            return RefType::Param(token.to_string());
        }

        // Fallback: if last part is "value" and first part looks like a component, treat as ComponentValue
        if parts.last() == Some(&"value")
            && (first.starts_with("input")
                || first.starts_with("text")
                || first.starts_with("select"))
        {
            return RefType::ComponentValue(first.to_string());
        }

        // Fallback: if first part starts with "model" or looks like a table name, treat as ModelField
        if first.starts_with("model") || first.starts_with("tbl") || first.starts_with("fact_") {
            return RefType::ModelField(first.to_string(), rest);
        }

        return RefType::Other(token.to_string());
    }

    if param_ids.contains(token) {
        return RefType::Param(token.to_string());
    }

    if component_ids.contains(token) {
        return RefType::ComponentValue(token.to_string());
    }

    RefType::Other(token.to_string())
}
fn extract_components(
    raw: &RawComponent,
    parent_id: Option<String>,
    components: &mut Vec<SpgComponent>,
    expressions: &mut Vec<ComponentExpr>,
) {
    if raw.id.is_empty() || raw.component_type.is_empty() {
        for child in &raw.components {
            extract_components(child, parent_id.clone(), components, expressions);
        }
        for child in &raw.panels {
            extract_components(child, parent_id.clone(), components, expressions);
        }
        for child in &raw.steps {
            extract_components(child, parent_id.clone(), components, expressions);
        }
        for child in &raw.comps {
            extract_components(child, parent_id.clone(), components, expressions);
        }
        return;
    }

    let mut comp = SpgComponent {
        id: raw.id.clone(),
        component_type: raw.component_type.clone(),
        parent_id: parent_id.clone(),
        properties: HashMap::new(),
        actions: Vec::new(),
        submit_data: raw.submit_data,
        res_path: raw.res_path.as_ref().and_then(|v| {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                v.as_i64().map(|n| n.to_string())
            }
        }),
    };

    // 总是表达式字段：所有非空值都视为表达式
    let always_expr_fields = [
        "exp",
        "itemFilter",
        "visibleCondition",
        "validExp",
        "calcCondition",
        "calcExp",
        "maskCondition",
        "submitCondition",
        "submitPageCondition",
        "defaultPanelCondition",
    ];
    // 条件表达式字段：包含 = 或 ${ 时才视为表达式
    let conditional_expr_fields = [
        "value",
        "defaultValue",
        "visible",
        "enable",
        "text",
        "formula",
        "html",
        "desc",
        "placeholder",
        "url",
        "documentTitle",
        "inputTitle",
        "labelValue",
        "panelName",
        "rootPath",
        "selectedCaption",
        "confirmCaption",
        "tip",
        "badge",
        "count",
        "attrCaption",
        "caption",
        "defaultSelect",
        "defaultCheck",
        "maxLevel",
    ];

    let fields = [
        ("value", &raw.value),
        ("defaultValue", &raw.default_value),
        ("visible", &raw.visible),
        ("exp", &raw.exp),
        ("enable", &raw.enable),
        ("text", &raw.text),
        ("formula", &raw.formula),
        ("html", &raw.html),
        ("calcCondition", &raw.calc_condition),
        ("itemFilter", &raw.item_filter),
        ("validExp", &raw.valid_exp),
        ("calcExp", &raw.calc_exp),
        ("maskCondition", &raw.mask_condition),
        ("submitCondition", &raw.submit_condition),
        ("visibleCondition", &raw.visible_condition),
        ("submitPageCondition", &raw.submit_page_condition),
        ("defaultPanelCondition", &raw.default_panel_condition),
        ("desc", &raw.desc),
        ("placeholder", &raw.placeholder),
        ("url", &raw.url),
        ("documentTitle", &raw.document_title),
        ("inputTitle", &raw.input_title),
        ("labelValue", &raw.label_value),
        ("panelName", &raw.panel_name),
        ("rootPath", &raw.root_path),
        ("selectedCaption", &raw.selected_caption),
        ("confirmCaption", &raw.confirm_caption),
        ("tip", &raw.tip),
        ("badge", &raw.badge),
        ("count", &raw.count),
        ("attrCaption", &raw.attr_caption),
        ("caption", &raw.caption),
        ("defaultSelect", &raw.default_select),
        ("defaultCheck", &raw.default_check),
        ("maxLevel", &raw.max_level),
    ];

    for (field_name, field_value) in &fields {
        if let Some(val) = field_value {
            // Only process string values as expressions; skip booleans, numbers, objects, arrays
            let val_str = match val {
                serde_json::Value::String(s) => s.as_str(),
                _ => continue,
            };
            let is_expr = if always_expr_fields.contains(field_name) {
                !val_str.is_empty() // 总是表达式字段：非空即表达式
            } else if conditional_expr_fields.contains(field_name) {
                val_str.starts_with('=') || val_str.contains("${") // 条件表达式字段
            } else {
                false
            };

            if is_expr {
                comp.properties
                    .insert(field_name.to_string(), val_str.to_string());
                let refs = parse_expression_refs(val_str);
                expressions.push(ComponentExpr {
                    component_id: raw.id.clone(),
                    field: field_name.to_string(),
                    raw_expr: val_str.to_string(),
                    refs: refs.clone(),
                    resolved_refs: Vec::new(),
                });
            } else if !val_str.is_empty() {
                comp.properties
                    .insert(field_name.to_string(), val_str.to_string());
            }
        }
    }

    // Handle submitField separately (always a string, no expression parsing needed)
    if let Some(ref submit_field) = raw.submit_field {
        comp.properties
            .insert("submitField".to_string(), submit_field.clone());
    }

    // Populate actions
    comp.actions = raw
        .actions
        .iter()
        .map(|a| SpgAction {
            id: a.id.clone(),
            action_type: a.action_type.clone(),
            trigger_type: a.trigger_type.clone(),
            submit_range: a.submit_range.clone(),
            data_set: a.data_set.as_ref().and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else if let Some(arr) = v.as_array() {
                    arr.first()
                        .and_then(|item| item.as_str())
                        .map(|s| s.to_string())
                } else {
                    None
                }
            }),
            data_range: a.data_range.clone(),
            field_values: a
                .field_values
                .iter()
                .map(|fv| {
                    (
                        fv.name.clone(),
                        fv.value.as_str().unwrap_or("").to_string(),
                        fv.value_type.clone(),
                    )
                })
                .collect(),
            submit_component: a.submit_component.clone(),
            path: a.path.as_ref().and_then(|v| {
                if let Some(s) = v.as_str() {
                    Some(s.to_string())
                } else {
                    v.as_i64().map(|n| n.to_string())
                }
            }),
            short_url: a.short_url.clone(),
            data: a
                .data
                .iter()
                .map(|d| (d.name.clone(), d.value.clone()))
                .collect(),
            params: a
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.clone()))
                .collect(),
            target_type: a.target_type.clone(),
        })
        .collect();

    components.push(comp);

    let parent = Some(raw.id.clone());
    for child in &raw.components {
        extract_components(child, parent.clone(), components, expressions);
    }
    for child in &raw.panels {
        extract_components(child, parent.clone(), components, expressions);
    }
    for child in &raw.steps {
        extract_components(child, parent.clone(), components, expressions);
    }
    for child in &raw.comps {
        extract_components(child, parent.clone(), components, expressions);
    }
}

// ============================================================

mod expr;
pub use expr::*;
