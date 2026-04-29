use crate::superpage::{ComponentExpr, RefType, SuperPageMetadata};
use serde_json;
use std::collections::HashMap;

/// 组件计算优先级分析模块
///
/// 分析 SuperPage 组件中多个表达式字段的优先级关系：
/// - defaultValue：初始默认值
/// - exp：表达式计算值
/// - calcCondition：条件计算值
///
/// 当多个字段同时存在时，按平台规则确定最终生效的值。
/// 组件计算优先级分析结果
#[derive(Debug, Clone)]
pub struct PriorityAnalysis {
    pub component_id: String,
    pub component_type: String,
    pub default_value_expr: Option<ComponentExpr>,
    pub calc_exp_expr: Option<ComponentExpr>,
    pub calc_condition_expr: Option<ComponentExpr>,
    pub priority_result: ExpDefaultValuePriority,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExpDefaultValuePriority {
    /// 只有 defaultValue，无 exp：按默认值计算
    OnlyDefaultValue,
    /// 只有 exp，无 defaultValue：按计算公式计算
    OnlyExp,
    /// exp 和 defaultValue 均存在，但无 calcCondition：exp 优先，defaultValue 被跳过
    ExpDominatesNoCalcCondition,
    /// exp、defaultValue 和 calcCondition 均存在：defaultValue 先算作为备用值
    /// 最终 value 取决于 calcCondition 的结果
    ExpWithCalcConditionDefaultValueFallback,
    /// 存在互斥冲突（两者均存在但无任何条件控制，提示风险）
    AmbiguousConflict,
}

/// 分析整个 SuperPage 中所有组件的计算优先级关系
pub fn analyze_priority(meta: &SuperPageMetadata) -> Vec<PriorityAnalysis> {
    let mut results = Vec::new();

    let mut expr_map: HashMap<(&str, &str), &ComponentExpr> = HashMap::new();
    for expr in &meta.expressions {
        expr_map.insert((&expr.component_id, &expr.field), expr);
    }

    for comp in &meta.components {
        let default_value_expr = expr_map
            .get(&(comp.id.as_str(), "defaultValue"))
            .map(|e| (*e).clone());

        let calc_exp_expr = expr_map
            .get(&(comp.id.as_str(), "exp"))
            .map(|e| (*e).clone())
            .or_else(|| {
                expr_map
                    .get(&(comp.id.as_str(), "value"))
                    .filter(|e| e.raw_expr.starts_with('=') || e.raw_expr.contains("${"))
                    .map(|e| (*e).clone())
            });

        let calc_condition_expr = expr_map
            .get(&(comp.id.as_str(), "calcCondition"))
            .map(|e| (*e).clone());
        let priority_result =
            determine_priority(&default_value_expr, &calc_exp_expr, &calc_condition_expr);

        // 只输出有表达式的组件
        if default_value_expr.is_some() || calc_exp_expr.is_some() || calc_condition_expr.is_some()
        {
            results.push(PriorityAnalysis {
                component_id: comp.id.clone(),
                component_type: comp.component_type.clone(),
                default_value_expr,
                calc_exp_expr,
                calc_condition_expr,
                priority_result,
            });
        }
    }

    results
}

/// 根据字段存在情况判定最终优先级
fn determine_priority(
    default_value: &Option<ComponentExpr>,
    calc_exp: &Option<ComponentExpr>,
    calc_condition: &Option<ComponentExpr>,
) -> ExpDefaultValuePriority {
    match (
        default_value.is_some(),
        calc_exp.is_some(),
        calc_condition.is_some(),
    ) {
        (true, false, false) => ExpDefaultValuePriority::OnlyDefaultValue,
        (false, true, false) => ExpDefaultValuePriority::OnlyExp,
        (false, true, true) => ExpDefaultValuePriority::OnlyExp,
        (true, true, false) => ExpDefaultValuePriority::ExpDominatesNoCalcCondition,
        (true, true, true) => ExpDefaultValuePriority::ExpWithCalcConditionDefaultValueFallback,
        (true, false, true) => ExpDefaultValuePriority::OnlyDefaultValue,
        /* 只有 calcCondition 但无 exp 和 defaultValue：无实际意义，忽略 */
        (false, false, true) => ExpDefaultValuePriority::OnlyDefaultValue,
        (false, false, false) => ExpDefaultValuePriority::OnlyDefaultValue,
    }
}

/// 生成人类可读的优先级报告
pub fn format_priority_human(analyses: &[PriorityAnalysis]) -> String {
    let mut lines = Vec::new();
    lines.push("=== 组件计算优先级分析 ===".to_string());
    lines.push(format!("共 {} 个组件含表达式", analyses.len()));
    lines.push("".to_string());

    for analysis in analyses {
        let icon = match &analysis.priority_result {
            ExpDefaultValuePriority::OnlyDefaultValue => "[D]",
            ExpDefaultValuePriority::OnlyExp => "[E]",
            ExpDefaultValuePriority::ExpDominatesNoCalcCondition => "[E>D]",
            ExpDefaultValuePriority::ExpWithCalcConditionDefaultValueFallback => "[E~D]",
            ExpDefaultValuePriority::AmbiguousConflict => "[!?]",
        };

        lines.push(format!(
            "{} {} ({}) — {:?}",
            icon, analysis.component_id, analysis.component_type, analysis.priority_result
        ));

        if let Some(dv) = &analysis.default_value_expr {
            lines.push(format!("    [defaultValue] {}", dv.raw_expr));
            if !dv.refs.is_empty() {
                lines.push(format!("        refs: {}", refs_to_string(&dv.refs)));
            }
        }

        if let Some(exp) = &analysis.calc_exp_expr {
            lines.push(format!("    [exp] {}", exp.raw_expr));
            if !exp.refs.is_empty() {
                lines.push(format!("        refs: {}", refs_to_string(&exp.refs)));
            }
        }

        if let Some(cc) = &analysis.calc_condition_expr {
            lines.push(format!("    [calcCondition] {}", cc.raw_expr));
            if !cc.refs.is_empty() {
                lines.push(format!("        refs: {}", refs_to_string(&cc.refs)));
            }
        }

        lines.push("".to_string());
    }

    lines.push("=== 图例 ===".to_string());
    lines.push("[D]   = 仅 defaultValue，无计算公式".to_string());
    lines.push("[E]   = 仅 exp 计算公式，无默认值".to_string());
    lines.push("[E>D] = exp 优先，defaultValue 被跳过（无 calcCondition）".to_string());
    lines.push("[E~D] = exp + defaultValue + calcCondition，defaultValue 先算作为备用".to_string());
    lines.push("".to_string());
    lines.push("=== 规则来源 ===".to_string());
    lines.push("源自 superpagecalculator.ts 的 calcp_defaultValue 逻辑：".to_string());
    lines.push(
        "1. 若 exp 存在且 calcCondition 不存在 → defaultValue 直接标记为 Calced（不计算）"
            .to_string(),
    );
    lines.push("2. 若 exp 存在且 calcCondition 存在 → 先计算 defaultValue 作为备用值".to_string());
    lines.push("3. 若 exp 不存在 → 正常计算 defaultValue".to_string());
    lines.push("4. 查看模式下（viewMode）defaultValue 只计算一次".to_string());

    lines.join("\n")
}

fn refs_to_string(refs: &[RefType]) -> String {
    refs.iter()
        .map(|r| match r {
            RefType::ComponentValue(id) => id.clone(),
            RefType::ComponentProperty(id, prop) => format!("{}.{}", id, prop),
            RefType::ModelField(model, field) => format!("{}.{}", model, field),
            RefType::Param(id) => id.clone(),
            RefType::UserProperty(prop) => prop.clone(),
            RefType::SystemVar(var) => var.clone(),
            RefType::Other(s) => s.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// 生成机器可消费的 JSON 优先级报告
#[allow(dead_code)]
fn format_priority_json(analyses: &[PriorityAnalysis]) -> String {
    let json_val = serde_json::json!({
        "schema_version": "1.0",
        "total": analyses.len(),
        "analyses": analyses.iter().map(|a| {
            let icon = match &a.priority_result {
                ExpDefaultValuePriority::OnlyDefaultValue => "[D]",
                ExpDefaultValuePriority::OnlyExp => "[E]",
                ExpDefaultValuePriority::ExpDominatesNoCalcCondition => "[E>D]",
                ExpDefaultValuePriority::ExpWithCalcConditionDefaultValueFallback => "[E~D]",
                ExpDefaultValuePriority::AmbiguousConflict => "[!?]",
            };
            serde_json::json!({
                "component_id": a.component_id,
                "component_type": a.component_type,
                "priority_icon": icon,
                "priority_result": format!("{:?}", a.priority_result),
                "default_value": a.default_value_expr.as_ref().map(|e| serde_json::json!({
                    "raw_expr": e.raw_expr,
                    "refs": e.refs.iter().map(|r| format!("{:?}", r)).collect::<Vec<String>>(),
                })),
                "exp": a.calc_exp_expr.as_ref().map(|e| serde_json::json!({
                    "raw_expr": e.raw_expr,
                    "refs": e.refs.iter().map(|r| format!("{:?}", r)).collect::<Vec<String>>(),
                })),
                "calc_condition": a.calc_condition_expr.as_ref().map(|e| serde_json::json!({
                    "raw_expr": e.raw_expr,
                    "refs": e.refs.iter().map(|r| format!("{:?}", r)).collect::<Vec<String>>(),
                })),
            })
        }).collect::<Vec<serde_json::Value>>(),
        "legend": {
            "[D]": "Only defaultValue, no exp",
            "[E]": "Only exp, no defaultValue",
            "[E>D]": "exp dominates, defaultValue skipped (no calcCondition)",
            "[E~D]": "exp + defaultValue + calcCondition, defaultValue as fallback",
            "[!?]": "Ambiguous conflict",
        },
    });
    serde_json::to_string_pretty(&json_val).unwrap_or_else(|_| "{}".to_string())
}
