# M58 CNB SSE 与小模型稳健性 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task with review checkpoints.

**Goal:** 在 CI Tester 内跑通 CNB AI Chat 的 SSE 协议，并通过提示词、协议诊断和可量化指标迭代，让较小模型在固定元数据评测任务上更稳定地选对查询、遵守预算并给出有证据边界的回答。

**Architecture:** `metadata-checker` 主 CLI 保持 provider-agnostic，只输出确定性的查询结果；`tests/support/m58_ai_eval.rs` 负责通用模型消息、CNB SSE adapter、命令 loop 和 report；`.cnb.yml` 只负责在非 PR 事件中提供 CNB runtime 环境并执行 ignored live test。提示词只增强决策顺序和 JSON 输出纪律，不绕过 `CommandPolicy` 或放宽 `AnswerJudge`。

**Tech Stack:** Rust integration tests、`serde`/`serde_json`、`reqwest::blocking`、本地 `TcpListener` SSE fixture、CNB `.cnb.yml`、现有 release binary。

## Global Constraints

- runner 代码只放在 `tests/support/` 与 `tests/m58_cnb_ai_runner_tests.rs`，不得新增 release CLI 或 stdio/MCP 面。
- `CnbChatAdapter` 固定发送 `stream: true`，按 SSE `data:` 行解析；不得退回 `stream:false`。
- `CNB_TOKEN` 只在 CNB pipeline 进程中读取并放在 Authorization header；不得写入消息、日志或报告。
- `project-dir`、graphdb path、binary path 由 runner 固定，模型不能覆盖；命令不得经过 shell。
- 小模型优化不得修改被测 CLI 输出 schema、答案键、命令白名单或失败分类语义。
- 普通 CI 只运行 fake/structural 层；真实 CNB tier 由 `api_trigger_m58_llm` 手动/定时运行。
- 所有新增 Rust 模块级、类型级、函数级文档注释使用中文；不使用 `#[allow(dead_code)]`。
- 每个行为先添加失败测试并确认因目标行为缺失而失败，再写最小实现。

## File Map

- Modify `tests/support/m58_ai_eval.rs`: SSE DTO/parser、CNB request headers、环境变量 fallback、bootstrap prompt 和诊断文本。
- Modify `tests/m58_cnb_ai_runner_tests.rs`: SSE fake server、transport/error tests、bootstrap contract tests、small-model protocol regression tests。
- Modify `.cnb.yml`: API trigger 分支位置、`CNB_REPO_SLUG` fallback、可复现模型默认值和 live preflight。
- Modify `docs/reference/ai-eval-run-template.md`: SSE provider contract、model default、prompt/logic iteration fields。
- Modify `docs/plans/2026-07-27-m58-cnb-ai-chat-runner-plan.md`: record SSE amendment and verification status.
- Modify `docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md`: record current live blocker and optimization baseline.
- Preserve untracked `docs/reference/m58-gemma4-test-prompt.md`; do not overwrite or stage it unless explicitly requested.

---

### Task 1: CNB SSE transport contract

**Files:**
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `tests/support/m58_ai_eval.rs`

**Interfaces:**
- Add private `parse_sse_content(body: &str) -> anyhow::Result<String>` in `tests/support/m58_ai_eval.rs`.
- `CnbChatAdapter::complete` must send JSON `stream: true`, `Accept: text/event-stream`, and return concatenated `choices[].delta.content`.

- [x] **Step 1: Write the failing SSE success test**

Change the fake CNB response in `test_m58_cnb_adapter_sends_redacted_safe_request` to three `data:` JSON chunks plus `data: [DONE]`. Assert the returned assistant content is the concatenation, request JSON has `stream == true`, and request headers include `accept: text/event-stream`.

- [x] **Step 2: Run the focused test to verify RED**

Run:

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests test_m58_cnb_adapter_sends_redacted_safe_request -- --exact
```

Expected: FAIL because the current adapter sends `stream:false` and attempts to parse a non-stream `message` object.

- [x] **Step 3: Write failing edge-case tests**

Add focused tests for an SSE stream with invalid JSON, a stream containing only `[DONE]`, and a chunk whose `choices` array is empty. Assert each returns an error without including the token.

- [x] **Step 4: Run edge-case tests to verify RED**

Run:

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests test_m58_cnb_sse -- --nocapture
```

Expected: the new tests fail because no SSE parser exists.

- [x] **Step 5: Implement the minimal parser and request change**

Add serde DTOs for `choices[].delta.content: Option<String>`. Parse non-empty `data:` lines, stop at `[DONE]`, concatenate content fragments, reject malformed JSON, missing choices, and blank final content. Set `stream=true` regardless of the caller preference and add the SSE `Accept` header. Keep HTTP error body summarization and token redaction unchanged.

- [x] **Step 6: Run focused tests to verify GREEN**

Run the two commands above. Expected: all focused SSE and adapter tests pass.

- [x] **Step 7: Commit**

```bash
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs docs/specs/2026-07-17-cheap-model-comprehension-eval-design.md
git commit -m "fix: support cnb streaming chat responses"
```

### Task 2: Pipeline runtime defaults and trigger routing

**Files:**
- Modify: `.cnb.yml`
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `docs/reference/ai-eval-run-template.md`

**Interfaces:**
- Live stage sets `M58_CNB_REPO="${M58_CNB_REPO:-${CNB_REPO_SLUG:?}}"`.
- Live stage sets `M58_CNB_MODEL="${M58_CNB_MODEL:-deepseek-v4-flash}"`.
- `M58_METADATA_CHECKER_BIN` remains a build-stage derived path, not a local credential/config requirement.

- [x] **Step 1: Write the failing pipeline contract check**

Add a CI Tester assertion or shell preflight test that the live script can run with only `CNB_TOKEN` and built-in `CNB_REPO_SLUG`, while using `deepseek-v4-flash` if no model override is passed. The current `.cnb.yml` must fail this check because it requires both M58 variables before deriving them.

- [x] **Step 2: Run the contract check to verify RED**

Run the repository's official validator and a shell extraction check:

```bash
node /Users/wuhaocheng/.agents/skills/cnb-pipeline/validator/validate.js .cnb.yml
rg -n "M58_CNB_REPO|M58_CNB_MODEL|api_trigger_m58_llm" .cnb.yml
```

Expected: schema passes but semantic validator warns that the API trigger is under `main`; the script still contains unconditional variable checks.

- [x] **Step 3: Implement the minimal pipeline fix**

Move `api_trigger_m58_llm` from `main` to `$`. In the live stage, derive the repo from `CNB_REPO_SLUG`, default the observed current CNB model to `deepseek-v4-flash`, keep explicit trigger overrides, then check `CNB_TOKEN` and the release binary. Do not echo token or prompt contents.

- [x] **Step 4: Run validator and local contract checks to verify GREEN**

Run the official validator, `cargo test --features cli-local --test m58_cnb_ai_runner_tests`, and `git diff --check`. Expected: validator has no semantic warning and no project file is modified by the check.

- [x] **Step 5: Commit**

```bash
git add .cnb.yml tests/support/m58_ai_eval.rs docs/reference/ai-eval-run-template.md
git commit -m "ci: make m58 cnb runner self-configuring"
```

### Task 3: Small-model prompt and protocol robustness

**Files:**
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `docs/reference/ai-eval-run-template.md`

**Interfaces:**
- Keep `build_bootstrap_message(skill: &str, case: &EvalCase) -> String` private and deterministic.
- Add a focused prompt contract test that checks required decision rules are present without embedding answer keys or hidden case data.

- [x] **Step 1: Write failing bootstrap contract tests**

Add tests asserting the bootstrap explicitly tells the model to: emit one raw JSON object without Markdown fences; copy command kind/target/args/budget from `SKILL.md`; start with the smallest allowed query and compact budget; use the next user message as evidence; stop after enough evidence; and mention diagnostics/truncation instead of guessing. Assert it does not contain `expected_facts`, `must_include`, `CNB_TOKEN`, or source paths outside the fixed skill text.

- [x] **Step 2: Run prompt tests to verify RED**

Run:

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests bootstrap -- --nocapture
```

Expected: FAIL because the current bootstrap does not state all small-model decision rules.

- [x] **Step 3: Implement concise prompt guidance**

Rewrite only the bootstrap instruction text. Use short numbered rules, one command JSON example, one final JSON example, and explicit post-command behavior. Do not inject minimal plans, answer assertions, expected facts, hidden documents, model metadata, or prior case answers. Keep `SKILL.md` and the case question as the only case-specific inputs.

- [x] **Step 4: Add protocol recovery tests**

Add a fake-runner test for a model that first returns a fenced JSON object or explanatory prefix, then a valid command. The runner must keep the strict protocol and record `protocol_error`; it must not silently repair or execute the malformed command. Add a separate test showing a valid command followed by a final answer with diagnostic disclaimer passes the existing judge.

- [x] **Step 5: Run prompt and runner tests to verify GREEN**

Run:

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests bootstrap protocol fake_runner -- --nocapture
```

- [x] **Step 6: Commit**

```bash
git add tests/support/m58_ai_eval.rs tests/m58_cnb_ai_runner_tests.rs docs/reference/ai-eval-run-template.md
git commit -m "test: harden m58 small-model protocol prompt"
```

### Task 4: Verification, live smoke, and optimization journal

**Files:**
- Modify: `docs/plans/2026-07-27-m58-cnb-ai-chat-runner-plan.md`
- Modify: `docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md`
- Modify: `docs/reference/ai-eval-run-template.md`

- [x] **Step 1: Run the complete local verification set**

Run:

```bash
cargo fmt --check
cargo test --features cli-local --test m58_cnb_ai_runner_tests
cargo test --features cli-local --test ai_eval_tests -- --skip test_ai_eval_commands_execute_and_assert
cargo check --features cli-local
cargo build --release --features cli-local --bin metadata-checker
git diff --check
```

- [ ] **Step 2: Push the current branch and create a draft CNB PR**

Push `codex/m58-slm-eval-foundation` to the CNB remote and create a draft PR against `main`. Use CNB PR diff/review APIs for the independent review; do not claim the previous timed-out local reviewer as evidence.

- [ ] **Step 3: Trigger the real SSE smoke on the pushed commit**

Use the logged-in CNB CLI to trigger the feature branch and exact SHA:

```bash
cnb build start-build --repo wu2305/metadata-checker \
  --branch codex/m58-slm-eval-foundation \
  --sha "$(git rev-parse HEAD)" \
  --event api_trigger_m58_llm \
  --sync true \
  --data '{"env":{"M58_CNB_MODEL":"deepseek-v4-flash"}}'
```

The pipeline supplies `CNB_TOKEN`, derives `M58_CNB_REPO`, builds `M58_METADATA_CHECKER_BIN`, and writes `target/m58-ai-eval/run.json` and `run.md`. Poll status/logs if asynchronous output is returned.

- [ ] **Step 4: Record the first real result and optimization baseline**

Record provider, model, build ID, pass rate, failure classes, average/max command metrics, and budget upgrade count. Do not copy raw prompts, raw model answers, token values, or authorization headers into repository docs. Any prompt or logic change must be compared against this baseline using the same three fixture cases.

- [ ] **Step 5: Run final review and commit documentation**

Run the official CNB code review on the draft PR, address P0/P1/P2 findings, then update the plan checklist and milestone journal with exact live evidence. Commit only the documentation and code changes belonging to this plan.
