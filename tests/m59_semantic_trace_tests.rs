use metadata_checker::dependency::{DependencyGraph, trace_value_source};
use metadata_checker::superpage::parse_superpage_from_value;
fn expanded(expression: &str) -> String {
    let meta = parse_superpage_from_value(serde_json::json!({
        "canvas": {"components": [
            {"id":"a", "type":"input", "value":expression},
            {"id":"b", "type":"input", "value":"=c.value"},
            {"id":"c", "type":"input", "value":"=1"}
        ]}
    }))
    .unwrap();
    let graph = DependencyGraph::new(&meta);
    trace_value_source(&meta, &graph, "a", "value", 8)
        .unwrap()
        .expanded_expr
}
// 同一组件的两种值引用应展开到相同叶子。
#[test]
fn mixed_value_then_bare() {
    assert_eq!(expanded("=CONCAT(b.value, b)"), "=CONCAT(1, 1)");
}
// 调换引用顺序不能改变展开深度。
#[test]
fn mixed_bare_then_value() {
    assert_eq!(expanded("=CONCAT(b, b.value)"), "=CONCAT(1, 1)");
}
// 非值属性不能提前消耗值追溯的访问状态。
#[test]
fn suffix_before_value() {
    assert_eq!(expanded("=CONCAT(b.step, b.value)"), "=CONCAT(b.step, 1)");
}
// 单一文法作为正对照。
#[test]
fn value_only_control() {
    assert_eq!(expanded("=CONCAT(b.value, b.value)"), "=CONCAT(1, 1)");
}

// 完整属性路径保留，不能把嵌套属性当成 value。
#[test]
fn preserves_member_path_and_original_occurrences() {
    use metadata_checker::superpage::{RefType, parse_expression_refs, reference_occurrences};
    assert_eq!(
        parse_expression_refs("=b.checked.value"),
        vec![RefType::ComponentProperty(
            "b".into(),
            "checked.value".into()
        )]
    );
    let expression = "=CONCAT(\"中文 b\", b . value, b.value)";
    let occurrences = reference_occurrences(expression);
    assert_eq!(
        occurrences
            .iter()
            .map(|item| item.token.as_str())
            .collect::<Vec<_>>(),
        vec!["b . value", "b.value"]
    );
    for occurrence in occurrences {
        assert_eq!(&expression[occurrence.range], occurrence.token);
    }
}

// 插入表达式不能改变运算优先级，也不能覆盖字面量。
#[test]
fn substitution_preserves_precedence_and_literals() {
    let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=2*b.value + LEN(\"b.value\")"},
        {"id":"b","type":"input","value":"=1+2"}
    ]}}))
    .unwrap();
    let trace = trace_value_source(&meta, &DependencyGraph::new(&meta), "a", "value", 8).unwrap();
    assert_eq!(trace.expanded_expr, "=2*(1+2) + LEN(\"b.value\")");
    assert_eq!(trace.issues.is_empty(), true);
}

// 真正的环与深度限制必须显式报告；共享上游不得误报环。
#[test]
fn trace_reports_cycle_and_depth_limits() {
    let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=b.value"},
        {"id":"b","type":"input","value":"=a.value"}
    ]}}))
    .unwrap();
    let graph = DependencyGraph::new(&meta);
    let cycle = trace_value_source(&meta, &graph, "a", "value", 8).unwrap();
    assert_eq!(
        cycle
            .issues
            .iter()
            .map(|issue| issue.code)
            .collect::<Vec<_>>(),
        vec!["TRACE_CYCLE"]
    );
    let limited = trace_value_source(&meta, &graph, "a", "value", 1).unwrap();
    assert_eq!(
        limited
            .issues
            .iter()
            .map(|issue| issue.code)
            .collect::<Vec<_>>(),
        vec!["TRACE_DEPTH_LIMIT"]
    );
}

// 菱形共享上游必须完整展开，且不能产生假循环诊断。
#[test]
fn shared_upstream_is_not_a_cycle() {
    let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=CONCAT(b.value, c.value)"},
        {"id":"b","type":"input","value":"=d.value"},
        {"id":"c","type":"input","value":"=d.value"},
        {"id":"d","type":"input","value":"=1"}
    ]}}))
    .unwrap();
    let trace = trace_value_source(&meta, &DependencyGraph::new(&meta), "a", "value", 8).unwrap();
    assert_eq!(trace.expanded_expr, "=CONCAT(1, 1)");
    assert_eq!(trace.issues.is_empty(), true);
}

// 未支持的宏不能被伪造成模型字段并伪报完整追溯。
#[test]
fn unsupported_macro_is_preserved_and_reported_incomplete() {
    let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=${IF(b.value, 1, 2)}"}
    ]}}))
    .unwrap();
    let trace = trace_value_source(&meta, &DependencyGraph::new(&meta), "a", "value", 8).unwrap();
    assert_eq!(trace.expanded_expr, "=${IF(b.value, 1, 2)}");
    assert_eq!(
        trace
            .issues
            .iter()
            .any(|issue| issue.code == "TRACE_UNRESOLVED_REFERENCE"),
        true
    );
}

// 无 AST 和未闭合结构不能被输出为 complete=true。
#[test]
fn malformed_syntax_never_claims_complete_trace() {
    for expression in ["=", "=(1", "=\"unclosed", "=${b", "=CONCAT(1"] {
        let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
            {"id":"a","type":"input","value":expression}
        ]}}))
        .unwrap();
        let trace =
            trace_value_source(&meta, &DependencyGraph::new(&meta), "a", "value", 8).unwrap();
        assert_eq!(
            trace
                .issues
                .iter()
                .any(|issue| issue.code == "TRACE_UNSUPPORTED_EXPRESSION"),
            true,
            "{expression}"
        );
    }
}

// human 与 JSON 输出都必须暴露未完成原因。
#[test]
fn partial_trace_is_visible_in_both_output_modes() {
    let meta = parse_superpage_from_value(serde_json::json!({"canvas":{"components":[
        {"id":"a","type":"input","value":"=b.value"}
    ]}}))
    .unwrap();
    let graph = DependencyGraph::new(&meta);
    let mut json_output = Vec::new();
    metadata_checker::output::print_component_query_json_to(
        &meta,
        &graph,
        "a",
        false,
        &mut json_output,
    )
    .unwrap();
    let output: serde_json::Value = serde_json::from_slice(&json_output).unwrap();
    assert_eq!(output["details"]["value_trace"]["complete"], false);
    assert_eq!(
        output["details"]["value_trace"]["issues"][0]["code"],
        "TRACE_MISSING_VALUE"
    );
    assert_eq!(
        output["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["code"] == "TRACE_INCOMPLETE"),
        true
    );
    let mut human_output = Vec::new();
    metadata_checker::output::print_component_query_human_to(
        &meta,
        &graph,
        "a",
        false,
        &mut human_output,
    )
    .unwrap();
    let human = String::from_utf8(human_output).unwrap();
    assert_eq!(human.contains("Complete  : false"), true);
    assert_eq!(human.contains("TRACE_MISSING_VALUE"), true);
}
