# M58 CNB AI Chat Runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task with a verification checkpoint after each task.
> 状态：approved（2026-07-27；runner 限定在 CI tester，真实后端使用 CNB AI Chat API）

**Goal:** 在 CI tester 中实现基于 CNB AI Chat API 的空上下文 LLM 评测 runner，完成 JSON 命令协议、白名单 CLI 执行、确定性 AnswerJudge、RunReport 和手动/定时 CNB 入口。

**Architecture:** runner 只存在于 `tests/` 评测支持代码，不进入 `src/`、release 二进制、stdio 或 MCP。`CnbChatAdapter` 在 CNB 流水线内使用 `CNB_TOKEN` 调用 `POST /{repo}/-/ai/chat/completions`；本地测试使用 `FakeModelAdapter`。模型只能返回 JSON command/final 消息，runner 对命令做 case plan 精确匹配后直接通过 `std::process::Command` 执行 CLI，不经过 shell。

**Tech Stack:** Rust integration test、`serde`/`serde_json`、`reqwest::blocking`、现有 `metadata-checker` release binary、CNB `.cnb.yml` `api_trigger_m58_llm` pipeline。

## Global Constraints

- runner 代码只放在 `tests/support/` 与 `tests/m58_cnb_ai_runner_tests.rs`，不得新增 release CLI 或 stdio/MCP 面。
- CNB AI Chat API 的 endpoint、消息结构和认证以当前 Swagger 为准；不使用未公开的 `tools` / `tool_calls` 字段。
- `CNB_TOKEN` 只可在 CNB pipeline 进程中读取，只出现在 Authorization header；不得进入模型消息、日志、错误、JSON/Markdown 报告。
- project-dir、graphdb path、binary path 由 runner 固定；模型不能覆盖路径，命令不得经 shell 解释。
- 真实 LLM 测试只通过 `api_trigger_m58_llm` 手动/定时执行，不进入 PR 阻塞 CI；本地和普通 CI 只运行 fake/structural 层。
- 不执行严格的 red-green TDD 仪式；每个行为必须有可重复的契约测试、安全测试或快照测试，真实 CNB smoke 单独验证。
- 所有新增 Rust 模块级、类型级、函数级文档注释使用中文；不使用 `#[allow(dead_code)]`。

---

### Task 1: 固化 tier 与 CI tester 入口

**Files:**
- Modify: `tests/fixtures/corpus/ai_eval/ai_eval_cases.json`
- Create: `tests/support/m58_ai_eval.rs`
- Create: `tests/m58_cnb_ai_runner_tests.rs`

**Interfaces:**
- `load_eval_cases(path: &Path) -> anyhow::Result<Vec<EvalCase>>`
- `EvalCase { case_id, question, tier, case_status, difficulty, value }`
- `EvalTier::{FixtureStructural, FixtureLlm, RealManual}`
- `fixture_llm_cases(cases: &[EvalCase]) -> Vec<EvalCase>`

- [x] **Step 1: 为现有 case 明确 tier**

将以下三个 fixture case 标记为 `"tier": "fixture_llm"`，作为首个真实 baseline：

```text
page_purpose_actions_test
button_submit_effect
field_lineage_model1_name
```

其余非真实项目 case 标记为 `fixture_structural`，`requires_real_project=true` 的 case 标记为 `real_manual`。不改变既有字段或断言内容。

- [x] **Step 2: 实现向后兼容 loader**

`load_eval_cases` 读取 `cases` 数组；显式读取 `tier`，缺失时按 `requires_real_project` 回退到 `real_manual`，否则回退到 `fixture_structural`。未知 tier 直接返回带 case_id 的错误。

- [x] **Step 3: 添加 tier 契约测试**

`tests/m58_cnb_ai_runner_tests.rs` 至少包含：

```rust
#[test]
fn test_m58_fixture_llm_has_three_active_cases() {
    let cases = load_eval_cases(Path::new("tests/fixtures/corpus/ai_eval/ai_eval_cases.json")).unwrap();
    let selected = fixture_llm_cases(&cases);
    assert_eq!(selected.len(), 3);
    assert!(selected.iter().all(|case| case.case_status == "active"));
}
```

同时验证旧 schema 中缺失 `tier` 的临时 JSON 会按默认规则加载。

- [x] **Step 4: 运行验证并提交**

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests tier
git add tests/fixtures/corpus/ai_eval/ai_eval_cases.json tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs
git commit -m "test: add m58 eval tier loader"
```

Expected: tier tests pass; no production source or release binary changes.

### Task 2: 实现 JSON 命令协议与白名单策略

**Files:**
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`

**Interfaces:**
- `ChatMessage { role: String, content: String }`
- `CommandRequest { command_kind: String, target: String, args: Vec<String>, budget: Option<String> }`
- `AgentTurn::{Command(CommandRequest), Final(String)}`
- `parse_agent_turn(content: &str) -> anyhow::Result<AgentTurn>`
- `CommandPolicy::from_case(case: &EvalCase, project_dir: PathBuf, graph_db_path: PathBuf, binary_path: PathBuf) -> anyhow::Result<CommandPolicy>`
- `CommandPolicy::validate(&self, request: &CommandRequest, used_steps: &[usize]) -> anyhow::Result<ValidatedCommand>`
- `ValidatedCommand::argv(&self) -> Vec<OsString>`

- [x] **Step 1: 定义严格 envelope**

仅接受以下两个 JSON 形态，拒绝未知字段、额外 JSON、空字符串和非 object 内容：

```json
{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg","args":[],"budget":"compact"}
{"kind":"final","answer":"页面包含用户入口……"}
```

使用 `#[serde(deny_unknown_fields)]` 的内部 payload；模型不能在 JSON 中提供 `project_dir`、`graph_db_path`、`binary_path`、环境变量或 shell 字符串。

- [x] **Step 2: 精确匹配 minimal_command_plan**

`CommandPolicy` 将 case 的 `minimal_command_plan` 转成允许集合。命令 kind、target、args、budget 必须与某个尚未使用的 plan step 一致；超过 `max_command_count`、重复 step、未知 target 或未知预算返回 protocol diagnostic。

- [x] **Step 3: 生成无 shell CLI argv**

project 命令的 argv 顺序固定为：

```text
metadata-checker --non-human --project-dir <fixed-project-dir> --graph-db-path <fixed-graphdb> <allowed-kind> <target> <allowed-args>
```

single-file 命令只使用 fixture 内允许的 `.spg` / `.tbl` 路径；所有参数通过 `Command::arg` 传递。拒绝 `;`, `&&`, `||`, backtick、`$(` 等 shell 元字符只是额外防线，真正安全边界是“不调用 shell + 精确 plan 匹配”。

- [x] **Step 4: 添加协议与安全测试并提交**

覆盖 valid command/final、unknown field、malformed JSON、target 越权、project-dir 覆盖、graphdb 覆盖、shell payload、超出 max command count 和重复 plan step。

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests protocol command_policy
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs
git commit -m "test: enforce m58 json command policy"
```

### Task 3: 实现 FakeModelAdapter、CNB Chat Adapter 与脱敏

**Files:**
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`

**Interfaces:**
- `trait ModelAdapter { fn complete(&mut self, request: &ChatRequest) -> anyhow::Result<String>; }`
- `FakeModelAdapter::from_responses(responses: Vec<String>) -> Self`
- `CnbChatAdapter::from_env() -> anyhow::Result<Self>`
- `CnbChatAdapter::new(endpoint: String, repo: String, token: String, model: String) -> anyhow::Result<Self>`
- `redact_secret(text: &str, secret: &str) -> String`

- [x] **Step 1: 实现 FakeModelAdapter**

Fake adapter 按顺序返回预置 assistant content，并记录收到的 `ChatRequest`，用于验证每轮 stdout 被作为下一条 user message 传入；队列耗尽返回 error，不伪造成功响应。

- [x] **Step 2: 实现 CNB request/response DTO**

`ChatRequest` 序列化为 `{messages, model, stream:false}`；响应只读取 `choices[0].message.content`，缺少 choices/message/content 时返回结构错误。消息角色只生成 `user` / `assistant`。

- [x] **Step 3: 实现 CNB adapter**

默认 endpoint 为 `https://api.cnb.cool`，完整 URL 为 `https://api.cnb.cool/{repo}/-/ai/chat/completions`。`from_env` 要求 `CNB_TOKEN`、`M58_CNB_REPO`、`M58_CNB_MODEL`，允许 `M58_CNB_API_BASE` 覆盖测试 endpoint；请求使用 blocking reqwest，超时固定 60 秒。HTTP 非 2xx 错误只保留 status 和已脱敏 body 摘要。

- [x] **Step 4: 添加脱敏测试并提交**

用本地 `TcpListener` fake endpoint 验证：请求 Authorization 存在但不会进入序列化消息；错误、debug、报告文本均不包含假 token；响应缺字段会失败；Fake adapter 不需要任何环境变量。

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests adapter redaction
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs
git commit -m "test: add m58 cnb chat adapters"
```

### Task 4: 实现 AnswerJudge 与 RunReport

**Files:**
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `docs/reference/ai-eval-run-template.md`

**Interfaces:**
- `JudgeResult { passed: bool, failure_classes: Vec<String>, judge_notes: Vec<String> }`
- `judge_answer(answer: &str, assertions: &Value, diagnostics: &[String], trace: &[CommandTrace]) -> JudgeResult`
- `RunReport { schema_version, provider, model_id, cnb_build_id, started_at, cases, summary }`
- `write_run_report(report: &RunReport, json_path: &Path, markdown_path: &Path) -> anyhow::Result<()>`

- [x] **Step 1: 固化确定性判分规则**

实现大小写、空白和 Unicode 标点归一化后的 substring matching：

- `must_include` 缺失 → `missed_fact`
- `must_not_include` 命中 → `hallucination`
- command trace 与 plan 不匹配 → `wrong_command`
- 有 diagnostics 且未出现保守表达 → `ignored_diagnostic`
- 在 compact/summary 前直接请求 `--detail`、`--budget full` 或未允许的 details → `over_read_details`
- `evidence_reference_required=true` 时，回答必须出现 `summary`、`details`、`evidence`、`diagnostics`、`primary_path`、`key_findings` 之一；无法确定的规则只记录 `needs_human_review`，不自动放宽。

- [x] **Step 2: 定义 RunReport snapshot schema**

报告至少包含 `schema_version`、`provider`、`model_id`、`cnb_build_id`、`cases[]`、`pass_rate`、`failure_classes`、`command_trace_stats`；不写 raw token、Authorization header、完整 prompt 或大段模型原文。

- [x] **Step 3: 实现 JSON 与 Markdown 输出**

JSON 为唯一事实源；Markdown 只由 JSON 重新生成。输出目录由 `M58_AI_EVAL_OUTPUT_DIR` 指定，默认使用 `target/m58-ai-eval`，避免普通测试修改 `docs/ai-eval-runs/`。

- [x] **Step 4: 添加 judge/report snapshot 与提交**

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests judge report
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs docs/reference/ai-eval-run-template.md
git commit -m "feat: add m58 judge and run report"
```

### Task 5: 串起 fake runner 与 CNB live runner

**Files:**
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`

**Interfaces:**
- `run_case(case: &EvalCase, adapter: &mut dyn ModelAdapter, config: &RunnerConfig) -> anyhow::Result<CaseReport>`
- `run_fixture_llm_cases(cases: &[EvalCase], adapter: &mut dyn ModelAdapter, config: &RunnerConfig) -> anyhow::Result<RunReport>`
- `RunnerConfig { binary_path, skill_path, project_root, output_dir, provider, model_id, cnb_build_id }`

- [x] **Step 1: 构造 bootstrap message**

读取仓库根目录 `SKILL.md`，追加固定评测协议、JSON command/final schema、当前 case question。由于 CNB 当前公开消息契约只声明 `user`/`assistant`，bootstrap 以第一条 `user` message 发送；不得读源码、历史运行记录或知识库。

- [x] **Step 2: 实现多轮 loop**

每个 case 新建 message history；assistant 返回 command 时校验、执行 CLI、收集 stdout/diagnostics，再以 `user` 消息回传仅允许的 CLI JSON 输出；assistant 返回 final 时交给 judge。空 stdout、非 JSON stdout、HTTP failure、协议 failure 都进入 case error，不伪造业务答案。

- [x] **Step 3: 添加 fake end-to-end 测试**

用三个预置 fake case 验证：模型先 command 后 final；多轮 history 正确；CLI stdout 被回传；错误命令被阻止；answer judge 与 command trace 汇总到 RunReport。

- [x] **Step 4: 添加 ignored CNB live test**

`#[test] #[ignore] fn test_m58_cnb_fixture_llm_baseline()` 要求 `CNB_TOKEN`、`M58_CNB_REPO`、`M58_CNB_MODEL` 和 release binary 存在；顺序运行三个 `fixture_llm` case，写入 `target/m58-ai-eval/`，打印不含 prompt/token 的结构化摘要。

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests fake_runner
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs
git commit -m "feat: add m58 fixture runner"
```

### Task 6: 接入 CNB 手动/定时 pipeline

**Files:**
- Modify: `.cnb.yml`
- Modify: `docs/README.md`
- Modify: `docs/milestones/INDEX.md`
- Modify: `docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md`
- Modify: `docs/reference/ai-eval-run-template.md`

- [ ] **Step 1: 添加 `api_trigger_m58_llm`**

新增非 PR 事件 pipeline：使用现有 Rust/CNB image，构建 `target/release/metadata-checker`，再执行：

```bash
test -n "${CNB_TOKEN:-}"
test -n "${M58_CNB_REPO:-}"
test -n "${M58_CNB_MODEL:-}"
export M58_METADATA_CHECKER_BIN="$PWD/target/release/metadata-checker"
cargo test --features cli-local --test m58_cnb_ai_runner_tests -- --ignored --nocapture
```

流水线不导出 token，不使用 `sandbox: true`（CNB AI Chat API 要求在 pipeline 内使用 `CNB_TOKEN`），不把 live run 加到 `rust-ci` PR gate。

- [ ] **Step 2: 固定 report artifact 边界**

只打印 summary、RunReport 路径和 CNB build 标识；原始模型回答不写入 Git。首次人工确认的结构化结果才复制到 `docs/ai-eval-runs/`。

- [ ] **Step 3: 更新 journal 与运行模板**

将权威状态从 `planned` 更新为 `active`，把 `Plan` 链接到本计划；运行模板补充 `provider=cnb-ai-chat`、`cnb_build_id`、`protocol=json-command`、`model_id` 和 `case_filter` 字段，仍保持 LLM tier 非阻塞。

- [ ] **Step 4: 提交 pipeline/docs**

```bash
git add .cnb.yml docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md docs/reference/ai-eval-run-template.md
git commit -m "ci: add m58 cnb ai eval pipeline"
```

### Task 7: 影响面验证与交付检查

**Files:**
- No new files; verify all M58 changes.

- [ ] **Step 1: 运行 CI tester 层**

```bash
cargo fmt --check
cargo test --features cli-local --test m58_cnb_ai_runner_tests
cargo test --features cli-local --test ai_eval_tests -- --skip test_ai_eval_commands_execute_and_assert
```

- [ ] **Step 2: 运行 release/contract 检查**

```bash
cargo check --features cli-local
cargo build --release --features cli-local --bin metadata-checker
git diff --check
```

确认 `src/`、`src/main.rs`、`src/stdio_server.rs` 没有 M58 runner 代码，release binary 没有新增 runner 面。

- [ ] **Step 3: 真实 CNB smoke**

在非 PR pipeline 中执行：

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests -- --ignored --nocapture
```

验收三个 fixture case、JSON command loop、白名单拒绝、无 token 泄漏和 RunReport 输出。

- [ ] **Step 4: 完成 journal 与 final review**

记录真实 provider/model/build、通过率、失败分类和未确定项；请求独立只读 review，确认无 P0/P1/P2 后再提交 M58 PR。
