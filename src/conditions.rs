use crate::superpage::{ExprDiagnostic, parse_expression_ast};
use crate::superpage::{RefType, SuperPageMetadata};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 条件类型枚举（技术来源）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType {
    VisibleCondition,
    EnableCondition,
    ActionConditionExp,
    ActionCondition,
    ItemFilter,
    CalcCondition,
    ValidExp,
    CalcExp,
    MaskCondition,
    SubmitCondition,
    SubmitPageCondition,
    DefaultPanelCondition,
    FieldExp,
    DefaultValueExp,
    SourceFilterExp,
    SourceFilterClause,
}

/// 条件所属对象类型（结构归属）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OwnerType {
    Page,
    Component,
    Action,
    ModelSource,
    FieldDefault,
}

/// 条件作用效果（业务语义）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectType {
    Show,
    Hide,
    Enable,
    Disable,
    Readonly,
    Execute,
    Filter,
    Compute,
    Validate,
    Mask,
    Submit,
    DefaultPanel,
}

/// 条件作用对象（语义主体）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubjectType {
    Component,
    Action,
    ModelSource,
    Field,
    Page,
}

/// 单条条件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionRecord {
    pub condition_id: String,
    pub condition_type: ConditionType,
    pub effect_type: EffectType,
    pub subject_type: SubjectType,
    pub raw_expr: String,
    pub normalized_expr: String,
    pub source_file: Option<String>,
    pub json_path: String,
    pub owner_type: OwnerType,
    pub owner_id: String,
    pub referenced_symbols: Vec<String>,
    pub diagnostics: Vec<ConditionDiagnostic>,
}

/// 条件诊断信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConditionDiagnostic {
    pub code: String,
    pub message: String,
}

impl From<&ExprDiagnostic> for ConditionDiagnostic {
    fn from(d: &ExprDiagnostic) -> Self {
        ConditionDiagnostic {
            code: d.code.clone(),
            message: d.message.clone(),
        }
    }
}

/// 将组件字段名映射为条件类型
fn field_to_condition_type(field: &str) -> ConditionType {
    match field {
        "visibleCondition" | "visible" => ConditionType::VisibleCondition,
        "enable" | "disableCondition" => ConditionType::EnableCondition,
        "conditionExp" => ConditionType::ActionConditionExp,
        "condition" => ConditionType::ActionCondition,
        "itemFilter" => ConditionType::ItemFilter,
        "calcCondition" => ConditionType::CalcCondition,
        "validExp" => ConditionType::ValidExp,
        "calcExp" => ConditionType::CalcExp,
        "maskCondition" => ConditionType::MaskCondition,
        "submitCondition" => ConditionType::SubmitCondition,
        "submitPageCondition" => ConditionType::SubmitPageCondition,
        "defaultPanelCondition" => ConditionType::DefaultPanelCondition,
        "defaultValue" | "defaultValueExp" => ConditionType::DefaultValueExp,
        "exp" | "value" | "text" | "formula" | "html" | "desc" | "placeholder" | "url"
        | "documentTitle" | "inputTitle" | "labelValue" | "panelName" | "rootPath"
        | "selectedCaption" | "confirmCaption" | "tip" | "badge" | "count" | "attrCaption"
        | "caption" | "defaultSelect" | "defaultCheck" | "maxLevel" | "submitField" => {
            ConditionType::FieldExp
        }
        _ => ConditionType::FieldExp,
    }
}

/// 根据条件类型推断效果类型
fn condition_type_to_effect_type(ct: &ConditionType) -> EffectType {
    match ct {
        ConditionType::VisibleCondition => EffectType::Show,
        ConditionType::EnableCondition => EffectType::Enable,
        ConditionType::ActionConditionExp | ConditionType::ActionCondition => EffectType::Execute,
        ConditionType::ItemFilter
        | ConditionType::SourceFilterExp
        | ConditionType::SourceFilterClause => EffectType::Filter,
        ConditionType::CalcCondition
        | ConditionType::CalcExp
        | ConditionType::FieldExp
        | ConditionType::DefaultValueExp => EffectType::Compute,
        ConditionType::ValidExp => EffectType::Validate,
        ConditionType::MaskCondition => EffectType::Mask,
        ConditionType::SubmitCondition | ConditionType::SubmitPageCondition => EffectType::Submit,
        ConditionType::DefaultPanelCondition => EffectType::DefaultPanel,
    }
}

/// 根据 owner_type 推断 subject_type
fn owner_type_to_subject_type(ot: &OwnerType) -> SubjectType {
    match ot {
        OwnerType::Component => SubjectType::Component,
        OwnerType::Action => SubjectType::Action,
        OwnerType::ModelSource => SubjectType::ModelSource,
        OwnerType::FieldDefault => SubjectType::Field,
        OwnerType::Page => SubjectType::Page,
    }
}

/// 将 RefType 转为可读的符号字符串
fn ref_type_to_string(r: &RefType) -> String {
    match r {
        RefType::ComponentValue(id) => format!("component:{}", id),
        RefType::ComponentProperty(id, prop) => format!("component:{}.{}", id, prop),
        RefType::ModelField(model, field) => format!("model:{}.{}", model, field),
        RefType::Param(name) => format!("param:{}", name),
        RefType::UserProperty(prop) => format!("user:{}", prop),
        RefType::SystemVar(name) => format!("system:{}", name),
        RefType::Other(s) => format!("other:{}", s),
    }
}

/// 递归收集组件 ID 到 JSON path 的映射
fn collect_json_paths(
    arr: &[serde_json::Value],
    path_prefix: &str,
    paths: &mut HashMap<String, String>,
) {
    for (index, comp) in arr.iter().enumerate() {
        let current = format!("{}[{}]", path_prefix, index);
        if let Some(comp_id) = comp.get("id").and_then(|v| v.as_str()) {
            paths.insert(comp_id.to_string(), current.clone());
        }
        for nested in ["components", "panels", "steps", "comps"] {
            if let Some(children) = comp.get(nested).and_then(|v| v.as_array()) {
                collect_json_paths(children, &format!("{}.{}", current, nested), paths);
            }
        }
    }
}

/// 递归扫描组件数组中的 action 条件（遍历 components/panels/steps/comps）
fn scan_actions_recursive(
    components: &[serde_json::Value],
    base_path: &str,
    source_file: Option<&str>,
) -> Vec<ConditionRecord> {
    let mut records = Vec::new();
    for (comp_idx, comp) in components.iter().enumerate() {
        let current_path = format!("{}[{}]", base_path, comp_idx);
        let comp_id = comp.get("id").and_then(|v| v.as_str()).unwrap_or("");

        // 扫描当前组件的 actions
        if let Some(actions) = comp.get("actions").and_then(|v| v.as_array()) {
            for (action_idx, action) in actions.iter().enumerate() {
                let action_id = action.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let action_base_path = format!("{}.actions[{}]", current_path, action_idx);

                if let Some(cond_exp) = action.get("conditionExp").and_then(|v| v.as_str()) {
                    let owner_type = OwnerType::Action;
                    let subject_type = owner_type_to_subject_type(&owner_type);
                    if cond_exp.is_empty() {
                        records.push(ConditionRecord {
                            condition_id: format!("{}#{}#conditionExp", comp_id, action_id),
                            condition_type: ConditionType::ActionConditionExp,
                            effect_type: EffectType::Execute,
                            subject_type: subject_type.clone(),
                            raw_expr: "".to_string(),
                            normalized_expr: "".to_string(),
                            source_file: source_file.map(|s| s.to_string()),
                            json_path: format!("{}.conditionExp", action_base_path),
                            owner_type: owner_type.clone(),
                            owner_id: format!("{}:{}", comp_id, action_id),
                            referenced_symbols: Vec::new(),
                            diagnostics: vec![ConditionDiagnostic {
                                code: "EMPTY_CONDITION".to_string(),
                                message: "Action conditionExp 为空".to_string(),
                            }],
                        });
                    } else {
                        let parse_result = parse_expression_ast(cond_exp);
                        let refs = parse_result.refs.iter().map(ref_type_to_string).collect();
                        records.push(ConditionRecord {
                            condition_id: format!("{}#{}#conditionExp", comp_id, action_id),
                            condition_type: ConditionType::ActionConditionExp,
                            effect_type: EffectType::Execute,
                            subject_type: subject_type.clone(),
                            raw_expr: cond_exp.to_string(),
                            normalized_expr: cond_exp.trim().to_string(),
                            source_file: source_file.map(|s| s.to_string()),
                            json_path: format!("{}.conditionExp", action_base_path),
                            owner_type: owner_type.clone(),
                            owner_id: format!("{}:{}", comp_id, action_id),
                            referenced_symbols: refs,
                            diagnostics: parse_result
                                .diagnostics
                                .iter()
                                .map(ConditionDiagnostic::from)
                                .collect(),
                        });
                    }
                }

                if let Some(cond) = action.get("condition").and_then(|v| v.as_str()) {
                    if !cond.is_empty() {
                        let owner_type = OwnerType::Action;
                        let subject_type = owner_type_to_subject_type(&owner_type);
                        let parse_result = parse_expression_ast(cond);
                        let refs = parse_result.refs.iter().map(ref_type_to_string).collect();
                        records.push(ConditionRecord {
                            condition_id: format!("{}#{}#condition", comp_id, action_id),
                            condition_type: ConditionType::ActionCondition,
                            effect_type: EffectType::Execute,
                            subject_type: subject_type.clone(),
                            raw_expr: cond.to_string(),
                            normalized_expr: cond.trim().to_string(),
                            source_file: source_file.map(|s| s.to_string()),
                            json_path: format!("{}.condition", action_base_path),
                            owner_type: owner_type.clone(),
                            owner_id: format!("{}:{}", comp_id, action_id),
                            referenced_symbols: refs,
                            diagnostics: parse_result
                                .diagnostics
                                .iter()
                                .map(ConditionDiagnostic::from)
                                .collect(),
                        });
                    }
                }
            }
        }

        // 递归进入嵌套子组件
        for nested in ["components", "panels", "steps", "comps"] {
            if let Some(children) = comp.get(nested).and_then(|v| v.as_array()) {
                let nested_path = format!("{}.{}", current_path, nested);
                records.extend(scan_actions_recursive(children, &nested_path, source_file));
            }
        }
    }
    records
}

/// 扫描 SuperPage 中所有条件表达式
pub fn scan_conditions(spg: &SuperPageMetadata, source_file: Option<&str>) -> Vec<ConditionRecord> {
    let mut records = Vec::new();
    let mut component_paths: HashMap<String, String> = HashMap::new();

    if let Some(components) = spg
        .raw
        .get("canvas")
        .and_then(|c| c.get("components"))
        .and_then(|v| v.as_array())
    {
        collect_json_paths(components, "canvas.components", &mut component_paths);
    }

    // 1. 从已解析的 expressions 提取组件级条件
    for (idx, expr) in spg.expressions.iter().enumerate() {
        let condition_type = field_to_condition_type(&expr.field);
        let effect_type = condition_type_to_effect_type(&condition_type);
        let json_path_base = component_paths
            .get(&expr.component_id)
            .cloned()
            .unwrap_or_else(|| format!("canvas.components[id='{}']", expr.component_id));
        let json_path = format!("{}.{}", json_path_base, expr.field);

        let referenced_symbols: Vec<String> = expr.refs.iter().map(ref_type_to_string).collect();

        let mut diagnostics = Vec::new();
        if expr.raw_expr.is_empty() {
            diagnostics.push(ConditionDiagnostic {
                code: "EMPTY_CONDITION".to_string(),
                message: "条件表达式为空字符串".to_string(),
            });
        }
        for diag in &expr.diagnostics {
            diagnostics.push(ConditionDiagnostic::from(diag));
        }

        let normalized_expr = expr.raw_expr.trim().to_string();
        let owner_type = OwnerType::Component;
        let subject_type = owner_type_to_subject_type(&owner_type);

        records.push(ConditionRecord {
            condition_id: format!("{}#{}#{}", expr.component_id, expr.field, idx),
            condition_type,
            effect_type,
            subject_type,
            raw_expr: expr.raw_expr.clone(),
            normalized_expr,
            source_file: source_file.map(|s| s.to_string()),
            json_path,
            owner_type,
            owner_id: expr.component_id.clone(),
            referenced_symbols,
            diagnostics,
        });
    }

    // 2. 从 actions 提取动作条件（递归遍历嵌套组件）
    if let Some(components) = spg
        .raw
        .get("canvas")
        .and_then(|c| c.get("components"))
        .and_then(|v| v.as_array())
    {
        records.extend(scan_actions_recursive(
            components,
            "canvas.components",
            source_file,
        ));
    }

    // 3. 从 sources 提取 filter
    if let Some(sources) = spg.raw.get("sources").and_then(|v| v.as_array()) {
        for (src_idx, src) in sources.iter().enumerate() {
            let src_id = src.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let base_path = format!("sources[{}]", src_idx);

            if let Some(filter) = src.get("filter") {
                if let Some(clauses) = filter.get("clauses").and_then(|v| v.as_array()) {
                    for (clause_idx, clause) in clauses.iter().enumerate() {
                        let clause_path = format!("{}.filter.clauses[{}]", base_path, clause_idx);
                        let owner_type = OwnerType::ModelSource;
                        let subject_type = owner_type_to_subject_type(&owner_type);

                        if let Some(exp) = clause.get("exp").and_then(|v| v.as_str()) {
                            let parse_result = parse_expression_ast(exp);
                            let refs = parse_result.refs.iter().map(ref_type_to_string).collect();
                            records.push(ConditionRecord {
                                condition_id: format!("{}#filter#{}#exp", src_id, clause_idx),
                                condition_type: ConditionType::SourceFilterExp,
                                effect_type: EffectType::Filter,
                                subject_type: subject_type.clone(),
                                raw_expr: exp.to_string(),
                                normalized_expr: exp.trim().to_string(),
                                source_file: source_file.map(|s| s.to_string()),
                                json_path: format!("{}.exp", clause_path),
                                owner_type: owner_type.clone(),
                                owner_id: src_id.to_string(),
                                referenced_symbols: refs,
                                diagnostics: parse_result
                                    .diagnostics
                                    .iter()
                                    .map(ConditionDiagnostic::from)
                                    .collect(),
                            });
                        }

                        if let Some(left_exp) = clause.get("leftExp").and_then(|v| v.as_str()) {
                            let right_value = clause
                                .get("rightValue")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let operator = clause
                                .get("operator")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let combined = format!("{} {} {}", left_exp, operator, right_value);
                            let parse_result = parse_expression_ast(left_exp);
                            let refs = parse_result.refs.iter().map(ref_type_to_string).collect();
                            records.push(ConditionRecord {
                                condition_id: format!("{}#filter#{}#clause", src_id, clause_idx),
                                condition_type: ConditionType::SourceFilterClause,
                                effect_type: EffectType::Filter,
                                subject_type: subject_type.clone(),
                                raw_expr: combined.clone(),
                                normalized_expr: combined.trim().to_string(),
                                source_file: source_file.map(|s| s.to_string()),
                                json_path: format!("{}.leftExp", clause_path),
                                owner_type: owner_type.clone(),
                                owner_id: src_id.to_string(),
                                referenced_symbols: refs,
                                diagnostics: parse_result
                                    .diagnostics
                                    .iter()
                                    .map(ConditionDiagnostic::from)
                                    .collect(),
                            });
                        }
                    }
                }
            }
        }
    }

    records
}
