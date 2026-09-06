use crate::superpage::{ExprDiagnostic, parse_expression_ast};
use crate::superpage::{RefType, SuperPageMetadata};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
        // 字段名为空（裸 `${modelN}`）时只写模型名——`model:modelN.` 尾点串会一路
        // 传到图构建，被切成 `field:modelN.` 垃圾节点，也让消费方误以为有个空名字段
        RefType::ModelField(model, field) if field.is_empty() => format!("model:{}", model),
        RefType::ModelField(model, field) => format!("model:{}.{}", model, field),
        RefType::Param(name) => format!("param:{}", name),
        RefType::UserProperty(prop) => format!("user:{}", prop),
        RefType::SystemVar(name) => format!("system:{}", name),
        RefType::Other(s) => format!("other:{}", s),
    }
}

/// 页面级引用上下文：组件 id / 数据源 id / 参数 id 三个集合。
///
/// M58.3 相邻缺口修复：`scan_conditions` 的动作条件与 source filter 走的是裸
/// `parse_expression_ast`，只有正则启发式，没有页面上下文——`${txtB.txt}`（txtB 是
/// 组件）会被判成 `ModelField("txtB", "txt")`，最终在图里建一条指向不存在的
/// `model:txtB.txt` 的悬挂边。组件表达式（`spg.expressions`）在
/// `resolve_expression_refs_with_context` 里已经归一过，这里给另外两类条件补上
/// **同一份**归一函数，三条路径的符号口径因此一致。
struct PageRefContext<'a> {
    component_ids: HashSet<&'a str>,
    source_ids: HashSet<&'a str>,
    param_ids: HashSet<&'a str>,
}

impl<'a> PageRefContext<'a> {
    fn from_spg(spg: &'a SuperPageMetadata) -> Self {
        Self {
            component_ids: spg.components.iter().map(|c| c.id.as_str()).collect(),
            source_ids: spg.sources.iter().map(|s| s.id.as_str()).collect(),
            param_ids: spg.params.iter().map(|p| p.id.as_str()).collect(),
        }
    }

    /// 把裸解析出的引用按页面上下文归一后转成符号串。
    fn symbols(&self, refs: &[RefType]) -> Vec<String> {
        refs.iter()
            .map(|r| {
                let (resolved, _corrected) = crate::superpage::resolve_ref_type(
                    r,
                    &self.component_ids,
                    &self.source_ids,
                    &self.param_ids,
                );
                ref_type_to_string(&resolved)
            })
            .collect()
    }
}

/// 递归收集组件 ID 到 JSON path 的映射，供条件扫描和输出复用。
///
/// M58.3 复核返修 P1-5：白名单四键之外，extra 键按与 superpage 提取侧**同一
/// 形态判定**（`is_component_array` + `NON_COMPONENT_CONTAINER_KEYS`）递归——
/// extra 键组件（columns/operateButtons/grid 等）的 json_path 因此真实可查，
/// 不再退回 `canvas.components[id='...']` 合成兜底
pub(crate) fn collect_json_paths(
    arr: &[serde_json::Value],
    path_prefix: &str,
    paths: &mut HashMap<String, String>,
) {
    for (index, comp) in arr.iter().enumerate() {
        collect_json_paths_from_node(comp, &format!("{}[{}]", path_prefix, index), paths);
    }
}

/// 单个组件节点的 json_path 登记与子树递归。
///
/// M58.3 相邻缺口修复：入口从「`canvas.components` 数组」上移到「任意组件节点」——
/// `extract_components` 的遍历起点是 `canvas` 对象本身，四个白名单键（含
/// `canvas.panels` / `canvas.steps` / `canvas.comps`）和 canvas 级 extra 组件数组
/// 都在它的覆盖面内。旧入口只喂 `canvas.components`，这些分支下的组件在
/// `component_paths` 里查不到，只能退回 `canvas.components[id='...']` 合成串——
/// 那不是能在原始 JSON 里定位到的路径，违反 raw JSON locator 契约。
pub(crate) fn collect_json_paths_from_node(
    node: &serde_json::Value,
    json_path: &str,
    paths: &mut HashMap<String, String>,
) {
    if let Some(comp_id) = node.get("id").and_then(|v| v.as_str()) {
        paths.insert(comp_id.to_string(), json_path.to_string());
    }
    for nested in ["components", "panels", "steps", "comps"] {
        if let Some(children) = node.get(nested).and_then(|v| v.as_array()) {
            collect_json_paths(children, &format!("{}.{}", json_path, nested), paths);
        }
    }
    // 形态感知递归（与 superpage 提取侧/scanner 裸 Value 递归同一份判定函数）
    let Some(obj) = node.as_object() else {
        return;
    };
    for (key, value) in obj {
        if crate::superpage::NON_COMPONENT_CONTAINER_KEYS.contains(&key.as_str())
            || ["components", "panels", "steps", "comps"].contains(&key.as_str())
        {
            continue;
        }
        if crate::superpage::is_component_array(value)
            && let Some(children) = value.as_array()
        {
            collect_json_paths(children, &format!("{}.{}", json_path, key), paths);
        }
    }
}

/// 从 superpage 提取结果（`spg.components`）收集动作条件记录。
///
/// M58.3 复核返修 P1-5：驱动源从第三份裸 JSON 递归（只走白名单四键）改为
/// 提取结果——白名单 + F1 形态感知递归发现的组件（含 extra 键组件）全覆盖，
/// 组件到达口径与图构建严格一致；json_path 取自同一形态判定的
/// `collect_json_paths`，extra 键组件的动作条件得到真实路径
fn collect_action_conditions(
    spg: &SuperPageMetadata,
    component_paths: &HashMap<String, String>,
    ref_context: &PageRefContext<'_>,
    source_file: Option<&str>,
) -> Vec<ConditionRecord> {
    let mut records = Vec::new();
    for comp in &spg.components {
        let comp_id = comp.id.as_str();
        let comp_base_path = component_paths
            .get(comp_id)
            .cloned()
            .unwrap_or_else(|| format!("canvas.components[id='{}']", comp_id));
        for (action_idx, action) in comp.actions.iter().enumerate() {
            let action_id = action.id.as_str();
            let action_base_path = format!("{}.actions[{}]", comp_base_path, action_idx);

            if let Some(cond_exp) = &action.condition_exp {
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
                    let refs = ref_context.symbols(&parse_result.refs);
                    records.push(ConditionRecord {
                        condition_id: format!("{}#{}#conditionExp", comp_id, action_id),
                        condition_type: ConditionType::ActionConditionExp,
                        effect_type: EffectType::Execute,
                        subject_type: subject_type.clone(),
                        raw_expr: cond_exp.clone(),
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

            if let Some(cond) = &action.condition
                && !cond.is_empty()
            {
                let owner_type = OwnerType::Action;
                let subject_type = owner_type_to_subject_type(&owner_type);
                let parse_result = parse_expression_ast(cond);
                let refs = ref_context.symbols(&parse_result.refs);
                records.push(ConditionRecord {
                    condition_id: format!("{}#{}#condition", comp_id, action_id),
                    condition_type: ConditionType::ActionCondition,
                    effect_type: EffectType::Execute,
                    subject_type: subject_type.clone(),
                    raw_expr: cond.clone(),
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
    records
}

/// 扫描 SuperPage 中所有条件表达式
pub fn scan_conditions(spg: &SuperPageMetadata, source_file: Option<&str>) -> Vec<ConditionRecord> {
    let mut records = Vec::new();
    let mut component_paths: HashMap<String, String> = HashMap::new();
    // 动作条件与 source filter 的裸解析结果都要经这里归一（组件表达式已在
    // superpage 解析末尾归一过，其 refs 再走一次是幂等的）
    let ref_context = PageRefContext::from_spg(spg);

    // 与 extract_components 同起点：canvas 对象本身（其四个白名单键与 extra
    // 组件数组一并覆盖），而不是只喂 canvas.components 一个分支
    if let Some(canvas) = spg.raw.get("canvas") {
        collect_json_paths_from_node(canvas, "canvas", &mut component_paths);
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

    // 2. 从 actions 提取动作条件（驱动源为 superpage 提取结果：白名单 +
    //    形态感知递归发现的组件全覆盖，组件到达口径与图构建一致）
    records.extend(collect_action_conditions(
        spg,
        &component_paths,
        &ref_context,
        source_file,
    ));

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
                            let refs = ref_context.symbols(&parse_result.refs);
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
                            let refs = ref_context.symbols(&parse_result.refs);
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
