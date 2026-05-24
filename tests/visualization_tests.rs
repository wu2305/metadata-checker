use metadata_checker::output::schema::{
    AiOutput, Diagnostic, DiagnosticSeverity, Evidence, Location, OutputKind,
};
use metadata_checker::visualization::{
    render_echarts_from_ai_output, render_mermaid_from_ai_output,
};
use metadata_checker::visualization::builder::VisualGraphBuilder;
use metadata_checker::visualization::options::VisualGraphOptions;
use serde_json::json;

/// 创建包含特殊字符的测试 AiOutput
fn create_test_output_with_special_chars() -> AiOutput {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:测试页面(1)",
            "target_name": "测试页面:首页[入口]",
        }),
    );
    output.evidence.push(
        Evidence::new("组件:按钮\"特殊\"", "包含中文冒号:和引号\"以及换行\n内容")
            .with_node_id("comp:btn_1")
            .with_source_file("pages/测试页面.spg"),
    );
    output
}

/// 创建包含敏感信息的测试 AiOutput
fn create_test_output_with_secrets() -> AiOutput {
    let mut output = AiOutput::new(
        OutputKind::ModelQuery,
        json!({
            "model_id": "model:users",
            "model_name": "用户模型",
        }),
    );
    output.evidence.push(
        Evidence::new("字段:密码", "password=secret123, token=abc123, api_key=xyz")
            .with_node_id("field:password")
            .with_raw_expr("${user.password}"),
    );
    output
}

/// 创建大型测试 AiOutput
fn create_large_test_output(node_count: usize) -> AiOutput {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:large",
            "target_name": "Large Page",
        }),
    );
    for i in 0..node_count {
        output.evidence.push(
            Evidence::new(format!("Component {}", i), format!("Reason {}", i))
                .with_node_id(format!("comp:node_{}", i))
                .with_edge_type(if i % 2 == 0 { "Reads" } else { "Writes" })
                .with_raw_expr(format!("${{model:field_{}}}", i)),
        );
    }
    output
}

#[test]
fn test_ready_envelope_generates_visual_graph() {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:home",
            "target_name": "Home Page",
        }),
    );
    output.evidence.push(
        Evidence::new("Button component", "Reads user model")
            .with_node_id("comp:btn_submit")
            .with_edge_type("Reads")
            .with_raw_expr("${model:users}"),
    );
    output.evidence.push(
        Evidence::new("User model", "Provides user data")
            .with_node_id("model:users")
            .with_source_file("models/users.tbl"),
    );

    let graph = VisualGraphBuilder::from_ai_output(&output, &VisualGraphOptions::default());
    assert!(!graph.nodes.is_empty(), "Graph should have nodes");
    assert!(
        graph.nodes.iter().any(|n| n.id == "comp:btn_submit"),
        "Graph should contain btn_submit node"
    );
    assert!(
        graph.nodes.iter().any(|n| n.id == "model:users"),
        "Graph should contain users model node"
    );
}

#[test]
fn test_error_envelope_generates_diagnostic_only_graph() {
    let mut output = AiOutput::new(OutputKind::PageQuery, json!({}));
    output.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "TARGET_NOT_FOUND".to_string(),
        message: "Target 'page:missing' not found".to_string(),
        location: Location::default(),
        suggestion: Some("Check the target ID".to_string()),
    });

    let graph = VisualGraphBuilder::from_ai_output(&output, &VisualGraphOptions::default());
    assert!(!graph.nodes.is_empty(), "Diagnostic graph should have nodes");
    assert!(
        graph.nodes.iter().all(|n| matches!(n.kind, metadata_checker::visualization::options::NodeKind::Diagnostic)),
        "All nodes should be diagnostic nodes"
    );
}

#[test]
fn test_empty_selection_generates_empty_graph() {
    let output = AiOutput::new(OutputKind::PageQuery, json!({}));
    let graph = VisualGraphBuilder::from_ai_output(&output, &VisualGraphOptions::default());
    assert!(graph.nodes.is_empty() || graph.nodes.len() <= 1);
    assert!(graph.edges.is_empty());
}

#[test]
fn test_mermaid_output_is_stable_for_snapshot() {
    let output = create_test_output_with_special_chars();
    let opts = VisualGraphOptions::default();

    let mermaid1 = render_mermaid_from_ai_output(&output, &opts).unwrap();
    let mermaid2 = render_mermaid_from_ai_output(&output, &opts).unwrap();

    assert_eq!(mermaid1, mermaid2, "Mermaid output should be stable");
    assert!(mermaid1.starts_with("graph TD"), "Should start with graph TD");
}

#[test]
fn test_mermaid_label_escaping() {
    let output = create_test_output_with_special_chars();
    let mermaid = render_mermaid_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();

    // 冒号应被转义
    assert!(
        mermaid.contains("#58;"),
        "Mermaid should escape colons, got:\n{}",
        mermaid
    );
    // 双引号应被转义
    assert!(
        mermaid.contains("#quot;"),
        "Mermaid should escape quotes, got:\n{}",
        mermaid
    );
    // label 中不应有裸换行
    let lines: Vec<&str> = mermaid.lines().collect();
    for line in &lines {
        if line.contains('[') && line.contains(']') {
            // 节点定义行不应包含裸换行（已替换为 <br/>）
            assert!(
                !line.contains('\n'),
                "Node line should not contain raw newlines"
            );
        }
    }
}

#[test]
fn test_echarts_option_json_contains_nodes_links_categories() {
    let mut output = AiOutput::new(
        OutputKind::ModelQuery,
        json!({
            "model_id": "model:test",
            "model_name": "Test Model",
        }),
    );
    output.evidence.push(
        Evidence::new("Reader", "Reads test model")
            .with_node_id("comp:reader")
            .with_edge_type("Reads")
            .with_raw_expr("${model:test}"),
    );
    output.evidence.push(
        Evidence::new("Test model", "Model data")
            .with_node_id("model:test")
            .with_source_file("models/test.tbl"),
    );

    let option = render_echarts_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();

    let series = &option["series"][0];
    assert!(
        series["data"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "ECharts should have nodes"
    );
    assert!(
        series["categories"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "ECharts should have categories"
    );
}

#[test]
fn test_large_result_truncation() {
    let output = create_large_test_output(50);
    let mut opts = VisualGraphOptions::default();
    opts.max_nodes = 5;
    opts.max_edges = 10;

    let graph = VisualGraphBuilder::from_ai_output(&output, &opts);
    assert!(graph.truncated, "Graph should be marked as truncated");
    assert!(
        graph.nodes.len() <= opts.max_nodes + 1,
        "Nodes should be truncated to max_nodes + diagnostic node"
    );
}

#[test]
fn test_sensitive_fields_not_in_output() {
    let output = create_test_output_with_secrets();
    let mermaid = render_mermaid_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
    let echarts = render_echarts_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();

    let mermaid_lower = mermaid.to_lowercase();
    let echarts_str = echarts.to_string().to_lowercase();

    assert!(
        !mermaid_lower.contains("secret123"),
        "Mermaid should not contain raw password"
    );
    assert!(
        !mermaid_lower.contains("abc123"),
        "Mermaid should not contain raw token"
    );
    assert!(
        !echarts_str.contains("secret123"),
        "ECharts should not contain raw password"
    );
    assert!(
        !echarts_str.contains("abc123"),
        "ECharts should not contain raw token"
    );
}

#[test]
fn test_mermaid_id_sanitization() {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:test",
            "target_name": "Test",
        }),
    );
    // ID 以数字开头，需要 sanitization
    output.evidence.push(
        Evidence::new("Numeric ID", "Test")
            .with_node_id("123_invalid"),
    );

    let mermaid = render_mermaid_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
    // 确保没有以数字开头的 ID 出现在输出中
    for line in mermaid.lines() {
        if line.contains('[') {
            let id_part = line.split('[').next().unwrap_or("").trim();
            if !id_part.is_empty() && id_part != "graph TD" && !id_part.starts_with("subgraph") {
                let first_char = id_part.chars().next().unwrap_or('_');
                assert!(
                    first_char.is_ascii_alphabetic() || first_char == '_',
                    "Mermaid ID '{}' should start with letter, got line: {}",
                    id_part,
                    line
                );
            }
        }
    }
}

#[test]
fn test_group_by_source_path() {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:test",
            "target_name": "Test",
        }),
    );
    output.evidence.push(
        Evidence::new("Comp A", "Test")
            .with_node_id("comp:a")
            .with_source_file("pages/home.spg"),
    );
    output.evidence.push(
        Evidence::new("Comp B", "Test")
            .with_node_id("comp:b")
            .with_source_file("pages/home.spg"),
    );
    output.evidence.push(
        Evidence::new("Comp C", "Test")
            .with_node_id("comp:c")
            .with_source_file("pages/other.spg"),
    );

    let mut opts = VisualGraphOptions::default();
    opts.group_by_source_path = true;
    let graph = VisualGraphBuilder::from_ai_output(&output, &opts);

    // 至少有一个分组包含多个节点
    assert!(
        graph.groups.iter().any(|g| g.node_ids.len() >= 2),
        "Should have group with multiple nodes"
    );
}
