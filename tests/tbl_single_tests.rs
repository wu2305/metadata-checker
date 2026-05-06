//! M10 单文件 .tbl 解析测试

use metadata_checker::output::schema::{AiOutput, OutputKind};
use metadata_checker::parser;
use metadata_checker::tbl_single;
use std::path::Path;

fn parse_tbl_fixture(name: &str) -> AiOutput {
    let path = Path::new("tests/fixtures/test_project/app").join(name);
    let meta = parser::parse_file(&path).expect("parse should succeed");
    let tbl = meta.tbl.expect("should have tbl metadata");
    tbl_single::build_tbl_output(&tbl)
}

#[test]
fn test_app_table_kind_is_table() {
    let out = parse_tbl_fixture("app_table.tbl");
    assert_eq!(out.kind, OutputKind::Table);
}

#[test]
fn test_dataflow_table_kind_is_dataflow() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    assert_eq!(out.kind, OutputKind::DataFlow);
}

#[test]
fn test_app_table_summary_fields() {
    let out = parse_tbl_fixture("app_table.tbl");
    let summary = out.summary.as_object().expect("summary should be object");
    assert_eq!(
        summary.get("table_type").and_then(|v| v.as_str()),
        Some("AppTable")
    );
    assert_eq!(summary.get("field_count").and_then(|v| v.as_i64()), Some(4));
    assert_eq!(summary.get("input_count").and_then(|v| v.as_i64()), Some(0));
    assert_eq!(
        summary.get("output_count").and_then(|v| v.as_i64()),
        Some(0)
    );
    assert!(
        summary
            .get("what_is_it")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("AppTable")
    );
}

#[test]
fn test_dataflow_summary_fields() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    let summary = out.summary.as_object().expect("summary should be object");
    assert_eq!(
        summary.get("table_type").and_then(|v| v.as_str()),
        Some("DataFlow")
    );
    assert_eq!(summary.get("field_count").and_then(|v| v.as_i64()), Some(3));
    assert!(
        summary
            .get("input_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0
    );
    assert_eq!(
        summary.get("output_count").and_then(|v| v.as_i64()),
        Some(1)
    );
    let what = summary
        .get("what_is_it")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        what.contains("DataFlow"),
        "what_is_it should mention DataFlow: {}",
        what
    );
}

#[test]
fn test_dataflow_details_structure() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    let details = out.details.expect("details should exist");
    let obj = details.as_object().expect("details should be object");
    assert!(obj.contains_key("fields"));
    assert!(obj.contains_key("dataflow_inputs"));
    assert!(obj.contains_key("dataflow_outputs"));
    assert!(obj.contains_key("internal_nodes"));
    assert!(obj.contains_key("field_lineage"));

    let fields = obj.get("fields").unwrap().as_array().unwrap();
    assert_eq!(fields.len(), 3);

    let inputs = obj.get("dataflow_inputs").unwrap().as_array().unwrap();
    assert!(!inputs.is_empty(), "should have dataflow inputs");
}

#[test]
fn test_app_table_details_fields() {
    let out = parse_tbl_fixture("app_table.tbl");
    let details = out.details.expect("details should exist");
    let obj = details.as_object().expect("details should be object");
    let fields = obj.get("fields").unwrap().as_array().unwrap();
    assert_eq!(fields.len(), 4);

    let lineage = obj.get("field_lineage").unwrap().as_array().unwrap();
    assert_eq!(lineage.len(), 0, "app table should have no lineage");
}

#[test]
fn test_evidence_non_empty() {
    let out = parse_tbl_fixture("app_table.tbl");
    assert!(!out.evidence.is_empty(), "evidence should not be empty");
    let first = &out.evidence[0];
    assert!(first.source_file.is_some());
    assert!(first.json_path.is_some());
}

#[test]
fn test_dataflow_evidence_has_nodes() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    let has_node_evidence = out.evidence.iter().any(|e| e.node_id.is_some());
    assert!(
        has_node_evidence,
        "should have evidence with node_id for dataflow nodes"
    );
}

#[test]
fn test_not_superpage() {
    let out_app = parse_tbl_fixture("app_table.tbl");
    assert_ne!(out_app.kind, OutputKind::SuperPage);
    let out_df = parse_tbl_fixture("dataflow_output.tbl");
    assert_ne!(out_df.kind, OutputKind::SuperPage);
}

#[test]
fn test_query_target_present() {
    let out = parse_tbl_fixture("app_table.tbl");
    assert!(out.query_target.is_some());
    let path = out.query_target.unwrap();
    assert!(path.contains("app_table.tbl"));
}

#[test]
fn test_next_queries_present() {
    let out = parse_tbl_fixture("app_table.tbl");
    assert!(!out.next_queries.is_empty());
    let out_df = parse_tbl_fixture("dataflow_output.tbl");
    assert!(!out_df.next_queries.is_empty());
    assert!(
        out_df
            .next_queries
            .iter()
            .any(|q| q.contains("query-dataflow")),
        "dataflow should suggest query-dataflow"
    );
}

#[test]
fn test_dataflow_diagnostics_no_output_warning() {
    // dataflow_output.tbl has dbTableName, so no warning
    let out = parse_tbl_fixture("dataflow_output.tbl");
    let has_no_output = out
        .diagnostics
        .iter()
        .any(|d| d.code == "DATAFLOW_NO_OUTPUT");
    assert!(
        !has_no_output,
        "dataflow_output has dbTableName, should not warn"
    );
}

#[test]
fn test_dataflow_no_output_diagnostic() {
    let out = parse_tbl_fixture("dataflow_table.tbl");
    assert_eq!(out.kind, OutputKind::DataFlow);
    let has_no_output = out
        .diagnostics
        .iter()
        .any(|d| d.code == "DATAFLOW_NO_OUTPUT");
    assert!(
        has_no_output,
        "dataflow_table has no dbTableName, should emit DATAFLOW_NO_OUTPUT"
    );
}

#[test]
fn test_field_lineage_structure() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    let details = out.details.expect("details should exist");
    let obj = details.as_object().expect("details should be object");
    let lineage = obj.get("field_lineage").unwrap().as_array().unwrap();
    assert!(!lineage.is_empty(), "field_lineage should not be empty");

    for item in lineage {
        let item = item.as_object().unwrap();
        assert!(
            !item
                .get("target_field")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty(),
            "target_field should not be empty"
        );
        assert!(item.contains_key("source_fields"));
        assert!(item.contains_key("source_expr"));
        assert!(
            !item
                .get("transform")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .is_empty(),
            "transform should not be empty when lineage exists"
        );
        let confidence = item
            .get("confidence")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            confidence == "high" || confidence == "medium" || confidence == "low",
            "confidence should be a valid enum string"
        );
    }
}

#[test]
fn test_evidence_json_path_non_empty() {
    let out = parse_tbl_fixture("dataflow_output.tbl");
    assert!(!out.evidence.is_empty());
    for ev in &out.evidence {
        if let Some(ref jp) = ev.json_path {
            assert!(!jp.is_empty(), "json_path should not be empty: {:?}", ev);
        }
        if let Some(ref sf) = ev.source_file {
            assert!(!sf.is_empty(), "source_file should not be empty");
        }
    }
}

#[test]
#[ignore = "requires real project path at /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi"]
fn test_real_project_tbl_not_superpage() {
    let path = Path::new(
        "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi/data/tables/销售/fact_saleContract.tbl",
    );
    let meta = parser::parse_file(path).expect("parse should succeed");
    let tbl = meta.tbl.expect("should have tbl metadata");
    let out = tbl_single::build_tbl_output(&tbl);
    assert_ne!(out.kind, OutputKind::SuperPage);
    assert_eq!(out.kind, OutputKind::Table);
    let summary = out.summary.as_object().expect("summary should be object");
    assert_eq!(
        summary.get("table_type").and_then(|v| v.as_str()),
        Some("PhysicalTable")
    );
    assert!(
        summary
            .get("field_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0,
        "real table should have fields"
    );
}

#[test]
fn test_real_physical_table_fixture() {
    let out = parse_tbl_fixture("real_physical_table.tbl");
    assert_eq!(out.kind, OutputKind::Table);
    let summary = out.summary.as_object().expect("summary should be object");
    assert_eq!(
        summary.get("table_type").and_then(|v| v.as_str()),
        Some("PhysicalTable")
    );
    assert!(
        summary
            .get("field_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0,
        "real physical table should have fields"
    );
    assert_eq!(
        summary.get("input_count").and_then(|v| v.as_i64()),
        Some(0),
        "physical table has no inputs"
    );
    assert_eq!(
        summary.get("output_count").and_then(|v| v.as_i64()),
        Some(0),
        "physical table has no outputs"
    );
    let details = out.details.as_ref().unwrap().as_object().unwrap();
    let fields = details.get("fields").unwrap().as_array().unwrap();
    assert!(!fields.is_empty(), "should have fields");
    assert!(
        out.evidence
            .iter()
            .any(|e| e.json_path.as_deref() == Some("dimensions[]")),
        "should have dimensions[] evidence"
    );
}

#[test]
fn test_real_dataflow_fixture() {
    let out = parse_tbl_fixture("real_dataflow.tbl");
    assert_eq!(out.kind, OutputKind::DataFlow);
    let summary = out.summary.as_object().expect("summary should be object");
    assert_eq!(
        summary.get("table_type").and_then(|v| v.as_str()),
        Some("DataFlow")
    );
    assert!(
        summary
            .get("field_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0,
        "real dataflow should have fields"
    );
    assert!(
        summary
            .get("input_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            > 0,
        "real dataflow should have inputs"
    );
    let details = out.details.as_ref().unwrap().as_object().unwrap();
    let inputs = details.get("dataflow_inputs").unwrap().as_array().unwrap();
    assert!(!inputs.is_empty(), "should have dataflow inputs");
    let lineage = details.get("field_lineage").unwrap().as_array().unwrap();
    assert!(!lineage.is_empty(), "should have field lineage");
}

#[test]
fn test_dataflow_input_path_unresolved_diagnostic() {
    // real_dataflow.tbl has moduleTablePath like $DATA:/售后/fact_serviceappointments.tbl
    // which ends with .tbl so should NOT trigger the diagnostic
    let out = parse_tbl_fixture("real_dataflow.tbl");
    let has_unresolved = out
        .diagnostics
        .iter()
        .any(|d| d.code == "DATAFLOW_INPUT_PATH_UNRESOLVED");
    assert!(
        !has_unresolved,
        "real_dataflow paths end with .tbl, should not trigger unresolved"
    );
}
