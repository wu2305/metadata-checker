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
/// 页面里的跨页引用（embedsuperpage / link 动作）解析不到一个 `.spg` 页面：前缀不认识、
/// 绝对路径、越出项目根、目标不是 `.spg`，或 `referenceResources` 下标越界。
/// 该引用对应的边**没有建**——图里缺这条边不等于页面没有这个引用。
pub const CODE_SCANNER_UNRESOLVED_REFERENCE: &str = "SCANNER_UNRESOLVED_REFERENCE";

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
/// M59-B3：源文件解析失败（内容非 UTF-8 或非法 JSON）。该文件本轮**不进候选图**，
/// 上一份有效图原样保留，因此图里这部分是**陈旧**而非缺失——两者对答案的影响
/// 完全不同，必须能区分。旧行为是把解析失败静默转成「空结果」提交，
/// 等于把「读不出来」说成「模型不存在」。
pub const CODE_SCANNER_FILE_PARSE_FAILED: &str = "SCANNER_FILE_PARSE_FAILED";
/// M58.3 复核返修：load_diagnostics 合并进查询响应时序列化失败的 fail-visible
/// 兜底 code（answer_impact 经 `answer_effect` 派生为 partial）
pub const CODE_DIAGNOSTIC_SERIALIZE_FAILED: &str = "DIAGNOSTIC_SERIALIZE_FAILED";
/// 事实表示版本不兼容，拒绝把旧图当成完整证据加载。
pub const CODE_GRAPH_SCHEMA_STALE: &str = "GRAPH_SCHEMA_STALE";
/// M59-2 C：同一实体 id 存在多个不兼容 Definition（来源冲突）。
///
/// 这是**持久状态**而非一次性扫描告警：它随账本落库，重启后仍在，修复后消失。
/// 冲突时重建按确定性规则择一，答案覆盖的是其中一个来源——因此 answer_impact
/// 为 partial（不是 none）。
pub const CODE_GRAPH_OWNERSHIP_CONFLICT: &str = "GRAPH_OWNERSHIP_CONFLICT";

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
        CODE_GRAPH_SCHEMA_STALE
        | "GRAPH_OWNERSHIP_SCHEMA_STALE"
        | "GRAPH_PROJECT_BINDING_REQUIRED"
        | "GRAPH_PROJECT_BINDING_MISMATCH" => IMPACT_BLOCKING,
        CODE_SCANNER_UNRECOGNIZED_CONTAINER_KEY => IMPACT_PARTIAL,
        CODE_SCANNER_DUPLICATE_COMPONENT_ID => IMPACT_PARTIAL,
        CODE_SCANNER_UNRESOLVED_REFERENCE => IMPACT_PARTIAL,
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
        // M59-B3：源文件解析失败，图里这部分是上一次成功解析的旧事实。
        // 查询照样能作答，但答的可能是过期内容——正是 partial 的定义。
        CODE_SCANNER_FILE_PARSE_FAILED => IMPACT_PARTIAL,
        // 来源冲突：择一后只覆盖其中一个来源的事实，答案不完整
        CODE_GRAPH_OWNERSHIP_CONFLICT => IMPACT_PARTIAL,
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
        | "GRAPH_OWNERSHIP_SCHEMA_STALE"
        | "GRAPH_PROJECT_BINDING_REQUIRED"
        | "GRAPH_PROJECT_BINDING_MISMATCH"
        | "TARGET_NOT_FOUND"
        // M59-2 A1：裸旧 target 匹配到多个节点。与 TARGET_NOT_FOUND 同属
        // 「寻址失败、查询无法完成」一类：两者都让调用方拿不到答案，只是前者
        // 交回多个候选而后者交回近似候选。answer_effect 也把两者并列登记为
        // Addressing（answer_effect.rs:140-147），severity 不应一 Error 一默认
        // Warning——否则同一类失败在 diagnostics 里显得一个更严重。
        | "AMBIGUOUS_TARGET" => DiagnosticSeverity::Error,

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
        | CODE_SCANNER_UNRESOLVED_REFERENCE
        | CODE_SCANNER_DIAGNOSTICS_REFRESH_FAILED
        | CODE_SCANNER_DIAGNOSTICS_LOAD_FAILED
        | CODE_GRAPH_DB_NODE_DECODE_FAILED
        | CODE_GRAPH_DB_EDGE_DECODE_FAILED
        | CODE_GRAPH_DB_EDGE_DANGLING_ENDPOINT
        | CODE_GRAPH_DB_V2_LAYOUT_UNREADABLE
        | CODE_GRAPH_DB_PARTIAL_HYDRATE
        | CODE_PAGE_SCOPED_TARGET_FALLBACK
        // 解析失败：图内容陈旧但可用，查询仍能作答（只是可能答的是旧事实），
        // 未达「图库不可用」的 Error 档
        | CODE_SCANNER_FILE_PARSE_FAILED
        // 来源冲突：图仍可用（择一构建），但归属不可信
        | CODE_GRAPH_OWNERSHIP_CONFLICT => DiagnosticSeverity::Warning,
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
