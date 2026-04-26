use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::{RefType, parse_expression_refs, parse_superpage};
use std::path::PathBuf;

// ============================================================
// 一、额外字段表达式测试 (text, html, formula)
// ============================================================

#[test]
fn test_text_field_expression() {
    let path = PathBuf::from("tests/fixtures/additional_fields.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let text1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text1" && e.field == "text")
        .expect("text1.text expression should be extracted");

    assert_eq!(text1_expr.raw_expr, "=input1.value");
    assert_eq!(
        text1_expr
            .refs
            .contains(&RefType::ComponentValue("input1".to_string())),
        true
    );
}

#[test]
fn test_html_field_expression() {
    let path = PathBuf::from("tests/fixtures/additional_fields.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let html1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "html1" && e.field == "html")
        .expect("html1.html expression should be extracted");

    assert_eq!(html1_expr.raw_expr, "=${model1.fieldA}");
    let has_model = html1_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "fieldA"));
    assert_eq!(has_model, true);
}

#[test]
fn test_formula_field_expression() {
    let path = PathBuf::from("tests/fixtures/additional_fields.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let formula1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "formula1" && e.field == "formula")
        .expect("formula1.formula expression should be extracted");

    assert_eq!(formula1_expr.raw_expr, "=input2.value + input3.value");
    assert_eq!(
        formula1_expr
            .refs
            .contains(&RefType::ComponentValue("input2".to_string())),
        true
    );
    assert_eq!(
        formula1_expr
            .refs
            .contains(&RefType::ComponentValue("input3".to_string())),
        true
    );
}

// ============================================================
// 二、缺少 version 字段测试
// ============================================================

#[test]
fn test_missing_version_field() {
    let path = PathBuf::from("tests/fixtures/missing_version.spg");
    let meta = parse_superpage(&path).expect("Should parse even without version");

    assert_eq!(
        meta.version.is_none(),
        true,
        "version should be None when missing"
    );
    assert_eq!(meta.theme, Some("default".to_string()));
    assert_eq!(meta.components.len(), 2); // canvas + text1
}

// ============================================================
// 三、超长表达式测试
// ============================================================

#[test]
fn test_long_expression_parsing() {
    let path = PathBuf::from("tests/fixtures/long_expression.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let text1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text1" && e.field == "value")
        .expect("Long expression should be extracted");

    assert_eq!(
        text1_expr.raw_expr.len() > 100,
        true,
        "Expression should be long"
    );

    // 验证所有组件引用都被正确解析
    assert_eq!(
        text1_expr
            .refs
            .contains(&RefType::ComponentValue("input1".to_string())),
        true
    );
    assert_eq!(
        text1_expr
            .refs
            .contains(&RefType::ComponentValue("input2".to_string())),
        true
    );
    assert_eq!(
        text1_expr
            .refs
            .contains(&RefType::ComponentValue("input3".to_string())),
        true
    );
    assert_eq!(
        text1_expr
            .refs
            .contains(&RefType::ComponentValue("input4".to_string())),
        true
    );
}

#[test]
fn test_long_expression_refs_count() {
    let refs = parse_expression_refs(
        "=CONCAT('前缀文本', input1.value, '中缀文本', input2.value, '后缀文本', input3.value, '补充文本', input4.value, '结束文本')",
    );

    // 4个组件引用，去重后仍为4个
    assert_eq!(refs.len(), 4);
}

// ============================================================
// 四、依赖关系边界测试
// ============================================================

#[test]
fn test_diamond_dependency() {
    // A → B, A → C, B → D, C → D（菱形依赖）
    // 这在 boundary_cases.spg 中隐含体现
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // input1 被 input3, input4, input5, input6, input7 依赖
    let reverse_input1 = graph.reverse_deps.get("input1");
    assert_ne!(
        reverse_input1, None,
        "input1 should have reverse dependencies"
    );
    if let Some(deps) = reverse_input1 {
        assert_eq!(
            deps.len() >= 3,
            true,
            "input1 should be depended on by multiple components"
        );
    }
}

#[test]
fn test_isolated_components() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // text3 (常量) 和 text4 (TODAY()) 没有入度也没有出度依赖
    let text3_deps = graph.dependencies.get("text3").unwrap();
    assert_eq!(text3_deps.len(), 0, "text3 should have no dependencies");

    let text4_deps = graph.dependencies.get("text4").unwrap();
    assert_eq!(text4_deps.len(), 0, "text4 should have no dependencies");
}
