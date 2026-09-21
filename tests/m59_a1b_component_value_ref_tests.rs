//! M59 阶段 A1b：值追溯的替换 pattern 必须与引用的来源文法一致。
//!
//! `classify_identifier` / `resolve_ref_type` 曾把 `x.value`、`x.step` 和裸 `${x}`
//! 三种来源文法都压成 `ComponentValue("x")`，值追溯的替换点没有信息可依：
//! - 裸 `${x}` 永远匹配不上，**静默不展开**（`a.value` 只追到 `=b` 就停了）；
//! - `x.step` 引用的是别的属性，一旦按 id 盲替就会被换成值的展开式。
//!
//! M59-1（A1 枚举元数半边）落地后 `RefType::ComponentValue` 携带来源文法
//! `ComponentValueForm`（Value / Suffix / Bare），替换点按文法取 pattern。
//! 本文件钉住行为契约：`RefType` 携带原始 token 前后，这些断言应当原样通过。

use metadata_checker::dependency::{DependencyGraph, trace_value_source};
use metadata_checker::superpage::{
    ComponentValueForm, RefType, parse_expression_refs, parse_superpage, parse_superpage_from_value,
};
use std::path::PathBuf;

fn fixture() -> (
    metadata_checker::superpage::SuperPageMetadata,
    DependencyGraph,
) {
    let path = PathBuf::from("tests/fixtures/value_trace_bare_ref_utf8.spg");
    let meta = parse_superpage(&path).expect("fixture should parse");
    let graph = DependencyGraph::new(&meta);
    (meta, graph)
}

/// 裸 `${b}` 必须展开到 `b` 自己的来源（`param1`），而不是停在 `=b`。
#[test]
fn bare_component_ref_expands_to_its_own_source() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "a", "value", 5).expect("a.value should be traceable");
    assert_eq!(trace.raw_expr, "=${b}");
    assert_eq!(
        trace.expanded_expr, "=(param1)",
        "裸引用应展开到 b 的来源参数，实际得到 {}",
        trace.expanded_expr
    );
}

/// `b.step` 取的是组件的 step 属性，不是值——不得被替换成 b 的值展开式。
#[test]
fn non_value_suffix_ref_is_left_alone() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "st", "value", 5).expect("st.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=b.step",
        "`.step` 引用的是别的属性，不该被值展开式替换，实际得到 {}",
        trace.expanded_expr
    );
}

/// 替换进去的是表达式片段，不带前导 `=`——否则会拼出 `CONCAT(x, =(param1))`。
#[test]
fn substituted_fragment_carries_no_leading_equals() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "zh", "value", 5).expect("zh.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=CONCAT(\"合同金额：\", (param1), \"元\")",
        "实际得到 {}",
        trace.expanded_expr
    );
    assert!(
        !trace.expanded_expr[1..].contains('='),
        "展开式内部不应再出现等号，实际得到 {}",
        trace.expanded_expr
    );
}

/// 裸引用的词边界判定必须挡住同前缀的更长标识符：替换 `b` 时不得吃掉 `bx.value`。
#[test]
fn bare_ref_does_not_match_longer_identifier_with_same_prefix() {
    let (meta, graph) = fixture();
    let trace = trace_value_source(&meta, &graph, "pre", "value", 5)
        .expect("pre.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=CONCAT(1, (param1))",
        "bx.value 应展开成 1、裸 b 应展开成 (param1)，两者互不串扰，实际得到 {}",
        trace.expanded_expr
    );
}

/// 组件 id 出现在**字符串字面量**里时不得被替换——那是文本，不是引用。
///
/// codex 复审发现（`6612a8b..5d7d973`）：裸 id 替换会匹配引号内的同名文本，
/// `=CONCAT("b", b.value)` 被展开成 `=CONCAT("(param1)", (param1))`，
/// **静默改变表达式语义**且无任何诊断。词边界判定挡不住它——引号不是词字符，
/// 边界检查照样通过。
#[test]
fn component_id_inside_string_literal_is_not_replaced() {
    let (meta, graph) = fixture();
    let trace = trace_value_source(&meta, &graph, "lit", "value", 5)
        .expect("lit.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=CONCAT(\"b\", (param1))",
        "字面量 \"b\" 必须原样保留，只有真正的引用 b.value 被展开，实际得到 {}",
        trace.expanded_expr
    );
}

/// 同一个坑在 `Param` / `ModelField` 侧走的是 `replace_with_boundary`，一并钉住。
#[test]
fn param_id_inside_string_literal_is_not_replaced() {
    let (meta, graph) = fixture();
    let trace = trace_value_source(&meta, &graph, "plit", "value", 5)
        .expect("plit.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=CONCAT(\"param1\", (param1))",
        "字面量 \"param1\" 必须原样保留，实际得到 {}",
        trace.expanded_expr
    );
}

/// A1b 原始 token 回归：`RefType::ComponentValue` 必须保留引用的来源文法，
/// `x.value` / `x.step` / 裸 `x` 三种形态分别映射到 Value / Suffix / Bare。
#[test]
fn component_value_ref_preserves_source_grammar_form() {
    let refs = parse_expression_refs("=b.value + b.step + b");
    assert_eq!(
        refs,
        vec![
            RefType::ComponentValue("b".to_string(), ComponentValueForm::Value),
            RefType::ComponentProperty("b".to_string(), "step".to_string()),
            RefType::ComponentValue("b".to_string(), ComponentValueForm::Bare),
        ],
        "三种来源文法必须各自保留，不得压成同一个 id"
    );
}

/// 非 value/step 后缀（`b.other` 等）归 `RefType::Other`，不进值追溯替换——
/// 这不是本批新行为：`83798da` 的 `classify_identifier` 对其它后缀同样返回
/// `Other`（不建依赖边、不替换）。此处钉住该已知边界：诊断产物欠账登记在
/// M59-2/query 层，不得在本批静默改成 ComponentValue 或凭空补边。
#[test]
fn non_value_step_suffix_stays_other_without_component_dependency() {
    let refs = parse_expression_refs("=CONCAT(b.other, 1)");
    assert_eq!(
        refs,
        vec![RefType::Other("b.other".to_string())],
        "非 value/step 后缀不得归为组件值引用（旧实现同口径）"
    );
}

/// A1b 原始 token 回归（parse 层归一路径）：裸 `${b}`（已知组件 id）经
/// `resolve_ref_type` 归一为 ComponentValue，来源文法必须是 Bare，
/// 替换点据此按裸 id 取词边界 pattern。
#[test]
fn bare_component_ref_normalizes_with_bare_form() {
    let raw = serde_json::json!({
        "canvas": {
            "components": [
                {"id": "exprA", "type": "text", "value": "${b}"},
                {"id": "b", "type": "text"}
            ]
        }
    });
    let meta = parse_superpage_from_value(raw).expect("fixture should parse");
    let expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "exprA")
        .expect("exprA expression should be extracted");
    assert_eq!(expr.raw_expr, "${b}");
    assert_eq!(
        expr.refs,
        vec![RefType::ComponentValue(
            "b".to_string(),
            ComponentValueForm::Bare
        )],
        "裸 ${{b}} 全组件引用归一为 ComponentValue 后必须携带 Bare 文法"
    );
}
