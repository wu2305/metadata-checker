//! M58.3 复核返修 P1-c：`severity_for` 显式映射表回归测试。
//!
//! 钉住每个诊断 code 的意图 severity：原构造点的事后补丁覆盖值已收敛进
//! `severity_for`，被默认档静默升档的良性 code（EVIDENCE_SAMPLED /
//! PRIMARY_PATHS_TRUNCATED / NO_WRITE_TARGETS 等）恢复为 Info。

use metadata_checker::diagnostics::{
    CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT, CODE_GRAPH_DB_EDGE_DECODE_FAILED,
    CODE_GRAPH_DB_NODE_DECODE_FAILED, CODE_GRAPH_DB_PARTIAL_HYDRATE,
    CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE, CODE_PAGE_SCOPED_TARGET_FALLBACK,
    CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED, CODE_SCANNER_DUPLICATE_COMPONENT_ID,
    CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY, envelope_diagnostic, severity_for,
};
use metadata_checker::output::{DiagnosticSeverity, Location};

/// Error 档：目标/图库不可用，查询无法完成
#[test]
fn severity_error_for_unavailable_target_or_db() {
    for code in [
        "GRAPH_DB_NOT_FOUND",
        "GRAPH_DB_LOCKED",
        "GRAPH_DB_PERMISSION_DENIED",
        "GRAPH_DB_OPEN_ERROR",
        "TARGET_NOT_FOUND",
    ] {
        assert_eq!(
            severity_for(code),
            DiagnosticSeverity::Error,
            "{code} 应为 Error"
        );
    }
}

/// Info 档：按设计发生或纯提示性信息
#[test]
fn severity_info_for_by_design_or_informational() {
    for code in [
        "GRAPH_DB_READ_ONLY",
        "OK",
        "NO_PRIORITY_RULES",
        "OUTPUT_TRUNCATED",
        // 原构造点未打补丁、被默认 Warning 静默升档的良性 code
        "EVIDENCE_SAMPLED",
        "PRIMARY_PATHS_TRUNCATED",
        "NO_WRITE_TARGETS",
        "NO_ENTRYPOINTS",
        "NO_MATCHES_FOUND",
        "AMBIGUOUS_RESOLUTION",
        "LINEAGE_SOURCE_MISSING",
        "LINEAGE_EXPR_UNPARSED",
        "EVIDENCE_LOCATION_MISSING",
        "DATAFLOW_INPUT_PATH_UNRESOLVED",
        "EXPR_UNPARSED",
    ] {
        assert_eq!(
            severity_for(code),
            DiagnosticSeverity::Info,
            "{code} 应为 Info"
        );
    }
}

/// Warning 档：数据/配置疑似有问题；未登记 code 也落默认档
#[test]
fn severity_warning_for_suspect_data_and_default() {
    for code in [
        CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY,
        CODE_SCANNER_DUPLICATE_COMPONENT_ID,
        CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED,
        CODE_GRAPH_DB_NODE_DECODE_FAILED,
        CODE_GRAPH_DB_EDGE_DECODE_FAILED,
        CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT,
        CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE,
        CODE_GRAPH_DB_PARTIAL_HYDRATE,
        CODE_PAGE_SCOPED_TARGET_FALLBACK,
        "CYCLE_DEPENDENCY",
        "DATAFLOW_NO_OUTPUT",
        "DATAFLOW_NO_INPUTS",
        "MODEL_UNRESOLVED",
        "PAGE_INPUTS_DEFERRED",
        "UNRESOLVED_PAGE_NAVIGATION",
        "UNRESOLVED_MODEL_WRITE",
        "VISIBILITY_RULE_UNRESOLVED",
        "ACTION_FLOW_INCOMPLETE",
        "UNKNOWN_ACTION_TYPE",
        "UNKNOWN_QUESTION_KIND",
        "EDGE_EVIDENCE_UNAVAILABLE",
        "EVIDENCE_INCOMPLETE",
        "SOME_FUTURE_UNREGISTERED_CODE",
    ] {
        assert_eq!(
            severity_for(code),
            DiagnosticSeverity::Warning,
            "{code} 应为 Warning"
        );
    }
}

/// envelope_diagnostic 的 severity 必须直接来自 severity_for，
/// 构造点无需（也不得）事后补丁式覆盖
#[test]
fn envelope_diagnostic_severity_comes_from_table() {
    let diag = envelope_diagnostic(
        "EVIDENCE_SAMPLED",
        1,
        Location::default(),
        "sampled evidence",
    );
    assert_eq!(diag.severity, DiagnosticSeverity::Info);

    let diag = envelope_diagnostic("TARGET_NOT_FOUND", 1, Location::default(), "missing");
    assert_eq!(diag.severity, DiagnosticSeverity::Error);

    let diag = envelope_diagnostic(
        CODE_GRAPH_DB_PARTIAL_HYDRATE,
        3,
        Location::default(),
        "partial",
    );
    assert_eq!(diag.severity, DiagnosticSeverity::Warning);
}
