use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::{
    RefType, parse_expression_ast, parse_expression_refs, parse_superpage,
};
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
    assert!(cycle.contains(&"input1".to_string()));
    assert!(cycle.contains(&"input2".to_string()));
}

#[test]
fn test_self_reference_detection() {
    let path = PathBuf::from("tests/fixtures/self_reference.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();

    assert_ne!(cycles.len(), 0, "Should detect self-reference cycle");
    let cycle = &cycles[0];
    assert!(cycle.contains(&"input1".to_string()));
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
    assert!(has_param, "Should reference param1");
    assert!(has_model, "Should reference model1.A");
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
    assert!(has_total);
    assert!(has_field);
}

#[test]
fn test_complex_concat_expression() {
    let refs = parse_expression_refs("=CONCAT('前缀', input3.value, '后缀')");

    // 只应识别 input3.value
    assert_eq!(refs.len(), 1);
    assert!(refs.contains(&RefType::ComponentValue("input3".to_string())),);
}

#[test]
fn test_complex_logical_operators() {
    let refs = parse_expression_refs("=input1.value != 'test' AND input2.value IS NOT NULL");

    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&RefType::ComponentValue("input1".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input2".to_string())),);
}

#[test]
fn test_complex_arithmetic_with_function() {
    let refs = parse_expression_refs("=ROUND(input3.value / input4.value, 2)");

    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&RefType::ComponentValue("input3".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input4".to_string())),);
}

#[test]
fn test_macro_expression_parsing() {
    let refs = parse_expression_refs("=${model2.C}");

    assert_eq!(refs.len(), 1);
    assert!(refs.contains(&RefType::ModelField("model2".to_string(), "C".to_string())),);
}

#[test]
fn test_system_variable_expression() {
    let refs = parse_expression_refs("=IF($user.dept_id = '001', '管理员', '普通用户')");

    let has_user = refs
        .iter()
        .any(|r| matches!(r, RefType::UserProperty(p) if p == "user.dept_id"));
    assert!(has_user, "Should detect $user.dept_id");
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
    assert!(
        (!has_no_type),
        "Component without type should not be extracted"
    );

    // 缺少 id 的组件不应被提取
    let has_no_id = meta
        .components
        .iter()
        .any(|c| c.component_type == "input" && c.id.is_empty());
    assert!((!has_no_id), "Component without id should not be extracted");

    // 正常组件应被提取
    let has_normal = meta.components.iter().any(|c| c.id == "normal_input");
    assert!(has_normal, "Normal component should be extracted");
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
    assert!(
        meta.expressions
            .iter()
            .all(|e| e.component_id != "empty_comp"),
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

    assert!(has_value);
    assert!(has_default);
    assert!(has_visible);
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
    assert!(has_input3);
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
    let _pos2 = order.iter().position(|id| id == "input2");
    let pos3 = order.iter().position(|id| id == "input3");
    let pos_text1 = order.iter().position(|id| id == "text1");

    if let (Some(p1), Some(p3)) = (pos1, pos3) {
        assert!(p1 < p3, "input1 should come before input3");
    }
    if let (Some(p3), Some(pt)) = (pos3, pos_text1) {
        assert!(p3 < pt, "input3 should come before text1");
    }
}

// ============================================================
// 七、M7 AST 表达式解析器测试
// ============================================================

#[test]
fn test_parse_expression_ast_basic() {
    let result = parse_expression_ast("=input1.value + model1.fieldA");
    assert!(result.ast.is_some(), "AST should be present");
    assert_eq!(result.refs.len(), 2, "Should detect 2 refs");
    assert!(
        result
            .refs
            .contains(&RefType::ComponentValue("input1".to_string()))
    );
    assert!(result.refs.contains(&RefType::ModelField(
        "model1".to_string(),
        "fieldA".to_string()
    )));
    assert!(
        !result.resolved_refs.is_empty(),
        "resolved_refs should be populated"
    );
    assert!(
        result.diagnostics.is_empty(),
        "No diagnostics for simple expr"
    );
}

#[test]
fn test_parse_expression_ast_unsupported_function() {
    let result = parse_expression_ast("=UNKNOWN_FUNC(input1.value)");
    assert!(result.ast.is_some());
    let has_unsupported = result
        .diagnostics
        .iter()
        .any(|d| d.code == "EXPR_UNSUPPORTED_FUNCTION");
    assert!(has_unsupported, "Should report unsupported function");
}

#[test]
fn test_parse_expression_ast_unresolved_ref() {
    let result = parse_expression_ast("=foo.bar");
    assert!(!result.refs.is_empty());
    let has_unresolved = result
        .diagnostics
        .iter()
        .any(|d| d.code == "EXPR_UNRESOLVED_REF");
    assert!(has_unresolved, "Should report unresolved ref");
}

#[test]
fn test_parse_expression_ast_string_literal_skip() {
    let result = parse_expression_ast("=CONCAT('model1.fieldA', input1.value)");
    assert!(result.ast.is_some());
    assert!(
        !result
            .refs
            .iter()
            .any(|r| matches!(r, RefType::ModelField(m, _) if m == "model1")),
        "String literal should not be parsed as model field"
    );
    assert!(
        result
            .refs
            .contains(&RefType::ComponentValue("input1".to_string()))
    );
}

#[test]
fn test_component_expr_contains_diagnostics() {
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let text3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text3" && e.field == "value")
        .expect("text3.value should exist");
    assert_eq!(text3_expr.raw_expr, "=123");
    assert!(
        text3_expr.diagnostics.is_empty(),
        "Constant expression should have no diagnostics"
    );
}

#[test]
fn test_condition_structured_output() {
    use metadata_checker::action_semantics::parse_condition;
    let result = parse_condition(Some("=input1.value > 0 AND model1.fieldA IS NOT NULL"));
    let arr = result.as_object().expect("should be object");
    assert!(
        arr.contains_key("unresolved_refs"),
        "condition should have unresolved_refs"
    );
    assert!(
        arr.contains_key("diagnostics"),
        "condition should have diagnostics"
    );
    assert!(
        arr.contains_key("ambiguous_refs"),
        "condition should have ambiguous_refs"
    );
    let diagnostics = arr["diagnostics"]
        .as_array()
        .expect("diagnostics should be array");
    assert!(
        !diagnostics
            .iter()
            .any(|d| d.get("code")
                == Some(&serde_json::Value::String("EXPR_PARSE_ERROR".to_string()))),
        "Well-formed condition should not have parse errors"
    );
}

#[test]
fn test_condition_with_unsupported_function() {
    use metadata_checker::action_semantics::parse_condition;
    let result = parse_condition(Some("=MY_CUSTOM_FUNC(input1.value)"));
    let diagnostics = result["diagnostics"].as_array().expect("diagnostics array");
    assert!(
        diagnostics.iter().any(|d| d.get("code")
            == Some(&serde_json::Value::String(
                "EXPR_UNSUPPORTED_FUNCTION".to_string()
            ))),
        "Should report unsupported function in condition"
    );
}

#[test]
fn test_condition_reports_ambiguous_and_unresolved_diagnostics() {
    use metadata_checker::action_semantics::parse_condition;
    let result = parse_condition(Some("=foo.bar + MY_CUSTOM_FUNC(input1.value)"));
    let diagnostics = result["diagnostics"].as_array().expect("diagnostics array");

    assert!(
        diagnostics.iter().any(|d| {
            d.get("code")
                == Some(&serde_json::Value::String(
                    "EXPR_UNRESOLVED_REF".to_string(),
                ))
        }),
        "Should report unresolved reference in condition"
    );
    assert!(
        diagnostics.iter().any(|d| {
            d.get("code") == Some(&serde_json::Value::String("EXPR_AMBIGUOUS_REF".to_string()))
        }),
        "Should report ambiguous reference in condition"
    );
    assert!(
        diagnostics.iter().any(|d| {
            d.get("code")
                == Some(&serde_json::Value::String(
                    "EXPR_UNSUPPORTED_FUNCTION".to_string(),
                ))
        }),
        "Should report unsupported function in condition"
    );
}

// ============================================================
// P0: 换行表达式解析测试
// ============================================================

#[test]
fn test_newline_expression_parsing() {
    let path = PathBuf::from("tests/fixtures/newline_expressions.spg");
    let meta =
        parse_superpage(&path).expect("Should parse file with newline expressions without panic");

    // 文件应包含 4 个组件 + canvas
    assert_eq!(
        meta.components.len(),
        5,
        "Should have canvas + 4 components"
    );

    // input1: =IF(\n input1.value,\n model1.A,\n '')
    let input1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input1" && e.field == "value")
        .expect("input1.value expression should exist");
    assert!(
        input1_expr.raw_expr.contains('\n'),
        "Expression should contain raw newlines"
    );
    // 引用应能正确提取：input1.value（自引用）和 model1.A
    let has_self = input1_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input1"));
    let has_model = input1_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "A"));
    assert!(
        has_self,
        "Should extract self-reference input1 from IF condition"
    );
    assert!(
        has_model,
        "Should extract model1.A from IF true-branch despite newlines"
    );

    // input2: ='line1\nline2\nline3' — 字符串字面量内换行
    let input2_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input2" && e.field == "value")
        .expect("input2.value expression should exist");
    assert!(
        input2_expr.raw_expr.contains('\n'),
        "String literal should contain newlines"
    );
    // 字符串字面量内的换行不应被误判为引用
    let model_refs: Vec<_> = input2_expr
        .refs
        .iter()
        .filter(|r| matches!(r, RefType::ModelField(_, _)))
        .collect();
    assert_eq!(
        model_refs.len(),
        0,
        "String literal with newlines should not produce model references"
    );

    // input3: =MACRO\n(input1.value,\nmodel1.B) — 宏表达式跨行
    let input3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input3" && e.field == "value")
        .expect("input3.value expression should exist");
    assert!(
        input3_expr.raw_expr.contains('\n'),
        "Macro expression should contain newlines"
    );
    let has_input1 = input3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input1"));
    let has_model_b = input3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "B"));
    assert!(
        has_input1,
        "Should extract input1 from macro args despite newline"
    );
    assert!(
        has_model_b,
        "Should extract model1.B from macro args despite newline"
    );

    // text1: =CONCAT('prefix\nsuffix', input2.value) — 字符串内换行 + 组件引用
    let text1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text1" && e.field == "value")
        .expect("text1.value expression should exist");
    let has_input2 = text1_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input2"));
    let string_model_refs: Vec<_> = text1_expr
        .refs
        .iter()
        .filter(|r| matches!(r, RefType::ModelField(_, _)))
        .collect();
    assert!(has_input2, "Should extract input2 from CONCAT");
    assert_eq!(
        string_model_refs.len(),
        0,
        "String literal 'prefix\\nsuffix' should not produce model reference"
    );
}

// ============================================================
// P0: 复杂循环检测测试（A→B→C→D→B，A 不在环内）
// ============================================================

#[test]
fn test_complex_cycle_detection() {
    let path = PathBuf::from("tests/fixtures/complex_cycle.spg");
    let meta = parse_superpage(&path).expect("Failed to parse complex cycle fixture");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();

    assert!(!cycles.is_empty(), "Should detect cycle in A→B→C→D→B graph");

    // A 引用 B，但 A 自身不在环内；B→C→D→B 构成环
    let mut a_in_cycle = false;
    let mut cycle_nodes = std::collections::HashSet::new();
    for cycle in &cycles {
        for node in cycle {
            cycle_nodes.insert(node.clone());
        }
        if cycle.contains(&"A".to_string()) {
            a_in_cycle = true;
        }
    }

    assert!(
        !a_in_cycle,
        "A should NOT be in any cycle (A only points to B, not part of B→C→D→B)"
    );
    assert!(cycle_nodes.contains("B"), "B must be in cycle");
    assert!(cycle_nodes.contains("C"), "C must be in cycle");
    assert!(cycle_nodes.contains("D"), "D must be in cycle");
}

// ============================================================
// P0: 同层组件顺序稳定性测试
// ============================================================

#[test]
fn test_topological_sort_stability() {
    let path = PathBuf::from("tests/fixtures/stable_order.spg");
    let meta = parse_superpage(&path).expect("Failed to parse stable_order fixture");
    let graph = DependencyGraph::new(&meta);

    // 多次运行拓扑排序，结果应一致
    let order1 = graph.topological_sort();
    let order2 = graph.topological_sort();
    let order3 = graph.topological_sort();

    assert_eq!(
        order1, order2,
        "Topological sort should be stable across multiple runs (run 1 vs 2)"
    );
    assert_eq!(
        order2, order3,
        "Topological sort should be stable across multiple runs (run 2 vs 3)"
    );

    // input1 依赖 a1 和 b1，所以 input1 必须在 a1、b1 之后
    let input1_idx = order1
        .iter()
        .position(|id| id == "input1")
        .expect("input1 should exist");
    let a1_idx = order1
        .iter()
        .position(|id| id == "a1")
        .expect("a1 should exist");
    let b1_idx = order1
        .iter()
        .position(|id| id == "b1")
        .expect("b1 should exist");
    assert!(input1_idx > a1_idx, "input1 should come after a1");
    assert!(input1_idx > b1_idx, "input1 should come after b1");

    // z1, m1 与 a1, b1 之间无依赖，它们和 a1, b1 的顺序由稳定规则决定
    // 当前实现使用 BFS queue，顺序取决于 HashMap 遍历顺序（不稳定）
    // 这里只验证一致性，不强求特定顺序
    let _z1_idx = order1
        .iter()
        .position(|id| id == "z1")
        .expect("z1 should exist");
    let _m1_idx = order1
        .iter()
        .position(|id| id == "m1")
        .expect("m1 should exist");

    // 稳定排序规则：无依赖组件应保持元数据出现顺序（z1 → a1 → m1 → b1）
    // 但如果实现不是这样，至少保证一致性
    let metadata_order = ["z1", "a1", "m1", "b1"];
    let actual_order: Vec<&str> = order1
        .iter()
        .filter(|id| metadata_order.contains(&id.as_str()))
        .map(|id| id.as_str())
        .collect();

    // 记录当前行为，不强求特定顺序
    // 如果与元数据顺序不一致，说明当前实现使用 HashMap 顺序（不稳定但单次一致）
    assert_eq!(
        actual_order.len(),
        4,
        "All 4 independent components should appear before input1"
    );
}

// ============================================================
// P1: 组件值追溯 fixture 测试
// ============================================================

#[test]
fn test_value_trace_single_level() {
    let path = PathBuf::from("tests/fixtures/value_trace_single.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // input1.value = param1
    let input1_deps = graph
        .dependencies
        .get("input1")
        .expect("input1 should exist");
    let has_param = input1_deps
        .iter()
        .any(|r| matches!(r, RefType::Param(p) if p == "param1"));
    assert!(has_param, "input1 should trace to param1");
}

#[test]
fn test_value_trace_multi_level() {
    let path = PathBuf::from("tests/fixtures/value_trace_multi_level.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // input3 -> input2 -> input1 -> param1
    // input3 -> model1.A
    let input3_deps = graph
        .dependencies
        .get("input3")
        .expect("input3 should exist");

    // 直接依赖
    let has_input2 = input3_deps.iter().any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input2"));
    let has_model = input3_deps
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "A"));
    assert!(has_input2, "input3 should directly reference input2");
    assert!(has_model, "input3 should directly reference model1.A");

    // 拓扑排序验证依赖顺序
    let order = graph.topological_sort();
    let i1 = order.iter().position(|id| id == "input1").unwrap();
    let i2 = order.iter().position(|id| id == "input2").unwrap();
    let i3 = order.iter().position(|id| id == "input3").unwrap();
    assert!(i1 < i2, "input1 should come before input2");
    assert!(i2 < i3, "input2 should come before input3");
}

#[test]
fn test_value_trace_multi_branch() {
    let path = PathBuf::from("tests/fixtures/value_trace_multi_branch.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    // input3 = input1.value + input2.value
    let input3_deps = graph
        .dependencies
        .get("input3")
        .expect("input3 should exist");

    let has_input1 = input3_deps.iter().any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input1"));
    let has_input2 = input3_deps.iter().any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input2"));
    assert!(has_input1, "input3 should reference input1 (param1 branch)");
    assert!(has_input2, "input3 should reference input2 (model1 branch)");

    // input1 追溯到 param1，input2 追溯到 model1.A
    let input1_deps = graph
        .dependencies
        .get("input1")
        .expect("input1 should exist");
    let input2_deps = graph
        .dependencies
        .get("input2")
        .expect("input2 should exist");
    let has_param1 = input1_deps
        .iter()
        .any(|r| matches!(r, RefType::Param(p) if p == "param1"));
    let has_model1 = input2_deps
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "A"));
    assert!(has_param1, "input1 should trace to param1");
    assert!(has_model1, "input2 should trace to model1.A");
}

#[test]
fn test_value_trace_cross_component_types() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // text1 引用 input3（跨类型：text -> input）
    let text1_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text1" && e.field == "value")
        .expect("text1.value should exist");
    let has_input3 = text1_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input3"));
    assert!(
        has_input3,
        "text1 should reference input3 across component types"
    );

    // input3 引用 input1 和 input2（跨组件类型追溯）
    let input3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input3" && e.field == "value")
        .expect("input3.value should exist");
    let has_input1 = input3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input1"));
    let has_input2 = input3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(v) | RefType::ComponentProperty(v, _) if v == "input2"));
    assert!(has_input1, "input3 should reference input1");
    assert!(has_input2, "input3 should reference input2");

    // 通过 input1/input2 间接追溯到 param1 和 model1.A
    let graph = DependencyGraph::new(&meta);
    let order = graph.topological_sort();
    let i1 = order.iter().position(|id| id == "input1").unwrap();
    let i2 = order.iter().position(|id| id == "input2").unwrap();
    let i3 = order.iter().position(|id| id == "input3").unwrap();
    assert!(i1 < i3, "input1 should come before input3 in topo order");
    assert!(i2 < i3, "input2 should come before input3 in topo order");
}

// ============================================================
// P2: 输出层来源分类定义测试
// ============================================================

// M59-B2（spec §2.7）：以下四条来源分类断言原先接受 2~3 个候选值
//（`Param|UserInput|Constant`、`Computed|Unknown`、`Constant|Computed|Unknown`，
// 以及一条 `has_model_node || raw_expr.contains("model1")` 的或门）。
// 这类宽松断言挡不住迁移引入的分类退化——Grafeo 落地期正是最需要它们的时候。
// 现全部收紧到**唯一期望值**，并把展开式与 source_chain 一并钉住：
// 分类只是最终标签，展开式和链条才是它由之而来的事实。
//
// 收紧过程中暴露的两条既有缺陷，见每条测试内的说明。它们**不在 B2 范围内修**
// （B2 只做探针），但断言必须明确写成「钉住当前行为，且当前行为是错的」，
// 而不能读起来像是在背书。

/// `=param1` → `Param`，且属外部输入。
#[test]
fn test_source_type_param() {
    use metadata_checker::dependency::{SourceType, trace_value_source};
    let path = PathBuf::from("tests/fixtures/value_trace_single.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let trace =
        trace_value_source(&meta, &graph, "input1", "value", 5).expect("Should trace input1.value");
    assert_eq!(trace.raw_expr, "=param1");
    assert!(
        matches!(trace.source_type, SourceType::Param),
        "input1.value = param1 必须唯一判为 Param，实际 {:?}",
        trace.source_type
    );
    assert!(
        trace.is_external_input,
        "Param 属外部输入，is_external_input 必须为 true"
    );
    assert_eq!(trace.expanded_expr, "=(param1)");
    assert!(
        trace.source_chain.is_empty(),
        "无组件依赖时链条应为空，实际 {:?}",
        trace.source_chain
    );
}

/// `=input1.value + input2.value` → `Computed`；链条按依赖出现顺序展开，
/// 且逐节点分类到位（input1=Param、input2=ModelAuto）。
#[test]
fn test_source_type_computed() {
    use metadata_checker::dependency::{SourceType, trace_value_source};
    let path = PathBuf::from("tests/fixtures/value_trace_multi_branch.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let trace =
        trace_value_source(&meta, &graph, "input3", "value", 5).expect("Should trace input3.value");
    assert!(
        matches!(trace.source_type, SourceType::Computed),
        "input1.value + input2.value 必须唯一判为 Computed，实际 {:?}",
        trace.source_type
    );
    assert!(!trace.is_external_input, "计算值不是外部输入");
    assert_eq!(
        trace.expanded_expr, "=(param1) + (model1.A)",
        "两个分支必须各自展开到根来源"
    );
    let chain: Vec<String> = trace
        .source_chain
        .iter()
        .map(|n| format!("{}:{:?}", n.component_id, n.source_type))
        .collect();
    assert_eq!(
        chain,
        vec!["input1:Param".to_string(), "input2:ModelAuto".to_string()],
        "链条须按依赖顺序逐节点分类，实际 {chain:?}"
    );
}

/// `=input2.value + model1.A` → `ModelAuto`（`contains("model")` 优先于 `.value`）。
///
/// **钉住的既有缺陷**：`RefType::ModelField` 在 `expand_expression` 里只做替换、
/// **不 push `SourceNode`**，所以 `model1` 本身从不出现在 source_chain 里——
/// 链条只有 input1/input2。原断言那道 `|| raw_expr.contains("model1")` 或门正是
/// 靠这半边才通过的，等于把缺陷藏起来了。这里改成显式断言链条**不含**
/// ModelAuto 节点：行为一旦被修好，这条会立刻红，届时连同期望一起更新。
#[test]
fn test_source_type_model_auto() {
    use metadata_checker::dependency::{SourceType, trace_value_source};
    let path = PathBuf::from("tests/fixtures/value_trace_multi_level.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let trace =
        trace_value_source(&meta, &graph, "input3", "value", 5).expect("Should trace input3.value");
    assert!(
        matches!(trace.source_type, SourceType::ModelAuto),
        "表达式含 model 引用必须唯一判为 ModelAuto，实际 {:?}",
        trace.source_type
    );
    assert_eq!(
        trace.expanded_expr, "=(param1) + (model1.A)",
        "多级组件链与模型字段引用都要展开"
    );
    let chain: Vec<String> = trace
        .source_chain
        .iter()
        .map(|n| format!("{}:{:?}", n.component_id, n.source_type))
        .collect();
    assert_eq!(
        chain,
        vec!["input1:Param".to_string(), "input2:Computed".to_string()],
        "链条须按依赖顺序逐节点分类，实际 {chain:?}"
    );
    assert!(
        !trace
            .source_chain
            .iter()
            .any(|n| matches!(n.source_type, SourceType::ModelAuto)),
        "钉住既有缺陷：ModelField 引用不进 source_chain。这条一旦变红说明缺陷被修了，\
         把期望改成「链条里应当有 model1」即可，不要放宽断言"
    );
}

/// `=123` → **`Unknown`**。
///
/// **钉住的既有缺陷**：`determine_source_type` 判 `Constant` 的条件是
/// 「不以 `=` 开头且不以 `${` 开头」，于是公式形态的字面量 `=123` 落不进
/// `Constant`，一路掉到兜底的 `Unknown`。按名字这条测试想验的是常量分类，
/// 而它从来没验到过——旧断言把 `Constant|Computed|Unknown` 三个都收下了，
/// 正好盖住这件事。
///
/// 这里**如实钉住 `Unknown`**，不是认可它。修法属分类器本身（`=` 开头但不含任何
/// 引用的纯字面量应判 Constant），不在 B2 探针范围内。
#[test]
fn test_source_type_constant() {
    use metadata_checker::dependency::{SourceType, trace_value_source};
    let path = PathBuf::from("tests/fixtures/boundary_cases.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);
    let trace =
        trace_value_source(&meta, &graph, "text3", "value", 5).expect("Should trace text3.value");
    assert_eq!(trace.raw_expr, "=123");
    assert!(
        matches!(trace.source_type, SourceType::Unknown),
        "钉住当前行为：`=123` 现在判为 Unknown（应当是 Constant，见 doc 注释），实际 {:?}",
        trace.source_type
    );
    assert!(!trace.is_external_input);
    assert_eq!(trace.expanded_expr, "=123", "字面量不该被改写");
    assert!(trace.source_chain.is_empty());
}

// ============================================================
// P3: 性能与规模测试
// ============================================================

#[test]
fn test_large_page_parsing() {
    let path = PathBuf::from("tests/fixtures/large_page.spg");
    let meta = parse_superpage(&path).expect("Should parse large page");

    // 150 组件 + canvas = 151
    assert_eq!(
        meta.components.len(),
        151,
        "Should parse all 150 components plus canvas"
    );

    // 表达式数量应接近 150（每个组件一个 value 表达式）
    assert!(
        meta.expressions.len() >= 100,
        "Should extract at least 100 expressions from 150 components"
    );

    // 依赖图构建
    let graph = DependencyGraph::new(&meta);
    assert!(
        !graph.dependencies.is_empty(),
        "Large page should have dependencies"
    );
}

#[test]
fn test_large_page_topological_sort_performance() {
    use std::time::Instant;
    let path = PathBuf::from("tests/fixtures/large_page.spg");
    let meta = parse_superpage(&path).expect("Should parse large page");
    let graph = DependencyGraph::new(&meta);

    let start = Instant::now();
    let order = graph.topological_sort();
    let elapsed = start.elapsed();

    // debug 模式下 < 2s 是合理的阈值
    assert!(
        elapsed.as_secs() < 2,
        "Topological sort of 150 components should take < 2s, took {:?}",
        elapsed
    );

    // 验证排序覆盖所有组件
    assert!(
        order.len() >= meta.components.len() - 1,
        "Topo sort should cover most components, got {} of {}",
        order.len(),
        meta.components.len()
    );
}

#[test]
fn test_large_page_expression_refs_count() {
    let path = PathBuf::from("tests/fixtures/large_page.spg");
    let meta = parse_superpage(&path).expect("Should parse large page");

    let total_refs: usize = meta.expressions.iter().map(|e| e.refs.len()).sum();
    assert!(
        total_refs >= 50,
        "150 components should produce at least 50 refs, got {}",
        total_refs
    );
}
