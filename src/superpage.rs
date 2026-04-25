use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

// ============================================================
// JSON 反序列化结构体
// ============================================================

#[derive(Debug, Deserialize)]
struct RawSuperPage {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    theme: Option<String>,
    #[serde(default)]
    params: Vec<RawParam>,
    #[serde(default)]
    sources: Vec<RawSource>,
    #[serde(default)]
    canvas: Option<RawComponent>,
}

#[derive(Debug, Deserialize)]
struct RawParam {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    desc: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawSource {
    #[serde(default)]
    id: String,
    #[serde(rename = "modelType", default)]
    model_type: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawAction {
    #[serde(default)]
    id: String,
    #[serde(rename = "actionType", default)]
    action_type: String,
    #[serde(rename = "triggerType", default)]
    trigger_type: String,
    #[serde(default)]
    submit_range: Option<String>,
    #[serde(rename = "submitComponent", default)]
    submit_component: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawComponent {
    #[serde(default)]
    id: String,
    #[serde(rename = "type", default)]
    component_type: String,
    #[serde(default)]
    value: Option<serde_json::Value>,
    #[serde(rename = "defaultValue", default)]
    default_value: Option<serde_json::Value>,
    #[serde(default)]
    visible: Option<serde_json::Value>,
    #[serde(default)]
    exp: Option<serde_json::Value>,
    #[serde(default)]
    enable: Option<serde_json::Value>,
    #[serde(default)]
    text: Option<serde_json::Value>,
    #[serde(default)]
    formula: Option<serde_json::Value>,
    #[serde(default)]
    html: Option<serde_json::Value>,
    #[serde(rename = "submitField", default)]
    submit_field: Option<String>,
    #[serde(rename = "submitData", default)]
    submit_data: Option<bool>,
    #[serde(default)]
    actions: Vec<RawAction>,
    #[serde(default)]
    components: Vec<RawComponent>,
    #[serde(default)]
    panels: Vec<RawComponent>,
    #[serde(default)]
    steps: Vec<RawComponent>,
    #[serde(default)]
    comps: Vec<RawComponent>,
}

// ============================================================
// 对外暴露的公共类型
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub struct SpgComponent {
    pub id: String,
    pub component_type: String,
    pub parent_id: Option<String>,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RefType {
    ComponentValue(String),
    ComponentProperty(String, String),
    ModelField(String, String),
    Param(String),
    UserProperty(String),
    SystemVar(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComponentExpr {
    pub component_id: String,
    pub field: String,
    pub raw_expr: String,
    pub refs: Vec<RefType>,
}

#[derive(Debug, Default, Clone)]
pub struct SuperPageMetadata {
    pub version: Option<String>,
    pub theme: Option<String>,
    pub params: Vec<SpgParam>,
    pub sources: Vec<SpgSource>,
    pub components: Vec<SpgComponent>,
    pub expressions: Vec<ComponentExpr>,
    pub raw: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct SpgParam {
    pub id: String,
    pub name: String,
    pub desc: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpgSource {
    pub id: String,
    pub model_type: Option<String>,
    pub path: Option<String>,
}

// ============================================================
// 解析入口
// ============================================================

pub fn parse_superpage(path: &Path) -> Result<SuperPageMetadata> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;
    let raw: RawSuperPage = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse JSON from: {}", path.display()))?;

    let raw_value: serde_json::Value = serde_json::from_str(&content)
        .unwrap_or(serde_json::Value::Null);

    let mut meta = SuperPageMetadata::default();
    meta.raw = raw_value;
    meta.version = raw.version;
    meta.theme = raw.theme;

    meta.params = raw.params.into_iter().map(|p| SpgParam {
        id: p.id,
        name: p.name,
        desc: p.desc,
        default_value: p.value,
    }).collect();

    meta.sources = raw.sources.into_iter().map(|s| SpgSource {
        id: s.id,
        model_type: s.model_type,
        path: s.path,
    }).collect();

    if let Some(canvas) = raw.canvas {
        extract_components(
&canvas,
            None,
            &mut meta.components,
            &mut meta.expressions,
        );
    }

    Ok(meta)
}

// ============================================================
// 递归提取组件
// ============================================================

fn extract_components(
    raw: &RawComponent,
    parent_id: Option<String>,
    components: &mut Vec<SpgComponent>,
    expressions: &mut Vec<ComponentExpr>,
) {
    if raw.id.is_empty() || raw.component_type.is_empty() {
        for child in &raw.components { extract_components(child, parent_id.clone(), components, expressions); }
        for child in &raw.panels { extract_components(child, parent_id.clone(), components, expressions); }
        for child in &raw.steps { extract_components(child, parent_id.clone(), components, expressions); }
        for child in &raw.comps { extract_components(child, parent_id.clone(), components, expressions); }
        return;
    }

    let mut comp = SpgComponent {
        id: raw.id.clone(),
        component_type: raw.component_type.clone(),
        parent_id: parent_id.clone(),
        properties: HashMap::new(),
    };

    // 总是表达式字段：所有非空值都视为表达式
    let always_expr_fields = [
        "exp", "itemFilter", "visibleCondition", "validExp",
        "calcCondition", "calcExp", "maskCondition",
        "submitCondition", "submitPageCondition", "defaultPanelCondition",
    ];
    // 条件表达式字段：包含 = 或 ${ 时才视为表达式
    let conditional_expr_fields = [
        "value", "defaultValue", "visible", "enable",
        "text", "formula", "html", "desc",
        "placeholder", "url", "documentTitle", "inputTitle",
        "labelValue", "panelName", "rootPath", "selectedCaption",
        "confirmCaption", "tip", "badge", "count",
        "attrCaption", "caption", "defaultSelect", "defaultCheck",
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
                comp.properties.insert(field_name.to_string(), val_str.to_string());
                let refs = parse_expression_refs(val_str);
                expressions.push(ComponentExpr {
                    component_id: raw.id.clone(),
                    field: field_name.to_string(),
                    raw_expr: val_str.to_string(),
                    refs,
                });
            } else if !val_str.is_empty() {
                comp.properties.insert(field_name.to_string(), val_str.to_string());
            }
        }
    }

    // Handle submitField separately (always a string, no expression parsing needed)
    if let Some(ref submit_field) = raw.submit_field {
        comp.properties.insert("submitField".to_string(), submit_field.clone());
    }

    components.push(comp);

    let parent = Some(raw.id.clone());
    for child in &raw.components { extract_components(child, parent.clone(), components, expressions); }
    for child in &raw.panels { extract_components(child, parent.clone(), components, expressions); }
    for child in &raw.steps { extract_components(child, parent.clone(), components, expressions); }
    for child in &raw.comps { extract_components(child, parent.clone(), components, expressions); }
}

// ============================================================
// 表达式引用解析
// ============================================================

pub fn parse_expression_refs(expr: &str) -> Vec<RefType> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();

    let re = Regex::new(r"[a-zA-Z@$][a-zA-Z0-9_]*(?:\.[a-zA-Z0-9_]+)+|\$[a-zA-Z0-9_]+|\bparam\w+\b|\bmodel\d+\b").unwrap();

    for mat in re.find_iter(expr) {
        let token = mat.as_str();

        let lower = token.to_lowercase();
        if ["if", "null", "true", "false", "undefined", "and", "or", "not", "is", "in", "between"].contains(&lower.as_str()) {
            continue;
        }
        if token.parse::<f64>().is_ok() {
            continue;
        }
        if token.starts_with('\'') || token.starts_with('"') {
            continue;
        }
        if seen.contains(token) {
            continue;
        }

        if is_in_string_literal(expr, mat.start()) {
            continue;
        }

        seen.insert(token.to_string());
        let ref_type = classify_ref(token);
        refs.push(ref_type);
    }

    refs
}

fn is_in_string_literal(expr: &str, pos: usize) -> bool {
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    for (i, c) in expr.char_indices() {
        if i >= pos {
            break;
        }
        if c == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
        } else if c == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
        }
    }

    in_single_quote || in_double_quote
}

fn classify_ref(token: &str) -> RefType {
    let token = token.trim();

    if token.starts_with('$') {
        let parts: Vec<&str> = token[1..].split('.').collect();
        if parts.len() >= 2 {
            return RefType::UserProperty(token[1..].to_string());
        }
        return RefType::SystemVar(token.to_string());
    }

    let parts: Vec<&str> = token.split('.').collect();

    if parts.len() >= 2 {
        let first = parts[0];
        if first.starts_with("model") {
            let field = parts[1..].join(".");
            return RefType::ModelField(first.to_string(), field);
        }
        if parts.last() == Some(&"value") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.last() == Some(&"step") {
            return RefType::ComponentValue(first.to_string());
        }
        if parts.len() >= 3 && parts[1] == "checked" && parts[2] == "value" {
            return RefType::ComponentProperty(first.to_string(), "checked.value".to_string());
        }
        return RefType::Other(token.to_string());
    }

    if token.starts_with("param") {
        return RefType::Param(token.to_string());
    }

    if !token.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true) {
        return RefType::ComponentValue(token.to_string());
    }

    RefType::Other(token.to_string())
}
