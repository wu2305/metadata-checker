use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::{RefType, parse_expression_refs, parse_superpage};
use std::path::PathBuf;

// ============================================================
// 一、边界情况测试
// ============================================================

#[test]
fn test_empty_expression_string() {
    // 空字符串不应被解析为表达式
    let refs = parse_expression_refs("");
    assert_eq!(refs.len(), 0, "Empty string should have no refs");
}

#[test]
fn test_pure_constant() {
    let refs = parse_expression_refs("=123");
    assert_eq!(refs.len(), 0, "Pure constant should have no refs");
}

#[test]
fn test_pure_function_no_refs() {
    let refs = parse_expression_refs("=TODAY()");
    assert_eq!(refs.len(), 0, "TODAY() has no component/model/param refs");
}

#[test]
fn test_nested_if_expression() {
    let refs = parse_expression_refs(
        "=IF(IF(input1.value > 0, input2.value, 0) = input3.value, 'yes', 'no')",
    );

    // 应该解析出 input1, input2, input3
    assert!(refs.contains(&RefType::ComponentValue("input1".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input2".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input3".to_string())),);
}

#[test]
fn test_logical_operators() {
    let refs = parse_expression_refs(
        "=input1.value = input2.value AND input3.value != input4.value OR input5.value >= input6.value",
    );

    assert_eq!(refs.len(), 6);
    for i in 1..=6 {
        assert!(refs.contains(&RefType::ComponentValue(format!("input{}", i))),);
    }
}

#[test]
fn test_concat_multiple_args() {
    let refs = parse_expression_refs("=CONCAT('前缀', input2.value, '中缀', input3.value, '后缀')");

    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&RefType::ComponentValue("input2".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input3".to_string())),);
}

#[test]
fn test_is_null_expression() {
    let refs = parse_expression_refs("=IF(input1.value IS NULL, '空值', input1.value)");

    // input1 去重后只出现一次
    let input1_count = refs
        .iter()
        .filter(|r| matches!(r, RefType::ComponentValue(id) if id == "input1"))
        .count();
    assert_eq!(input1_count, 1);
}

#[test]
fn test_chinese_in_expression() {
    let _refs = parse_expression_refs("=${中文变量}");
    // 当前正则可能无法匹配中文变量名，这是一个已知限制
    // 但至少不应 panic
}

#[test]
fn test_list_column_reference() {
    let refs = parse_expression_refs("=list1.column1.value + list1.column2.value");

    // list1.column1.value 应该被解析为 ComponentValue("list1")
    let has_list = refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "list1"));
    assert!(has_list, "Should detect list1 reference");
}

#[test]
fn test_steps_step_reference() {
    let refs = parse_expression_refs("=steps1.step");

    let has_steps = refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "steps1"));
    assert!(has_steps, "Should detect steps1 reference");
}

#[test]
fn test_boundary_case_parsing() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // 验证空字符串 value 没有被当作表达式
    let text1_exprs: Vec<_> = meta
        .expressions
        .iter()
        .filter(|e| e.component_id == "text1")
        .collect();
    assert_eq!(
        text1_exprs.len(),
        0,
        "Empty string value should not be an expression"
    );

    // 验证纯静态文本 value 没有被当作表达式
    let text2_exprs: Vec<_> = meta
        .expressions
        .iter()
        .filter(|e| e.component_id == "text2")
        .collect();
    assert_eq!(
        text2_exprs.len(),
        0,
        "Static text should not be an expression"
    );

    // 验证常量表达式被正确提取
    let text3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text3" && e.field == "value")
        .expect("text3 constant expr should be extracted");
    assert_eq!(text3_expr.raw_expr, "=123");

    // 验证 TODAY() 表达式被正确提取
    let text4_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text4" && e.field == "value")
        .expect("text4 TODAY expr should be extracted");
    assert_eq!(text4_expr.raw_expr, "=TODAY()");
}

#[test]
fn test_cross_dependency() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // input3 依赖 input1 和 input2
    let input3_deps = graph.dependencies.get("input3").expect("input3 not found");
    let has_input1 = input3_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input1"));
    let has_input2 = input3_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input2"));
    assert!(has_input1);
    assert!(has_input2);

    // input7 只依赖 input1（分支依赖验证）
    let input7_deps = graph.dependencies.get("input7").expect("input7 not found");
    let has_input1_only = input7_deps
        .iter()
        .all(|r| matches!(r, RefType::ComponentValue(id) if id == "input1"));
    assert!(has_input1_only);
}

#[test]
fn test_multi_source_dependency() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // text7 依赖 input2 和 input3（多源依赖）
    let text7_deps = graph.dependencies.get("text7").expect("text7 not found");
    assert_eq!(
        text7_deps.len(),
        2,
        "text7 should depend on exactly 2 components"
    );
}

#[test]
fn test_independent_components_topological() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let order = graph.topological_sort();

    // text3 (纯常量) 和 text4 (TODAY()) 应该是独立的，可以在任何位置
    let pos_text3 = order.iter().position(|id| id == "text3");
    let pos_text4 = order.iter().position(|id| id == "text4");
    assert_ne!(pos_text3, None);
    assert_ne!(pos_text4, None);
}

// ============================================================
// 二、错误处理测试
// ============================================================

#[test]
fn test_file_not_found() {
    let path = PathBuf::from("tests/fixtures/nonexistent.spg");
    let result = parse_superpage(&path);
    assert!(result.is_err(), "Should error for nonexistent file");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(err_msg.contains("Failed to read file") || err_msg.contains("No such file"),);
}

#[test]
fn test_invalid_json() {
    let path = PathBuf::from("tests/fixtures/invalid_json.spg");
    let result = parse_superpage(&path);
    assert!(result.is_err(), "Should error for invalid JSON");
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("Failed to parse JSON"),
        "Error should mention JSON parsing: {}",
        err_msg
    );
}

#[test]
fn test_empty_file() {
    let path = PathBuf::from("tests/fixtures/empty_file.spg");
    let result = parse_superpage(&path);
    assert!(result.is_err(), "Should error for empty file");
}

#[test]
fn test_missing_canvas() {
    let path = PathBuf::from("tests/fixtures/missing_canvas.spg");
    let meta = parse_superpage(&path).expect("Should parse even without canvas");

    // 没有 canvas 应该返回空的 components
    assert_eq!(meta.components.len(), 0);
    assert_eq!(meta.expressions.len(), 0);
}

#[test]
fn test_incomplete_expression() {
    let path = PathBuf::from("tests/fixtures/incomplete_expression.spg");
    let meta = parse_superpage(&path).expect("Should parse even with incomplete expressions");

    // text1.value = "=input1." 应该被提取
    let text1_expr = meta.expressions.iter().find(|e| e.component_id == "text1");
    assert_ne!(
        text1_expr, None,
        "Incomplete expression should still be extracted"
    );

    // text2.value = "=$unknown.var" 应该被解析
    let text2_expr = meta.expressions.iter().find(|e| e.component_id == "text2");
    assert_ne!(
        text2_expr, None,
        "Unknown system var expression should be extracted"
    );
}
