use metadata_checker::output::schema::{
    AiOutput, Diagnostic, DiagnosticSeverity, Evidence, Location, OutputKind,
};
use metadata_checker::visualization::builder::VisualGraphBuilder;
use metadata_checker::visualization::options::VisualGraphOptions;
use metadata_checker::visualization::{
    render_echarts_from_ai_output, render_mermaid_from_ai_output,
};
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
            .with_source_file("models/users.tbl")
            .with_edge_type("Reads token=abc123")
            .with_raw_expr("${model:users}; password=raw_user_secret_123"),
    );
    output.evidence.push(
        Evidence::new("model users", "Target model")
            .with_node_id("model:users")
            .with_source_file("models/users.tbl"),
    );
    output
}

/// 创建覆盖嵌套元数据、诊断和标识字段泄露的测试 AiOutput
fn create_test_output_with_nested_visual_secrets() -> AiOutput {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:token=summary_token_001",
            "target_name": "Dashboard password=summary_password_001",
            "source_file": "pages/cookie=summary_cookie_001.spg",
        }),
    );
    output.details = Some(json!({
        "readers": [{
            "id": "comp:cookie=detail_cookie_001",
            "name": "Reader api_key=detail_api_key_001",
            "source_file": "pages/secret=detail_secret_001.spg",
            "from": "comp:cookie=detail_cookie_001",
            "to": "model:token=detail_token_001",
            "edge_type": "reads",
            "edge_label": "uses token=detail_edge_token_001",
            "config": {
                "api_key": "nested_api_key_001",
                "headers": {
                    "cookie": "nested_cookie_001"
                }
            },
            "raw_expr": "password=nested_raw_password_001"
        }]
    }));
    output.evidence.push(
        Evidence::new(
            "Evidence secret=evidence_label_secret_001",
            "reason token=evidence_reason_token_001",
        )
        .with_node_id("comp:token=evidence_node_token_001")
        .with_source_file("pages/password=evidence_source_password_001.spg")
        .with_edge_type("DependsOn api_key=evidence_edge_api_key_001")
        .with_raw_expr(
            "${model:secret=evidence_target_secret_001}; cookie=evidence_raw_cookie_001",
        ),
    );
    output.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "TOKEN_DIAGNOSTIC".to_string(),
        message: "diagnostic leaked token=diagnostic_token_001".to_string(),
        location: Location {
            source_file: Some("pages/api_key=diagnostic_api_key_001.spg".to_string()),
            node_id: Some("comp:cookie=diagnostic_cookie_001".to_string()),
            json_path: Some("$.password=diagnostic_password_001".to_string()),
        },
        suggestion: Some("rotate secret=diagnostic_secret_001".to_string()),
    });
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
    assert!(
        !graph.nodes.is_empty(),
        "Diagnostic graph should have nodes"
    );
    assert!(
        graph.nodes.iter().all(|n| matches!(
            n.kind,
            metadata_checker::visualization::options::NodeKind::Diagnostic
        )),
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
    assert!(
        mermaid1.starts_with("graph TD"),
        "Should start with graph TD"
    );
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
        series["data"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "ECharts should have nodes"
    );
    assert!(
        series["categories"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
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

    let sensitive_values = ["secret123", "abc123", "xyz", "raw_user_secret_123"];
    let mermaid_lower = mermaid.to_lowercase();

    for value in &sensitive_values {
        assert!(
            !mermaid_lower.contains(value),
            "Mermaid should not contain sensitive plain value: {}",
            value
        );
    }

    let echarts_str = echarts.to_string().to_lowercase();
    for value in &sensitive_values {
        assert!(
            !echarts_str.contains(value),
            "ECharts JSON should not contain sensitive plain value: {}",
            value
        );
    }

    let series = &echarts["series"][0];
    let node_tooltips: Vec<&str> = series["data"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|node| node.get("tooltip").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();

    assert!(
        !node_tooltips
            .iter()
            .any(|tooltip| tooltip.to_lowercase().contains("raw_user_secret_123")),
        "ECharts node tooltip should not contain raw metadata value"
    );

    let link_tooltips: Vec<&str> = series["links"]
        .as_array()
        .map(|links| {
            links
                .iter()
                .filter_map(|link| link.get("tooltip").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();

    assert!(
        !link_tooltips
            .iter()
            .any(|tooltip| tooltip.contains("secret123") || tooltip.contains("abc123")),
        "ECharts edge tooltip should not contain sensitive raw values"
    );
}

#[test]
fn test_nested_metadata_sensitive_keys_are_redacted() {
    let output = create_test_output_with_nested_visual_secrets();
    let mut opts = VisualGraphOptions::default();
    opts.include_evidence = true;

    let graph = VisualGraphBuilder::from_ai_output(&output, &opts);
    let graph_str = serde_json::to_string(&graph).unwrap().to_lowercase();

    for leaked in [
        "nested_api_key_001",
        "nested_cookie_001",
        "nested_raw_password_001",
        "detail_api_key_001",
        "detail_secret_001",
    ] {
        assert!(
            !graph_str.contains(leaked),
            "VisualGraph should redact nested metadata value: {}",
            leaked
        );
    }
}

#[test]
fn test_diagnostic_values_are_redacted_in_graph_mermaid_and_echarts() {
    let mut output = AiOutput::new(OutputKind::PageQuery, json!({}));
    output.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "AUTH_FAILED".to_string(),
        message: "token=diagnostic_token_002 password=diagnostic_password_002".to_string(),
        location: Location {
            source_file: Some("pages/cookie=diagnostic_cookie_002.spg".to_string()),
            node_id: Some("comp:secret=diagnostic_secret_002".to_string()),
            json_path: Some("$.api_key=diagnostic_api_key_002".to_string()),
        },
        suggestion: Some("replace api_key=diagnostic_suggestion_api_key_002".to_string()),
    });

    let graph = VisualGraphBuilder::from_ai_output(&output, &VisualGraphOptions::default());
    let mermaid = render_mermaid_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
    let echarts = render_echarts_from_ai_output(&output, &VisualGraphOptions::default()).unwrap();
    let combined = format!(
        "{}\n{}\n{}",
        serde_json::to_string(&graph).unwrap(),
        mermaid,
        echarts
    )
    .to_lowercase();

    for leaked in [
        "diagnostic_token_002",
        "diagnostic_password_002",
        "diagnostic_cookie_002",
        "diagnostic_secret_002",
        "diagnostic_api_key_002",
        "diagnostic_suggestion_api_key_002",
    ] {
        assert!(
            !combined.contains(leaked),
            "diagnostic rendering should redact sensitive value: {}",
            leaked
        );
    }
}

#[test]
fn test_visual_identifiers_and_edge_values_are_redacted() {
    let output = create_test_output_with_nested_visual_secrets();
    let mut opts = VisualGraphOptions::default();
    opts.include_evidence = true;

    let graph = VisualGraphBuilder::from_ai_output(&output, &opts);
    let mermaid = render_mermaid_from_ai_output(&output, &opts).unwrap();
    let echarts = render_echarts_from_ai_output(&output, &opts).unwrap();
    let combined = format!(
        "{}\n{}\n{}",
        serde_json::to_string(&graph).unwrap(),
        mermaid,
        echarts
    )
    .to_lowercase();

    for leaked in [
        "summary_token_001",
        "summary_password_001",
        "summary_cookie_001",
        "detail_cookie_001",
        "detail_token_001",
        "detail_edge_token_001",
        "evidence_node_token_001",
        "evidence_source_password_001",
        "evidence_target_secret_001",
        "evidence_raw_cookie_001",
        "evidence_reason_token_001",
        "diagnostic_token_001",
    ] {
        assert!(
            !combined.contains(leaked),
            "visual output should redact identifier/edge value: {}",
            leaked
        );
    }
}

#[test]
fn test_include_evidence_false_excludes_evidence_details() {
    let output = create_test_output_with_nested_visual_secrets();
    let mut opts = VisualGraphOptions::default();
    opts.include_evidence = false;

    let graph = VisualGraphBuilder::from_ai_output(&output, &opts);
    let echarts = render_echarts_from_ai_output(&output, &opts).unwrap();
    let combined =
        format!("{}\n{}", serde_json::to_string(&graph).unwrap(), echarts).to_lowercase();

    assert!(
        graph.edges.iter().all(|edge| edge.evidence.is_none()),
        "include_evidence=false should remove edge evidence"
    );
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| !node.metadata.contains_key("raw_expr")),
        "include_evidence=false should remove raw_expr metadata"
    );
    assert!(
        !combined.contains("raw_expr"),
        "include_evidence=false output should not contain raw_expr key"
    );
    assert!(
        !combined.contains("evidence_reason_token_001"),
        "include_evidence=false output should not contain edge evidence details"
    );
}

#[test]
fn test_sensitive_node_ids_keep_distinct_safe_identities() {
    let mut output = AiOutput::new(
        OutputKind::PageQuery,
        json!({
            "target_id": "page:collision",
            "target_name": "Collision Page",
        }),
    );
    output.evidence.push(
        Evidence::new("Token A node", "reads model")
            .with_node_id("comp:token=a")
            .with_edge_type("Reads")
            .with_raw_expr("${model:orders}"),
    );
    output.evidence.push(
        Evidence::new("Token B node", "reads model")
            .with_node_id("comp:token=b")
            .with_edge_type("Reads")
            .with_raw_expr("${model:customers}"),
    );

    let graph = VisualGraphBuilder::from_ai_output(&output, &VisualGraphOptions::default());
    let colliding_nodes = graph
        .nodes
        .iter()
        .filter(|node| node.id.starts_with("comp:token=***__h"))
        .collect::<Vec<_>>();

    assert_eq!(
        colliding_nodes.len(),
        2,
        "different sensitive raw ids should remain distinct after redaction"
    );
    assert_ne!(
        colliding_nodes[0].id, colliding_nodes[1].id,
        "safe ids should include distinct stable hash suffixes"
    );

    let edge_sources = graph
        .edges
        .iter()
        .map(|edge| edge.from.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(
        colliding_nodes
            .iter()
            .all(|node| edge_sources.contains(node.id.as_str())),
        "edge endpoints should use the same safe id mapping as nodes"
    );

    let graph_str = serde_json::to_string(&graph).unwrap();
    assert!(!graph_str.contains("comp:token=a"));
    assert!(!graph_str.contains("comp:token=b"));
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
    output
        .evidence
        .push(Evidence::new("Numeric ID", "Test").with_node_id("123_invalid"));

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
