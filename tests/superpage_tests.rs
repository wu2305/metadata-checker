use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::{RefType, parse_expression_refs, parse_superpage};
use std::path::PathBuf;

#[test]
fn test_parse_superpage_basic() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");

    assert_eq!(meta.version, Some("4.19.7".to_string()));
    assert_eq!(meta.theme, Some("default".to_string()));
    assert_eq!(meta.params.len(), 2);
    assert_eq!(meta.sources.len(), 2);
    // canvas + 6 child components = 7 components total
    assert_eq!(meta.components.len(), 8);
}

#[test]
fn test_parse_params() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");

    assert_eq!(meta.params[0].id, "param1");
    assert_eq!(meta.params[0].name, "参数A");
    assert_eq!(meta.params[1].id, "param2");
    assert_eq!(meta.params[1].name, "参数B");
}

#[test]
fn test_parse_sources() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");

    assert_eq!(meta.sources[0].id, "model1");
    assert_eq!(meta.sources[0].model_type, Some("dwtable".to_string()));
    assert_eq!(meta.sources[1].id, "model2");
    assert_eq!(meta.sources[1].model_type, Some("query".to_string()));
}

#[test]
fn test_parse_components() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");

    let ids: Vec<String> = meta.components.iter().map(|c| c.id.clone()).collect();
    assert!(ids.contains(&"input1".to_string()));
    assert!(ids.contains(&"input2".to_string()));
    assert!(ids.contains(&"input3".to_string()));
    assert!(ids.contains(&"text1".to_string()));
    assert!(ids.contains(&"panel1".to_string()));
    assert!(ids.contains(&"input4".to_string()));
    assert!(ids.contains(&"text2".to_string()));
}

#[test]
fn test_parse_expressions() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");

    // 应该有 7 个表达式（input1.value, input2.value, input3.value, text1.value, panel1.visible, input4.value, text2.value）
    assert_eq!(meta.expressions.len(), 7);

    // 检查 input3 的表达式
    let input3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "input3" && e.field == "value")
        .expect("input3.value expression not found");
    assert_eq!(input3_expr.raw_expr, "=input2.value/input1.value");
}

#[test]
fn test_parse_expression_refs_simple() {
    let refs = parse_expression_refs("=input2.value/input1.value");

    // 应该只有 input2 和 input1（.value 是属性，不是独立引用）
    // 实际上正则匹配的是 input2.value 和 input1.value
    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&RefType::ComponentValue("input2".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input1".to_string())),);
}

#[test]
fn test_parse_expression_refs_model() {
    let refs = parse_expression_refs("=param1-model1.A");

    // param1 和 model1.A 都应该被匹配
    let has_param1 = refs.contains(&RefType::Param("param1".to_string()));
    let has_model1 = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model1" && f == "A"));
    assert!((has_param1), "Expected param1 reference, got: {:?}", refs);
    assert!((has_model1), "Expected model1.A reference, got: {:?}", refs);
}

#[test]
fn test_parse_expression_refs_if() {
    let refs = parse_expression_refs("=IF(input3.value > 0, input3.value, 0)");

    // 去重后 input3.value 应该只出现一次
    let input3_count = refs
        .iter()
        .filter(|r| matches!(r, RefType::ComponentValue(id) if id == "input3"))
        .count();
    assert_eq!(
        input3_count, 1,
        "input3 should appear exactly once due to deduplication"
    );

    // 总共应该只有 1 个引用（input3.value）
    assert_eq!(refs.len(), 1);
}

#[test]
fn test_parse_expression_refs_user() {
    let refs = parse_expression_refs("=$user.dept_id = '001'");

    let has_user = refs
        .iter()
        .any(|r| matches!(r, RefType::UserProperty(p) if p == "user.dept_id"));
    assert!(
        (has_user),
        "Expected $user.dept_id reference, got: {:?}",
        refs
    );
}

#[test]
fn test_parse_expression_refs_macro() {
    let refs = parse_expression_refs("${model2.C}");

    let has_model = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model2" && f == "C"));
    assert!((has_model), "Expected model2.C reference, got: {:?}", refs);
}

#[test]
fn test_dependency_graph() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");
    let graph = DependencyGraph::new(&meta);

    // input3 依赖 input2 和 input1
    let input3_deps = graph
        .dependencies
        .get("input3")
        .expect("input3 not found in graph");
    let has_input2 = input3_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input2"));
    let has_input1 = input3_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input1"));
    assert!(has_input2, "input3 should depend on input2");
    assert!(has_input1, "input3 should depend on input1");

    // text1 依赖 input3
    let text1_deps = graph
        .dependencies
        .get("text1")
        .expect("text1 not found in graph");
    let has_input3 = text1_deps
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input3"));
    assert!(has_input3, "text1 should depend on input3");
}

#[test]
fn test_topological_sort() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");
    let graph = DependencyGraph::new(&meta);
    let order = graph.topological_sort();

    // input1 和 input2 应该在 input3 之前
    let pos_input1 = order.iter().position(|id| id == "input1");
    let pos_input2 = order.iter().position(|id| id == "input2");
    let pos_input3 = order.iter().position(|id| id == "input3");
    let pos_text1 = order.iter().position(|id| id == "text1");

    if let (Some(p1), Some(p3)) = (pos_input1, pos_input3) {
        assert!(p1 < p3, "input1 should come before input3");
    }
    if let (Some(p2), Some(p3)) = (pos_input2, pos_input3) {
        assert!(p2 < p3, "input2 should come before input3");
    }
    if let (Some(p3), Some(pt)) = (pos_input3, pos_text1) {
        assert!(p3 < pt, "input3 should come before text1");
    }
}

#[test]
fn test_no_cycles() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse superpage");
    let graph = DependencyGraph::new(&meta);
    let cycles = graph.detect_cycles();

    assert_eq!(cycles.len(), 0, "Expected no cycles in test_superpage.spg");
}

#[test]
fn test_parse_expression_refs_complex() {
    let refs = parse_expression_refs("=input1.value + input2.value");

    assert_eq!(refs.len(), 2);
    assert!(refs.contains(&RefType::ComponentValue("input1".to_string())),);
    assert!(refs.contains(&RefType::ComponentValue("input2".to_string())),);
}

#[test]
fn test_skip_string_literals() {
    let refs = parse_expression_refs(
        "=IF(model5.EstimatedAnnualInterestRate!=NULL,model5.EstimatedAnnualInterestRate*100,'')",
    );

    // 不应该把空字符串 '' 当作引用
    let has_empty_string = refs
        .iter()
        .any(|r| matches!(r, RefType::Other(s) if s == "''"));
    assert!(!has_empty_string, "String literals should be skipped");

    // 应该包含 model5
    let has_model5 = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, _) if m == "model5"));
    assert!(has_model5, "Expected model5 reference");

    // model5.EstimatedAnnualInterestRate 被去重后应该只出现一次
    let model5_count = refs
        .iter()
        .filter(|r| matches!(r, RefType::ModelField(m, _) if m == "model5"))
        .count();
    assert_eq!(
        model5_count, 1,
        "model5 should appear exactly once due to deduplication"
    );
}

#[test]
fn test_parse_expression_refs_model22_phonenumber() {
    let refs = parse_expression_refs("model22.phoneNumber");
    let has_model22 = refs
        .iter()
        .any(|r| matches!(r, RefType::ModelField(m, f) if m == "model22" && f == "phoneNumber"));
    assert!(has_model22, "Expected model22.phoneNumber reference, got: {:?}", refs);
}
