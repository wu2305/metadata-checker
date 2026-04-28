use metadata_checker::superpage::{RefType, parse_expression_refs, parse_superpage};
use std::path::PathBuf;

#[test]
fn test_missing_version_field_tolerated() {
    let path = PathBuf::from("tests/fixtures/edge_cases.spg");
    let meta = parse_superpage(&path).expect("Should parse even without version");

    assert!(meta.version.is_none());
    // 其他字段应正常解析
    assert_eq!(meta.theme, Some("default".to_string()));
}

#[test]
fn test_param_missing_id() {
    let path = PathBuf::from("tests/fixtures/edge_cases.spg");
    let meta = parse_superpage(&path).expect("Should parse");

    // 第一个 param 缺少 id，解析后 id 为空字符串
    assert_eq!(meta.params.len(), 2);
    assert_eq!(meta.params[0].id, "");
    assert_eq!(meta.params[0].name, "无名参数");

    // 第二个 param 正常
    assert_eq!(meta.params[1].id, "param2");
}

#[test]
fn test_source_missing_id() {
    let path = PathBuf::from("tests/fixtures/edge_cases.spg");
    let meta = parse_superpage(&path).expect("Should parse");

    // 第一个 source 缺少 id，解析后 id 为空字符串
    assert_eq!(meta.sources.len(), 2);
    assert_eq!(meta.sources[0].id, "");
    assert_eq!(meta.sources[0].model_type, Some("dwtable".to_string()));

    // 第二个 source 正常
    assert_eq!(meta.sources[1].id, "model2");
}

#[test]
fn test_system_variable_project() {
    let refs = parse_expression_refs("=$project.name");

    assert_eq!(refs.len(), 1);
    assert!(refs.contains(&RefType::UserProperty("project.name".to_string())),);
}

#[test]
fn test_system_variable_user_no_property() {
    let refs = parse_expression_refs("=$user");

    assert_eq!(refs.len(), 1);
    assert!(refs.contains(&RefType::SystemVar("$user".to_string())),);
}

#[test]
fn test_nonexistent_component_reference() {
    let path = PathBuf::from("tests/fixtures/edge_cases.spg");
    let meta = parse_superpage(&path).expect("Should parse");

    // text3 引用了 nonexistent.value，解析时不会报错
    let text3_expr = meta
        .expressions
        .iter()
        .find(|e| e.component_id == "text3" && e.field == "value")
        .expect("text3 expression should exist");

    assert_eq!(text3_expr.raw_expr, "=nonexistent.value + input1.value");

    // 解析出 nonexistent 和 input1 的引用
    let has_nonexistent = text3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "nonexistent"));
    let has_input1 = text3_expr
        .refs
        .iter()
        .any(|r| matches!(r, RefType::ComponentValue(id) if id == "input1"));

    assert!(
        (has_nonexistent),
        "Should parse reference to nonexistent component"
    );
    assert!(has_input1, "Should parse reference to input1");
}

#[test]
fn test_component_id_with_space() {
    let path = PathBuf::from("tests/fixtures/edge_cases.spg");
    let meta = parse_superpage(&path).expect("Should parse");

    // "input 4" 带空格，应被解析为组件
    let comp = meta.components.iter().find(|c| c.id == "input 4");
    assert_ne!(comp, None, "Component with space in id should be parsed");

    if let Some(c) = comp {
        assert_eq!(c.component_type, "input");
    }
}
