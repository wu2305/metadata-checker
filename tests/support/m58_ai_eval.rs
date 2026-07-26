//! M58 CI tester 的评测 runner 支持代码。

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// M58 评测分层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvalTier {
    /// 只校验 CLI 结构化输出，不调用 LLM。
    FixtureStructural,
    /// 使用 fixture 项目调用真实或 fake LLM。
    FixtureLlm,
    /// 仅允许人工或 Agent 执行的真实项目 case。
    RealManual,
}

impl EvalTier {
    /// 从 JSON 字符串解析评测分层。
    fn parse(value: &str, case_id: &str) -> Result<Self> {
        match value {
            "fixture_structural" => Ok(Self::FixtureStructural),
            "fixture_llm" => Ok(Self::FixtureLlm),
            "real_manual" => Ok(Self::RealManual),
            _ => bail!("case {case_id} 的 tier '{value}' 无效"),
        }
    }
}

/// 已加载的评测 case；原始 JSON 保留给判分器读取既有断言。
#[derive(Debug, Clone)]
pub(crate) struct EvalCase {
    /// case 稳定标识。
    pub(crate) case_id: String,
    /// 交给模型的问题。
    pub(crate) question: String,
    /// 评测分层。
    pub(crate) tier: EvalTier,
    /// case 生命周期状态。
    pub(crate) case_status: String,
    /// case 难度。
    pub(crate) difficulty: String,
    /// 原始 case JSON。
    pub(crate) value: Value,
}

/// 加载并校验 M58 评测 case。
pub(crate) fn load_eval_cases(path: &Path) -> Result<Vec<EvalCase>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("读取评测 case 失败: {}", path.display()))?;
    let root: Value = serde_json::from_str(&content)
        .with_context(|| format!("解析评测 case JSON 失败: {}", path.display()))?;
    let cases = root
        .get("cases")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("评测 JSON 缺少 cases 数组"))?;

    cases
        .iter()
        .map(|value| {
            let case_id = value
                .get("case_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("评测 case 缺少 case_id"))?;
            let question = value
                .get("question")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("case {case_id} 缺少 question"))?;
            let tier = match value.get("tier").and_then(Value::as_str) {
                Some(raw) => EvalTier::parse(raw, case_id)?,
                None => {
                    if value
                        .get("requires_real_project")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        EvalTier::RealManual
                    } else {
                        EvalTier::FixtureStructural
                    }
                }
            };

            Ok(EvalCase {
                case_id: case_id.to_string(),
                question: question.to_string(),
                tier,
                case_status: value
                    .get("case_status")
                    .and_then(Value::as_str)
                    .unwrap_or("active")
                    .to_string(),
                difficulty: value
                    .get("difficulty")
                    .and_then(Value::as_str)
                    .unwrap_or("basic")
                    .to_string(),
                value: value.clone(),
            })
        })
        .collect()
}

/// 选择 active 的 fixture LLM case。
pub(crate) fn fixture_llm_cases(cases: &[EvalCase]) -> Vec<EvalCase> {
    cases
        .iter()
        .filter(|case| case.tier == EvalTier::FixtureLlm && case.case_status == "active")
        .cloned()
        .collect()
}

/// 模型返回的命令请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommandRequest {
    /// CLI 命令 kind，例如 `--query-page-logic`。
    pub(crate) command_kind: String,
    /// 命令目标。
    pub(crate) target: String,
    /// 已在 case plan 中允许的参数。
    pub(crate) args: Vec<String>,
    /// 输出预算。
    pub(crate) budget: Option<String>,
}

/// 模型一轮响应的语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentTurn {
    /// 要求 runner 执行一个白名单命令。
    Command(CommandRequest),
    /// 模型给出的最终答案。
    Final(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommandEnvelope {
    kind: String,
    command_kind: String,
    target: String,
    args: Vec<String>,
    budget: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFinalEnvelope {
    kind: String,
    answer: String,
}

/// 解析模型的 JSON command/final 消息。
pub(crate) fn parse_agent_turn(content: &str) -> Result<AgentTurn> {
    let value: Value = serde_json::from_str(content).context("模型响应不是合法 JSON")?;
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("模型响应缺少 kind"))?;

    match kind {
        "command" => {
            let envelope: RawCommandEnvelope =
                serde_json::from_value(value).context("command envelope 无效")?;
            if envelope.kind != "command" {
                bail!("command envelope 的 kind 不一致");
            }
            if envelope.command_kind.trim().is_empty() {
                bail!("command_kind 不能为空");
            }
            if envelope.target.trim().is_empty() {
                bail!("target 不能为空");
            }
            if contains_shell_metacharacters(&envelope.command_kind)
                || contains_shell_metacharacters(&envelope.target)
            {
                bail!("command_kind 或 target 包含禁止的 shell 元字符");
            }
            if envelope
                .args
                .iter()
                .any(|arg| arg.contains('\0') || contains_shell_metacharacters(arg))
            {
                bail!("args 包含禁止的 shell 元字符");
            }
            Ok(AgentTurn::Command(CommandRequest {
                command_kind: envelope.command_kind,
                target: envelope.target,
                args: envelope.args,
                budget: envelope.budget,
            }))
        }
        "final" => {
            let envelope: RawFinalEnvelope =
                serde_json::from_value(value).context("final envelope 无效")?;
            if envelope.kind != "final" {
                bail!("final envelope 的 kind 不一致");
            }
            if envelope.answer.trim().is_empty() {
                bail!("final answer 不能为空");
            }
            Ok(AgentTurn::Final(envelope.answer))
        }
        _ => bail!("不支持的模型响应 kind: {kind}"),
    }
}

/// 评测计划中的单个命令步骤。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct PlanStep {
    command_kind: String,
    target: String,
    args: Vec<String>,
    requires_project_dir: bool,
    budget: Option<String>,
}

/// 通过 case plan 固定命令路径和运行目录。
#[derive(Debug, Clone)]
pub(crate) struct CommandPolicy {
    steps: Vec<PlanStep>,
    max_command_count: usize,
    project_dir: PathBuf,
    graph_db_path: PathBuf,
    binary_path: PathBuf,
}

/// 经白名单校验后可执行的命令。
#[derive(Debug, Clone)]
pub(crate) struct ValidatedCommand {
    step_index: usize,
    step: PlanStep,
    project_dir: PathBuf,
    graph_db_path: PathBuf,
    binary_path: PathBuf,
}

impl CommandPolicy {
    /// 从 case 的 minimal_command_plan 构造固定策略。
    pub(crate) fn from_case(
        case: &EvalCase,
        project_dir: PathBuf,
        graph_db_path: PathBuf,
        binary_path: PathBuf,
    ) -> Result<Self> {
        let steps_value = case
            .value
            .get("minimal_command_plan")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("case {} 缺少 minimal_command_plan", case.case_id))?;
        let steps = steps_value
            .iter()
            .cloned()
            .map(|value| serde_json::from_value(value).context("minimal_command_plan step 无效"))
            .collect::<Result<Vec<PlanStep>>>()?;
        let max_command_count = case
            .value
            .get("max_command_count")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("case {} 缺少 max_command_count", case.case_id))?
            as usize;
        if max_command_count == 0 || max_command_count < steps.len() {
            bail!(
                "case {} 的 max_command_count {} 小于 plan 步数 {}",
                case.case_id,
                max_command_count,
                steps.len()
            );
        }

        Ok(Self {
            steps,
            max_command_count,
            project_dir,
            graph_db_path,
            binary_path,
        })
    }

    /// 校验模型命令只能使用尚未消费的 plan step。
    pub(crate) fn validate(
        &self,
        request: &CommandRequest,
        used_steps: &[usize],
    ) -> Result<ValidatedCommand> {
        if used_steps.len() >= self.max_command_count {
            bail!("命令数超过 max_command_count={}", self.max_command_count);
        }
        if contains_shell_metacharacters(&request.target) {
            bail!("target 包含禁止的 shell 元字符");
        }
        let step_index = self
            .steps
            .iter()
            .enumerate()
            .find(|(index, step)| {
                !used_steps.contains(index)
                    && step.command_kind == request.command_kind
                    && step.target == request.target
                    && step.args == request.args
                    && step.budget == request.budget
            })
            .map(|(index, _)| index)
            .ok_or_else(|| anyhow!("模型命令不匹配任何未使用的 minimal_command_plan step"))?;

        Ok(ValidatedCommand {
            step_index,
            step: self.steps[step_index].clone(),
            project_dir: self.project_dir.clone(),
            graph_db_path: self.graph_db_path.clone(),
            binary_path: self.binary_path.clone(),
        })
    }
}

impl ValidatedCommand {
    /// 返回不经过 shell 的 CLI 参数。
    pub(crate) fn argv(&self) -> Vec<OsString> {
        let mut argv = vec![OsString::from("--non-human")];
        if self.step.requires_project_dir {
            argv.push(OsString::from("--project-dir"));
            argv.push(self.project_dir.as_os_str().to_os_string());
            argv.push(OsString::from("--graph-db-path"));
            argv.push(self.graph_db_path.as_os_str().to_os_string());
            argv.push(OsString::from(&self.step.command_kind));
            argv.push(OsString::from(&self.step.target));
        } else {
            argv.push(OsString::from(&self.step.target));
        }
        argv.extend(self.step.args.iter().map(OsString::from));
        argv
    }

    /// 返回 plan step 索引，用于阻止重复执行。
    pub(crate) fn step_index(&self) -> usize {
        self.step_index
    }

    /// 返回 runner 选定的二进制路径。
    pub(crate) fn binary_path(&self) -> &Path {
        &self.binary_path
    }
}

/// 判断输入是否含有 shell 注入常见元字符。
fn contains_shell_metacharacters(value: &str) -> bool {
    [";", "&&", "||", "`", "$(", "\n", "\r", "\0"]
        .iter()
        .any(|marker| value.contains(marker))
}

/// CNB AI Chat 使用的消息结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChatMessage {
    /// 消息角色；runner 只生成 user 和 assistant。
    pub(crate) role: String,
    /// 消息文本。
    pub(crate) content: String,
}

/// CNB AI Chat 请求结构。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChatRequest {
    /// 当前完整对话历史。
    pub(crate) messages: Vec<ChatMessage>,
    /// CNB 上配置的模型标识。
    pub(crate) model: String,
    /// M58 runner 固定使用非流式响应。
    pub(crate) stream: bool,
}

/// CNB AI Chat 响应结构，只消费公开 contract 中的首个 choice。
#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

/// LLM adapter 的最小同步接口。
pub(crate) trait ModelAdapter {
    /// 发送一轮对话并返回 assistant content。
    fn complete(&mut self, request: &ChatRequest) -> Result<String>;
}

/// 本地契约测试使用的确定性模型 adapter。
#[derive(Debug, Default)]
pub(crate) struct FakeModelAdapter {
    responses: VecDeque<String>,
    requests: Vec<ChatRequest>,
}

impl FakeModelAdapter {
    /// 按顺序返回预置响应，并记录每次收到的完整请求。
    pub(crate) fn from_responses(responses: Vec<String>) -> Self {
        Self {
            responses: responses.into(),
            requests: Vec::new(),
        }
    }

    /// 返回已记录的请求，供多轮 history 契约测试检查。
    pub(crate) fn requests(&self) -> &[ChatRequest] {
        &self.requests
    }
}

impl ModelAdapter for FakeModelAdapter {
    /// 返回下一个 fake 响应；响应耗尽时明确失败。
    fn complete(&mut self, request: &ChatRequest) -> Result<String> {
        self.requests.push(request.clone());
        self.responses
            .pop_front()
            .ok_or_else(|| anyhow!("fake model response queue exhausted"))
    }
}

/// 通过 CNB AI Chat HTTP API 调用模型的 adapter。
pub(crate) struct CnbChatAdapter {
    client: reqwest::blocking::Client,
    endpoint: String,
    repo: String,
    token: String,
    model: String,
}

impl fmt::Debug for CnbChatAdapter {
    /// 只输出非敏感配置，绝不输出 CNB token。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CnbChatAdapter")
            .field("endpoint", &self.endpoint)
            .field("repo", &self.repo)
            .field("token", &"[REDACTED]")
            .field("model", &self.model)
            .finish()
    }
}

impl CnbChatAdapter {
    /// 从 CNB pipeline 环境读取 adapter 配置。
    pub(crate) fn from_env() -> Result<Self> {
        let token = required_env("CNB_TOKEN")?;
        let repo = required_env("M58_CNB_REPO")?;
        let model = required_env("M58_CNB_MODEL")?;
        let endpoint = std::env::var("M58_CNB_API_BASE")
            .unwrap_or_else(|_| "https://api.cnb.cool".to_string());
        Self::new(endpoint, repo, token, model)
    }

    /// 创建 CNB adapter；endpoint 可替换为本地 HTTP fake server。
    pub(crate) fn new(
        endpoint: String,
        repo: String,
        token: String,
        model: String,
    ) -> Result<Self> {
        if endpoint.trim().is_empty() {
            bail!("CNB API endpoint 不能为空");
        }
        if repo.trim().is_empty() {
            bail!("CNB repo 不能为空");
        }
        if token.is_empty() {
            bail!("CNB token 不能为空");
        }
        if model.trim().is_empty() {
            bail!("CNB model 不能为空");
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .context("创建 CNB AI Chat HTTP client 失败")?;
        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            repo: repo.trim_matches('/').to_string(),
            token,
            model,
        })
    }

    /// 返回完整的 CNB 仓库 AI Chat endpoint。
    fn chat_url(&self) -> String {
        format!("{}/{}/-/ai/chat/completions", self.endpoint, self.repo)
    }
}

impl ModelAdapter for CnbChatAdapter {
    /// 发送非流式 CNB AI Chat 请求并提取首个 choice 的内容。
    fn complete(&mut self, request: &ChatRequest) -> Result<String> {
        let mut payload = request.clone();
        payload.model = self.model.clone();
        payload.stream = false;
        let response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.token)
            .json(&payload)
            .send()
            .context("请求 CNB AI Chat 失败")?;
        let status = response.status();
        let body = response.text().context("读取 CNB AI Chat 响应失败")?;
        if !status.is_success() {
            let summary = redact_secret(&summarize_body(&body), &self.token);
            bail!("CNB AI Chat HTTP {}: {}", status, summary);
        }

        let parsed: ChatResponse =
            serde_json::from_str(&body).context("CNB AI Chat 响应不是合法的 choices JSON")?;
        let content = parsed
            .choices
            .first()
            .ok_or_else(|| anyhow!("CNB AI Chat 响应缺少 choices[0]"))?
            .message
            .content
            .clone();
        if content.trim().is_empty() {
            bail!("CNB AI Chat choices[0].message.content 为空");
        }
        Ok(content)
    }
}

/// 读取必需的非空环境变量。
fn required_env(name: &str) -> Result<String> {
    let value = std::env::var(name).with_context(|| format!("缺少环境变量 {name}"))?;
    if value.trim().is_empty() {
        bail!("环境变量 {name} 不能为空");
    }
    Ok(value)
}

/// 截断 HTTP 错误 body，避免把远端大响应写入测试日志。
fn summarize_body(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary: String = compact.chars().take(512).collect();
    if compact.chars().count() > 512 {
        format!("{summary}…")
    } else {
        summary
    }
}

/// 将敏感 token 从错误或诊断文本中替换掉。
pub(crate) fn redact_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "[REDACTED]")
    }
}

/// 单个 CLI 命令在一次 LLM case 中的结构化轨迹。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandTrace {
    /// CLI 命令 kind。
    pub(crate) command_kind: String,
    /// 命令 target。
    pub(crate) target: String,
    /// 经过 plan 校验的参数。
    pub(crate) args: Vec<String>,
    /// 经过 plan 校验的预算。
    pub(crate) budget: Option<String>,
    /// 对应的 minimal_command_plan step；拒绝的命令为 None。
    pub(crate) plan_step_index: Option<usize>,
    /// 是否通过 runner 白名单。
    pub(crate) accepted: bool,
    /// 是否请求了 detail/full 等过度读取路径。
    pub(crate) detail_request: bool,
    /// CLI 输出中实际回传的顶层 sections。
    pub(crate) output_sections: Vec<String>,
}

/// AnswerJudge 的确定性判分结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JudgeResult {
    /// 没有失败分类时为 true。
    pub(crate) passed: bool,
    /// 一个回答可以同时命中多个失败分类。
    pub(crate) failure_classes: Vec<String>,
    /// 不包含原始回答的短诊断说明。
    pub(crate) judge_notes: Vec<String>,
}

/// 单个 case 的脱敏报告记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CaseReport {
    /// case 稳定标识。
    pub(crate) case_id: String,
    /// `pass`、`fail` 或 `error`。
    pub(crate) status: String,
    /// 是否通过 AnswerJudge。
    pub(crate) passed: bool,
    /// 确定性失败分类。
    pub(crate) failure_classes: Vec<String>,
    /// 不包含模型原文的短说明。
    pub(crate) judge_notes: Vec<String>,
    /// 命令使用轨迹。
    pub(crate) command_trace: Vec<CommandTrace>,
}

/// RunReport 中的命令统计快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommandTraceStats {
    /// 所有 case 的命令尝试数。
    pub(crate) total_commands: usize,
    /// 通过白名单的命令数。
    pub(crate) accepted_commands: usize,
    /// 被协议或 policy 拒绝的命令数。
    pub(crate) rejected_commands: usize,
    /// 至少执行过一个命令的 case 数。
    pub(crate) cases_with_commands: usize,
}

/// M58 一次评测的唯一结构化事实源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RunReport {
    /// 报告 schema 版本。
    pub(crate) schema_version: String,
    /// provider 名称，如 `cnb-ai-chat` 或 `fake`。
    pub(crate) provider: String,
    /// 模型标识。
    pub(crate) model_id: String,
    /// CNB 构建标识；本地 fake run 可以为空。
    pub(crate) cnb_build_id: Option<String>,
    /// 运行开始时间，由 runner 注入。
    pub(crate) started_at: String,
    /// 不含 prompt/answer/token 的 case 记录。
    pub(crate) cases: Vec<CaseReport>,
    /// 通过 case 数除以总 case 数，空运行固定为 0。
    pub(crate) pass_rate: f64,
    /// 按失败分类聚合的数量。
    pub(crate) failure_classes: BTreeMap<String, usize>,
    /// 命令轨迹统计。
    pub(crate) command_trace_stats: CommandTraceStats,
}

impl RunReport {
    /// 从 case 记录计算稳定的报告聚合字段。
    pub(crate) fn from_cases(
        provider: String,
        model_id: String,
        cnb_build_id: Option<String>,
        started_at: String,
        cases: Vec<CaseReport>,
    ) -> Self {
        let passed_count = cases.iter().filter(|case| case.passed).count();
        let pass_rate = if cases.is_empty() {
            0.0
        } else {
            passed_count as f64 / cases.len() as f64
        };
        let mut failure_classes = BTreeMap::new();
        let mut command_trace_stats = CommandTraceStats {
            total_commands: 0,
            accepted_commands: 0,
            rejected_commands: 0,
            cases_with_commands: 0,
        };
        for case in &cases {
            for failure_class in &case.failure_classes {
                *failure_classes.entry(failure_class.clone()).or_insert(0) += 1;
            }
            if !case.command_trace.is_empty() {
                command_trace_stats.cases_with_commands += 1;
            }
            command_trace_stats.total_commands += case.command_trace.len();
            command_trace_stats.accepted_commands += case
                .command_trace
                .iter()
                .filter(|trace| trace.accepted)
                .count();
            command_trace_stats.rejected_commands += case
                .command_trace
                .iter()
                .filter(|trace| !trace.accepted)
                .count();
        }
        Self {
            schema_version: "1.0.0".to_string(),
            provider,
            model_id,
            cnb_build_id,
            started_at,
            cases,
            pass_rate,
            failure_classes,
            command_trace_stats,
        }
    }
}

/// 对 answer_assertions 执行不依赖 LLM 的确定性判分。
pub(crate) fn judge_answer(
    answer: &str,
    assertions: &Value,
    diagnostics: &[String],
    trace: &[CommandTrace],
) -> JudgeResult {
    let normalized_answer = normalize_for_match(answer);
    let mut failure_classes = Vec::new();
    let mut judge_notes = Vec::new();

    let must_include = read_string_assertions(assertions, "must_include", &mut failure_classes);
    for expected in must_include {
        if !normalized_answer.contains(&normalize_for_match(&expected)) {
            add_failure(&mut failure_classes, "missed_fact");
            judge_notes.push(format!("missing must_include assertion: {expected}"));
        }
    }

    let must_not_include =
        read_string_assertions(assertions, "must_not_include", &mut failure_classes);
    for forbidden in must_not_include {
        if normalized_answer.contains(&normalize_for_match(&forbidden)) {
            add_failure(&mut failure_classes, "hallucination");
            judge_notes.push(format!("hit must_not_include assertion: {forbidden}"));
        }
    }

    let diagnostic_required = read_bool_assertion(
        assertions,
        "diagnostic_disclaimer_required",
        &mut failure_classes,
    );
    if diagnostic_required
        && diagnostics
            .iter()
            .any(|diagnostic| !diagnostic.trim().is_empty())
    {
        if !contains_conservative_marker(&normalized_answer) {
            add_failure(&mut failure_classes, "ignored_diagnostic");
            judge_notes.push("diagnostics 存在但回答没有保守表达".to_string());
        }
    }

    let evidence_required = read_bool_assertion(
        assertions,
        "evidence_reference_required",
        &mut failure_classes,
    );
    if evidence_required && !contains_evidence_reference(&normalized_answer) {
        add_failure(&mut failure_classes, "needs_human_review");
        judge_notes.push("回答没有引用允许的 CLI evidence section".to_string());
    }

    if trace.is_empty()
        || trace
            .iter()
            .any(|entry| !entry.accepted || entry.plan_step_index.is_none())
    {
        add_failure(&mut failure_classes, "wrong_command");
        judge_notes.push("command trace 为空或包含未通过 plan 的命令".to_string());
    }
    if trace.iter().any(|entry| {
        entry.detail_request
            || entry.budget.as_deref() == Some("full")
            || entry.args.iter().any(|arg| arg == "--detail")
            || entry
                .args
                .windows(2)
                .any(|window| window == ["--budget", "full"])
    }) {
        add_failure(&mut failure_classes, "over_read_details");
        judge_notes.push("command trace 请求了 detail 或 full 输出".to_string());
    }

    JudgeResult {
        passed: failure_classes.is_empty(),
        failure_classes,
        judge_notes,
    }
}

/// 将 RunReport 写成 JSON，并由同一对象生成 Markdown 摘要。
pub(crate) fn write_run_report(
    report: &RunReport,
    json_path: &Path,
    markdown_path: &Path,
) -> Result<()> {
    for path in [json_path, markdown_path] {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建评测报告目录失败: {}", parent.display()))?;
        }
    }
    let json = serde_json::to_string_pretty(report).context("序列化 RunReport 失败")?;
    std::fs::write(json_path, format!("{json}\n"))
        .with_context(|| format!("写入 JSON 评测报告失败: {}", json_path.display()))?;
    std::fs::write(markdown_path, render_report_markdown(report))
        .with_context(|| format!("写入 Markdown 评测报告失败: {}", markdown_path.display()))?;
    Ok(())
}

/// 将唯一 JSON 事实源投影为不含模型原文的 Markdown 表格。
fn render_report_markdown(report: &RunReport) -> String {
    let mut markdown = String::new();
    markdown.push_str("# M58 AI Eval Run\n\n");
    markdown.push_str(&format!(
        "- provider: `{}`\n- model: `{}`\n- cnb_build_id: `{}`\n- started_at: `{}`\n- pass_rate: `{:.4}`\n\n",
        escape_markdown_cell(&report.provider),
        escape_markdown_cell(&report.model_id),
        escape_markdown_cell(report.cnb_build_id.as_deref().unwrap_or("")),
        escape_markdown_cell(&report.started_at),
        report.pass_rate,
    ));
    markdown.push_str("| case_id | status | failure_classes | command_count |\n");
    markdown.push_str("| --- | --- | --- | ---: |\n");
    for case in &report.cases {
        let failures = if case.failure_classes.is_empty() {
            "-".to_string()
        } else {
            case.failure_classes.join(", ")
        };
        markdown.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            escape_markdown_cell(&case.case_id),
            escape_markdown_cell(&case.status),
            escape_markdown_cell(&failures),
            case.command_trace.len(),
        ));
    }
    markdown.push_str("\n## Failure classes\n\n");
    for (failure_class, count) in &report.failure_classes {
        markdown.push_str(&format!("- `{failure_class}`: {count}\n"));
    }
    markdown
}

/// 读取字符串数组断言；schema 异常只进入 needs_human_review，不 panic。
fn read_string_assertions(
    assertions: &Value,
    field: &str,
    failure_classes: &mut Vec<String>,
) -> Vec<String> {
    let Some(value) = assertions.get(field) else {
        add_failure(failure_classes, "needs_human_review");
        return Vec::new();
    };
    let Some(values) = value.as_array() else {
        add_failure(failure_classes, "needs_human_review");
        return Vec::new();
    };
    let mut result = Vec::new();
    for item in values {
        if let Some(value) = item.as_str() {
            if !value.trim().is_empty() {
                result.push(value.to_string());
            }
        } else {
            add_failure(failure_classes, "needs_human_review");
        }
    }
    result
}

/// 读取布尔断言；schema 异常只进入 needs_human_review。
fn read_bool_assertion(assertions: &Value, field: &str, failure_classes: &mut Vec<String>) -> bool {
    match assertions.get(field).and_then(Value::as_bool) {
        Some(value) => value,
        None => {
            add_failure(failure_classes, "needs_human_review");
            false
        }
    }
}

/// 将大小写、空白和常见中英文标点归一化后再做 substring matching。
fn normalize_for_match(value: &str) -> String {
    let mut normalized = String::new();
    for character in value.to_lowercase().chars() {
        if character.is_whitespace()
            || matches!(
                character,
                ',' | '.'
                    | ':'
                    | ';'
                    | '!'
                    | '?'
                    | '，'
                    | '。'
                    | '：'
                    | '；'
                    | '！'
                    | '？'
                    | '、'
                    | '（'
                    | '）'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '【'
                    | '】'
            )
        {
            normalized.push(' ');
        } else {
            normalized.push(character);
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 判断回答是否显式表达不确定或证据边界。
fn contains_conservative_marker(answer: &str) -> bool {
    [
        "不确定",
        "未知",
        "可能",
        "无法确认",
        "证据不足",
        "不完整",
        "需人工",
        "保守",
        "uncertain",
        "unknown",
        "may",
        "cannot confirm",
        "insufficient evidence",
        "incomplete",
    ]
    .iter()
    .any(|marker| answer.contains(marker))
}

/// 判断回答是否引用了允许的 CLI 证据 sections。
fn contains_evidence_reference(answer: &str) -> bool {
    [
        "summary",
        "details",
        "evidence",
        "diagnostics",
        "primary_path",
        "key_findings",
    ]
    .iter()
    .any(|marker| answer.contains(marker))
}

/// 向失败分类列表中加入不重复的分类。
fn add_failure(failure_classes: &mut Vec<String>, failure_class: &str) {
    if !failure_classes.iter().any(|item| item == failure_class) {
        failure_classes.push(failure_class.to_string());
    }
}

/// 转义 Markdown 表格中的分隔符，避免报告结构被 case 文本破坏。
fn escape_markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}
