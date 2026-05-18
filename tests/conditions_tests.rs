use metadata_checker::conditions::{
    ConditionRecord, ConditionType, EffectType, OwnerType, SubjectType, scan_conditions,
};
use metadata_checker::parser::parse_file;
use std::path::Path;

/// 加载 fixture 并返回条件列表
fn load_conditions(fixture_name: &str) -> Vec<ConditionRecord> {
    let path = Path::new("tests/fixtures").join(fixture_name);
    let meta = parse_file(&path).expect("Failed to parse fixture file");
    let spg = meta.superpage.expect("Not a SuperPage fixture");
    scan_conditions(&spg, meta.input_path.as_deref())
}

/// 按条件类型过滤
fn by_type(records: &[ConditionRecord], ct: ConditionType) -> Vec<&ConditionRecord> {
    records.iter().filter(|r| r.condition_type == ct).collect()
}

/// 按 owner_id 过滤

#[test]
fn test_conditions_count_and_types() {
    let records = load_conditions("test_conditions.spg");
    assert!(
        !records.is_empty(),
        "Should extract at least some conditions"
    );

    // 确认条件类型覆盖
    let visible = by_type(&records, ConditionType::VisibleCondition);
    assert!(!visible.is_empty(), "Should have visibleCondition records");

    let enable = by_type(&records, ConditionType::EnableCondition);
    assert!(!enable.is_empty(), "Should have enable/disable records");

    let action_ce = by_type(&records, ConditionType::ActionConditionExp);
    assert!(
        !action_ce.is_empty(),
        "Should have action conditionExp records"
    );

    let filter = by_type(&records, ConditionType::SourceFilterExp);
    assert!(!filter.is_empty(), "Should have source filter exp records");

    let filter_clause = by_type(&records, ConditionType::SourceFilterClause);
    assert!(
        !filter_clause.is_empty(),
        "Should have source filter clause records"
    );
}

#[test]
fn test_effect_and_subject_types() {
    let records = load_conditions("test_conditions.spg");

    // visibleCondition 对应 Show + Component
    let visible = by_type(&records, ConditionType::VisibleCondition);
    for r in &visible {
        assert_eq!(
            r.effect_type,
            EffectType::Show,
            "visible should map to Show"
        );
        assert_eq!(
            r.subject_type,
            SubjectType::Component,
            "visible owner is component"
        );
    }

    // action conditionExp 对应 Execute + Action
    let action_ce = by_type(&records, ConditionType::ActionConditionExp);
    for r in &action_ce {
        assert_eq!(
            r.effect_type,
            EffectType::Execute,
            "action conditionExp should map to Execute"
        );
        assert_eq!(
            r.subject_type,
            SubjectType::Action,
            "action conditionExp owner is action"
        );
    }

    // source filter 对应 Filter + ModelSource
    let filter = by_type(&records, ConditionType::SourceFilterExp);
    for r in &filter {
        assert_eq!(
            r.effect_type,
            EffectType::Filter,
            "source filter should map to Filter"
        );
        assert_eq!(
            r.subject_type,
            SubjectType::ModelSource,
            "source filter owner is model_source"
        );
    }
}

#[test]
fn test_referenced_symbols() {
    let records = load_conditions("test_conditions.spg");

    // input1 visibleCondition 引用 input1.value
    let input1_visible = records
        .iter()
        .find(|r| r.owner_id == "input1" && r.condition_type == ConditionType::VisibleCondition)
        .expect("input1 visibleCondition should exist");
    let refs: Vec<String> = input1_visible.referenced_symbols.clone();
    assert!(
        refs.iter().any(|s| s.contains("input1")),
        "input1 visibleCondition should reference input1"
    );

    // button1 disableCondition 引用 input1.value
    let btn1_disable = records
        .iter()
        .find(|r| r.owner_id == "button1" && r.condition_type == ConditionType::EnableCondition)
        .expect("button1 disableCondition should exist");
    assert!(
        btn1_disable
            .referenced_symbols
            .iter()
            .any(|s| s.contains("input1")),
        "button1 disableCondition should reference input1"
    );

    // panel1 visibleCondition 引用 $user.dept_id
    let panel1_visible = records
        .iter()
        .find(|r| r.owner_id == "panel1" && r.condition_type == ConditionType::VisibleCondition)
        .expect("panel1 visibleCondition should exist");
    assert!(
        panel1_visible
            .referenced_symbols
            .iter()
            .any(|s| s.contains("user")),
        "panel1 visibleCondition should reference user property"
    );
}

#[test]
fn test_json_paths_present() {
    let records = load_conditions("test_conditions.spg");
    for r in &records {
        assert!(
            !r.json_path.is_empty(),
            "Every condition must have a json_path: {:?}",
            r.condition_id
        );
        assert!(
            r.json_path.contains("canvas.components") || r.json_path.contains("sources["),
            "json_path should point to canvas or sources: {}",
            r.json_path
        );
    }
}

#[test]
fn test_source_file_present() {
    let records = load_conditions("test_conditions.spg");
    for r in &records {
        assert!(
            r.source_file.is_some(),
            "Every condition should have a source_file"
        );
    }
}

#[test]
fn test_action_condition_json_path() {
    let records = load_conditions("test_conditions.spg");
    let action_ce = records
        .iter()
        .find(|r| r.condition_type == ConditionType::ActionConditionExp)
        .expect("action conditionExp should exist");
    assert!(
        action_ce.json_path.contains("actions["),
        "action json_path should contain actions[]"
    );
}

#[test]
fn test_source_filter_clause_combined_expr() {
    let records = load_conditions("test_conditions.spg");
    let clause = records
        .iter()
        .find(|r| r.condition_type == ConditionType::SourceFilterClause)
        .expect("filter clause should exist");
    assert!(
        clause.raw_expr.contains("="),
        "combined clause should contain operator"
    );
    assert!(
        clause
            .referenced_symbols
            .iter()
            .any(|s| s.contains("model1")),
        "filter clause should reference model1"
    );
}

#[test]
fn test_empty_condition_diagnostic() {
    // 使用 visibility_contract.spg：text_unsupported 的 hidden 字段不是标准表达式
    // 这里主要验证条件记录不为空且包含诊断
    let records = load_conditions("test_superpage.spg");
    assert!(
        !records.is_empty(),
        "test_superpage.spg should produce conditions"
    );
    for r in &records {
        if r.raw_expr.is_empty() {
            assert!(
                !r.diagnostics.is_empty(),
                "Empty condition should have diagnostics: {:?}",
                r.condition_id
            );
        }
    }
}

#[test]
fn test_multiline_expression_preserved() {
    let records = load_conditions("test_conditions.spg");
    let input1_default = records
        .iter()
        .find(|r| r.owner_id == "input1" && r.condition_type == ConditionType::DefaultValueExp)
        .expect("input1 defaultValue should exist");
    assert!(
        input1_default.raw_expr.contains("IF"),
        "Multiline/defaultValue expression should be preserved"
    );
    assert_eq!(
        input1_default.normalized_expr.trim(),
        input1_default.raw_expr.trim(),
        "normalized_expr should be a trimmed version of raw_expr"
    );
}

#[test]
fn test_condition_id_unique() {
    let records = load_conditions("test_conditions.spg");
    let mut ids = Vec::new();
    for r in &records {
        assert!(
            !ids.contains(&r.condition_id),
            "condition_id should be unique: {}",
            r.condition_id
        );
        ids.push(r.condition_id.clone());
    }
}

#[test]
fn test_default_value_exp_extraction() {
    let records = load_conditions("test_conditions.spg");

    // 验证 defaultValueExp 作为 DefaultValueExp 被抽取
    let dv_exp = records
        .iter()
        .filter(|r| {
            r.owner_id == "input2"
                && r.condition_type == ConditionType::DefaultValueExp
                && r.raw_expr.contains("IF")
        })
        .collect::<Vec<_>>();
    assert!(
        !dv_exp.is_empty(),
        "input2 defaultValueExp should be extracted as DefaultValueExp"
    );

    // 确认 defaultValueExp 的 raw_expr 包含平台表达式
    let record = dv_exp[0];
    assert_eq!(
        record.effect_type,
        EffectType::Compute,
        "defaultValueExp should map to Compute effect"
    );
    assert!(
        record
            .referenced_symbols
            .iter()
            .any(|s| s.contains("input1")),
        "defaultValueExp should reference input1"
    );
    assert!(
        record.json_path.contains("defaultValueExp"),
        "json_path should contain 'defaultValueExp': {}",
        record.json_path
    );
}

#[test]
fn test_nested_action_extraction() {
    let records = load_conditions("test_conditions.spg");

    // 验证嵌套 action.conditionExp 被抽取
    let nested_ce = records
        .iter()
        .filter(|r| {
            r.condition_type == ConditionType::ActionConditionExp
                && r.owner_id.contains("nestedButton")
        })
        .collect::<Vec<_>>();
    assert!(
        !nested_ce.is_empty(),
        "nested action conditionExp should be extracted"
    );

    let ce = nested_ce[0];
    assert_eq!(
        ce.effect_type,
        EffectType::Execute,
        "nested action conditionExp should map to Execute"
    );
    assert_eq!(
        ce.subject_type,
        SubjectType::Action,
        "nested action subject should be Action"
    );
    assert!(
        ce.json_path
            .starts_with("canvas.components[3].components[1]")
            && ce.json_path.contains("actions")
            && ce.json_path.contains("conditionExp"),
        "nested action json_path should traverse nested structure into nestedPanel: {}",
        ce.json_path
    );
    assert!(
        ce.referenced_symbols.iter().any(|s| s.contains("input2")),
        "nested action conditionExp should reference input2"
    );

    // 验证嵌套 action.condition 被抽取
    let nested_cond = records
        .iter()
        .filter(|r| {
            r.condition_type == ConditionType::ActionCondition
                && r.owner_id.contains("nestedButton")
        })
        .collect::<Vec<_>>();
    assert!(
        !nested_cond.is_empty(),
        "nested action condition should be extracted"
    );

    let cond = nested_cond[0];
    assert!(
        cond.json_path.contains(".condition"),
        "nested condition json_path should end with .condition: {}",
        cond.json_path
    );
    assert_eq!(
        cond.raw_expr, "=nestedButton.value != ''",
        "nested action condition raw_expr should be preserved"
    );
}

#[test]
fn test_cli_json_output_snake_case_enums() {
    // 通过 serde_json::to_value 验证枚举序列化为 snake_case
    let record = ConditionRecord {
        condition_id: "test#visibleCondition#0".to_string(),
        condition_type: ConditionType::VisibleCondition,
        effect_type: EffectType::Show,
        subject_type: SubjectType::Component,
        raw_expr: "=true".to_string(),
        normalized_expr: "=true".to_string(),
        source_file: Some("test.spg".to_string()),
        json_path: "canvas.components[0].visibleCondition".to_string(),
        owner_type: OwnerType::Component,
        owner_id: "test".to_string(),
        referenced_symbols: vec![],
        diagnostics: vec![],
    };

    let json_val = serde_json::to_value(&record).unwrap();
    let obj = json_val.as_object().unwrap();

    assert_eq!(obj["condition_type"], "visible_condition");
    assert_eq!(obj["effect_type"], "show");
    assert_eq!(obj["subject_type"], "component");
    assert_eq!(obj["owner_type"], "component");

    // 验证 ActionConditionExp 的串行化
    let record2 = ConditionRecord {
        condition_id: "test#action#0".to_string(),
        condition_type: ConditionType::ActionConditionExp,
        effect_type: EffectType::Execute,
        subject_type: SubjectType::Action,
        raw_expr: "=input1.value".to_string(),
        normalized_expr: "=input1.value".to_string(),
        source_file: Some("test.spg".to_string()),
        json_path: "canvas.components[0].actions[0].conditionExp".to_string(),
        owner_type: OwnerType::Action,
        owner_id: "btn:act1".to_string(),
        referenced_symbols: vec![],
        diagnostics: vec![],
    };

    let json_val2 = serde_json::to_value(&record2).unwrap();
    let obj2 = json_val2.as_object().unwrap();
    assert_eq!(obj2["condition_type"], "action_condition_exp");
    assert_eq!(obj2["effect_type"], "execute");
    assert_eq!(obj2["subject_type"], "action");
    assert_eq!(obj2["owner_type"], "action");

    // 验证 SourceFilterExp 的串行化
    let record3 = ConditionRecord {
        condition_id: "src#filter#0".to_string(),
        condition_type: ConditionType::SourceFilterExp,
        effect_type: EffectType::Filter,
        subject_type: SubjectType::ModelSource,
        raw_expr: "=x".to_string(),
        normalized_expr: "=x".to_string(),
        source_file: Some("test.spg".to_string()),
        json_path: "sources[0].filter.clauses[0].exp".to_string(),
        owner_type: OwnerType::ModelSource,
        owner_id: "model1".to_string(),
        referenced_symbols: vec![],
        diagnostics: vec![],
    };

    let json_val3 = serde_json::to_value(&record3).unwrap();
    let obj3 = json_val3.as_object().unwrap();
    assert_eq!(obj3["condition_type"], "source_filter_exp");
    assert_eq!(obj3["effect_type"], "filter");
    assert_eq!(obj3["subject_type"], "model_source");
    assert_eq!(obj3["owner_type"], "model_source");
}

#[test]
fn test_cli_json_output_has_source_file_and_normalized_expr() {
    let records = load_conditions("test_conditions.spg");

    // 确认每条纪录都包含必要字段（通过 serde 串行化）
    for r in &records {
        let json_val = serde_json::to_value(r).unwrap();
        let obj = json_val.as_object().unwrap();

        assert!(
            obj.contains_key("source_file"),
            "condition {} should have source_file",
            r.condition_id
        );
        assert!(
            obj.contains_key("normalized_expr"),
            "condition {} should have normalized_expr",
            r.condition_id
        );

        // source_file 不能为 null（虽然 Option 但 fixture 里应有值）
        let sf = obj["source_file"].as_str();
        assert!(
            sf.is_some() && !sf.unwrap().is_empty(),
            "condition {} should have non-empty source_file",
            r.condition_id
        );
    }
}
