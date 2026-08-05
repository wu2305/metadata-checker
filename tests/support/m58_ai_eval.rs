//! M58 CI tester 的评测 runner 支持代码。

use anyhow::{Context, Result, anyhow, bail};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// 用于解释 case 难度和任务覆盖面的结构化维度。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvaluationDimensions {
    /// 任务所属的主要能力族，例如 page_logic 或 condition。
    pub(crate) task_family: String,
    /// 模型需要如何得到 target，例如 explicit、search 或 cross_file。
    pub(crate) target_resolution: String,
    /// 评测中预期存在的相似工具/命令干扰数量。
    pub(crate) distractor_count: usize,
    /// 任务中需要跟踪的依赖深度。
    pub(crate) dependency_depth: usize,
    /// 是否需要跨多个状态/回合保持上下文。
    pub(crate) stateful: bool,
    /// 是否显式注入工具或环境错误。
    pub(crate) error_injection: bool,
    /// 是否将截断/预算升级作为任务难点。
    pub(crate) output_truncation: bool,
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
    /// case 的主要任务族，供报告按能力切片。
    pub(crate) task_family: String,
    /// case 的结构化难度维度。
    pub(crate) evaluation_dimensions: EvaluationDimensions,
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
            let difficulty = value
                .get("difficulty")
                .and_then(Value::as_str)
                .unwrap_or("basic")
                .to_string();
            let task_family = value
                .get("evaluation_dimensions")
                .and_then(|dimensions| dimensions.get("task_family"))
                .and_then(Value::as_str)
                .or_else(|| {
                    value
                        .get("risk_tags")
                        .and_then(Value::as_array)
                        .and_then(|tags| tags.first())
                        .and_then(Value::as_str)
                })
                .unwrap_or("unclassified")
                .to_string();
            let evaluation_dimensions =
                parse_evaluation_dimensions(&value, &task_family, &difficulty)?;

            Ok(EvalCase {
                case_id: case_id.to_string(),
                question: question.to_string(),
                tier,
                case_status: value
                    .get("case_status")
                    .and_then(Value::as_str)
                    .unwrap_or("active")
                    .to_string(),
                difficulty,
                task_family,
                evaluation_dimensions,
                value: value.clone(),
            })
        })
        .collect()
}

/// 读取可选的难度维度，并为旧 schema 提供稳定默认值。
fn parse_evaluation_dimensions(
    value: &Value,
    task_family: &str,
    _difficulty: &str,
) -> Result<EvaluationDimensions> {
    let dimensions = value
        .get("evaluation_dimensions")
        .and_then(Value::as_object);
    let string_dimension = |name: &str, default: &str| -> Result<String> {
        match dimensions.and_then(|object| object.get(name)) {
            None => Ok(default.to_string()),
            Some(raw) => raw
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("evaluation_dimensions.{name} 必须是字符串")),
        }
    };
    let usize_dimension = |name: &str, default: usize| -> Result<usize> {
        match dimensions.and_then(|object| object.get(name)) {
            None => Ok(default),
            Some(raw) => raw
                .as_u64()
                .map(|number| number as usize)
                .ok_or_else(|| anyhow!("evaluation_dimensions.{name} 必须是非负整数")),
        }
    };
    let bool_dimension = |name: &str, default: bool| -> Result<bool> {
        match dimensions.and_then(|object| object.get(name)) {
            None => Ok(default),
            Some(raw) => raw
                .as_bool()
                .ok_or_else(|| anyhow!("evaluation_dimensions.{name} 必须是布尔值")),
        }
    };
    let dependency_default = value
        .get("minimal_command_plan")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(1)
        .max(1);

    Ok(EvaluationDimensions {
        task_family: string_dimension("task_family", task_family)?,
        target_resolution: string_dimension("target_resolution", "explicit")?,
        distractor_count: usize_dimension("distractor_count", 0)?,
        dependency_depth: usize_dimension("dependency_depth", dependency_default)?,
        stateful: bool_dimension("stateful", false)?,
        error_injection: bool_dimension("error_injection", false)?,
        output_truncation: bool_dimension("output_truncation", false)?,
    })
}

/// 选择 active 的 fixture LLM case。
pub(crate) fn fixture_llm_cases(cases: &[EvalCase]) -> Vec<EvalCase> {
    cases
        .iter()
        .filter(|case| case.tier == EvalTier::FixtureLlm && case.case_status == "active")
        .cloned()
        .collect()
}

/// 校验 live baseline 使用固定数量的 active fixture LLM case。
pub(crate) fn validate_fixture_llm_case_count(cases: &[EvalCase], expected: usize) -> Result<()> {
    let actual = fixture_llm_cases(cases).len();
    if actual != expected {
        bail!("fixture_llm case count mismatch: expected {expected}, got {actual}");
    }
    Ok(())
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
/// 该解析只处理 adapter 已返回的 assistant content，输入必须是单一 JSON 对象；
/// 任何 malformed、Markdown fence、解释性前缀/后缀都直接返回错误，而不做修复。
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

/// 计划步骤的一种可接受写法。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct PlanVariant {
    command_kind: String,
    target: String,
    #[serde(default)]
    args: Vec<String>,
    requires_project_dir: bool,
    budget: Option<String>,
}

impl PlanVariant {
    /// 与模型命令比较：只比动词和 target。
    ///
    /// budget 和 intent 都被排除在判据之外，因为它们**都不改变结论的对错**：
    ///
    /// - `--budget` 只控制输出大小。M58 里 `page_purpose_actions_test` 有 2 条命令动词对、
    ///   target 对，仅因为写了 `normal` 而不是 `compact` 就被记成路由失败。多花预算是
    ///   浪费，不是走错路，`budget_upgrade` 列已经如实记录了它。
    /// - `--intent` 只收窄 `condition_facts` 的输出。SKILL.md 要求模型按问题选 intent，
    ///   plan 却因为它没写 intent 而拒绝写了 intent 的命令——`--intent display` 回答
    ///   「为什么不显示」恰恰是对的，却贡献了 17 条拒绝。接口不能一边要求、一边惩罚。
    ///
    /// 两者仍逐条记录在 `command_trace` 里，过度用量看得见，只是不再冒充路由缺陷。
    fn matches(&self, request: &CommandRequest, resolver: &TargetResolver) -> bool {
        self.command_kind == request.command_kind
            && targets_match(&self.command_kind, &self.target, &request.target, resolver)
    }
}

/// 用工具自己的归一逻辑判断两个 target 是不是同一个节点。
///
/// M59 评测里 109 条拒绝有 91 条是 `page:actions_test` 这种没写全的 target。工具现在会
/// 确定性地把它补成 `page:app/actions_test.spg`，但 plan 是逐字符比较 target 的，命令在
/// 进 CLI 之前就被判负了——评测于是测的是「有没有原样打出我们写下的那个字符串」，而
/// 不是「工具能不能被走通」。这与此前放宽 budget / intent / `model:` 前缀是同一件事：
/// 接口不能一边把某件事做掉，一边为没做那件事扣分。
///
/// 判据仍然是确定性的、且完全来自被测工具本身：调用 `route::normalize_prefixed_target`，
/// 只有归一到同一个真实节点 id 才算命中。归一不了、或归一到别的节点，一律照旧拒绝。
#[derive(Debug, Clone, Default)]
pub(crate) struct TargetResolver {
    known_ids: Vec<String>,
}

impl TargetResolver {
    /// 从 case 的 graphdb 读出全部节点 id。读不到就退化成纯字符串比较。
    fn from_graph_db(graph_db_path: &Path) -> Self {
        let Ok(runtime) = metadata_checker::runtime::GraphRuntime::load(graph_db_path) else {
            return Self::default();
        };
        Self {
            known_ids: runtime.graph.node_indices.keys().cloned().collect(),
        }
    }

    /// 是否真的读到了图。读不到时所有归一判断都不成立，调用方必须退回纯字符串比较，
    /// 否则「归一不出真实节点」会对每一个 target 都成立。
    fn has_graph(&self) -> bool {
        !self.known_ids.is_empty()
    }

    /// 归一到真实节点 id；归一不了返回 `None`。
    fn canonical(&self, target: &str) -> Option<String> {
        use metadata_checker::route::PrefixedTargetResolution;
        if self.known_ids.is_empty() {
            return None;
        }
        match metadata_checker::route::normalize_prefixed_target(
            target,
            self.known_ids.iter().map(String::as_str),
        ) {
            PrefixedTargetResolution::Exact => Some(target.to_string()),
            PrefixedTargetResolution::Resolved { target, .. } => Some(target),
            PrefixedTargetResolution::Ambiguous { .. }
            | PrefixedTargetResolution::NotFound { .. } => None,
        }
    }
}

/// `--query-dataflow` 的 target 在 CLI 内部会被规范化成 `model:<name>`：裸名和带前缀写法
/// 解析到同一个节点，输出逐字节相同。用精确字符串比较会把「动词选对、实体也选对，只是多写了
/// 一个类型前缀」记成路由失败，于是 `command_rejected` 和 `command_routing_confusion` 里混进
/// 了根本不是接口缺陷的条目——评测本身谎报了工具表面的质量。
///
/// 只对模型/DataFlow 关系查询放宽：它们的 CLI 参数本来就写作 `<MODEL>`，前缀是可选修饰。
/// `comp:` / `action:` / `field:` 前缀是消歧义所必需的，不能一并剥掉。
fn targets_match(
    command_kind: &str,
    plan_target: &str,
    request_target: &str,
    resolver: &TargetResolver,
) -> bool {
    if plan_target == request_target {
        return true;
    }
    if matches!(
        command_kind,
        "--relations" | "--query-dataflow" | "--query-model"
    ) && strip_dataflow_prefix(plan_target) == strip_dataflow_prefix(request_target)
    {
        return true;
    }
    // 两个写法归一到同一个真实节点，就是同一条命令。见 [`TargetResolver`]。
    match (
        resolver.canonical(plan_target),
        resolver.canonical(request_target),
    ) {
        (Some(plan), Some(request)) => plan == request,
        _ => false,
    }
}

fn strip_dataflow_prefix(target: &str) -> &str {
    target.strip_prefix("model:").unwrap_or(target)
}

/// 命令命中 plan 的方式。
///
/// 放宽接受面之后仍然要能区分路由质量，否则「模型选对动词」和「模型选了另一条也能拿到
/// 事实的路」会被压成同一个数字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteKind {
    /// 命中 milestone 文档记录的规范命令。
    Primary,
    /// 命中语义等价的备选命令。
    Alternate,
}

impl RouteKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Alternate => "alternate",
        }
    }
}

/// 评测计划中的单个命令步骤。
///
/// `primary` 是规范命令，`alternatives` 是语义等价的其它路由写法。真实用户提问是模糊的，
/// 且 `.spg` 不进模型上下文，模型只能在没看过任何数据前盲选动词；因此凡是能拿到同一批
/// 事实的命令都应算路由成功，否则评测测的是「有没有猜中我们写下的动词」而不是「工具表面
/// 能不能被走通」。具体走了哪一条仍逐条记录在 `command_trace` 与 `RunReport` 中。
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct PlanStep {
    #[serde(flatten)]
    primary: PlanVariant,
    #[serde(default)]
    alternatives: Vec<PlanVariant>,
}

impl PlanStep {
    /// 返回命中的写法，primary 优先。
    fn match_variant(
        &self,
        request: &CommandRequest,
        resolver: &TargetResolver,
    ) -> Option<(&PlanVariant, RouteKind)> {
        if self.primary.matches(request, resolver) {
            return Some((&self.primary, RouteKind::Primary));
        }
        self.alternatives
            .iter()
            .find(|variant| variant.matches(request, resolver))
            .map(|variant| (variant, RouteKind::Alternate))
    }
}

/// 通过 case plan 固定命令路径和运行目录。
#[derive(Debug, Clone)]
pub(crate) struct CommandPolicy {
    steps: Vec<PlanStep>,
    max_command_count: usize,
    project_dir: PathBuf,
    graph_db_path: PathBuf,
    binary_path: PathBuf,
    /// 用工具自己的归一逻辑比较 target；见 [`TargetResolver`]。
    resolver: TargetResolver,
}

/// 一次已执行命令的身份，用于判断重发是否能拿到新东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutedCommand {
    step_index: usize,
    command_kind: String,
    target: String,
    budget: String,
}

impl ExecutedCommand {
    /// 供测试构造一条已执行记录。
    pub(crate) fn for_test(step_index: usize, request: &CommandRequest) -> Self {
        Self::from_request(step_index, request)
    }

    fn from_request(step_index: usize, request: &CommandRequest) -> Self {
        Self {
            step_index,
            command_kind: request.command_kind.clone(),
            target: request.target.clone(),
            budget: request
                .budget
                .clone()
                .unwrap_or_else(|| "compact".to_string()),
        }
    }

    /// 这次请求相对本条已执行命令，能不能拿到不一样的输出。
    fn yields_something_new(&self, request: &CommandRequest) -> bool {
        if self.command_kind != request.command_kind || self.target != request.target {
            return true;
        }
        let rank = |budget: &str| match budget {
            "compact" => Some(0),
            "normal" => Some(1),
            "full" => Some(2),
            _ => None,
        };
        match (rank(&self.budget), request.budget.as_deref().and_then(rank)) {
            (Some(previous), Some(requested)) => requested > previous,
            _ => false,
        }
    }
}

/// 一次 trial 里允许的免费 `--find` 次数。
///
/// 定位不是回答，不该占用 plan 步数：M58 里 `context_button1_neighbors` 问的是
/// 「button1 周围还有什么」，根本没给页面，模型 8 次选择先 `--find-component button1`
/// 都被记成路由失败——它做对了，只是 plan 没给它做对的余地。给两次，够定位加一次纠正，
/// 又不至于让模型靠反复搜索绕过命令数上限。
const MAX_DISCOVERY_COMMANDS: usize = 2;

/// 经白名单校验后可执行的命令。
#[derive(Debug, Clone)]
pub(crate) struct ValidatedCommand {
    /// 命中的 plan step；`None` 表示这是一次不占步数的定位命令。
    step_index: Option<usize>,
    /// 实际命中的写法；必须执行模型选的那一条，而不是 plan 的规范命令，
    /// 否则报告里的 trace 与真正跑过的 CLI 不是同一个命令。
    variant: PlanVariant,
    route: RouteKind,
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

        let resolver = TargetResolver::from_graph_db(&graph_db_path);
        Ok(Self {
            steps,
            max_command_count,
            project_dir,
            graph_db_path,
            binary_path,
            resolver,
        })
    }

    /// 构造一条不占 plan 步数的定位命令。
    fn locating_command(&self, request: &CommandRequest) -> ValidatedCommand {
        ValidatedCommand {
            step_index: None,
            variant: PlanVariant {
                command_kind: request.command_kind.clone(),
                target: request.target.clone(),
                args: Vec::new(),
                budget: Some("compact".to_string()),
                requires_project_dir: true,
            },
            route: RouteKind::Primary,
            project_dir: self.project_dir.clone(),
            graph_db_path: self.graph_db_path.clone(),
            binary_path: self.binary_path.clone(),
        }
    }

    /// 校验模型命令只能使用尚未消费的 plan step；`--find` 例外，见 [`MAX_DISCOVERY_COMMANDS`]。
    pub(crate) fn validate(
        &self,
        request: &CommandRequest,
        used_steps: &[ExecutedCommand],
        discovery_used: usize,
    ) -> Result<ValidatedCommand> {
        if contains_shell_metacharacters(&request.target) {
            bail!("target 包含禁止的 shell 元字符");
        }

        if request.command_kind == "--find" {
            if discovery_used >= MAX_DISCOVERY_COMMANDS {
                bail!("定位命令数超过 {MAX_DISCOVERY_COMMANDS}");
            }
            return Ok(self.locating_command(request));
        }

        if used_steps.len() >= self.max_command_count {
            bail!("命令数超过 max_command_count={}", self.max_command_count);
        }
        let matched = self
            .steps
            .iter()
            .enumerate()
            .filter(|(index, _)| !used_steps.iter().any(|used| used.step_index == *index))
            .find_map(|(index, step)| {
                step.match_variant(request, &self.resolver)
                    .map(|(variant, route)| (index, variant.clone(), route))
            });

        let (step_index, variant, route) = match matched {
            Some(matched) => matched,
            // 已消费的 step 上换个写法再问一次，只要能拿到不一样的输出，就不是走错路。
            //
            // 拒绝会让整条 trial 立刻判负，所以它必须只留给「工具服务不了」和「去错了
            // 地方」。M59 评测里死在这上面的全是既没走错、工具也答得出来的命令：
            //
            // - `field_lineage_model1_name` 9 次 trial 全部因为按 SKILL.md 的指示把 budget
            //   从 compact 升到 normal。接口不能一边要求升级、一边把升级判成路由失败。
            // - `dataflow_output_source` 7 次因为先 `--relations` 再 `--explain` 同一个节点，
            //   而这两条本来就都写在同一个 step 的可接受写法里。
            //
            // 是否冗余由 max_command_count 兜底，答得对不对由最终答案决定；`command_trace`
            // 仍逐条记录，多花的命令看得见。原样重发同一条命令仍然拒绝——那拿到的是同一
            // 份输出，模型在原地打转。
            None => {
                let retried = self
                    .steps
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        used_steps
                            .iter()
                            .filter(|used| used.step_index == *index)
                            .all(|used| used.yields_something_new(request))
                            && used_steps.iter().any(|used| used.step_index == *index)
                    })
                    .find_map(|(index, step)| {
                        step.match_variant(request, &self.resolver)
                            .map(|(variant, route)| (index, variant.clone(), route))
                    });
                match retried {
                    Some(retried) => retried,
                    // 工具寻址不到的 target 交给工具去说，别替它判负。
                    //
                    // 归一不出真实节点时，CLI 返回的是 AMBIGUOUS_TARGET / TARGET_NOT_FOUND
                    // 加一串真实候选——那正是它该说的话，也正是模型下一轮能用的东西。在
                    // 这里直接拒绝，整条 trial 立刻判负，模型永远看不到候选，评测于是测不到
                    // 「工具能不能帮人把目标找对」这件事，而那恰恰是这套接口的核心承诺。
                    //
                    // 这和 `--find` 免费是同一条理由：只交回候选、不交回答案的命令是定位，
                    // 不是回答。同样吃 MAX_DISCOVERY_COMMANDS 的额度，编不出真实 target 的
                    // 模型仍然会用完额度后失败。
                    None if self.resolver.has_graph()
                        && self.resolver.canonical(&request.target).is_none()
                        && discovery_used < MAX_DISCOVERY_COMMANDS =>
                    {
                        return Ok(self.locating_command(request));
                    }
                    None => {
                        bail!("模型命令不匹配任何未使用的 minimal_command_plan step");
                    }
                }
            }
        };

        // 执行模型写的那个 target，不是 plan 的规范写法。两者现在可能只是归一后相等，
        // 换成 plan 的写法等于替模型把 target 补全了——那条补全路径正是被测对象本身，
        // 而且 trace 里记的命令会和真正跑过的 CLI 不是同一条。
        let mut variant = variant;
        variant.target = request.target.clone();

        Ok(ValidatedCommand {
            step_index: Some(step_index),
            variant,
            route,
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
        if self.variant.requires_project_dir {
            argv.push(OsString::from("--project-dir"));
            argv.push(self.project_dir.as_os_str().to_os_string());
            argv.push(OsString::from("--graph-db-path"));
            argv.push(self.graph_db_path.as_os_str().to_os_string());
            argv.push(OsString::from(&self.variant.command_kind));
            argv.push(OsString::from(&self.variant.target));
        } else {
            argv.push(OsString::from(&self.variant.target));
        }
        argv.extend(self.variant.args.iter().map(OsString::from));
        let budget = self.variant.budget.as_deref().unwrap_or("compact");
        if !budget.is_empty() {
            argv.push(OsString::from("--budget"));
            argv.push(OsString::from(budget));
        }
        argv
    }

    /// 返回 plan step 索引，用于阻止重复执行；定位命令不占步数，返回 `None`。
    pub(crate) fn step_index(&self) -> Option<usize> {
        self.step_index
    }

    /// 返回命中方式，用于在 trace 中保留路由信号。
    pub(crate) fn route(&self) -> RouteKind {
        self.route
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
    /// 是否请求流式响应。
    pub(crate) stream: bool,
    /// 推理力度。CNB 的 swagger body schema 只声明了 messages/model/stream，但该端点
    /// 实测会把 `reasoning_effort` 透传给上游：不带该字段时 `reasoning_content` 恒为空
    /// （多次实测均为 0 字符），带上任意取值就会产出 200~340 字符的推理内容。
    /// 因此默认必须是 None——省略字段时序列化出的请求体与历史基线逐字节一致，
    /// 已有的 run 仍然可比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
}

/// CNB SSE 流中的单个增量块，包含一个 choices 列表。
#[derive(Debug, Deserialize)]
struct SseChatChunk {
    choices: Option<Vec<ChatChoice>>,
    /// 末个 chunk 上的服务端用量。这是权威的 token 计数（客户端按字符数统计只是代理指标），
    /// 空回答时它直接说明预算烧在了哪里。
    #[serde(default)]
    usage: Option<ChatUsage>,
    /// 实际服务本次请求的模型。CNB 对未知模型名会静默回退，必须按这个字段校验。
    #[serde(default)]
    model: Option<String>,
}

/// 服务端返回的用量统计，只取归因空回答需要的两个字段。
#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    completion_tokens: Option<u64>,
    /// 思考消耗的 token。与 ai-audit 的 thinking_tokens 同源，但按请求给出。
    #[serde(default)]
    completion_thinking_tokens: Option<u64>,
}

/// CNB SSE choice 中的增量内容载体。
#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: Option<ChatDelta>,
    /// 上游给出的结束原因。`length` 表示输出预算被耗尽——开了 reasoning 之后
    /// 思考会先把预算吃掉，回答就成了空串。不解析这个字段，「模型想太久被截断」
    /// 和「上游真的什么都没返回」在报告里长得一模一样。
    #[serde(default)]
    finish_reason: Option<String>,
}

/// CNB SSE delta 里的文本片段。
#[derive(Debug, Deserialize)]
struct ChatDelta {
    content: Option<String>,
    /// 模型的思考内容。只用来统计长度，绝不进入 history、报告或日志。
    #[serde(default)]
    reasoning_content: Option<String>,
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
    /// 透传给上游的 reasoning_effort；None 表示完全不发该字段。
    reasoning_effort: Option<String>,
    /// 本 adapter 累计收到的 reasoning_content 字符数。
    ///
    /// 只记长度不记内容：推理原文和模型回答一样不允许进报告。这个计数的用途是证明
    /// `reasoning_effort` 真的生效了——该字段未被 swagger 声明，如果哪天上游默默忽略它，
    /// 报告会像模型名被静默替换那样，标着「开了推理」其实一次都没推理。
    reasoning_chars: usize,
}

/// 构造带指定超时的 HTTP client。
fn build_http_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("创建 CNB AI Chat HTTP client 失败")
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
        let mut adapter = Self::new(endpoint, repo, token, model)?;
        // 缺省不发 reasoning_effort，保持与历史基线逐字节相同的请求体。
        adapter.reasoning_effort = std::env::var("M58_CNB_REASONING_EFFORT")
            .ok()
            .map(|effort| effort.trim().to_string())
            .filter(|effort| !effort.is_empty());
        // 开了推理之后单次响应要多花几倍时间（实测 completion+thinking token 从 6k 涨到 38k），
        // 60s 的默认超时会把「模型想得久」变成 runner_error，读起来像模型答错了。
        // 不开推理时保持 60s 不变，控制组请求路径与历史基线一致。
        if adapter.reasoning_effort.is_some() {
            adapter.client = build_http_client(Duration::from_secs(300))?;
        }
        Ok(adapter)
    }

    /// 覆盖 reasoning_effort，供测试与显式配置使用。
    pub(crate) fn set_reasoning_effort(&mut self, reasoning_effort: Option<String>) {
        self.reasoning_effort = reasoning_effort;
    }

    /// 累计收到的 reasoning_content 字符数（不含内容本身）。
    pub(crate) fn reasoning_chars(&self) -> usize {
        self.reasoning_chars
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
        let client = build_http_client(Duration::from_secs(60))?;
        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            repo: repo.trim_matches('/').to_string(),
            token,
            model,
            reasoning_effort: None,
            reasoning_chars: 0,
        })
    }

    /// 返回完整的 CNB 仓库 AI Chat endpoint。
    fn chat_url(&self) -> String {
        format!("{}/{}/-/ai/chat/completions", self.endpoint, self.repo)
    }
}

impl ModelAdapter for CnbChatAdapter {
    /// 发送流式 CNB AI Chat 请求并拼接 SSE 增量内容。
    fn complete(&mut self, request: &ChatRequest) -> Result<String> {
        let mut payload = request.clone();
        payload.model = self.model.clone();
        payload.stream = true;
        payload.reasoning_effort = self.reasoning_effort.clone();
        let response = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.token)
            .header(ACCEPT, "text/event-stream")
            .json(&payload)
            .send()
            .context("请求 CNB AI Chat 失败")?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        let body = response.text().context("读取 CNB AI Chat 响应失败")?;
        if !status.is_success() {
            let summary = redact_secret(&summarize_body(&body), &self.token);
            bail!("CNB AI Chat HTTP {}: {}", status, summary);
        }
        let media_type = content_type.split(';').next().unwrap_or("").trim();
        if !media_type.eq_ignore_ascii_case("text/event-stream") {
            bail!("CNB AI Chat 响应 Content-Type 必须为 text/event-stream，实际为 {content_type}");
        }

        let parsed = parse_sse_content(&body)?;
        self.reasoning_chars += parsed.reasoning_chars;
        // 有回答但被截断同样是坏结果：答案会缺尾巴（JSON 协议体往往解析不了），
        // 只是不像空回答那样显眼。让它以确定性文案失败，而不是当成正常回答喂给 judge。
        if parsed.finish_reason.as_deref() == Some("length") {
            return Err(anyhow!(ModelAnswerError {
                diagnostic: format!(
                    "CNB AI Chat 回答被输出预算截断：finish_reason=length（thinking {} 字符）",
                    parsed.reasoning_chars
                ),
            }));
        }
        // CNB 对未知模型名不会报错，而是静默用默认模型服务请求（已实测：
        // 请求 `definitely-not-a-real-model-xyz` 同样返回 deepseek-v4-flash）。
        // 不校验的话，把 M58_CNB_MODEL 换成 gemma4-31b 只会得到一份「标着 gemma4、
        // 实际由默认模型回答」的报告——整个跨模型对比会静默失效，而这正是 M58 的目的。
        if let Some(served_model) = parsed.served_model
            && served_model != self.model
        {
            bail!(
                "CNB AI Chat 实际服务模型为 {served_model}，与请求的 {} 不一致；\
                 该端点对未知模型名会静默回退，这份结果不能当作 {} 的基线",
                self.model,
                self.model
            );
        }
        Ok(parsed.content)
    }
}

/// 模型返回了一次结构完整的响应，但里面没有可用的回答文本（空回答或被截断的回答）。
///
/// 与 runner_error 的区别在于责任方：连接、鉴权、SSE 帧、`[DONE]`、模型身份全部正常，
/// 是模型自己把 completion 预算花在思考上、没有产出回答。M58 评的是「模型能否借助工具
/// 答对」，判据只能是它的**回答**；把「只思考不作答」记成我们的 runner_error，等于用
/// 基础设施故障掩盖模型的协议违规，A/B 里 15 个 trial 因此被算在错误的账上。
#[derive(Debug)]
pub(crate) struct ModelAnswerError {
    diagnostic: String,
}

impl std::fmt::Display for ModelAnswerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.diagnostic)
    }
}

impl std::error::Error for ModelAnswerError {}

/// 判断一条 adapter 错误链上是否挂着 [`ModelAnswerError`]。
///
/// 用 `chain()` 而不是 `downcast_ref()`：错误在 adapter 里会被 `context()` 包一层，
/// 只看最外层类型会漏判。
pub(crate) fn is_model_answer_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<ModelAnswerError>())
}

/// 一次 SSE 流的解析结果。
struct SseParse {
    /// 已剥掉思考内容的 assistant 回答。
    content: String,
    /// 实际服务本次请求的模型。
    served_model: Option<String>,
    /// 思考内容的总字符数（两个通道之和），不含思考原文。
    reasoning_chars: usize,
    /// 上游最后给出的非空 finish_reason。
    finish_reason: Option<String>,
}

/// 把 assistant 文本里的 `<think>…</think>` 段落切掉，返回（回答, 被切掉的字符数）。
///
/// CNB 当前的 deepseek 走的是 `reasoning_content` 旁路通道，但 M58 的目的正是换成
/// gemma4 / nemotron 这类便宜模型，而它们普遍把思考内联在 content 里。不剥的话，
/// 思考原文会跟着 assistant content 进 history、进报告、进 judge 的输入——既污染
/// 上下文，也让「答案」里混进模型的自言自语。剥在拼接完成后做，因为 `<think>`
/// 起止标记会被 SSE 切在两个 chunk 里。
fn split_inline_thinking(raw: &str) -> (String, usize) {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut answer = String::with_capacity(raw.len());
    let mut thinking_chars = 0usize;
    let mut rest = raw;

    while let Some(open_at) = rest.find(OPEN) {
        answer.push_str(&rest[..open_at]);
        let after_open = &rest[open_at + OPEN.len()..];
        match after_open.find(CLOSE) {
            Some(close_at) => {
                thinking_chars += after_open[..close_at].chars().count();
                rest = &after_open[close_at + CLOSE.len()..];
            }
            None => {
                // 没有闭合标记 = 思考写到一半被截断，后面不可能还有回答。
                thinking_chars += after_open.chars().count();
                rest = "";
                break;
            }
        }
    }
    answer.push_str(rest);
    (answer, thinking_chars)
}

/// 把预算信息拼成一句短诊断。有服务端 usage 就用它（权威的 token 计数），
/// 没有就退回客户端的字符数代理指标。
fn format_usage(usage: Option<&ChatUsage>, reasoning_chars: usize) -> String {
    match usage {
        Some(usage) => format!(
            "completion_tokens={}, thinking_tokens={}, thinking {reasoning_chars} 字符",
            usage
                .completion_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
            usage
                .completion_thinking_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "<none>".to_string()),
        ),
        None => format!("thinking {reasoning_chars} 字符"),
    }
}

/// 解析 CNB SSE 响应，返回拼接后的文本与实际服务模型。
fn parse_sse_content(body: &str) -> Result<SseParse> {
    let mut content = String::new();
    let mut served_model: Option<String> = None;
    let mut reasoning_chars = 0usize;
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<ChatUsage> = None;
    let mut saw_done = false;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(payload) = line.strip_prefix("data:") else {
            bail!("CNB AI Chat SSE contains unexpected SSE line");
        };
        let payload = payload.trim_start();
        if payload.is_empty() {
            continue;
        }
        if payload == "[DONE]" {
            saw_done = true;
            break;
        }

        let chunk: SseChatChunk =
            serde_json::from_str(payload).context("CNB AI Chat SSE chunk 不是合法 JSON")?;
        if let Some(model) = chunk.model.filter(|model| !model.trim().is_empty()) {
            served_model.get_or_insert(model);
        }
        let carries_usage = chunk.usage.is_some();
        if carries_usage {
            usage = chunk.usage;
        }
        let choices = chunk.choices.unwrap_or_default();
        if choices.is_empty() {
            // 只带 usage、不带 choices 的收尾 chunk 是 OpenAI 兼容流的常见形态
            // （deepseek 把 usage 挂在最后一个 choices chunk 上，但换模型后未必）。
            // 其余情况下的空 choices 仍然说明流坏了。
            if carries_usage {
                continue;
            }
            bail!("CNB AI Chat SSE chunk 缺少 choices");
        }

        for choice in choices {
            if let Some(reason) = choice
                .finish_reason
                .filter(|reason| !reason.trim().is_empty())
            {
                finish_reason = Some(reason);
            }
            let delta = choice
                .delta
                .ok_or_else(|| anyhow!("CNB AI Chat SSE chunk 缺少 delta"))?;
            if let Some(fragment) = delta.content {
                content.push_str(&fragment);
            }
            // 只累加长度：推理原文与模型回答同级敏感，不允许离开这个函数。
            if let Some(reasoning) = delta.reasoning_content {
                reasoning_chars += reasoning.chars().count();
            }
        }
    }

    if !saw_done {
        bail!("CNB AI Chat SSE 响应缺少 [DONE]");
    }

    let (content, inline_thinking_chars) = split_inline_thinking(&content);
    reasoning_chars += inline_thinking_chars;

    if content.trim().is_empty() {
        // 「内容为空」有三种成因，之前它们共用同一句报错，在 CI 日志里无法归因：
        // 预算被思考吃光、模型只思考不回答、上游真的什么都没发。分开报。
        let reason = finish_reason.as_deref().unwrap_or("<none>");
        let budget = format_usage(usage.as_ref(), reasoning_chars);
        // 前两种是模型把预算花在思考上、自己没写回答——责任在模型，判 protocol_error。
        // 第三种（一个 thinking 字符都没有）说明上游确实什么都没发，那才是我们这侧的
        // 传输问题，继续留在 runner_error。
        if reason == "length" {
            return Err(anyhow!(ModelAnswerError {
                diagnostic: format!(
                    "CNB AI Chat 未返回回答内容：finish_reason=length，模型在思考阶段耗尽输出预算（{budget}）"
                ),
            }));
        }
        if reasoning_chars > 0 {
            return Err(anyhow!(ModelAnswerError {
                diagnostic: format!(
                    "CNB AI Chat 只返回了思考内容、没有回答内容（{budget}，finish_reason={reason}）"
                ),
            }));
        }
        bail!("CNB AI Chat SSE 内容为空（finish_reason={reason}）");
    }

    Ok(SseParse {
        content,
        served_model,
        reasoning_chars,
        finish_reason,
    })
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
    /// 命中方式：`primary` 规范命令、`alternate` 等价备选；拒绝的命令为 None。
    #[serde(default)]
    pub(crate) route: Option<String>,
    /// 是否通过 runner 白名单。
    pub(crate) accepted: bool,
    /// 是否请求了 detail/full 等过度读取路径。
    pub(crate) detail_request: bool,
    /// 相对 compact 默认预算的升级次数统计标记。
    pub(crate) budget_upgrade: bool,
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
    /// 同一 case 内的独立 trial 序号，从 0 开始。
    pub(crate) trial_index: usize,
    /// case 的主要任务族。
    pub(crate) task_family: String,
    /// case 的难度标签。
    pub(crate) difficulty: String,
    /// `pass`、`fail` 或 `error`。
    pub(crate) status: String,
    /// 是否通过 AnswerJudge。
    pub(crate) passed: bool,
    /// case 允许的最大命令数。
    pub(crate) max_command_count: usize,
    /// 确定性失败分类。
    pub(crate) failure_classes: Vec<String>,
    /// 不包含模型原文的短说明。
    pub(crate) judge_notes: Vec<String>,
    /// 命令使用轨迹。
    pub(crate) command_trace: Vec<CommandTrace>,
}

/// RunReport 中的命令统计快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommandTraceStats {
    /// 所有 case 的命令尝试数。
    pub(crate) total_commands: usize,
    /// 通过白名单的命令数。
    pub(crate) accepted_commands: usize,
    /// 被协议或 policy 拒绝的命令数。
    pub(crate) rejected_commands: usize,
    /// 至少执行过一个命令的 case 数。
    pub(crate) cases_with_commands: usize,
    /// 每个 case 的平均命令尝试数。
    pub(crate) average_commands_per_case: f64,
    /// 每个 trial 的平均命令尝试数。
    pub(crate) average_commands_per_trial: f64,
    /// 命令轨迹超过 case 上限的 case 数。
    pub(crate) max_command_count_exceeded_cases: usize,
    /// 从 compact 默认预算升级的命令数。
    pub(crate) budget_upgrade_count: usize,
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
    /// 所有 trial 的通过率；与 pass_rate 保持兼容同值。
    pub(crate) trial_pass_rate: f64,
    /// trial 行总数。
    pub(crate) trial_count: usize,
    /// 去重后的 case 总数。
    pub(crate) case_count: usize,
    /// 所有 trial 均通过的 case 比例。
    pub(crate) case_stable_pass_rate: f64,
    /// 同一 case 同时出现通过和失败的数量。
    pub(crate) cases_with_flaky_trials: usize,
    /// 按失败分类聚合的数量。
    pub(crate) failure_classes: BTreeMap<String, usize>,
    /// 命令轨迹统计。
    pub(crate) command_trace_stats: CommandTraceStats,
    /// 被拒绝命令的路由混淆矩阵，键为 `<task_family> -> <模型选择的 command_kind>`。
    ///
    /// 这是**工具接口指标**：命令被拒绝说明模型读完 `SKILL.md` 后选错了动词，
    /// 即 CLI 表面对某类问题不是自解释的。按 task_family 聚合可以直接读出
    /// 「哪类问题会被误路由到哪个动词」，从而定位需要合并或改名的命令，
    /// 而不必把它当成模型能力不足。
    #[serde(default)]
    pub(crate) command_routing_confusion: BTreeMap<String, usize>,
    /// 被接受命令的实际路由分布，键为 `<task_family> -> <命令> (<primary|alternate>)`。
    ///
    /// plan 接受语义等价的备选命令后，「模型选中规范动词」与「模型选了另一条同样能拿到
    /// 事实的路」都会记为 accepted。若只看通过率，这两种情况无法区分，工具表面是否自
    /// 解释的信号就被抹平了。这里保留每一条实际走过的路由，使放宽接受面不以丢失诊断
    /// 能力为代价：`alternate` 占比高说明规范动词不是模型的自然选择。
    #[serde(default)]
    pub(crate) command_route_usage: BTreeMap<String, usize>,
    /// 本次 run 请求的 reasoning_effort；None 表示完全没有发该字段。
    ///
    /// 必须落在报告里：不发该字段时模型的 `reasoning_content` 恒为空，等于整个 run 在零思考
    /// 状态下跑完。不记录的话，「开了推理」和「没开推理」的两份报告长得一模一样，无法比较。
    pub(crate) reasoning_effort: Option<String>,
    /// 本次 run 收到的 reasoning_content 总字符数（不含内容本身）。
    ///
    /// 用于证明 `reasoning_effort` 真的生效：该字段未被 CNB swagger 声明，上游一旦默默忽略它，
    /// 这里会是 0，报告就不会谎称跑的是「带思考」的基线。
    pub(crate) reasoning_chars: usize,
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
        let mut case_results = BTreeMap::<String, (usize, usize)>::new();
        for case in &cases {
            let entry = case_results.entry(case.case_id.clone()).or_default();
            entry.0 += 1;
            if case.passed {
                entry.1 += 1;
            }
        }
        let stable_case_count = case_results
            .values()
            .filter(|(trial_count, passed_count)| *trial_count > 0 && trial_count == passed_count)
            .count();
        let flaky_case_count = case_results
            .values()
            .filter(|(trial_count, passed_count)| *passed_count > 0 && passed_count < trial_count)
            .count();
        let case_count = case_results.len();
        let case_stable_pass_rate = if case_count == 0 {
            0.0
        } else {
            stable_case_count as f64 / case_count as f64
        };
        let mut failure_classes = BTreeMap::new();
        let mut command_routing_confusion = BTreeMap::<String, usize>::new();
        let mut command_route_usage = BTreeMap::<String, usize>::new();
        let mut case_ids_with_commands = BTreeSet::new();
        let mut command_trace_stats = CommandTraceStats {
            total_commands: 0,
            accepted_commands: 0,
            rejected_commands: 0,
            cases_with_commands: 0,
            average_commands_per_case: 0.0,
            average_commands_per_trial: 0.0,
            max_command_count_exceeded_cases: 0,
            budget_upgrade_count: 0,
        };
        for case in &cases {
            for failure_class in &case.failure_classes {
                *failure_classes.entry(failure_class.clone()).or_insert(0) += 1;
            }
            if !case.command_trace.is_empty() {
                case_ids_with_commands.insert(case.case_id.clone());
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
            command_trace_stats.budget_upgrade_count += case
                .command_trace
                .iter()
                .filter(|trace| trace.budget_upgrade)
                .count();
            if case.command_trace.len() > case.max_command_count {
                command_trace_stats.max_command_count_exceeded_cases += 1;
            }
            for trace in case.command_trace.iter().filter(|trace| !trace.accepted) {
                // command_kind 来自模型，长度不受控；截断避免单条异常输出撑大报告键空间。
                // 该值已原样记录在 command_trace 中，这里不引入新的信息泄露。
                let chosen: String = trace.command_kind.chars().take(64).collect();
                *command_routing_confusion
                    .entry(format!("{} -> {}", case.task_family, chosen))
                    .or_insert(0) += 1;
            }
            for trace in case.command_trace.iter().filter(|trace| trace.accepted) {
                // 与上面同样的截断理由：command_kind 来自模型，长度不受控。
                let chosen: String = trace.command_kind.chars().take(64).collect();
                let route = trace.route.as_deref().unwrap_or("primary");
                *command_route_usage
                    .entry(format!("{} -> {} ({})", case.task_family, chosen, route))
                    .or_insert(0) += 1;
            }
        }
        command_trace_stats.cases_with_commands = case_ids_with_commands.len();
        command_trace_stats.average_commands_per_case = if case_count == 0 {
            0.0
        } else {
            command_trace_stats.total_commands as f64 / case_count as f64
        };
        command_trace_stats.average_commands_per_trial = if cases.is_empty() {
            0.0
        } else {
            command_trace_stats.total_commands as f64 / cases.len() as f64
        };
        Self {
            schema_version: "1.4.0".to_string(),
            provider,
            model_id,
            cnb_build_id,
            started_at,
            cases,
            pass_rate,
            trial_pass_rate: pass_rate,
            trial_count: case_results.values().map(|(count, _)| count).sum(),
            case_count,
            case_stable_pass_rate,
            cases_with_flaky_trials: flaky_case_count,
            failure_classes,
            command_trace_stats,
            command_routing_confusion,
            command_route_usage,
            reasoning_effort: None,
            reasoning_chars: 0,
        }
    }

    /// 记录本次 run 实际请求的推理力度与收到的推理字符数。
    pub(crate) fn set_reasoning(
        &mut self,
        reasoning_effort: Option<String>,
        reasoning_chars: usize,
    ) {
        self.reasoning_effort = reasoning_effort;
        self.reasoning_chars = reasoning_chars;
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

    // must_include 的每一项是一个同义组：命中任意一个表述即视为覆盖该事实。
    // 小模型回答简短，不应因为没有复述内部术语而判失败。
    let must_include =
        read_alternative_assertions(assertions, "must_include", &mut failure_classes);
    for alternatives in must_include {
        let hit = alternatives
            .iter()
            .any(|expected| normalized_answer.contains(&normalize_for_match(expected)));
        if !hit {
            add_failure(&mut failure_classes, "missed_fact");
            judge_notes.push(format!(
                "missing must_include assertion: {}",
                alternatives.join(" | ")
            ));
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
        && diagnostics.iter().any(|diagnostic| {
            !diagnostic.trim().is_empty() && diagnostic_demands_hedging(diagnostic)
        })
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
    // 证据要求应当检查回答是否真的建立在已执行的 CLI 命令之上，而不是要求
    // 回答里出现 summary/details 这类内部 section 英文名——后者只会训练模型
    // 复述固定词，与证据强度无关。
    if evidence_required {
        let grounded = trace.iter().any(|entry| entry.accepted);
        if !grounded {
            add_failure(&mut failure_classes, "ungrounded_answer");
            judge_notes.push("要求证据但没有任何被接受的 CLI 命令可作为回答依据".to_string());
        }
    }

    // 原实现把三种完全不同的情况压成同一个 wrong_command，导致无法判断
    // 「模型没选命令」「命令被 plan 拒绝」是不是同一个问题。拆开分类，
    // 便于区分真实路由错误与仅仅偏离手写 plan 的合理命令。
    if trace.is_empty() {
        add_failure(&mut failure_classes, "no_command");
        judge_notes.push("模型没有执行任何 CLI 命令".to_string());
    } else {
        let rejected = trace.iter().filter(|entry| !entry.accepted).count();
        if rejected > 0 {
            add_failure(&mut failure_classes, "command_rejected");
            judge_notes.push(format!("{rejected} 条命令未通过 minimal_command_plan 校验"));
        }
        if trace
            .iter()
            .any(|entry| entry.accepted && entry.plan_step_index.is_none())
        {
            add_failure(&mut failure_classes, "wrong_command");
            judge_notes.push("命令已执行但没有对应的 plan step".to_string());
        }
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
        "- provider: `{}`\n- model: `{}`\n- cnb_build_id: `{}`\n- started_at: `{}`\n- pass_rate: `{:.4}`\n- trial_pass_rate: `{:.4}`\n- trial_count: `{}`\n- case_count: `{}`\n- case_stable_pass_rate: `{:.4}`\n- cases_with_flaky_trials: `{}`\n- average_commands_per_case: `{:.4}`\n- average_commands_per_trial: `{:.4}`\n- budget_upgrade_count: `{}`\n- reasoning_effort: `{}`\n- reasoning_chars: `{}`\n\n",
        escape_markdown_cell(&report.provider),
        escape_markdown_cell(&report.model_id),
        escape_markdown_cell(report.cnb_build_id.as_deref().unwrap_or("")),
        escape_markdown_cell(&report.started_at),
        report.pass_rate,
        report.trial_pass_rate,
        report.trial_count,
        report.case_count,
        report.case_stable_pass_rate,
        report.cases_with_flaky_trials,
        report.command_trace_stats.average_commands_per_case,
        report.command_trace_stats.average_commands_per_trial,
        report.command_trace_stats.budget_upgrade_count,
        escape_markdown_cell(report.reasoning_effort.as_deref().unwrap_or("none")),
        report.reasoning_chars,
    ));
    markdown.push_str(
        "| case_id | trial_index | task_family | difficulty | status | failure_classes | command_count |\n",
    );
    markdown.push_str("| --- | ---: | --- | --- | --- | --- | ---: |\n");
    for case in &report.cases {
        let failures = if case.failure_classes.is_empty() {
            "-".to_string()
        } else {
            case.failure_classes.join(", ")
        };
        markdown.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            escape_markdown_cell(&case.case_id),
            case.trial_index,
            escape_markdown_cell(&case.task_family),
            escape_markdown_cell(&case.difficulty),
            escape_markdown_cell(&case.status),
            escape_markdown_cell(&failures),
            case.command_trace.len(),
        ));
    }
    markdown.push_str("\n## Failure classes\n\n");
    for (failure_class, count) in &report.failure_classes {
        markdown.push_str(&format!("- `{failure_class}`: {count}\n"));
    }
    markdown.push_str("\n## Command routing confusion\n\n");
    if report.command_routing_confusion.is_empty() {
        markdown.push_str("- 无被拒绝命令\n");
    } else {
        markdown.push_str("被拒绝的命令按 `<task_family> -> <模型选择的命令>` 聚合，用于定位需要合并或改名的 CLI 动词。\n\n");
        for (route, count) in &report.command_routing_confusion {
            markdown.push_str(&format!("- `{}`: {count}\n", escape_markdown_cell(route)));
        }
    }

    markdown.push_str("\n## Command route usage\n\n");
    if report.command_route_usage.is_empty() {
        markdown.push_str("- 无被接受命令\n");
    } else {
        markdown.push_str("被接受的命令按 `<task_family> -> <命令> (primary|alternate)` 聚合。`alternate` 表示模型走的是等价备选路由而非规范动词，占比越高说明规范动词越不自解释。\n\n");
        for (route, count) in &report.command_route_usage {
            markdown.push_str(&format!("- `{}`: {count}\n", escape_markdown_cell(route)));
        }
    }

    // judge_notes 是不含模型原文的确定性诊断，是「为什么判 missed_fact」的唯一线索；
    // 只列失败 case，通过的 case 没有可复盘的东西。
    markdown.push_str("\n## Judge notes (failing cases)\n\n");
    let mut has_notes = false;
    for case in &report.cases {
        if case.passed || case.judge_notes.is_empty() {
            continue;
        }
        has_notes = true;
        markdown.push_str(&format!(
            "- `{}` trial {}: {}\n",
            escape_markdown_cell(&case.case_id),
            case.trial_index,
            case.judge_notes
                .iter()
                .map(|note| escape_markdown_cell(note))
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    if !has_notes {
        markdown.push_str("- 无\n");
    }

    // 逐条命令轨迹必须落在报告里，不能靠 CI 日志 grep 反推：grep 只能匹配键名所在行，
    // 拿不到对象体，被截断的轨迹会让「模型编了个不存在的路径」这类失败无法复盘。
    // 这些字段都是被 plan 校验过的命令入参，不含模型原文。
    markdown.push_str("\n## Command trace\n\n");
    if report
        .cases
        .iter()
        .all(|case| case.command_trace.is_empty())
    {
        markdown.push_str("- 无命令\n");
    } else {
        markdown.push_str(
            "| case_id | trial_index | command_kind | target | args | budget | plan_step_index | route | accepted | detail_request | budget_upgrade | output_sections |\n",
        );
        markdown.push_str(
            "| --- | ---: | --- | --- | --- | --- | ---: | --- | --- | --- | --- | --- |\n",
        );
        for case in &report.cases {
            for trace in &case.command_trace {
                markdown.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                    escape_markdown_cell(&case.case_id),
                    case.trial_index,
                    escape_markdown_cell(&trace.command_kind),
                    escape_markdown_cell(&trace.target),
                    escape_markdown_cell(&trace.args.join(" ")),
                    escape_markdown_cell(trace.budget.as_deref().unwrap_or("-")),
                    trace
                        .plan_step_index
                        .map(|index| index.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    escape_markdown_cell(trace.route.as_deref().unwrap_or("-")),
                    trace.accepted,
                    trace.detail_request,
                    trace.budget_upgrade,
                    escape_markdown_cell(&trace.output_sections.join(" ")),
                ));
            }
        }
    }
    markdown
}

/// 读取「同义组」断言：每一项可以是单个字符串，也可以是一组等价表述。
/// 字符串等价于只有一个元素的同义组，保持既有 case 文件向后兼容。
/// schema 异常只进入 needs_human_review，不 panic。
fn read_alternative_assertions(
    assertions: &Value,
    field: &str,
    failure_classes: &mut Vec<String>,
) -> Vec<Vec<String>> {
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
        match item {
            Value::String(single) if !single.trim().is_empty() => {
                result.push(vec![single.to_string()]);
            }
            Value::Array(group) => {
                let alternatives: Vec<String> = group
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .map(str::to_string)
                    .collect();
                if alternatives.is_empty() {
                    add_failure(failure_classes, "needs_human_review");
                } else {
                    result.push(alternatives);
                }
            }
            _ => add_failure(failure_classes, "needs_human_review"),
        }
    }
    result
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
/// 这条诊断是否要求答案降级表达。
///
/// RESOLVED_TARGET 是「你写的 target 我替你补全了」——它讲的是命令怎么送到节点上的，
/// 与答案有多确定无关。readonly_page_check 里模型写 `page:dataflow_embedded`、工具补成
/// 全路径，结果这条路由说明让整个答案被要求降级，9/9 全灭。
///
/// 「没有写入目标」「没有用户入口」是判定结果，不是证据缺失：readonly_page_check 的
/// uncertainty_policy 自己就写着「页面确实无任何 action 时可明确回答只读」，而它的
/// must_include 要的正是「只读」。对这类诊断也要求模型说「可能/不确定」，等于一边要
/// 它下确定结论、一边罚它下确定结论。用工具自己的影响分类来区分，未登记的 code 一律
/// 按需要降级处理。
pub(crate) fn diagnostic_demands_hedging(diagnostic: &str) -> bool {
    use metadata_checker::output::answer_effect::{AnswerImpact, answer_effect};
    let code = diagnostic.split(':').next().unwrap_or_default().trim();
    !matches!(
        answer_effect(code),
        Some((AnswerImpact::Determinate | AnswerImpact::Routing, _))
    )
}

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

/// M58 runner 的固定运行配置；模型不能覆盖其中任何路径。
#[derive(Debug, Clone)]
pub(crate) struct RunnerConfig {
    /// metadata-checker release/debug binary 路径。
    pub(crate) binary_path: PathBuf,
    /// 提供给模型的仓库 SKILL.md 路径。
    pub(crate) skill_path: PathBuf,
    /// fixture 或真实项目源目录。
    pub(crate) project_root: PathBuf,
    /// 本次 run 的输出目录和 case 隔离目录。
    pub(crate) output_dir: PathBuf,
    /// 报告中的 provider 名称。
    pub(crate) provider: String,
    /// CNB 或 fake 模型标识。
    pub(crate) model_id: String,
    /// 当前 CNB build 标识；本地 run 可以为空。
    pub(crate) cnb_build_id: Option<String>,
    /// 每个 fixture LLM case 执行的独立 trial 数。
    pub(crate) trial_count: usize,
}

/// 运行一个 fixture LLM case，返回不含模型原文的 case 报告。
pub(crate) fn run_case(
    case: &EvalCase,
    adapter: &mut dyn ModelAdapter,
    config: &RunnerConfig,
) -> Result<CaseReport> {
    run_case_for_trial(case, adapter, config, 0)
}

/// 运行一个指定 trial 的 fixture LLM case。
fn run_case_for_trial(
    case: &EvalCase,
    adapter: &mut dyn ModelAdapter,
    config: &RunnerConfig,
    trial_index: usize,
) -> Result<CaseReport> {
    let case_dir = prepare_case_workspace(case, config, trial_index)?;
    let project_dir = case_dir.join("project");
    let graph_db_path = case_dir.join("case.graphdb");
    let policy =
        CommandPolicy::from_case(case, project_dir, graph_db_path, config.binary_path.clone())?;
    let skill = std::fs::read_to_string(&config.skill_path)
        .with_context(|| format!("读取 SKILL.md 失败: {}", config.skill_path.display()))?;
    let mut history = vec![ChatMessage {
        role: "user".to_string(),
        content: build_bootstrap_message(&skill, case),
    }];
    let mut used_steps = Vec::new();
    let mut discovery_used = 0usize;
    let mut diagnostics = Vec::new();
    let mut trace = Vec::new();

    for _turn_index in 0..=policy.max_command_count {
        let request = ChatRequest {
            messages: history.clone(),
            model: config.model_id.clone(),
            stream: false,
            reasoning_effort: None,
        };
        // 只有 adapter 成功返回 assistant content 时，runner 才进入协议解析路径。
        // adapter/HTTP/SSE 层失败说明 provider/runtime 未产出可解析文本，归为 runner_error；
        // 唯一例外是 ModelAnswerError（流完好但模型没给回答），见下方分类。
        let response = match adapter.complete(&request) {
            Ok(response) => response,
            Err(error) => {
                // 丢掉 error 会让 runner_error 变成一句无法复盘的「失败了」——reasoning_effort
                // 的 A/B 里正是这类空诊断把 7 个 trial 变成不可解释的噪声。adapter 层的错误
                // 全部是我们自己构造的确定性文本（HTTP 状态、SSE 解析、模型替换断言），
                // 不含模型回答，可以安全落进 judge_notes；仍然截断以免异常长的响应体灌进报告。
                //
                // 「流是好的、但模型没给出回答」不是基础设施故障，判 protocol_error：
                // M58 只按模型的回答记分，思考不算交付。
                let failure_class = if is_model_answer_error(&error) {
                    "protocol_error"
                } else {
                    "runner_error"
                };
                return Ok(error_case_report(
                    case,
                    trial_index,
                    policy.max_command_count,
                    failure_class,
                    &format!(
                        "{}: {}",
                        if failure_class == "protocol_error" {
                            "model returned no usable answer"
                        } else {
                            "model adapter completion failed"
                        },
                        truncate_diagnostic(&format!("{error:#}"))
                    ),
                    trace,
                ));
            }
        };
        history.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });
        // 这里解析的是已拿到的 assistant content，故 malformed/fence 均判为模型协议错误。
        let turn = match parse_agent_turn(&response) {
            Ok(turn) => turn,
            Err(error) => {
                return Ok(error_case_report(
                    case,
                    trial_index,
                    policy.max_command_count,
                    "protocol_error",
                    &format_protocol_error_note(&error),
                    trace,
                ));
            }
        };
        match turn {
            AgentTurn::Final(answer) => {
                let assertions = case.value.get("answer_assertions").unwrap_or(&Value::Null);
                let judge = judge_answer(&answer, assertions, &diagnostics, &trace);
                return Ok(CaseReport {
                    case_id: case.case_id.clone(),
                    trial_index,
                    task_family: case.task_family.clone(),
                    difficulty: case.difficulty.clone(),
                    status: if judge.passed {
                        "pass".to_string()
                    } else {
                        "fail".to_string()
                    },
                    passed: judge.passed,
                    max_command_count: policy.max_command_count,
                    failure_classes: judge.failure_classes,
                    judge_notes: judge.judge_notes,
                    command_trace: trace,
                });
            }
            AgentTurn::Command(command_request) => {
                let validated = match policy.validate(&command_request, &used_steps, discovery_used)
                {
                    Ok(validated) => validated,
                    Err(_) => {
                        trace.push(rejected_command_trace(&command_request));
                        return Ok(error_case_report(
                            case,
                            trial_index,
                            policy.max_command_count,
                            "command_rejected",
                            "model command was rejected by minimal_command_plan",
                            trace,
                        ));
                    }
                };
                let step_index = validated.step_index();
                let route = validated.route();
                let detail_request = is_detail_request(&command_request);
                let execution = match execute_validated_command(&validated, case) {
                    Ok(execution) => execution,
                    Err(_) => {
                        trace.push(accepted_command_trace(
                            &command_request,
                            step_index,
                            route,
                            detail_request,
                            Vec::new(),
                        ));
                        return Ok(error_case_report(
                            case,
                            trial_index,
                            policy.max_command_count,
                            "runner_error",
                            "metadata-checker command execution failed",
                            trace,
                        ));
                    }
                };
                diagnostics.extend(execution.diagnostics);
                trace.push(accepted_command_trace(
                    &command_request,
                    step_index,
                    route,
                    detail_request,
                    execution.output_sections,
                ));
                match step_index {
                    Some(index) => {
                        used_steps.push(ExecutedCommand::from_request(index, &command_request))
                    }
                    // 定位命令不占 plan 步数，只吃自己的定位额度。
                    None => discovery_used += 1,
                }
                history.push(ChatMessage {
                    role: "user".to_string(),
                    content: execution.filtered_output,
                });
            }
        }
    }

    Ok(error_case_report(
        case,
        trial_index,
        policy.max_command_count,
        "protocol_error",
        "model did not return final before runner turn limit",
        trace,
    ))
}

/// 串行运行 active fixture_llm cases并计算 RunReport。
pub(crate) fn run_fixture_llm_cases(
    cases: &[EvalCase],
    adapter: &mut dyn ModelAdapter,
    config: &RunnerConfig,
) -> Result<RunReport> {
    if config.trial_count == 0 {
        bail!("trial_count 必须大于 0");
    }
    std::fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("创建 M58 输出目录失败: {}", config.output_dir.display()))?;
    let mut reports = Vec::new();
    for case in cases
        .iter()
        .filter(|case| case.tier == EvalTier::FixtureLlm && case.case_status == "active")
    {
        for trial_index in 0..config.trial_count {
            let report = match run_case_for_trial(case, adapter, config, trial_index) {
                Ok(report) => report,
                Err(_) => error_case_report(
                    case,
                    trial_index,
                    case_max_command_count(case),
                    "runner_error",
                    "case workspace setup failed",
                    Vec::new(),
                ),
            };
            reports.push(report);
        }
    }
    Ok(RunReport::from_cases(
        config.provider.clone(),
        config.model_id.clone(),
        config.cnb_build_id.clone(),
        current_run_timestamp(),
        reports,
    ))
}

/// 构造第一条 user bootstrap，不注入源码、历史记录或知识库内容。
pub(crate) fn build_bootstrap_message(skill: &str, case: &EvalCase) -> String {
    format!(
        "M58 empty-context evaluation.\\n\
Use only the following SKILL.md, fixed CLI outputs, and the current case question.\\n\
Do not read source code, history, hidden knowledge bases, environment variables, prior runs, or case metadata.\\n\
Do not include hidden context.\\n\
Do not include answer keys.\\n\
Do not include expected_facts.\\n\
Do not include must_include.\\n\
Do not include minimal plans.\\n\
Do not include CNB_TOKEN or other secrets.\\n\
Do not include actual case target or fixture answers.\\n\
Rules:\\n\
1. Return exactly one raw JSON object per turn. No Markdown fences. No prefix or suffix. 每轮只输出一行 JSON；final 对象只能有 kind 和 answer 两个键，answer 必须是简短字符串，不能添加 sources、evidence 或其他键。\\n\
2. 只有三个查询动词，按问题选一个：\\n\
   - `--find`：不知道目标叫什么全名/在哪个文件时用。\\n\
   - `--explain`：问某一个节点是什么、做什么、为什么这样表现（显示、可用、值来源、写入、动作门禁都算）。\\n\
   - `--relations`：问某个页面或模型和别人的关系（页面整体逻辑、谁读写这张表、DataFlow 链路）。\\n\
3. 动词只决定问什么，target 前缀决定去哪。同一个组件写成 comp: 还是 action: 返回同一组事实块，不必纠结；--explain 一次就同时给出语义和条件成因，不需要分两条命令。\\n\
4. target 只写你确实知道的部分：文件路径可以只写文件名、可以省掉扩展名、也可以整个省掉（page:首页、comp:首页.spg|button1、comp:button1 都能被定位）。工具会补全并在 diagnostics 里留 RESOLVED_TARGET；有歧义会返回 candidate_targets，从里面挑一个重发。绝对不要自己拼造文件路径——写不出来的那一段就别写，也不要把说明文字里的占位符当成真实路径。\\n\
5. budget 字段仅在命令需要时设置，合法值为 compact / normal / full（compact 作为默认第一轮；仅在 diagnostics 或 OUTPUT_TRUNCATED 时升级）。budget 只能放在 JSON 顶层字段，不能放进 args。\\n\
6. 命令 JSON 形状：{{\"kind\":\"command\",\"command_kind\":\"--explain\",\"target\":\"<target>\",\"args\":[],\"budget\":\"compact\"}}。\\n\
7. This is only a shape example, not the current case answer/target/plan. Choose actual command_kind/target/args/budget from SKILL.md and the question.\\n\
8. After each command, use the next user message as the only evidence. Read summary first, then read the declared primary fact block and only read more if needed.\\n\
9. 主证据块为空或 result=null 时，不要直接作答；如果仍有查询机会，换一个更合适的 target 重试；否则明确说明证据不足。\\n\
10. final answer 必须包含至少一个 literal section name（summary/details/evidence/diagnostics）；页面整体回答至少说明入口/写入计数和一个 action；组件或动作类问题的回答至少说明组件、action 和写入目标；字段回答至少说明字段和写入者或来源。证据已足够时立即返回 final，不要再补命令。\\n\
11. As soon as you have enough evidence, return {{\"kind\":\"final\",\"answer\":\"...\"}}.\\n\
12. If the evidence contains diagnostics, truncation, or uncertainty, mention that explicitly in the final answer and do not guess.\\n\
\\n\
SKILL.md:\\n\
{skill}\\n\
\\n\
M58 runner answer-shape override (apply after SKILL.md; 只约束最终回答的写法，不改变上面的选命令规则):\\n\\
- 页面整体最终回答必须写出用户入口与写入目标的实际情况（有就说有多少、是什么；没有就说没有），并引用 literal section name。\\n\\
- summary.conclusion 存在时，final answer 必须逐字照抄它给出的结论表述，不要换成同义词。\\n\\
- summary.absent 非空时，把其中每条 statement 的说法逐条写进 final answer。\\n\\
- summary.confidence.level 不是 full 时，必须按 summary.confidence.statement 与各 reason 的 effect 保守表达。\\n\\
field final 必须 literal 包含 页面 action、写入、字段。\\n\\
\\n\\
Case Question: {}",
        case.question
    )
}

/// 为单个 case 创建项目副本、graphdb 和输出隔离目录。
fn prepare_case_workspace(
    case: &EvalCase,
    config: &RunnerConfig,
    trial_index: usize,
) -> Result<PathBuf> {
    if !config.project_root.is_dir() {
        bail!(
            "fixture project directory 不存在: {}",
            config.project_root.display()
        );
    }
    std::fs::create_dir_all(&config.output_dir)
        .with_context(|| format!("创建输出目录失败: {}", config.output_dir.display()))?;
    let case_dir = config
        .output_dir
        .join("cases")
        .join(safe_case_id(&case.case_id))
        .join(format!("trial-{trial_index}"));
    if case_dir.exists() {
        std::fs::remove_dir_all(&case_dir)
            .with_context(|| format!("清理旧 case workspace 失败: {}", case_dir.display()))?;
    }
    let project_dir = case_dir.join("project");
    copy_directory(&config.project_root, &project_dir)?;
    let graph_db_path = case_dir.join("case.graphdb");
    metadata_checker::scanner::scan_project(&project_dir, &graph_db_path)
        .with_context(|| format!("构建 case graphdb 失败: {}", case.case_id))?;
    Ok(case_dir)
}

/// 递归复制 fixture 项目，避免 runner 修改源 fixture 或共享 graphdb。
fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("创建项目副本目录失败: {}", destination.display()))?;
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("读取项目目录失败: {}", source.display()))?
    {
        let entry = entry.context("读取项目目录项失败")?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().context("读取项目目录项类型失败")?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            std::fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "复制项目文件失败: {} -> {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        } else {
            bail!("项目包含不支持的目录项: {}", source_path.display());
        }
    }
    Ok(())
}

/// 将 case id 转成不含路径分隔符的 workspace 名称。
fn safe_case_id(case_id: &str) -> String {
    case_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// 执行已通过 policy 的命令，并只返回允许 section 的 JSON。
fn execute_validated_command(
    command: &ValidatedCommand,
    case: &EvalCase,
) -> Result<CommandExecution> {
    let output = Command::new(command.binary_path())
        .args(command.argv())
        .output()
        .context("启动 metadata-checker CLI 失败")?;
    if !output.status.success() {
        bail!("metadata-checker CLI 返回失败状态");
    }
    let stdout = String::from_utf8(output.stdout).context("metadata-checker stdout 不是 UTF-8")?;
    let stdout = stdout.trim();
    if stdout.is_empty() {
        bail!("metadata-checker stdout 为空");
    }
    let value: Value = serde_json::from_str(stdout).context("metadata-checker stdout 不是 JSON")?;
    let diagnostics = extract_diagnostics(&value);
    let allowed_sections = allowed_output_sections(case)?;
    let (filtered_output, output_sections) = filter_cli_output(&value, &allowed_sections)?;
    Ok(CommandExecution {
        filtered_output,
        diagnostics,
        output_sections,
    })
}

/// CLI 命令执行后的脱敏中间结果。
struct CommandExecution {
    filtered_output: String,
    diagnostics: Vec<String>,
    output_sections: Vec<String>,
}

/// 从 case 中读取允许回传给模型的顶层 section。
fn allowed_output_sections(case: &EvalCase) -> Result<Vec<String>> {
    let Some(values) = case
        .value
        .get("allowed_output_sections")
        .and_then(Value::as_array)
    else {
        return Ok(vec!["summary".to_string()]);
    };
    let sections = values
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if sections.is_empty() {
        bail!("case {} 的 allowed_output_sections 不能为空", case.case_id);
    }
    Ok(sections)
}

/// 只投影 schema 元信息和 case 明确允许的 section，禁止把整个 raw JSON 回传。
fn filter_cli_output(value: &Value, allowed_sections: &[String]) -> Result<(String, Vec<String>)> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("metadata-checker 输出顶层不是 object"))?;
    let mut filtered = serde_json::Map::new();
    for key in ["schema_version", "kind", "query_target"] {
        if let Some(value) = object.get(key) {
            filtered.insert(key.to_string(), value.clone());
        }
    }
    let mut output_sections = Vec::new();
    for section in allowed_sections {
        if !matches!(
            section.as_str(),
            "summary" | "details" | "evidence" | "diagnostics" | "next_queries"
        ) {
            continue;
        }
        if let Some(value) = object.get(section) {
            filtered.insert(section.clone(), value.clone());
            output_sections.push(section.clone());
        }
    }
    if let Some(value) = object.get("diagnostics") {
        if !filtered.contains_key("diagnostics") {
            filtered.insert("diagnostics".to_string(), value.clone());
            output_sections.push("diagnostics".to_string());
        }
    }
    Ok((
        serde_json::to_string(&Value::Object(filtered))?,
        output_sections,
    ))
}

/// 将 CLI diagnostics 压缩成短字符串，避免把完整错误对象转给 judge/report。
fn extract_diagnostics(value: &Value) -> Vec<String> {
    value
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|diagnostics| {
            diagnostics
                .iter()
                .take(32)
                .filter_map(|diagnostic| {
                    if let Some(text) = diagnostic.as_str() {
                        return Some(bound_text(text));
                    }
                    let code = diagnostic.get("code").and_then(Value::as_str);
                    let message = diagnostic.get("message").and_then(Value::as_str);
                    match (code, message) {
                        (Some(code), Some(message)) => {
                            Some(bound_text(&format!("{code}: {message}")))
                        }
                        (Some(code), None) => Some(code.to_string()),
                        (None, Some(message)) => Some(bound_text(message)),
                        (None, None) => None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 判断命令是否显式要求 detail/full 输出。
fn is_detail_request(request: &CommandRequest) -> bool {
    request.budget.as_deref() == Some("full")
        || request.args.iter().any(|arg| arg == "--detail")
        || request
            .args
            .windows(2)
            .any(|window| window == ["--budget", "full"])
}

/// 将 command request 记录为被拒绝的轨迹。
fn rejected_command_trace(request: &CommandRequest) -> CommandTrace {
    CommandTrace {
        command_kind: request.command_kind.clone(),
        target: request.target.clone(),
        args: request.args.clone(),
        budget: request.budget.clone(),
        plan_step_index: None,
        route: None,
        accepted: false,
        detail_request: is_detail_request(request),
        budget_upgrade: is_budget_upgrade(request),
        output_sections: Vec::new(),
    }
}

/// 将已执行命令记录为通过 policy 的轨迹。
fn accepted_command_trace(
    request: &CommandRequest,
    step_index: Option<usize>,
    route: RouteKind,
    detail_request: bool,
    output_sections: Vec<String>,
) -> CommandTrace {
    CommandTrace {
        command_kind: request.command_kind.clone(),
        target: request.target.clone(),
        args: request.args.clone(),
        budget: request.budget.clone(),
        plan_step_index: step_index,
        route: Some(route.as_str().to_string()),
        accepted: true,
        detail_request,
        budget_upgrade: is_budget_upgrade(request),
        output_sections,
    }
}

/// 构造命令/adapter 失败的 case 报告，不复制原始输出或模型回答。
/// judge_notes 是给人在 CI 日志里读的短诊断，不是日志转储。
/// 按字符（不是字节）截断，避免把多字节错误信息切成非法 UTF-8。
fn truncate_diagnostic(message: &str) -> String {
    const MAX_CHARS: usize = 200;
    let mut normalized: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() > MAX_CHARS {
        normalized = normalized.chars().take(MAX_CHARS).collect::<String>() + "…";
    }
    normalized
}

fn error_case_report(
    case: &EvalCase,
    trial_index: usize,
    max_command_count: usize,
    failure_class: &str,
    note: &str,
    command_trace: Vec<CommandTrace>,
) -> CaseReport {
    CaseReport {
        case_id: case.case_id.clone(),
        trial_index,
        task_family: case.task_family.clone(),
        difficulty: case.difficulty.clone(),
        status: "error".to_string(),
        passed: false,
        max_command_count,
        failure_classes: vec![failure_class.to_string()],
        judge_notes: vec![note.to_string()],
        command_trace,
    }
}

/// 生成不含模型原文的协议错误摘要，便于 CI 判断是 JSON 语法还是封装字段失败。
fn format_protocol_error_note(error: &anyhow::Error) -> String {
    let summary = error.to_string().replace(['\r', '\n'], " ");
    format!("model response violated command/final JSON protocol: {summary}")
}

/// 判断命令预算是否相对 compact 默认值发生升级。
fn is_budget_upgrade(request: &CommandRequest) -> bool {
    request
        .budget
        .as_deref()
        .is_some_and(|budget| budget == "normal" || budget == "full")
        || request
            .args
            .windows(2)
            .any(|window| window[0] == "--budget" && (window[1] == "normal" || window[1] == "full"))
}

/// 从 case schema 读取最大命令数，供 workspace 初始化失败记录使用。
fn case_max_command_count(case: &EvalCase) -> usize {
    case.value
        .get("max_command_count")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

/// 限制诊断摘要长度。
fn bound_text(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let text: String = compact.chars().take(512).collect();
    if compact.chars().count() > 512 {
        format!("{text}…")
    } else {
        text
    }
}

/// 生成不依赖额外时间库的稳定运行时间标识。
fn current_run_timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}
