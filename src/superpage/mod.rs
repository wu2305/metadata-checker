use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

/// SuperPage 元数据解析模块
///
/// 负责解析 .spg 文件的 JSON 结构，提取：
/// - 组件树（白名单键 components/panels/steps/comps + M58.3 F1 形态感知递归：
///   未知子键中非空且每个元素都带字符串 id+type 的数组也视为子组件数组）
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
        expr.resolved_refs.clear();
        for ref_type in &mut expr.refs {
            let (new_ref, corrected_model_guess) =
                resolve_ref_type(ref_type, &component_ids, &source_ids, &param_ids);
            let (confidence, reason, unresolved) =
                classify_confidence(&new_ref, &component_ids, &source_ids, &param_ids);
            let reason = if corrected_model_guess {
                match &new_ref {
                    RefType::ComponentProperty(id, _) => format!(
                        "Corrected: '{}' was initially guessed as model but is actually a known component ID",
                        id
                    ),
                    // 裸 `${id}` 全组件引用归一为 ComponentValue 后的改写说明
                    RefType::ComponentValue(id) => format!(
                        "Corrected: '{}' was initially guessed as model but is actually a known component ID (bare reference, treated as component value)",
                        id
                    ),
                    // 裸 `${paramN}` 归一为 Param 后的改写说明（P1-6）
                    RefType::Param(id) => format!(
                        "Corrected: '{}' was initially guessed as model but is actually a known param ID (bare reference, treated as param)",
                        id
                    ),
                    _ => reason,
                }
            } else {
                reason
            };
            *ref_type = new_ref.clone();
            expr.resolved_refs.push(ResolvedRef {
                ref_type: new_ref,
                confidence,
                reason,
                unresolved,
            });
        }
    }
}

/// 将已解析的引用统一归一化：把误判为模型的组件/参数引用改写为正确语义。
/// 归一后不变式：`ComponentProperty` 的 property 永不为空——裸 `${id}` 全组件
/// 引用（field 为空）改写为 `ComponentValue`，语义即依赖组件值本身；
/// 裸 `${paramN}` 命中页面 param id 时改写为 `Param`（M58.3 复核返修 P1-6），
/// 避免下游产出 `model:paramN` 垃圾节点与 `field:paramN.` 尾点节点
pub(crate) fn resolve_ref_type(
    ref_type: &RefType,
    component_ids: &std::collections::HashSet<&str>,
    source_ids: &std::collections::HashSet<&str>,
    param_ids: &std::collections::HashSet<&str>,
) -> (RefType, bool) {
    match ref_type {
        RefType::Other(token) => (
            resolve_ref_token(token, component_ids, source_ids, param_ids),
            false,
        ),
        RefType::ModelField(model, field) if component_ids.contains(model.as_str()) => {
            if field.is_empty() {
                // 裸 `${id}` 全组件引用（expr_ast 先判为 ModelField(id, "")），
                // 归一为 ComponentValue，避免下游出现 "comp:id." 尾点形态
                (RefType::ComponentValue(model.clone()), true)
            } else {
                (
                    RefType::ComponentProperty(model.clone(), field.clone()),
                    true,
                )
            }
        }
        // 裸 `${id}` 命中页面 param id（expr_ast 判为 ModelField(id, "")）时归一为
        // Param，与上方裸组件 id 归一 ComponentValue 同族；命名启发式（param 前缀）
        // 之外的 param id 只有这里能凭上下文捕获
        RefType::ModelField(model, field)
            if field.is_empty() && param_ids.contains(model.as_str()) =>
        {
            (RefType::Param(model.clone()), true)
        }
        _ => (ref_type.clone(), false),
    }
}

/// 根据解析结果和上下文推断置信度
fn classify_confidence(
    resolved: &RefType,
    component_ids: &std::collections::HashSet<&str>,
    source_ids: &std::collections::HashSet<&str>,
    param_ids: &std::collections::HashSet<&str>,
) -> (Confidence, String, bool) {
    match resolved {
        RefType::ComponentValue(id) => {
            if component_ids.contains(id.as_str()) {
                (
                    Confidence::High,
                    format!("Exact match: '{}' is a known component ID", id),
                    false,
                )
            } else {
                (
                    Confidence::Medium,
                    format!(
                        "'{}' looks like a component reference but not found in current page; may be from parent or external context",
                        id
                    ),
                    false,
                )
            }
        }
        RefType::ComponentProperty(id, _prop) => {
            if component_ids.contains(id.as_str()) {
                (
                    Confidence::High,
                    format!("Exact match: '{}' is a known component ID", id),
                    false,
                )
            } else {
                (
                    Confidence::Medium,
                    format!(
                        "'{}' looks like a component but not found in current page",
                        id
                    ),
                    false,
                )
            }
        }
        RefType::ModelField(model, _field) => {
            if source_ids.contains(model.as_str()) {
                (
                    Confidence::High,
                    format!("Exact match: '{}' is a known source (model) ID", model),
                    false,
                )
            } else if model.starts_with("model")
                || model.starts_with("tbl")
                || model.starts_with("fact_")
            {
                (
                    Confidence::Medium,
                    format!("Heuristic: '{}' matches model naming pattern", model),
                    false,
                )
            } else {
                (
                    Confidence::Low,
                    format!(
                        "Ambiguous: '{}' does not match known model or component IDs",
                        model
                    ),
                    true,
                )
            }
        }
        RefType::Param(name) => {
            if param_ids.contains(name.as_str()) {
                (
                    Confidence::High,
                    format!("Exact match: '{}' is a known param ID", name),
                    false,
                )
            } else {
                (
                    Confidence::Medium,
                    format!(
                        "'{}' looks like a param but not found in current page",
                        name
                    ),
                    false,
                )
            }
        }
        RefType::UserProperty(prop) => (
            Confidence::High,
            format!("System user property: '{}'", prop),
            false,
        ),
        RefType::SystemVar(var) => (
            Confidence::High,
            format!("System variable: '{}'", var),
            false,
        ),
        RefType::Other(token) => (
            Confidence::Unresolved,
            format!(
                "Cannot resolve: '{}' does not match any known context",
                token
            ),
            true,
        ),
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
            // 尾点 token（如 "txtB."，split 后 rest 为空）语义等同裸 `${id}` 全组件
            // 引用，归一为 ComponentValue，维持 ComponentProperty property 非空不变式
            if rest.is_empty() {
                return RefType::ComponentValue(first.to_string());
            }
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
// ============================================================

/// 已知非组件容器键排除列表（M58.3 F1，附录 A 实测初值）。
///
/// - `effectStyles`/`conditionStyles`/`labelFields`/`stateFields`：附录 A 登记的
///   无行为证据样式/字段容器（`type` 取值是状态名而非组件类型），排除优先于形态判定；
/// - `actions`：动作容器（元素带 `id`/`actionType`、无 `type`，天然不满足组件形态），
///   排除以避免误入未识别计数（`tests/fixtures/test_project` 实测会误报）。
///
/// `buttons` 样本不足，按附录 A 决议不进排除列表。superpage 解析侧与 scanner 侧
/// 共用本列表，避免两份规则漂移。
pub(crate) const NON_COMPONENT_CONTAINER_KEYS: &[&str] = &[
    "actions",
    "effectStyles",
    "conditionStyles",
    "labelFields",
    "stateFields",
];

/// 组件数组形态判定（M58.3 F1）：value 是「子组件数组」当且仅当它是非空数组，
/// 且**每个**元素都是带字符串 `id` 和字符串 `type` 的对象。
///
/// - 字符串形态的多态键（如 action 的 `panel: "nextPage"`）天然不匹配；
/// - 混合形态数组（部分元素缺 `id`/`type`）整体判非组件，由 scanner 侧计入
///   `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 安全网。
///
/// superpage 解析侧与 scanner 裸 `Value` 递归共用本判定，避免两份规则漂移。
pub(crate) fn is_component_array(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(items) => {
            !items.is_empty()
                && items.iter().all(|item| {
                    item.as_object().is_some_and(|obj| {
                        obj.get("id").is_some_and(|v| v.is_string())
                            && obj.get("type").is_some_and(|v| v.is_string())
                    })
                })
        }
        _ => false,
    }
}

/// 递归提取组件的全部子容器：白名单键（components/panels/steps/comps）+
/// extra 中形态吻合的未知键（M58.3 F1 形态感知递归，json_path 由 scanner 侧逐层保留）。
fn extract_child_components(
    raw: &RawComponent,
    parent_id: Option<String>,
    components: &mut Vec<SpgComponent>,
    expressions: &mut Vec<ComponentExpr>,
) {
    for child in raw
        .components
        .iter()
        .chain(&raw.panels)
        .chain(&raw.steps)
        .chain(&raw.comps)
    {
        extract_components(child, parent_id.clone(), components, expressions);
    }
    // 形态感知递归：排除列表键与混合形态数组不递归（后者由 scanner 安全网计数）；
    // 单个元素反序列化失败只跳过该元素，不阻塞其余子树
    for (key, value) in raw.extra.iter() {
        if NON_COMPONENT_CONTAINER_KEYS.contains(&key.as_str()) || !is_component_array(value) {
            continue;
        }
        let Some(items) = value.as_array() else {
            continue;
        };
        for item in items {
            // 单个元素反序列化失败只跳过该元素，不阻塞其余子树。
            // 已知取舍：scanner 侧按原始 Value 判定（不反序列化），此处静默跳过
            // 会造成 scanner 计入组件而 superpage 丢元素且两侧都无信号；
            // 形态判定已过滤绝大多数不匹配样本，暂不为此新增诊断。
            if let Ok(child) = serde_json::from_value::<RawComponent>(item.clone()) {
                extract_components(&child, parent_id.clone(), components, expressions);
            }
        }
    }
}

fn extract_components(
    raw: &RawComponent,
    parent_id: Option<String>,
    components: &mut Vec<SpgComponent>,
    expressions: &mut Vec<ComponentExpr>,
) {
    if raw.id.is_empty() || raw.component_type.is_empty() {
        extract_child_components(raw, parent_id, components, expressions);
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
        "disableCondition",
        "defaultValueExp",
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
        "defaultValueExp",
    ];

    let fields = [
        ("value", &raw.value),
        ("defaultValue", &raw.default_value),
        ("defaultValueExp", &raw.default_value_exp),
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
        ("disableCondition", &raw.disable_condition),
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
                let parse_result = parse_expression_ast(val_str);
                expressions.push(ComponentExpr {
                    component_id: raw.id.clone(),
                    field: field_name.to_string(),
                    raw_expr: val_str.to_string(),
                    refs: parse_result.refs,
                    resolved_refs: parse_result.resolved_refs,
                    diagnostics: parse_result.diagnostics,
                });
            } else if !val_str.is_empty() {
                comp.properties
                    .insert(field_name.to_string(), val_str.to_string());
            }
        }
    }

    for (field_name, field_value) in [("source", &raw.source), ("dataSet", &raw.data_set)] {
        if let Some(val) = field_value
            && let Some(val_str) = json_scalar_to_string(val)
            && !val_str.is_empty()
        {
            comp.properties.insert(field_name.to_string(), val_str);
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
            wait_prev: a.wait_prev.clone(),
            condition: a.condition.clone(),
            condition_exp: a.condition_exp.clone(),
            target_component: a.target_component.clone(),
            panelbook: a.panelbook.clone(),
            panel: a.panel.clone(),
            dialog: a.dialog.clone(),
        })
        .collect();

    components.push(comp);

    extract_child_components(raw, Some(raw.id.clone()), components, expressions);
}

/// 将组件属性中的标量 JSON 值转换为字符串，数组取第一个字符串项
fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Array(arr) => arr
            .iter()
            .find_map(|item| item.as_str().map(|s| s.to_string())),
        _ => None,
    }
}

// ============================================================

mod expr_ast;
pub use expr_ast::ExprDiagnostic;
pub use expr_ast::ExprParseResult;
pub use expr_ast::parse_expression_ast;
pub use expr_ast::parse_expression_refs;
