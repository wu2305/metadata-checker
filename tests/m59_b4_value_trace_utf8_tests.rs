//! M59 阶段 B4：值追溯的切片越界 panic 与中文乱码回归测试。
//!
//! 两个缺陷都在 `dependency::replace_with_boundary`：
//! - pattern 长于被替换串时 `saturating_sub` 归零，`s_bytes[0..pat_len]` 越界 panic；
//! - `s_bytes[i] as char` 逐字节转换，把多字节 UTF-8 拆成乱码。

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

/// 裸 `${b}` 归一为 `ComponentValue("b")`，被追溯串只有 1 字节，
/// 而调用点构造的 pattern 是 7 字节的 `b.value`——旧实现在此 panic。
#[test]
fn bare_component_ref_shorter_than_pattern_does_not_panic() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "a", "value", 5).expect("a.value should be traceable");
    assert_eq!(trace.raw_expr, "=${b}");
    // 本文件只钉「不 panic、不吞内容」；裸引用**应当展开**这件事属于 A1b，
    // 断言在 tests/m59_a1b_component_value_ref_tests.rs。
    assert!(!trace.expanded_expr.is_empty());
}

/// 中文表达式经过替换后必须逐字符保留，不能出现逐字节转 char 的乱码。
#[test]
fn utf8_expression_survives_boundary_replacement() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "zh", "value", 5).expect("zh.value should be traceable");
    assert!(
        trace.expanded_expr.contains("合同金额："),
        "中文前缀应原样保留，实际得到 {}",
        trace.expanded_expr
    );
    assert!(
        trace.expanded_expr.contains('元'),
        "中文后缀应原样保留，实际得到 {}",
        trace.expanded_expr
    );
    assert!(
        !trace.expanded_expr.contains('\u{fffd}'),
        "不应出现替换字符，实际得到 {}",
        trace.expanded_expr
    );
    // 替换本身仍然生效：b.value 被展开成 b 自己的来源表达式。
    assert!(
        !trace.expanded_expr.contains("b.value"),
        "b.value 应被展开，实际得到 {}",
        trace.expanded_expr
    );
}
