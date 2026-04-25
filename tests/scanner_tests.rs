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
    assert_eq!(button2.actions[0].field_values[0], ("status".to_string(), "input1.value".to_string()));
    assert_eq!(button2.actions[0].field_values[1], ("updatedAt".to_string(), "TODAY()".to_string()));

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

    assert!(field_names.contains(&"订单号".to_string()), "Should have field 订单号");
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
