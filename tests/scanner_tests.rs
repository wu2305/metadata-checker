use metadata_checker::superpage::parse_superpage;
use metadata_checker::graph::{EdgeType, GraphDB};
use metadata_checker::scanner::scan_project;
use std::path::{Path, PathBuf};
use petgraph::visit::EdgeRef;

// ============================================================
// 一、SpgAction 解析测试
// ============================================================

#[test]
fn test_parse_actions_basic() {
    let path = PathBuf::from("tests/fixtures/actions_test.spg");
    let meta = parse_superpage(&path).expect("Failed to parse actions_test.spg");

    let button1 = meta.components.iter().find(|c| c.id == "button1").expect("button1 should exist");
    assert_eq!(button1.actions.len(), 1);
    assert_eq!(button1.actions[0].action_type, "submitData");
    assert_eq!(button1.actions[0].trigger_type, "click");
    assert_eq!(button1.actions[0].submit_component, vec!["input1"]);

    let button2 = meta.components.iter().find(|c| c.id == "button2").expect("button2 should exist");
    assert_eq!(button2.actions.len(), 1);
    assert_eq!(button2.actions[0].action_type, "updateData");
    assert_eq!(button2.actions[0].data_set.as_ref().unwrap(), "model2");
    assert_eq!(button2.actions[0].data_range.as_ref().unwrap(), "resultset");
    assert_eq!(button2.actions[0].field_values.len(), 2);
    assert_eq!(button2.actions[0].field_values[0], ("status".to_string(), "input1.value".to_string(), "exp".to_string()));
    assert_eq!(button2.actions[0].field_values[1], ("updatedAt".to_string(), "TODAY()".to_string(), "exp".to_string()));

    let button4 = meta.components.iter().find(|c| c.id == "button4").expect("button4 should exist");
    assert_eq!(button4.actions.len(), 1);
    assert_eq!(button4.actions[0].action_type, "deleteData");

    let button5 = meta.components.iter().find(|c| c.id == "button5").expect("button5 should exist");
    assert_eq!(button5.actions.len(), 1);
    assert_eq!(button5.actions[0].action_type, "submitData");
    assert_eq!(button5.actions[0].submit_component.is_empty(), true);
}

#[test]
fn test_parse_actions_data_set_array() {
    let path = PathBuf::from("tests/fixtures/actions_test.spg");
    let meta = parse_superpage(&path).expect("Failed to parse actions_test.spg");

    let button3 = meta.components.iter().find(|c| c.id == "button3").expect("button3 should exist");
    assert_eq!(button3.actions.len(), 1);
    assert_eq!(button3.actions[0].action_type, "insertData");
    assert_eq!(button3.actions[0].data_set.as_ref().unwrap(), "model1");
}

// ============================================================
// 二、Scanner actions 集成测试（GraphDB）
// ============================================================

#[test]
fn test_scanner_submit_data_action() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-submit.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Verify action node exists
    let action_node = graph.node_indices.get("action:actions_test/button1/action1");
    assert!(action_node.is_some(), "Action node should exist");

    // Verify submitData creates ActionWrites edge to model1
    let model1_id = "model:model1";
    let writers = graph.find_writers(model1_id);
    let has_action_write = writers.iter().any(|(node, edge)| {
        node.id == "action:actions_test/button1/action1" && matches!(edge.edge_type, EdgeType::ActionWrites)
    });
    assert!(has_action_write, "submitData action should create ActionWrites edge to model1");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_update_data_action() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-update.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let writers = graph.find_writers("model:model2");
    let has_update_action = writers.iter().any(|(node, edge)| {
        node.id == "action:actions_test/button2/action2"
            && matches!(edge.edge_type, EdgeType::ActionWrites)
            && edge.field_path == Some("model2.status".to_string())
    });
    assert!(has_update_action, "updateData action should create ActionWrites edge to model2.status");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_insert_delete_data_actions() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-insert-delete.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // insertData: model1.id
    let writers_model1 = graph.find_writers("model:model1");
    let has_insert = writers_model1.iter().any(|(node, edge)| {
        node.id == "action:actions_test/button3/action3"
            && matches!(edge.edge_type, EdgeType::ActionWrites)
            && edge.field_path == Some("model1.id".to_string())
    });
    assert!(has_insert, "insertData action should create ActionWrites edge to model1.id");

    // deleteData: model2.deletedFlag
    let writers_model2 = graph.find_writers("model:model2");
    let has_delete = writers_model2.iter().any(|(node, edge)| {
        node.id == "action:actions_test/button4/action4"
            && matches!(edge.edge_type, EdgeType::ActionWrites)
            && edge.field_path == Some("model2.deletedFlag".to_string())
    });
    assert!(has_delete, "deleteData action should create ActionWrites edge to model2.deletedFlag");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_submit_data_without_submit_component() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-global-submit.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // button5 submitData without submitComponent: should collect ALL submitField components
    let writers = graph.find_writers("model:model1");
    let has_global_submit = writers.iter().any(|(node, edge)| {
        node.id == "action:actions_test/button5/action5"
            && matches!(edge.edge_type, EdgeType::ActionWrites)
    });
    assert!(has_global_submit, "submitData without submitComponent should collect all submitField components");

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 三、.tbl App 类型解析测试
// ============================================================

#[test]
fn test_scanner_tbl_app_parsing() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-app-tbl.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let model_id = "model:app_table";
    let model_idx = graph.node_indices.get(model_id);
    assert!(model_idx.is_some(), "App table model node should exist");

    // Verify field nodes
    let field_names: Vec<String> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter_map(|e| {
            if matches!(e.weight().edge_type, EdgeType::Contains) {
                graph.graph.node_weight(e.target()).map(|n| n.name.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(field_names.contains(&"订单号".to_string()), "Should have field orderNo");
    assert!(field_names.contains(&"客户名称".to_string()), "Should have field 客户名称");
    assert!(field_names.contains(&"金额".to_string()), "Should have field 金额");
    assert!(field_names.contains(&"创建时间".to_string()), "Should have field 创建时间");

    // Verify model is NOT marked as DataFlow
    if let Some(node) = graph.graph.node_weight(*model_idx.unwrap()) {
        let model_type = node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str());
        assert_eq!(model_type, Some("App"), "App table should have modelType=App");
    }

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 四、.tbl DataFlow 类型解析测试
// ============================================================

#[test]
fn test_scanner_tbl_dataflow_parsing() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-tbl.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let model_id = "model:dataflow_table";
    let model_idx = graph.node_indices.get(model_id);
    assert!(model_idx.is_some(), "DataFlow model node should exist");

    // Verify field nodes exist
    let field_names: Vec<String> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter_map(|e| {
            if matches!(e.weight().edge_type, EdgeType::Contains) {
                graph.graph.node_weight(e.target()).map(|n| n.name.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(field_names.contains(&"工单号".to_string()), "Should have field 工单号");
    assert!(field_names.contains(&"预约单号".to_string()), "Should have field 预约单号");

    // Verify model is marked as DataFlow
    if let Some(node) = graph.graph.node_weight(*model_idx.unwrap()) {
        let model_type = node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str());
        assert_eq!(model_type, Some("DataFlow"), "DataFlow table should have modelType=DataFlow");
    }

    // Verify DataflowInput edges exist
    let outgoing_edges: Vec<_> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::DataflowInput))
        .collect();

    assert!(!outgoing_edges.is_empty(), "DataFlow should have DataflowInput edges");

    let target_names: Vec<String> = outgoing_edges.iter()
        .filter_map(|e| graph.graph.node_weight(e.target()).map(|n| n.name.clone()))
        .collect();

    assert!(target_names.contains(&"fact_serviceappointments".to_string()),
        "Should reference fact_serviceappointments table");
    assert!(target_names.contains(&"customer_info".to_string()),
        "Should reference customer_info table");

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 五、DataFlow 内嵌模型解析测试
// ============================================================

#[test]
fn test_parse_dataflow_source_content() {
    let path = PathBuf::from("tests/fixtures/dataflow_embedded.spg");
    let meta = parse_superpage(&path).expect("Failed to parse dataflow_embedded.spg");

    let source = meta.sources.iter().find(|s| s.id == "model2").expect("model2 source should exist");
    assert_eq!(source.model_type, Some("dataflow".to_string()));
    assert!(source.content.is_some(), "DataFlow source should have content");

    let content = source.content.as_ref().unwrap();
    assert!(content.get("dimensions").is_some(), "content should have dimensions");
    assert!(content.get("dataFlow").is_some(), "content should have dataFlow");
}

#[test]
fn test_scanner_dataflow_embedded_parsing() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-embedded.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Verify embedded DataFlow model node exists
    let model_id = "model:model2";
    let model_idx = graph.node_indices.get(model_id);
    assert!(model_idx.is_some(), "Embedded DataFlow model node should exist");

    // Verify model is marked as DataFlow
    if let Some(node) = graph.graph.node_weight(*model_idx.unwrap()) {
        let model_type = node.meta.as_ref().and_then(|m| m.get("modelType")).and_then(|v| v.as_str());
        assert_eq!(model_type, Some("DataFlow"), "Embedded model should have modelType=DataFlow");
        let embedded = node.meta.as_ref().and_then(|m| m.get("embeddedIn")).and_then(|v| v.as_str());
        assert_eq!(embedded, Some("dataflow_embedded"), "Should record embeddedIn page");
    }

    // Verify field nodes from dimensions
    let field_names: Vec<String> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter_map(|e| {
            if matches!(e.weight().edge_type, EdgeType::Contains) {
                graph.graph.node_weight(e.target()).map(|n| n.name.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(field_names.contains(&"订单号".to_string()), "Should have field orderNo");
    assert!(field_names.contains(&"金额".to_string()), "Should have field processedAmount");

    // Verify DataflowInput edges from moduleTablePath nodes
    let outgoing_edges: Vec<_> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::DataflowInput))
        .collect();

    assert!(!outgoing_edges.is_empty(), "Embedded DataFlow should have DataflowInput edges");

    let target_names: Vec<String> = outgoing_edges.iter()
        .filter_map(|e| graph.graph.node_weight(e.target()).map(|n| n.name.clone()))
        .collect();

    assert!(target_names.contains(&"fact_orders".to_string()),
        "Should reference fact_orders table via moduleTablePath");
    assert!(target_names.contains(&"customer_info".to_string()),
        "Should reference customer_info table via moduleTablePath");

    // Verify component reads from embedded DataFlow model
    let comp_reads: Vec<_> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Incoming)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::Reads))
        .collect();
    
    assert!(!comp_reads.is_empty(), "Component should read from embedded DataFlow model");

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 六、SPG-SPG 关系识别测试
// ============================================================

#[test]
fn test_scanner_embedsuperpage_relation() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-embed.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Verify embedsuperpage creates EmbedsPage edge
    let embed_comp = graph.node_indices.get("comp:page_relations/embed1");
    assert!(embed_comp.is_some(), "embed1 component should exist");

    let outgoing: Vec<_> = graph.graph
        .edges_directed(*embed_comp.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::EmbedsPage))
        .collect();

    assert!(!outgoing.is_empty(), "embedsuperpage should create EmbedsPage edge");

    let target = graph.graph.node_weight(outgoing[0].target());
    assert!(target.is_some(), "Target page should exist");
    assert_eq!(target.unwrap().name, "目标详情", "Should resolve to 目标详情.spg");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_link_opens_page_relation() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-link.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Debug: print action node outgoing edges
    let action_node = graph.node_indices.get("action:page_relations/button1/action1");
    if let Some(&idx) = action_node {
        for e in graph.graph.edges_directed(idx, petgraph::Direction::Outgoing) {
            let target = graph.graph.node_weight(e.target());
            eprintln!("DEBUG edge: {:?} -> {} ({:?})", e.weight().edge_type, target.map(|n| n.id.clone()).unwrap_or_default(), e.weight().field_path);
        }
    }
    assert!(action_node.is_some(), "link action should exist");

    let outgoing: Vec<_> = graph.graph
        .edges_directed(*action_node.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::OpensPage))
        .collect();

    assert!(!outgoing.is_empty(), "link action should create OpensPage edge");

    let target = graph.graph.node_weight(outgoing[0].target());
    assert!(target.is_some(), "Target page should exist");
    assert_eq!(target.unwrap().name, "目标详情", "Should resolve to 目标详情.spg");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_link_passes_param() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-link-param.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Verify link action creates PassesParam edges
    let action_node = graph.node_indices.get("action:page_relations/button1/action1");
    assert!(action_node.is_some(), "link action should exist");

    let param_edges: Vec<_> = graph.graph
        .edges_directed(*action_node.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::PassesParam))
        .collect();

    assert_eq!(param_edges.len(), 2, "Should pass 2 parameters");

    // Verify Reads edges from param expressions (=model1.fieldA)
    let reads_edges: Vec<_> = graph.graph
        .edges_directed(*action_node.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::Reads))
        .collect();

    assert!(!reads_edges.is_empty(), "Should create Reads edges for expression params");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_set_param_value() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-setparam.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Verify setParamValue creates SetsParam edge
    let action_node = graph.node_indices.get("action:page_relations/button2/action2");
    assert!(action_node.is_some(), "setParamValue action should exist");

    let param_edges: Vec<_> = graph.graph
        .edges_directed(*action_node.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::SetsParam))
        .collect();

    assert!(!param_edges.is_empty(), "setParamValue should create SetsParam edge");

    // Verify Reads edges from param expression (=model1.fieldB)
    let reads_edges: Vec<_> = graph.graph
        .edges_directed(*action_node.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::Reads))
        .collect();

    assert!(!reads_edges.is_empty(), "Should create Reads edges for expression param values");

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 七、DataFlow 物理表输出测试
// ============================================================

#[test]
fn test_scanner_dataflow_outputs_to_physical_table() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-output.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let model_id = "model:dataflow_output";
    let model_idx = graph.node_indices.get(model_id);
    assert!(model_idx.is_some(), "DataFlow model node should exist");

    // Verify OutputsTo edge to physical table
    let outgoing: Vec<_> = graph.graph
        .edges_directed(*model_idx.unwrap(), petgraph::Direction::Outgoing)
        .filter(|e| matches!(e.weight().edge_type, EdgeType::OutputsTo))
        .collect();

    assert!(!outgoing.is_empty(), "DataFlow should have OutputsTo edge");

    let target = graph.graph.node_weight(outgoing[0].target());
    assert!(target.is_some(), "Target physical table should exist");
    assert_eq!(target.unwrap().name, "fact_dailyworkorders", "Should output to fact_dailyworkorders");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_scanner_dataflow_internal_deps() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-internal.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    let model_id = "model:dataflow_output";
    let model_idx = graph.node_indices.get(model_id);
    assert!(model_idx.is_some(), "DataFlow model node should exist");

    // Verify internalDeps are stored in model metadata
    let model = graph.graph.node_weight(*model_idx.unwrap());
    assert!(model.is_some(), "Model should exist");

    let internal_deps = model.unwrap().meta
        .as_ref()
        .and_then(|m| m.get("internalDeps"));
    assert!(internal_deps.is_some(), "Should store internalDeps in metadata");

    let _ = std::fs::remove_file(&db_path);
}

// ============================================================
// 八、DataFlow subGraph 展开查询测试
// ============================================================

use metadata_checker::query::query_dataflow;

#[test]
fn test_query_dataflow_human_output() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-query.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Test human-readable output (should not panic)
    let result = query_dataflow(&graph, "model:dataflow_output", true);
    assert!(result.is_ok(), "query_dataflow human mode should succeed");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_query_dataflow_json_output() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-query-json.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Test JSON output (should not panic)
    let result = query_dataflow(&graph, "model:dataflow_output", false);
    assert!(result.is_ok(), "query_dataflow JSON mode should succeed");

    let _ = std::fs::remove_file(&db_path);
}

#[test]
fn test_query_dataflow_not_found() {
    let db_path = std::env::temp_dir().join("metadata-checker-test-dataflow-notfound.db");
    let _ = std::fs::remove_file(&db_path);
    let project_dir = Path::new("tests/fixtures/test_project");

    scan_project(project_dir, &db_path).expect("scan_project failed");

    let graph = GraphDB::open(&db_path).expect("Failed to open graph db");

    // Test querying a non-existent DataFlow
    let result = query_dataflow(&graph, "model:nonexistent", true);
    assert!(result.is_ok(), "query_dataflow for non-existent model should not panic");

    let _ = std::fs::remove_file(&db_path);
}
