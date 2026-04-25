use metadata_checker::superpage::parse_superpage;
use metadata_checker::priority::{analyze_priority, ExpDefaultValuePriority};
use std::path::PathBuf;

#[test]
fn test_priority_only_default_value() {
    let path = PathBuf::from("tests/fixtures/real_world_1.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    // combobox2 有 defaultValue=1 但没有 exp
    let combobox2 = analyses.iter()
        .find(|a| a.component_id == "combobox2")
        .expect("combobox2 should be analyzed");
    
    assert_eq!(combobox2.priority_result, ExpDefaultValuePriority::OnlyDefaultValue);
    assert!(combobox2.default_value_expr.is_some());
    assert!(combobox2.calc_exp_expr.is_none());
}

#[test]
fn test_priority_only_exp() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    // input3 有 exp=input2.value/input1.value 但没有 defaultValue
    let input3 = analyses.iter()
        .find(|a| a.component_id == "input3")
        .expect("input3 should be analyzed");
    
    assert_eq!(input3.priority_result, ExpDefaultValuePriority::OnlyExp);
    assert!(input3.calc_exp_expr.is_some());
    assert!(input3.default_value_expr.is_none());
}

#[test]
fn test_priority_exp_with_calc_condition() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    // text1 有 exp=IF(input3.value > 0, input3.value, 0) 但没有 defaultValue 和 calcCondition
    let text1 = analyses.iter()
        .find(|a| a.component_id == "text1")
        .expect("text1 should be analyzed");
    
    // 没有 calcCondition，所以 exp 优先
    assert_eq!(text1.priority_result, ExpDefaultValuePriority::OnlyExp);
    assert!(text1.calc_exp_expr.is_some());
    assert!(text1.calc_condition_expr.is_none());
}

#[test]
fn test_priority_combobox2_real_world() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    let combobox2 = analyses.iter()
        .find(|a| a.component_id == "combobox2");
    
    if let Some(a) = combobox2 {
        assert_eq!(a.priority_result, ExpDefaultValuePriority::OnlyDefaultValue);
        assert_eq!(a.default_value_expr.as_ref().unwrap().raw_expr, "=IF(combobox9.value = '5','2','1')");
    }
}

#[test]
fn test_priority_numberinput6_real_world() {
    let path = PathBuf::from("tests/fixtures/real_world_2.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    let n6 = analyses.iter()
        .find(|a| a.component_id == "numberinput6");
    
    if let Some(a) = n6 {
        assert_eq!(a.priority_result, ExpDefaultValuePriority::OnlyExp);
        assert!(a.calc_exp_expr.as_ref().unwrap().raw_expr.contains("EstimatedAnnualInterestRate"));
    }
}

#[test]
fn test_priority_format_human() {
    let path = PathBuf::from("tests/fixtures/test_superpage.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);
    let report = metadata_checker::priority::format_priority_human(&analyses);

    assert!(report.contains("[D]"), "Report should contain [D] marker");
    assert!(report.contains("[E]"), "Report should contain [E] marker");
    assert!(report.contains("=== 图例 ==="), "Report should have legend");
    assert!(report.contains("=== 规则来源 ==="), "Report should have rules source");
}

#[test]
fn test_priority_combobox2_refs() {
    let path = PathBuf::from("tests/fixtures/real_world_1.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let analyses = analyze_priority(&meta);

    let combobox2 = analyses.iter()
        .find(|a| a.component_id == "combobox2")
        .expect("combobox2 should be analyzed");

    let dv = combobox2.default_value_expr.as_ref().expect("has defaultValue");
    assert!(dv.refs.iter().any(|r| matches!(r, metadata_checker::superpage::RefType::ComponentValue(id) if id == "combobox9")));
}
