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
            out.diagnostics.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: "EVIDENCE_INCOMPLETE".to_string(),
                message: "Summary contains claims but no structured evidence was generated"
                    .to_string(),
                location: Location::default(),
                suggestion: Some(
                    "Use --detail for manual verification, and treat conclusions as low confidence"
                        .to_string(),
                ),
            });
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub location: Location,
    pub suggestion: Option<String>,
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

    let find_cmd = match kind {
        OutputKind::ModelQuery | OutputKind::DataFlowQuery | OutputKind::DataFlow => {
            "--find-model {} to search globally"
        }
        OutputKind::ComponentQuery | OutputKind::Explain | OutputKind::SuperPage => {
            "--find-component {} to search globally"
        }
        _ => "--find-page {} to search globally",
    };

    let mut out = AiOutput::new(kind, summary);
    out.query_target = Some(target_id.to_string());
    out.details = Some(serde_json::json!({
        "candidate_targets": candidate_targets,
    }));
    out.diagnostics.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: "TARGET_NOT_FOUND".to_string(),
        message: format!("Target '{}' not found in graph", target_id),
        location: Location::default(),
        suggestion: if candidates.is_empty() {
            Some(format!(
                "Verify the target ID or use {} to search",
                find_cmd.replace("{}", "<keyword>")
            ))
        } else {
            Some("Did you mean one of the candidate targets below?".to_string())
        },
    });

    for (node, _) in candidates {
        out.next_queries.push(format_next_query(
            "--explain {} for semantic summary",
            &node.id,
        ));
    }
    if candidates.is_empty() {
        out.next_queries
            .push(format_next_query(find_cmd, target_id));
    }
    out.validate()
}
