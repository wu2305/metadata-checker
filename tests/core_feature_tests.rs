use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::{RefType, parse_expression_refs, parse_superpage};
use std::path::PathBuf;

// ============================================================
// 一、循环依赖检测测试
// ============================================================

#[test]
fn test_cycle_detection_simple() {
    let path = PathBuf::from("tests/fixtures/cycle_dependency.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();

    assert_ne!(
        cycles.len(),
        0,
        "Should detect cycle between input1 and input2"
    );
    assert_eq!(cycles.len(), 1, "Should detect exactly one cycle");

    let cycle = &cycles[0];
    assert_eq!(cycle.contains(&"input1".to_string()), true);
    assert_eq!(cycle.contains(&"input2".to_string()), true);
}

#[test]
fn test_self_reference_detection() {
    let path = PathBuf::from("tests/fixtures/self_reference.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();

    assert_ne!(cycles.len(), 0, "Should detect self-reference cycle");
    let cycle = &cycles[0];
    assert_eq!(cycle.contains(&"input1".to_string()), true);
}

#[test]
fn test_no_cycles_in_acyclic() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();
    assert_eq!(cycles.len(), 0, "Acyclic graph should have no cycles");
}

// ============================================================
// 二、深层嵌套组件测试
// ============================================================

#[test]
fn test_deep_nesting_parsing() {
    let path = PathBuf::from("tests/fixtures/deep_nesting.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // canvas + panel1 + panel2 + panel3 + panel4 + input1 = 6 components
    assert_eq!(meta.components.len(), 6);

    // 验证嵌套层级
    let input1 = meta.components.iter().find(|c| c.id == "input1").unwrap();
    assert_eq!(input1.component_type, "input");
    assert_eq!(input1.parent_id, Some("panel4".to_string()));

    let panel4 = meta.components.iter().find(|c| c.id == "panel4").unwrap();
    assert_eq!(panel4.parent_id, Some("panel3".to_string()));
}

#[test]
fn test_deep_nesting_expression() {
    let path = PathBuf::from("tests/fixtures/deep_nesting.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input1" && e.field == "value")
        .expect("Should find input1.value expression");

    assert_eq!(expr.raw_expr, "=param1-model1.A");

    // 验证引用解析
    let has_param = expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::Param(p) if p == "param1"));
    let has_model = expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "A"));
    assert_eq!(has_param, true, "Should reference param1");
    assert_eq!(has_model, true, "Should reference model1.A");
}

// ============================================================
// 三、复杂表达式解析测试
// ============================================================

#[test]
fn test_complex_if_nested() {
    let refs = parse_expression_refs("=IF(model2.totalRowCount__ > 0, model2.fieldB, '')");

    // 应该识别 model2.totalRowCount__ 和 model2.fieldB
    let model_refs: Vec<_> = refs
        .iter()
        .filter(|r| matches!(r, RefType::ModelField(_, _)))
        .collect();
    assert_eq!(model_refs.len(), 2, "Should detect 2 model references");

    // 去重后，相同 model 的不同字段应分别出现
    let has_total = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model2" && f == "totalRowCount__"));
    let has_field = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model2" && f == "fieldB"));
    assert_eq!(has_total, true);
    assert_eq!(has_field, true);
}

#[test]
fn test_complex_concat_expression() {
    let refs = parse_expression_refs("=CONCAT('前缀', input3.value, '后缀')");

    // 只应识别 input3.value
    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs.contains(&RefType::ComponentValue("input3".to_string())),
        true
    );
}

#[test]
fn test_complex_logical_operators() {
    let refs = parse_expression_refs("=input1.value != 'test' AND input2.value IS NOT NULL");

    assert_eq!(refs.len(), 2);
    assert_eq!(
        refs.contains(&RefType::ComponentValue("input1".to_string())),
        true
    );
    assert_eq!(
        refs.contains(&RefType::ComponentValue("input2".to_string())),
        true
    );
}

#[test]
fn test_complex_arithmetic_with_function() {
    let refs = parse_expression_refs("=ROUND(input3.value / input4.value, 2)");

    assert_eq!(refs.len(), 2);
    assert_eq!(
        refs.contains(&RefType::ComponentValue("input3".to_string())),
        true
    );
    assert_eq!(
        refs.contains(&RefType::ComponentValue("input4".to_string())),
        true
    );
}

#[test]
fn test_macro_expression_parsing() {
    let refs = parse_expression_refs("=${model2.C}");

    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs.contains(&RefType::ModelField("model2".to_string(), "C".to_string())),
        true
    );
}

#[test]
fn test_system_variable_expression() {
    let refs = parse_expression_refs("=IF($user.dept_id = '001', '管理员', '普通用户')");

    let has_user = refs
        .iter()
        .any(|r| matches!(r, RefType::UserProperty(p) if p == "user.dept_id"));
    assert_eq!(has_user, true, "Should detect $user.dept_id");
}

#[test]
fn test_no_reference_expression() {
    let refs = parse_expression_refs("=TODAY()");

    assert_eq!(refs.len(), 0, "TODAY() has no references");
}

#[test]
fn test_constant_expression() {
    let refs = parse_expression_refs("=123");

    assert_eq!(refs.len(), 0, "Pure constant has no references");
}

// ============================================================
// 四、组件边界情况测试
// ============================================================

#[test]
fn test_empty_fields_tolerance() {
    let path = PathBuf::from("tests/fixtures/empty_fields.spg");
    let meta = parse_superpage(&path).expect("Should parse even with missing fields");

    // 缺少 type 的组件不应被提取
    let has_no_type = meta.components.iter().any(|c| c.id == "comp_without_type");
    assert_eq!(
        !has_no_type, true,
        "Component without type should not be extracted"
    );

    // 缺少 id 的组件不应被提取
    let has_no_id = meta
        .components
        .iter()
        .any(|c| c.component_type == "input" && c.id.is_empty());
    assert_eq!(
        !has_no_id, true,
        "Component without id should not be extracted"
    );

    // 正常组件应被提取
    let has_normal = meta.components.iter().any(|c| c.id == "normal_input");
    assert_eq!(has_normal, true, "Normal component should be extracted");
}

#[test]
fn test_empty_components_array() {
    let path = PathBuf::from("tests/fixtures/empty_fields.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let empty_panel = meta
        .components
        .iter()
        .find(|c| c.id == "empty_comp")
        .unwrap();
    assert_eq!(empty_panel.component_type, "panel");
    assert_eq!(
        meta.expressions
            .iter()
            .all(|e| e.component_id != "empty_comp"),
        true
    );
}

#[test]
fn test_multi_field_expressions() {
    let path = PathBuf::from("tests/fixtures/multi_field_expr.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    let input1_exprs: Vec<_> = meta
        .expressions
        .iter()
        .filter(|e| e.component_id == "input1")
        .collect();

    assert_eq!(
        input1_exprs.len(),
        4,
        "input1 should have 4 expressions: value, defaultValue, visible, enable"
    );

    let has_value = input1_exprs.iter().any(|e| e.field == "value");
    let has_default = input1_exprs.iter().any(|e| e.field == "defaultValue");
    let has_visible = input1_exprs.iter().any(|e| e.field == "visible");

    assert_eq!(has_value, true);
    assert_eq!(has_default, true);
    assert_eq!(has_visible, true);
}

// ============================================================
// 五、依赖关系边界测试
// ============================================================

#[test]
fn test_independent_component() {
    let path = PathBuf::from("tests/fixtures/complex_expressions.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // text3 只依赖 TODAY()，没有组件引用
    let text3_deps = graph.dependencies.get("text3").unwrap();
    assert_eq!(
        text3_deps.len(),
        0,
        "text3 should have no component dependencies"
    );
}

#[test]
fn test_multi_source_dependency() {
    let path = PathBuf::from("tests/fixtures/complex_expressions.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // text1 依赖 input3
    let text1_deps = graph.dependencies.get("text1").unwrap();
    let has_input3 = text1_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input3"));
    assert_eq!(has_input3, true);
}

// ============================================================
// 六、拓扑排序扩展测试
// ============================================================

#[test]
fn test_topological_sort_with_independent() {
    let path = PathBuf::from("tests/fixtures/complex_expressions.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let order = graph.topological_sort();

    // input1, input2 应该在 input3 之前
    let pos1 = order.iter().position(|id| id == "input1");
    let pos2 = order.iter().position(|id| id == "input2");
    let pos3 = order.iter().position(|id| id == "input3");
    let pos_text1 = order.iter().position(|id| id == "text1");

    if let (Some(p1), Some(p3)) = (pos1, pos3) {
        assert_eq!(p1 < p3, true, "input1 should come before input3");
    }
    if let (Some(p3), Some(pt)) = (pos3, pos_text1) {
        assert_eq!(p3 < pt, true, "input3 should come before text1");
    }
}
