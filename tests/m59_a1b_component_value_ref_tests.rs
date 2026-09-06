//! M59 阶段 A1b：`RefType::ComponentValue` 丢掉原始引用后缀导致的值追溯错配。
//!
//! `classify_identifier` / `resolve_ref_type` 把 `x.value`、`x.step` 和裸 `${x}`
//! 三种来源文法都压成 `ComponentValue("x")`，而值追溯的替换点固定构造
//! `format!("{x}.value")` 作 pattern。后果是：
//! - 裸 `${x}` 永远匹配不上，**静默不展开**（`a.value` 只追到 `=b` 就停了）；
//! - `x.step` 引用的是别的属性，一旦按 id 盲替就会被换成值的展开式。
//!
//! 在身份文法（A1）定稿前，替换点按后缀显式判定；本文件钉住这个行为契约，
//! 等 `RefType` 真正携带原始 token 后，这些断言应当原样通过。

use metadata_checker::dependency::{DependencyGraph, trace_value_source};
use metadata_checker::superpage::parse_superpage;
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
    let trace =
        trace_value_source(&meta, &graph, "pre", "value", 5).expect("pre.value should be traceable");
    assert_eq!(
        trace.expanded_expr, "=CONCAT(1, (param1))",
        "bx.value 应展开成 1、裸 b 应展开成 (param1)，两者互不串扰，实际得到 {}",
        trace.expanded_expr
    );
}
