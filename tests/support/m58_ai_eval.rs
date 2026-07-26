//! M58 CI tester 的评测 runner 支持代码。

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::VecDeque;
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
