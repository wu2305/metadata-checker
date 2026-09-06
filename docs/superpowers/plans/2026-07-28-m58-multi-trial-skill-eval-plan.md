# M58 Multi-trial Skill Comprehension Evaluation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task with review checkpoints.

**Goal:** 将 M58 从单次三 case smoke 扩展为可重复、多任务、多 trial 的 Skill 理解力评测，同时保留 CNB provider 与 metadata-checker 主工具的边界。

**Architecture:** `tests/support/m58_ai_eval.rs` 继续提供 provider adapter、命令白名单、case workspace 和确定性 judge。每次 trial 调用相同的 case runner，但使用新的消息 history、临时 workspace 和 graphdb；`RunReport` 以脱敏的 trial rows 为事实源，同时计算 trial 通过率、case 稳定通过率和 flaky case 数。fixture case 复用现有 structural 数据，只通过 tier 与 `evaluation_dimensions` 声明它们的评测用途。

**Tech Stack:** Rust integration tests、`serde`/`serde_json`、现有 `FakeModelAdapter`/`CnbChatAdapter`、fixture metadata project、CNB pipeline。

## Global Constraints

- `metadata-checker` release CLI、stdio 和 `SKILL.md` 不依赖 CNB API。
- 每个 trial 的 `project-dir`、graphdb、binary path 和 message history 由 runner 固定，模型不能覆盖。
- `CNB_TOKEN` 只进入 CNB adapter 的 Authorization header，不进入消息、日志、报告或 case metadata。
- 默认 `trial_count=1`，只有 CNB live 入口显式覆盖为 `3` 或更高。
- 不把 `expected_facts`、`answer_assertions`、`minimal_command_plan` 或隐藏答案注入 bootstrap。
- 新增 Rust 模块、类型、函数注释使用中文；普通 CI 不访问 CNB 网络。
- 每个行为遵循 RED → GREEN → REFACTOR，先观察目标测试失败再写实现。

### Task 1: Trial and difficulty metadata contract

**Files:**
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `tests/fixtures/corpus/ai_eval/ai_eval_cases.json`

**Interfaces:**
- `EvalCase` exposes `task_family`, `difficulty`, and optional `evaluation_dimensions` defaults.
- `RunnerConfig` exposes `trial_count` with default helper value `1`.
- `CaseReport` exposes `trial_index` and `task_family` without storing prompt or answer.

- [x] **Step 1: Write failing loader/report tests**

  Add tests that load an explicit `evaluation_dimensions` object, reject invalid `trial_count=0`, and assert a report row records `trial_index=2`, `task_family`, and `difficulty`.

- [x] **Step 2: Run focused tests and verify RED**

  Run `cargo test --features cli-local --test m58_cnb_ai_runner_tests trial_metadata -- --nocapture`.
  Expected: compile/test failure because the new fields and trial configuration do not exist.

- [x] **Step 3: Implement minimal metadata types and defaults**

  Parse optional dimension fields with deterministic defaults derived from `risk_tags` and `difficulty`; add the trial count field and report metadata while preserving existing JSON fields.

- [x] **Step 4: Run focused tests and verify GREEN**

  Re-run the focused command and the existing report tests.

### Task 2: Multi-trial runner and aggregate metrics

**Files:**
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `tests/support/m58_ai_eval.rs`
- Modify: `docs/reference/ai-eval-run-template.md`

**Interfaces:**
- `run_case` remains a one-trial compatibility wrapper.
- `run_fixture_llm_cases` runs `RunnerConfig.trial_count` independent trials per active `fixture_llm` case.
- `RunReport` adds `case_stable_pass_rate` and `cases_with_flaky_trials` while keeping `pass_rate` as trial-level pass rate.

- [x] **Step 1: Write failing multi-trial tests**

  Add a fake run with `trial_count=2` and responses that pass trial 0 and fail trial 1. Assert two report rows for one case, distinct trial indexes, `pass_rate=0.5`, `case_stable_pass_rate=0.0`, and one flaky case.

- [x] **Step 2: Run focused tests and verify RED**

  Run `cargo test --features cli-local --test m58_cnb_ai_runner_tests multi_trial -- --nocapture`.
  Expected: failure because the runner currently executes each case exactly once.

- [x] **Step 3: Implement the smallest independent-trial loop**

  Loop trials inside `run_fixture_llm_cases`; route all failures through the existing redacted report constructor; make `prepare_case_workspace` include a trial-specific directory; do not reuse history across calls.

- [x] **Step 4: Implement aggregate metrics and report rendering**

  Compute trial pass rate, stable case pass rate, flaky case count, and trial-level command averages from report rows. Keep Markdown generated from the JSON report and omit raw model content.

- [x] **Step 5: Run focused and complete runner tests**

  Run `cargo test --features cli-local --test m58_cnb_ai_runner_tests` and confirm the existing one-trial fake behavior remains green.

### Task 3: Expand fixture comprehension coverage

**Files:**
- Modify: `tests/fixtures/corpus/ai_eval/ai_eval_cases.json`
- Modify: `tests/m58_cnb_ai_runner_tests.rs`
- Modify: `docs/reference/ai-eval.md`

**Interfaces:**
- Active `fixture_llm` selection contains exactly 13 cases across page logic, explain, dataflow/lineage, navigation, diagnostic, condition, positive and negative task families; the live baseline rejects any count drift.
- Existing `tests/ai_eval_tests.rs` structural behavior remains unchanged.

- [x] **Step 1: Write failing coverage assertions**

  Assert the selected fixture LLM set has at least 10 cases and includes `page_logic`, `diagnostic`, `condition`, `navigation`, `dataflow`/`lineage`, plus a negative/readonly case.

- [x] **Step 2: Run coverage test and verify RED**

  Run `cargo test --features cli-local --test m58_cnb_ai_runner_tests fixture_llm -- --nocapture`.
  Expected: the current three-case set fails the coverage assertions.

- [x] **Step 3: Promote existing deterministic fixture cases**

  Change only the tier and add explicit `evaluation_dimensions` to suitable existing fixture cases; do not duplicate answer keys or alter their structural assertions.

- [x] **Step 4: Run structural and fixture loader tests**

  Run `cargo test --features cli-local --test m58_cnb_ai_runner_tests` and `cargo test --features cli-local --test ai_eval_tests -- --skip test_ai_eval_commands_execute_and_assert`.

### Task 4: CNB live contract and documentation

**Files:**
- Modify: `.cnb.yml`
- Modify: `docs/reference/ai-eval-run-template.md`
- Modify: `docs/milestones/ai-eval/m58-cheap-model-comprehension-eval.md`
- Modify: `docs/plans/2026-07-27-m58-cnb-ai-chat-runner-plan.md`

- [x] **Step 1: Add failing live configuration test**

  Assert the live stage exposes `M58_AI_EVAL_TRIALS` with default `3`, while local/fake execution stays at `1`; assert the report contract documents trial pass rate and stable case pass rate.

- [x] **Step 2: Run the contract test and verify RED**

  Run the focused pipeline/documentation contract test and confirm the current `.cnb.yml` and report template lack the new variables/fields.

- [x] **Step 3: Implement the pipeline default and documentation**

  Export the trial count only inside `api_trigger_m58_llm`, pass it to the ignored test through an environment variable, and document that live LLM results are observational rather than PR-blocking.

- [x] **Step 4: Run final local verification**

  Run `cargo fmt --check`, the M58 runner tests, the structural AI eval tests, `cargo check --features cli-local`, `git diff --check`, and the official CNB pipeline validator.

### Task 5: Live baseline and review

- [x] Run the existing CNB live trigger with `M58_AI_EVAL_TRIALS=3` after the branch is pushed.
- [x] Verify the report has no token, Authorization header, prompt, or raw model answer.
- [x] Summarize per-case trial outcomes and update the milestone with the actual build ID and failure classes.
- [x] Run the independent CNB review, address P0/P1/P2 findings, and only then mark this extension complete（最终复验范围 `fcfc59d..6ad0d43`，P0/P1/P2/P3 均无；CNB `cnb-voo-1jujgnsbf` 完成 13×3 baseline）。
