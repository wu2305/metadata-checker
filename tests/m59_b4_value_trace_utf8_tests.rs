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

/// 裸 `${b}` 归一为 `ComponentValue("b", Bare)`，被追溯串只有 1 字节，
/// 而调用点构造的 pattern 是 7 字节的 `b.value`——旧实现在此 panic。
#[test]
fn bare_component_ref_shorter_than_pattern_does_not_panic() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "a", "value", 5).expect("a.value should be traceable");
    assert_eq!(trace.raw_expr, "=${b}");
    // codex 复审：`!is_empty()` 太松——只要不 panic、随便吐点什么都能过，
    // 而「吞掉内容」正是本文件要防的另一半。改成完整比对。
    // 语义归属仍在 A1b（tests/m59_a1b_component_value_ref_tests.rs）；
    // 这里重复一次是刻意的：panic 类缺陷的回归网必须自带完整期望值，
    // 不能依赖另一个文件恰好也在断言同一件事。
    assert_eq!(
        trace.expanded_expr, "=(param1)",
        "裸引用应完整展开，实际得到 {}",
        trace.expanded_expr
    );
}

/// 中文表达式经过替换后必须逐字符保留，不能出现逐字节转 char 的乱码。
#[test]
fn utf8_expression_survives_boundary_replacement() {
    let (meta, graph) = fixture();
    let trace =
        trace_value_source(&meta, &graph, "zh", "value", 5).expect("zh.value should be traceable");
    // codex 复审：原先这里是四条 `contains` / `!contains`。子串断言各自都很松——
    // 中文前后缀都在、没有替换字符、`b.value` 没了，中间照样可以是任何东西。
    // 逐字节转 char 的乱码有多种形态，只有完整比对能一次盖住全部。
    assert_eq!(
        trace.expanded_expr, "=CONCAT(\"合同金额：\", (param1), \"元\")",
        "中文表达式应逐字符原样保留、且 b.value 已展开，实际得到 {}",
        trace.expanded_expr
    );
    // 单独留一条：把「不得出现替换字符」这个意图写在测试里，
    // 将来期望值随语义变化被改动时，这条仍然守着乱码这一类回归。
    assert!(
        !trace.expanded_expr.contains('\u{fffd}'),
        "不应出现替换字符，实际得到 {}",
        trace.expanded_expr
    );
}
