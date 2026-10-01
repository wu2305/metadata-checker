use serde::{Deserialize, Serialize};

/// 统一机器输出 JSON 的顶层结构
///
/// 所有非 human JSON 输出必须通过此结构序列化，禁止手写 json!。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiOutput {
    pub schema_version: String,
    pub kind: OutputKind,
    pub query_target: Option<String>,
    pub summary: serde_json::Value,
    pub details: Option<serde_json::Value>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
    pub next_queries: Vec<String>,
}

impl AiOutput {
    /// 创建最小输出结构，query_target 为 None
    pub fn new(kind: OutputKind, summary: serde_json::Value) -> Self {
        AiOutput {
            schema_version: "1.0".to_string(),
            kind,
            query_target: None,
            summary,
            details: None,
            evidence: Vec::new(),
            diagnostics: Vec::new(),
            next_queries: Vec::new(),
        }
    }

    /// 验证输出契约：evidence 不能为空（若 summary 有实质内容）
    pub fn validate(self) -> Self {
        let mut out = self;
        // 如果 summary 不是空对象或空数组，且 evidence 为空，则追加 diagnostic
        let has_substantive_summary = !out.summary.is_null()
            && out
                .summary
                .as_object()
                .map(|o| !o.is_empty())
                .unwrap_or(true)
            && out
                .summary
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(true);

        if has_substantive_summary && out.evidence.is_empty() {
            out.evidence.push(Evidence::new(
                "Output generated from parsed metadata",
                "Fallback evidence injected because this query path did not emit structured evidence",
            ).with_confidence(Confidence::Low));
            // 局部构造好再一次 push，避免「先 push 再 last_mut() 改」的两段式写法
            let mut diag = crate::diagnostics::envelope_diagnostic(
                "EVIDENCE_INCOMPLETE",
                1,
                Location::default(),
                "Summary contains claims but no structured evidence was generated",
            );
            diag.suggestion = Some(
                "Use --detail for manual verification, and treat conclusions as low confidence"
                    .to_string(),
            );
            out.diagnostics.push(diag);
        }
        out
    }
}

/// 输出类别枚举，禁止随意字符串
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum OutputKind {
    SuperPage,
    PageQuery,
    ModelQuery,
    CrossPageQuery,
    DataFlowQuery,
    ComponentQuery,
    PriorityQuery,
    Explain,
    Context,
    PageLogic,
    Table,
    DataFlow,
    GraphDbCheck,
    /// `--advise-query` 的路由建议。
    QueryAdvice,
}

/// 证据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub claim: String,
    pub source_file: Option<String>,
    pub node_id: Option<String>,
    pub edge_type: Option<String>,
    pub raw_expr: Option<String>,
    pub json_path: Option<String>,
    pub confidence: Confidence,
    pub reason: String,
}

impl Evidence {
    pub fn new(claim: impl Into<String>, reason: impl Into<String>) -> Self {
        Evidence {
            claim: claim.into(),
            source_file: None,
            node_id: None,
            edge_type: None,
            raw_expr: None,
            json_path: None,
            confidence: Confidence::Medium,
            reason: reason.into(),
        }
    }

    pub fn with_source_file(mut self, path: impl Into<String>) -> Self {
        self.source_file = Some(path.into());
        self
    }

    pub fn with_node_id(mut self, id: impl Into<String>) -> Self {
        self.node_id = Some(id.into());
        self
    }

    pub fn with_edge_type(mut self, edge: impl Into<String>) -> Self {
        self.edge_type = Some(edge.into());
        self
    }

    pub fn with_raw_expr(mut self, expr: impl Into<String>) -> Self {
        self.raw_expr = Some(expr.into());
        self
    }

    pub fn with_json_path(mut self, path: impl Into<String>) -> Self {
        self.json_path = Some(path.into());
        self
    }

    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        self.confidence = confidence;
        self
    }
}

/// 置信度
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// 诊断结构
///
/// 统一诊断信封字段（M58.3 Phase 1）：`{ code, severity, count, sample_location, answer_impact, first_seen_phase }`
/// 内部字段名为 `location`，序列化/反序列化均使用 `sample_location`；`location` 仅作反序列化别名以兼容旧数据。
#[derive(Debug, Clone, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(rename = "sample_location", alias = "location")]
    pub location: Location,
    pub suggestion: Option<String>,
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default, rename = "answer_impact")]
    pub answer_impact: Option<String>,
    #[serde(default)]
    pub first_seen_phase: Option<String>,
}

impl Diagnostic {
    pub fn with_count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }
    pub fn with_answer_impact(mut self, impact: impl Into<String>) -> Self {
        self.answer_impact = Some(impact.into());
        self
    }
    pub fn with_first_seen_phase(mut self, phase: impl Into<String>) -> Self {
        self.first_seen_phase = Some(phase.into());
        self
    }
}

impl Serialize for Diagnostic {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error as _, SerializeStruct};
        let effect = crate::output::answer_effect::answer_effect(&self.code);
        // 信封字段缺失是构造方 bug：显式报序列化错误，不静默编值、不 panic。
        let count = self.count.ok_or_else(|| {
            S::Error::custom("Diagnostic.count must be set via envelope_diagnostic")
        })?;
        let answer_impact = self.answer_impact.as_deref().ok_or_else(|| {
            S::Error::custom("Diagnostic.answer_impact must be set via envelope_diagnostic")
        })?;
        let first_seen_phase = self.first_seen_phase.as_deref().ok_or_else(|| {
            S::Error::custom("Diagnostic.first_seen_phase must be set via envelope_diagnostic")
        })?;
        let mut extra = 8;
        if effect.is_some() {
            extra += 1;
        }
        let mut state = serializer.serialize_struct("Diagnostic", extra)?;
        state.serialize_field("severity", &self.severity)?;
        state.serialize_field("code", &self.code)?;
        state.serialize_field("message", &self.message)?;
        state.serialize_field("count", &count)?;
        state.serialize_field("sample_location", &self.location)?;
        state.serialize_field("answer_impact", answer_impact)?;
        state.serialize_field("first_seen_phase", first_seen_phase)?;
        state.serialize_field("suggestion", &self.suggestion)?;
        if let Some((_, effect)) = effect {
            state.serialize_field("answer_effect", effect)?;
        }
        state.end()
    }
}

/// 诊断严重级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

/// 位置信息
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Location {
    pub source_file: Option<String>,
    pub node_id: Option<String>,
    pub json_path: Option<String>,
}

impl Location {
    pub fn new() -> Self {
        Location {
            source_file: None,
            node_id: None,
            json_path: None,
        }
    }
}

/// 预算级别
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Budget {
    Compact,
    Normal,
    Full,
}

impl std::str::FromStr for Budget {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "compact" => Ok(Budget::Compact),
            "normal" => Ok(Budget::Normal),
            "full" => Ok(Budget::Full),
            _ => Err(format!(
                "Unknown budget '{}'. Expected compact|normal|full",
                s
            )),
        }
    }
}

/// Shell-safe 单引号包裹 CLI 参数
///
/// 当参数包含空格、|、中文、$、(、) 等需要转义的字符时，用单引号包裹。
/// 如果参数本身不含特殊字符，原样返回，避免不必要的引号。
pub fn quote_cli_arg(arg: &str) -> String {
    let needs_quote = arg.chars().any(|c| {
        !c.is_ascii()
            || c.is_whitespace()
            || matches!(
                c,
                '|' | '&'
                    | ';'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '$'
                    | '`'
                    | '"'
                    | '\''
                    | '<'
                    | '>'
                    | '*'
                    | '?'
                    | '['
                    | ']'
                    | '#'
                    | '\\'
                    | '!'
            )
    });
    if needs_quote || arg.is_empty() {
        let escaped = arg.replace("'", "'\\''");
        format!("'{}'", escaped)
    } else {
        arg.to_string()
    }
}

/// 生成 shell-safe 的 next_query 字符串
///
/// template 中的 {} 占位符会被 quote_cli_arg 处理后的值替换。
/// 例如 format_next_query("--explain {} for summary", target_id)
pub fn format_next_query(template: &str, arg: &str) -> String {
    let quoted = quote_cli_arg(arg);
    template.replacen("{}", &quoted, 1)
}

/// 多参数版本，按顺序替换 template 中的 {} 占位符
pub fn format_next_query_multi(template: &str, args: &[&str]) -> String {
    let mut result = template.to_string();
    for arg in args {
        result = result.replacen("{}", &quote_cli_arg(arg), 1);
    }
    result
}

/// 由 target 的类型前缀推出该用哪条 `--find-*` 命令，以及该拿什么当搜索关键词。
///
/// 该建议的是「按节点种类去搜」，而节点种类由 **target 前缀**决定，与发起查询的
/// `OutputKind` 无关：`--context model:X` 与 `--query-model model:X` 问的是同一类
/// 节点，都该建议 `--find-model`。此前按 `OutputKind` 分支，`Context` 落到
/// `_ => --find-page`，于是 `--context model:ordersView` 会给出
/// `--find-page 'model:ordersView'` —— 搜的是 Page 类型，一条都搜不到。
///
/// 关键词还要**去掉前缀**：`find_nodes` 是拿关键词去匹配 `id`/`name`/`path`
/// 的子串，而 id 形如 `model:app/a.spg|ordersView`。带前缀的
/// `--find-model 'model:ordersView'` 匹配不到任何节点（前缀 + 局部名的组合从不是
/// 任何 id 的子串），去掉前缀后的 `ordersView` 才命中。字段没有独立 find 动词，
/// 按所属模型定位。
///
/// 返回 `None` 表示 target 无前缀（裸名），由调用方退回按 `OutputKind` 推断。
fn find_command_for_target(target: &str) -> Option<(&'static str, &str)> {
    let (prefix, bare) = target.split_once(':')?;
    if bare.is_empty() {
        return None;
    }
    match prefix {
        "model" | "dataflow" => Some(("--find-model {}", bare)),
        "page" => Some(("--find-page {}", bare)),
        "comp" | "action" => Some(("--find-component {}", bare)),
        // 字段没有独立的 find 动词，`--find-model` 又只匹配 Model 类型节点，
        // 拿字段全名去搜必然空手。退到「它所属的模型」：`field:<PAGE>|<model>.<field>`
        // 的裸名取最后一个 `.` 之前的部分即模型名。
        "field" => {
            let model = bare
                .rsplit_once('.')
                .map(|(model, _)| model)
                .unwrap_or(bare);
            (!model.is_empty()).then_some(("--find-model {}", model))
        }
        _ => None,
    }
}

/// 构建目标不存在时的结构化输出，附带候选建议
pub fn build_target_not_found_output(
    kind: OutputKind,
    target_id: &str,
    candidates: &[(crate::graph::Node, String)],
) -> AiOutput {
    let candidate_targets: Vec<serde_json::Value> = candidates
        .iter()
        .map(|(node, reason)| {
            serde_json::json!({
                "id": node.id,
                "name": node.name,
                "node_type": format!("{:?}", node.node_type),
                "reason": reason,
            })
        })
        .collect();

    let summary = serde_json::json!({
        "target_id": target_id,
        "resolved_count": 0,
        "what_is_it": format!("Target '{}' not found in graph", target_id),
        "candidate_count": candidate_targets.len(),
    });

    // 先按 target 前缀定节点种类与搜索关键词（权威），再退回按 OutputKind 推断
    // （裸名 target 没有前缀可用）。两条路径都给出可执行的 `--find-*`。
    let (find_cmd, find_keyword) = match find_command_for_target(target_id) {
        Some((command, keyword)) => (command, keyword.to_string()),
        None => {
            let command = match kind {
                OutputKind::ModelQuery | OutputKind::DataFlowQuery | OutputKind::DataFlow => {
                    "--find-model {}"
                }
                OutputKind::ComponentQuery | OutputKind::Explain | OutputKind::SuperPage => {
                    "--find-component {}"
                }
                _ => "--find-page {}",
            };
            (command, target_id.to_string())
        }
    };

    let mut out = AiOutput::new(kind, summary);
    out.query_target = Some(target_id.to_string());
    out.details = Some(serde_json::json!({
        "candidate_targets": candidate_targets,
    }));
    let mut target_diag = crate::diagnostics::envelope_diagnostic(
        "TARGET_NOT_FOUND",
        1,
        Location::default(),
        format!("Target '{}' not found in graph", target_id),
    );
    target_diag.suggestion = Some(if candidates.is_empty() {
        format!(
            "Verify the target ID or use {} to search globally",
            find_cmd.replace("{}", "<keyword>")
        )
    } else {
        "Did you mean one of the candidate targets below?".to_string()
    });
    out.diagnostics.push(target_diag);

    for (node, _) in candidates {
        out.next_queries.push(format_next_query(
            "--explain {} for semantic summary",
            &node.id,
        ));
    }
    if candidates.is_empty() {
        out.next_queries
            .push(format_next_query(find_cmd, &find_keyword));
    }
    out.validate()
}
