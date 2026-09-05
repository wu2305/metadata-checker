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
/// M58.3 复核返修：diff-refresh 刷新 live scanner 诊断缓存失败，
/// 已透出的 SCANNER_* 诊断可能陈旧
pub const CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED: &str = "SCANNER_DIAGNOSTICS_REFRESH_FAILED";
/// M58.3 复核返修（P1-1）：加载期 scanner 诊断缓存读取/合并失败，
/// SCANNER_* 诊断整体缺失（区别于 REFRESH_FAILED 的「可能陈旧」）
pub const CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED: &str = "SCANNER_DIAGNOSTICS_LOAD_FAILED";
/// M58.3 复核返修（P1-1）：read model（性能层派生索引）构建失败或整体跳过，
/// 只影响延迟不影响答案正确性
pub const CODE_RUNTIME_READ_MODEL_DEGRADED: &str = "RUNTIME_READ_MODEL_DEGRADED";
pub const CODE_GRAPH_DB_NODE_DECODE_FAILED: &str = "GRAPH_DB_NODE_DECODE_FAILED";
pub const CODE_GRAPH_DB_EDGE_DECODE_FAILED: &str = "GRAPH_DB_EDGE_DECODE_FAILED";
pub const CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT: &str = "GRAPH_DB_EDGE_DANGLING_ENDPOINT";
pub const CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE: &str = "GRAPH_DB_V2_LAYOUT_UNREADABLE";
pub const CODE_GRAPH_DB_PARTIAL_HYDRATE: &str = "GRAPH_DB_PARTIAL_HYDRATE";
pub const CODE_PAGE_SCOPED_TARGET_FALLBACK: &str = "PAGE_SCOPED_TARGET_FALLBACK";
/// M58.3 复核返修：load_diagnostics 合并进查询响应时序列化失败的 fail-visible
/// 兜底 code（answer_impact 经 `answer_effect` 派生为 partial）
pub const CODE_DIAGNOSTIC_SERIALIZE_FAILED: &str = "DIAGNOSTIC_SERIALIZE_FAILED";
/// 仅保留 PR1 阶段的 code；PR4a 的 GRAPH_SCHEMA_STALE 待该 PR 再引入。

/// answer_impact 映射
///
/// 显式列出的 code 以本表为准（权威）。未显式列出的 code 从
/// [`crate::output::answer_effect::answer_effect`] 派生，保证序列化 JSON 中
/// `answer_impact` 与 `answer_effect` 的置信度语义一致：
/// - `Uncertain` / `Partial` → [`IMPACT_PARTIAL`]（语义不确定或只覆盖部分数据）
/// - `Determinate` / `Addressing` / `Routing` → [`IMPACT_NONE`]（确定判定、寻址与路由说明不降低答案置信度）
/// - 未登记的 code（`answer_effect` 返回 `None`）→ [`IMPACT_NONE`]
pub fn answer_impact_for(code: &str) -> &'static str {
    match code {
        CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY => IMPACT_PARTIAL,
        CODE_SCANNER_DUPLICATE_COMPONENT_ID => IMPACT_PARTIAL,
        // 诊断缓存可能陈旧：涉及 SCANNER_* 诊断的结论只能视为部分可靠
        CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED => IMPACT_PARTIAL,
        // 诊断整体缺失（加载失败）：同上，涉及 SCANNER_* 覆盖面的结论只能视为部分可靠
        CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED => IMPACT_PARTIAL,
        // read model 是性能层派生索引，降级不影响答案正确性
        CODE_RUNTIME_READ_MODEL_DEGRADED => IMPACT_NONE,
        CODE_GRAPH_DB_NODE_DECODE_FAILED => IMPACT_PARTIAL,
        CODE_GRAPH_DB_EDGE_DECODE_FAILED => IMPACT_PARTIAL,
        CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT => IMPACT_PARTIAL,
        CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE => IMPACT_NONE,
        CODE_GRAPH_DB_PARTIAL_HYDRATE => IMPACT_PARTIAL,
        CODE_PAGE_SCOPED_TARGET_FALLBACK => IMPACT_PARTIAL,
        _ => match crate::output::answer_effect::answer_effect(code) {
            Some((
                crate::output::answer_effect::AnswerImpact::Uncertain
                | crate::output::answer_effect::AnswerImpact::Partial,
                _,
            )) => IMPACT_PARTIAL,
            // Determinate / Addressing / Routing 与未登记 code 一样，不降低答案置信度
            _ => IMPACT_NONE,
        },
    }
}

/// severity 映射（权威来源）
///
/// M58.3 复核返修 P1-c：severity 只从本表派生，构造点不再事后补丁式覆盖
/// （覆盖值已收敛进本表）。分档口径：
/// - `Error`：目标/资源不可用，查询无法完成（库缺失、锁占用、权限、打开失败、目标不存在）
/// - `Info`：按设计发生或纯提示性信息（采样、截断、空结果、只读回退、lineage 缺失等）
/// - `Warning`：数据或配置疑似有问题、结论可能不可靠（默认档，未登记 code 也落这里）
pub fn severity_for(code: &str) -> DiagnosticSeverity {
    match code {
        // ---- Error：查询目标/图库不可用 ----
        // 原 graph_redb.rs / output/schema.rs / query.rs 构造点事后覆盖为 Error
        "GRAPH_DB_NOT_FOUND"
        | "GRAPH_DB_LOCKED"
        | "GRAPH_DB_PERMISSION_DENIED"
        | "GRAPH_DB_OPEN_ERROR"
        | "TARGET_NOT_FOUND" => DiagnosticSeverity::Error,

        // ---- Info：按设计发生或纯提示 ----
        // redb 只读回退是可操作状态，不阻断查询
        "GRAPH_DB_READ_ONLY" => DiagnosticSeverity::Info,
        // 健康检查通过 / 无优先级冲突的肯定性结论
        "OK" | "NO_PRIORITY_RULES" => DiagnosticSeverity::Info,
        // 预算截断是按设计行为（page_logic.rs 构造点原无覆盖，随同 code 统一为 Info）
        "OUTPUT_TRUNCATED" => DiagnosticSeverity::Info,
        // 页面逻辑：按设计采样 / 主路径按设计截断 / 只读页面无写目标，均为低噪音提示
        // （原构造点未打补丁被默认档静默升档，本表恢复意图 severity）
        "EVIDENCE_SAMPLED" | "PRIMARY_PATHS_TRUNCATED" | "NO_WRITE_TARGETS" => {
            DiagnosticSeverity::Info
        }
        // 无用户入口在只读页面属常态（page_logic 构造点原无覆盖，随 page_dataflow 统一 Info）
        "NO_ENTRYPOINTS" => DiagnosticSeverity::Info,
        // 检索/解析的良性空结果与多候选
        "NO_MATCHES_FOUND" | "AMBIGUOUS_RESOLUTION" => DiagnosticSeverity::Info,
        // M58.3 复核返修（D1）：路由层表面诊断统一信封后，severity 由本表权威派生，
        // 保持原 surface 层的 info 意图——目标补全留痕 / 多候选逐个作答 /
        // 补充块取不到但主结果完整，均为按设计发生或纯提示
        "RESOLVED_TARGET" | "AMBIGUOUS_TARGET_ANSWERED" | "SUPPLEMENT_UNAVAILABLE" => {
            DiagnosticSeverity::Info
        }
        // lineage 推断缺失 / 表达式超出解析能力：降低置信度提示，非数据错误
        "LINEAGE_SOURCE_MISSING" | "LINEAGE_EXPR_UNPARSED" | "EVIDENCE_LOCATION_MISSING" => {
            DiagnosticSeverity::Info
        }
        // DataFlow 输入路径未解析、表达式过复杂未完整解析：启发式提示
        "DATAFLOW_INPUT_PATH_UNRESOLVED" | "EXPR_UNPARSED" => DiagnosticSeverity::Info,

        // ---- Warning：数据/配置疑似有问题（默认档，显式列出以固定意图） ----
        CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY
        | CODE_SCANNER_DUPLICATE_COMPONENT_ID
        | CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED
        | CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED
        | CODE_GRAPH_DB_NODE_DECODE_FAILED
        | CODE_GRAPH_DB_EDGE_DECODE_FAILED
        | CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT
        | CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE
        | CODE_GRAPH_DB_PARTIAL_HYDRATE
        | CODE_PAGE_SCOPED_TARGET_FALLBACK => DiagnosticSeverity::Warning,
        // read model 构建失败属异常信号（非按设计），但只是性能层降级，
        // 答案正确性不受影响由 answer_impact=none 表达
        CODE_RUNTIME_READ_MODEL_DEGRADED => DiagnosticSeverity::Warning,
        // 循环依赖、DataFlow 结构残缺、模型/导航/可见性规则解析失败等
        "CYCLE_DEPENDENCY"
        | "DATAFLOW_NO_OUTPUT"
        | "DATAFLOW_NO_INPUTS"
        | "MODEL_UNRESOLVED"
        | "PAGE_INPUTS_DEFERRED"
        | "UNRESOLVED_PAGE_NAVIGATION"
        | "UNRESOLVED_MODEL_WRITE"
        | "VISIBILITY_RULE_UNRESOLVED"
        | "ACTION_FLOW_INCOMPLETE"
        | "UNKNOWN_ACTION_TYPE"
        | "UNKNOWN_QUESTION_KIND"
        | "EDGE_EVIDENCE_UNAVAILABLE"
        | "EVIDENCE_INCOMPLETE" => DiagnosticSeverity::Warning,

        // 未登记 code 维持 Warning 默认档
        _ => DiagnosticSeverity::Warning,
    }
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
        if self.has_partial_loss() {
            let total = self.node_decode_failed + self.edge_decode_failed + self.dangling_edge;
            out.push(envelope_diagnostic(
                CODE_GRAPH_DB_PARTIAL_HYDRATE,
                total,
                self.sample_node_location
                    .clone()
                    .or_else(|| self.sample_edge_location.clone())
                    .or_else(|| self.sample_dangling_location.clone())
                    .unwrap_or_default(),
                format!(
                    "GraphDB partial hydrate: {} rows lost (node {} / edge {} / dangling {})",
                    total, self.node_decode_failed, self.edge_decode_failed, self.dangling_edge
                ),
            ));
        }
        if self.node_decode_failed > 0 {
            out.push(envelope_diagnostic(
                CODE_GRAPH_DB_NODE_DECODE_FAILED,
                self.node_decode_failed,
                self.sample_node_location.clone().unwrap_or_default(),
                format!(
                    "GraphDB hydrate: {} node rows failed to decode",
                    self.node_decode_failed
                ),
            ));
        }
        if self.edge_decode_failed > 0 {
            out.push(envelope_diagnostic(
                CODE_GRAPH_DB_EDGE_DECODE_FAILED,
                self.edge_decode_failed,
                self.sample_edge_location.clone().unwrap_or_default(),
                format!(
                    "GraphDB hydrate: {} edge rows failed to decode",
                    self.edge_decode_failed
                ),
            ));
        }
        if self.dangling_edge > 0 {
            out.push(envelope_diagnostic(
                CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT,
                self.dangling_edge,
                self.sample_dangling_location.clone().unwrap_or_default(),
                format!(
                    "GraphDB hydrate: {} edges have dangling endpoints and were dropped",
                    self.dangling_edge
                ),
            ));
        }
        if self.v2_layout_unreadable > 0 {
            out.push(envelope_diagnostic(
                CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE,
                self.v2_layout_unreadable,
                Location::default(),
                "GraphDB v2 layout unreadable, fell back to v1",
            ));
        }
        if let Some(warning) = &self.v2_hydrate_warning {
            if self.v2_layout_unreadable == 0 {
                out.push(envelope_diagnostic(
                    CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE,
                    1,
                    Location::default(),
                    warning.clone(),
                ));
            }
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
        count: Some(count),
        answer_impact: Some(answer_impact_for(code).to_string()),
        first_seen_phase: Some(PHASE_PR1.to_string()),
    }
}
