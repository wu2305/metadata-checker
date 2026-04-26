use metadata_checker::dependency::DependencyGraph;
use metadata_checker::superpage::parse_superpage;
use std::path::PathBuf;

// ============================================================
// 一、新增活动（已有表达式覆盖）
// ============================================================

#[test]
fn test_real_world_parse_新增活动() {
    let path = PathBuf::from("tests/fixtures/real_world_1.spg");
    let meta = parse_superpage(&path).expect("Failed to parse real world spg");

    assert_ne!(meta.version, None);
    assert_ne!(meta.theme, None);
    assert_ne!(meta.params.len(), 0);
    assert_ne!(meta.sources.len(), 0);
    assert_ne!(meta.components.len(), 0);
    assert_ne!(
        meta.expressions.len(),
        0,
        "Should have expressions in 新增活动"
    );

    println!("Components: {}", meta.components.len());
    println!("Expressions: {}", meta.expressions.len());
}

#[test]
fn test_real_world_dependency_graph_新增活动() {
    let path = PathBuf::from("tests/fixtures/real_world_1.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    let order = graph.topological_sort();
    assert_ne!(order.len(), 0);

    let cycles = graph.detect_cycles();
    println!("Cycles found: {}", cycles.len());
    for cycle in &cycles {
        println!("  Cycle: {}", cycle.join(" → "));
    }
}

// ============================================================
// 二、购车贷款（exp 过滤条件为主）
// ============================================================

#[test]
fn test_real_world_parse_购车贷款() {
    let path = PathBuf::from("tests/fixtures/real_world_2.spg");
    let meta = parse_superpage(&path).expect("Failed to parse real world spg");

    assert_ne!(meta.version, None);
    assert_ne!(meta.components.len(), 0);
    // 购车贷款主要是 exp 过滤条件，不以 = 开头
    assert_ne!(meta.components.len(), 0);
}

#[test]
fn test_real_world_dependency_graph_购车贷款() {
    let path = PathBuf::from("tests/fixtures/real_world_2.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    let order = graph.topological_sort();
    assert_ne!(order.len(), 0);

    let cycles = graph.detect_cycles();
    println!("Cycles found: {}", cycles.len());
}

// ============================================================
// 三、代付款协议（数据加工流程：dataflow + filter exp）
// ============================================================

#[test]
fn test_real_world_parse_代付款协议() {
    let path = PathBuf::from("tests/fixtures/real_world_3.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    assert_ne!(meta.version, None);
    assert_eq!(meta.theme, Some("mobile".to_string()));
    assert_eq!(meta.params.len(), 7, "Should have 7 params");
    assert_eq!(meta.sources.len(), 8, "Should have 8 sources");

    // 验证有 dataflow 类型的数据源（数据加工流程）
    let dataflows: Vec<_> = meta
        .sources
        .iter()
        .filter(|s| s.model_type.as_deref() == Some("dataflow"))
        .collect();
    assert_eq!(dataflows.len(), 2, "Should have 2 dataflow sources");

    // 验证有 dwtable 类型的数据源
    let dwtables: Vec<_> = meta
        .sources
        .iter()
        .filter(|s| s.model_type.as_deref() == Some("dwtable"))
        .collect();
    assert_eq!(dwtables.len(), 6, "Should have 6 dwtable sources");
}

#[test]
fn test_real_world_代付款协议_组件结构() {
    let path = PathBuf::from("tests/fixtures/real_world_3.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // 验证有 mobile 主题的组件
    let has_components = !meta.components.is_empty();
    assert_eq!(has_components, true);

    // 验证有 text 组件
    let text_comps: Vec<_> = meta
        .components
        .iter()
        .filter(|c| c.component_type == "text")
        .collect();
    assert_ne!(text_comps.len(), 0, "Should have text components");

    // text1 应该有宏表达式
    let text1_exprs: Vec<_> = meta
        .expressions
        .iter()
        .filter(|e| e.component_id == "text1")
        .collect();
    assert_ne!(text1_exprs.len(), 0, "text1 should have expressions");
}

#[test]
fn test_real_world_代付款协议_数据源过滤条件() {
    let path = PathBuf::from("tests/fixtures/real_world_3.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // 解析 raw JSON 检查 sources 中的 filter 表达式
    let raw = &meta.raw;
    if let Some(sources) = raw.get("sources").and_then(|v| v.as_array()) {
        // model7 应该有 TOSTR 过滤条件
        let model7 = sources
            .iter()
            .find(|s| s.get("id").and_then(|v| v.as_str()) == Some("model7"));
        assert_ne!(model7, None, "Should have model7");

        if let Some(m7) = model7 {
            if let Some(filter) = m7.get("filter") {
                let filter_str = serde_json::to_string(filter).unwrap();
                assert_eq!(
                    filter_str.contains("TOSTR"),
                    true,
                    "model7 filter should contain TOSTR"
                );
                assert_eq!(
                    filter_str.contains("TODAY()"),
                    true,
                    "model7 filter should contain TODAY()"
                );
            }
        }

        // model8 应该有 AND 和 IS NOT NULL 过滤条件
        let model8 = sources
            .iter()
            .find(|s| s.get("id").and_then(|v| v.as_str()) == Some("model8"));
        assert_ne!(model8, None, "Should have model8");

        if let Some(m8) = model8 {
            if let Some(filter) = m8.get("filter") {
                let filter_str = serde_json::to_string(filter).unwrap();
                assert_eq!(
                    filter_str.contains("AND"),
                    true,
                    "model8 filter should contain AND"
                );
                assert_eq!(
                    filter_str.contains("IS NOT NULL"),
                    true,
                    "model8 filter should contain IS NOT NULL"
                );
            }
        }

        // model1 应该有 param 引用
        let model1 = sources
            .iter()
            .find(|s| s.get("id").and_then(|v| v.as_str()) == Some("model1"));
        if let Some(m1) = model1 {
            if let Some(filter) = m1.get("filter") {
                let filter_str = serde_json::to_string(filter).unwrap();
                assert_eq!(
                    filter_str.contains("param3") || filter_str.contains("param4"),
                    true,
                    "model1 filter should reference params"
                );
            }
        }
    }
}

#[test]
fn test_real_world_代付款协议_依赖图() {
    let path = PathBuf::from("tests/fixtures/real_world_3.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    let order = graph.topological_sort();
    assert_ne!(order.len(), 0);

    let cycles = graph.detect_cycles();
    assert_eq!(cycles.len(), 0, "代付款协议 should have no cycles");
}

// ============================================================
// 四、销售合同（复杂表达式：IF + CONCAT + USER_INGROUP）
// ============================================================

#[test]
fn test_real_world_parse_销售合同() {
    let path = PathBuf::from("tests/fixtures/real_world_4.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    assert_ne!(meta.version, None);
    // 销售合同没有 params
    assert_eq!(meta.params.len(), 0, "销售合同 should have no params");
    // 有数据源
    assert_eq!(meta.sources.len(), 1, "销售合同 should have 1 source");

    // 验证有 fieldsFilter 特殊组件
    let fields_filter: Vec<_> = meta
        .components
        .iter()
        .filter(|c| c.component_type == "fieldsFilter")
        .collect();
    assert_ne!(fields_filter.len(), 0, "Should have fieldsFilter component");

    // 验证有 fieldsFilter.comp 组件
    let filter_comps: Vec<_> = meta
        .components
        .iter()
        .filter(|c| c.component_type == "fieldsFilter.comp")
        .collect();
    assert_ne!(
        filter_comps.len(),
        0,
        "Should have fieldsFilter.comp components"
    );
}

#[test]
fn test_real_world_销售合同_数据源表达式() {
    let path = PathBuf::from("tests/fixtures/real_world_4.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // 检查 sources 中复杂的 exp 表达式
    let raw = &meta.raw;
    if let Some(sources) = raw.get("sources").and_then(|v| v.as_array()) {
        if let Some(model1) = sources.first() {
            if let Some(filter) = model1.get("filter") {
                let filter_str = serde_json::to_string(filter).unwrap();

                // 验证有 USER_INGROUP 函数
                assert_eq!(
                    filter_str.contains("USER_INGROUP"),
                    true,
                    "销售合同 filter should contain USER_INGROUP"
                );
                // 验证有 OR 逻辑
                assert_eq!(
                    filter_str.contains("OR"),
                    true,
                    "销售合同 filter should contain OR"
                );
                // 验证有 IS NOT NULL
                assert_eq!(
                    filter_str.contains("IS NOT NULL"),
                    true,
                    "销售合同 filter should contain IS NOT NULL"
                );
            }
        }
    }
}

#[test]
fn test_real_world_销售合同_组件表达式() {
    let path = PathBuf::from("tests/fixtures/real_world_4.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");

    // 检查 canvas 下的 panel 中是否有 IF/CONCAT 表达式
    let raw = &meta.raw;
    if let Some(canvas) = raw.get("canvas").and_then(|v| v.as_object()) {
        if let Some(comps) = canvas.get("components").and_then(|v| v.as_array()) {
            for comp in comps {
                if let Some(obj) = comp.as_object() {
                    // 检查 value 或 text 中的 IF/CONCAT
                    for field in ["value", "text", "defaultValue"] {
                        if let Some(val) = obj.get(field).and_then(|v| v.as_str()) {
                            if val.starts_with('=') || val.starts_with("${") {
                                println!(
                                    "Found expression: {}.{} = {}",
                                    obj.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                    field,
                                    val
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_real_world_销售合同_依赖图() {
    let path = PathBuf::from("tests/fixtures/real_world_4.spg");
    let meta = parse_superpage(&path).expect("Failed to parse");
    let graph = DependencyGraph::new(&meta);

    let order = graph.topological_sort();
    assert_ne!(order.len(), 0);

    let cycles = graph.detect_cycles();
    assert_eq!(cycles.len(), 0, "销售合同 should have no cycles");
}
