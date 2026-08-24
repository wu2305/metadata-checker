//! 统一诊断信封（M58.3 Phase 1）
//!
//! 所有新增与既有诊断共用一个信封，字段写死：
//! `{ code, severity, count, sample_location, answer_impact, first_seen_phase }`
//! 本模块定义 code 常量、answer_impact 映射与 hydrate 统计转 Diagnostic 的 helper。

use crate::output::schema::{Diagnostic, DiagnosticSeverity, Location};
use serde::{Deserialize, Serialize};

/// 诊断首次出现的阶段
pub const PHASE_PR1: &str = "PR1";

/// answer_impact 取值
pub const IMPACT_NONE: &str = "none";
pub const IMPACT_PARTIAL: &str = "partial";
pub const IMPACT_BLOCKING: &str = "blocking";

/// 初始诊断 code
pub const CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY: &str = "SCANNER_UNRECOGNIZED_CONTAINER_KEY";
pub const CODE_SCANNER_DUPLICATE_COMPONENT_ID: &str = "SCANNER_DUPLICATE_COMPONENT_ID";
pub const CODE_GRAPH_DB_NODE_DECODE_FAILED: &str = "GRAPH_DB_NODE_DECODE_FAILED";
pub const CODE_GRAPH_DB_EDGE_DECODE_FAILED: &str = "GRAPH_DB_EDGE_DECODE_FAILED";
pub const CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT: &str = "GRAPH_DB_EDGE_DANGLING_ENDPOINT";
pub const CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE: &str = "GRAPH_DB_V2_LAYOUT_UNREADABLE";
pub const CODE_GRAPH_DB_PARTIAL_HYDRATE: &str = "GRAPH_DB_PARTIAL_HYDRATE";
pub const CODE_PAGE_SCOPED_TARGET_FALLBACK: &str = "PAGE_SCOPED_TARGET_FALLBACK";
/// 仅保留 PR1 阶段的 code；PR4a 的 GRAPH_SCHEMA_STALE 待该 PR 再引入。

/// answer_impact 映射
pub fn answer_impact_for(code: &str) -> &'static str {
    match code {
        CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY => IMPACT_PARTIAL,
        CODE_SCANNER_DUPLICATE_COMPONENT_ID => IMPACT_PARTIAL,
        CODE_GRAPH_DB_NODE_DECODE_FAILED => IMPACT_PARTIAL,
        CODE_GRAPH_DB_EDGE_DECODE_FAILED => IMPACT_PARTIAL,
        CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT => IMPACT_PARTIAL,
        CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE => IMPACT_NONE,
        CODE_GRAPH_DB_PARTIAL_HYDRATE => IMPACT_PARTIAL,
        CODE_PAGE_SCOPED_TARGET_FALLBACK => IMPACT_PARTIAL,
        _ => IMPACT_NONE,
    }
}

/// severity 映射
pub fn severity_for(_code: &str) -> DiagnosticSeverity {
    DiagnosticSeverity::Warning
}

/// hydrate 阶段统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HydrateDiagnostics {
    pub node_decode_failed: usize,
    pub edge_decode_failed: usize,
    pub dangling_edge: usize,
    pub v2_layout_unreadable: usize,
    pub sample_node_location: Option<Location>,
    pub sample_edge_location: Option<Location>,
    pub sample_dangling_location: Option<Location>,
    pub v2_hydrate_warning: Option<String>,
}

impl HydrateDiagnostics {
    pub fn has_partial_loss(&self) -> bool {
        self.node_decode_failed > 0 || self.edge_decode_failed > 0 || self.dangling_edge > 0
    }

    pub fn is_empty(&self) -> bool {
        self.node_decode_failed == 0
            && self.edge_decode_failed == 0
            && self.dangling_edge == 0
            && self.v2_layout_unreadable == 0
            && self.v2_hydrate_warning.is_none()
    }

    /// 转为结构化 Diagnostic 列表（按 code 去重，每类一条，带 count 与 sample_location）
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if self.node_decode_failed > 0 {
            out.push(
                Diagnostic {
                    severity: severity_for(CODE_GRAPH_DB_NODE_DECODE_FAILED),
                    code: CODE_GRAPH_DB_NODE_DECODE_FAILED.to_string(),
                    message: format!(
                        "GraphDB hydrate: {} node rows failed to decode",
                        self.node_decode_failed
                    ),
                    location: self.sample_node_location.clone().unwrap_or_default(),
                    suggestion: Some("Rebuild graphdb with --build-graph".to_string()),
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
                }
                .with_count(self.node_decode_failed)
                .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_NODE_DECODE_FAILED))
                .with_first_seen_phase(PHASE_PR1),
            );
        }
        if self.edge_decode_failed > 0 {
            out.push(
                Diagnostic {
                    severity: severity_for(CODE_GRAPH_DB_EDGE_DECODE_FAILED),
                    code: CODE_GRAPH_DB_EDGE_DECODE_FAILED.to_string(),
                    message: format!(
                        "GraphDB hydrate: {} edge rows failed to decode",
                        self.edge_decode_failed
                    ),
                    location: self.sample_edge_location.clone().unwrap_or_default(),
                    suggestion: Some("Rebuild graphdb with --build-graph".to_string()),
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
                }
                .with_count(self.edge_decode_failed)
                .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_EDGE_DECODE_FAILED))
                .with_first_seen_phase(PHASE_PR1),
            );
        }
        if self.dangling_edge > 0 {
            out.push(
                Diagnostic {
                    severity: severity_for(CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT),
                    code: CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT.to_string(),
                    message: format!(
                        "GraphDB hydrate: {} edges have dangling endpoints and were dropped",
                        self.dangling_edge
                    ),
                    location: self.sample_dangling_location.clone().unwrap_or_default(),
                    suggestion: Some("Rebuild graphdb with --build-graph".to_string()),
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
                }
                .with_count(self.dangling_edge)
                .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT))
                .with_first_seen_phase(PHASE_PR1),
            );
        }
        if self.v2_layout_unreadable > 0 {
            out.push(
                Diagnostic {
                    severity: severity_for(CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE),
                    code: CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE.to_string(),
                    message: "GraphDB v2 layout unreadable, fell back to v1".to_string(),
                    location: Location::default(),
                    suggestion: Some("Rebuild graphdb to refresh v2 shadow".to_string()),
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
                }
                .with_count(self.v2_layout_unreadable)
                .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE))
                .with_first_seen_phase(PHASE_PR1),
            );
        }
        if let Some(warning) = &self.v2_hydrate_warning {
            if self.v2_layout_unreadable == 0 {
                out.push(
                    Diagnostic {
                        severity: severity_for(CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE),
                        code: CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE.to_string(),
                        message: warning.clone(),
                        location: Location::default(),
                        suggestion: Some("Rebuild graphdb to refresh v2 shadow".to_string()),
                        count: None,
                        answer_impact: None,
                        first_seen_phase: None,
                    }
                    .with_count(1)
                    .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE))
                    .with_first_seen_phase(PHASE_PR1),
                );
            }
        }
        if self.has_partial_loss() {
            let total = self.node_decode_failed + self.edge_decode_failed + self.dangling_edge;
            out.push(
                Diagnostic {
                    severity: severity_for(CODE_GRAPH_DB_PARTIAL_HYDRATE),
                    code: CODE_GRAPH_DB_PARTIAL_HYDRATE.to_string(),
                    message: format!(
                        "GraphDB partial hydrate: {} rows lost (node {} / edge {} / dangling {})",
                        total, self.node_decode_failed, self.edge_decode_failed, self.dangling_edge
                    ),
                    location: self
                        .sample_node_location
                        .clone()
                        .or_else(|| self.sample_edge_location.clone())
                        .or_else(|| self.sample_dangling_location.clone())
                        .unwrap_or_default(),
                    suggestion: Some("Answers may be incomplete; rebuild graphdb".to_string()),
                    count: None,
                    answer_impact: None,
                    first_seen_phase: None,
                }
                .with_count(total)
                .with_answer_impact(answer_impact_for(CODE_GRAPH_DB_PARTIAL_HYDRATE))
                .with_first_seen_phase(PHASE_PR1),
            );
        }
        out
    }
}

/// 供 scanner / query 侧构造单条信封诊断
pub fn envelope_diagnostic(
    code: &str,
    count: usize,
    sample_location: Location,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        severity: severity_for(code),
        code: code.to_string(),
        message: message.into(),
        location: sample_location,
        suggestion: None,
        count: None,
        answer_impact: None,
        first_seen_phase: None,
    }
    .with_count(count)
    .with_answer_impact(answer_impact_for(code))
    .with_first_seen_phase(PHASE_PR1)
}
