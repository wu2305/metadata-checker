# M57 Phase 3 Long-Lived Persist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 LongLived diff refresh 在 `install` 后立即提供内存查询结果，并按 dirty-node 阈值或轮次兜底批量持久化，同时在 persist 失败后保留内存状态并可重试。

**Architecture:** `DiffRefreshOrchestrator` 持有会话内 pending checkpoint、pending dirty-node 集合、pending commit 和 pending rounds。非空刷新先 mirror → prepare → read-model replacement → `install_replacement`，再按策略对当前 runtime graph 执行 durable persist；未触发时只推进内存 watermark，下一轮以 pending watermark poll。失败不回滚已安装 runtime，保留 pending commit 供下一轮重试。one-shot 仍使用现有同步 persist 语义。

**Tech Stack:** Rust 2024、`anyhow::Result`、serde JSON、redb GraphDB、现有 M54–M56 `ProjectIndexer`/`IndexCommit`/LongLived read model。

## Global Constraints

- 核心实现保持 Rust；不引入新依赖或第二套 mirror/graph 构建栈。
- 默认 `persist_dirty_threshold=100`，只有 `pending_dirty_total > 100` 触发阈值 persist。
- 默认 `persist_max_rounds=10`，pending 状态连续达到 10 个成功 refresh round 时强制 persist。
- 每轮 poll 使用内存 pending watermark；持久化成功后清空 pending 状态。
- persist 失败不得撤销已安装的内存 runtime；错误继续向调用方返回，下一轮可重试。
- 报告必须暴露 `persisted` 与 `pending_dirty_total`，并保持 one-shot/stdio 共享 `DiffRefreshReport`。

---

### Task 1: Phase 3 RED contract tests

**Files:**
- Create: `tests/m57_phase3_long_lived_persist_tests.rs`
- Modify: `tests/m57_ai_contract_tests.rs`
- Modify: `tests/m57_refresh_report_scope_tests.rs`
- Modify: `tests/m57_tick_loop_tests.rs`

**Interfaces:**
- Tests consume the existing `DiffRefreshOrchestrator::new`, fixture session helpers/patterns, `DiffRefreshReport`, and a test-configurable persist policy/hook that implementation must expose.
- Tests produce executable contracts for immediate install visibility, threshold/round persistence, pending watermark advancement, and retry after persist failure.

- [ ] **Step 1: Write the failing report-field and Phase 3 behavior tests.**
  Add assertions that a deferred round returns `persisted == false`, reports the non-zero `pending_dirty_total`, and serves the changed page from the orchestrator runtime before disk checkpoint advancement. Add threshold-triggered persistence, ten-round fallback persistence, and injected persist failure followed by a successful retry. Use `assert_eq!` for exact fields and keep failure messages tied to the Phase 3 behavior.

- [ ] **Step 2: Run only the new/affected tests to verify RED.**

  ```bash
  cargo test --features cli-local --test m57_phase3_long_lived_persist_tests --test m57_ai_contract_tests --test m57_refresh_report_scope_tests --test m57_tick_loop_tests
  ```

  Expected: compile or assertion failures because `DiffRefreshReport` lacks the Phase 3 fields and the orchestrator still persists before install.

- [ ] **Step 3: Commit the failing tests.**

  ```bash
  git add tests/m57_phase3_long_lived_persist_tests.rs tests/m57_ai_contract_tests.rs tests/m57_refresh_report_scope_tests.rs tests/m57_tick_loop_tests.rs
  git commit -m "test: define m57 long-lived persist policy"
  ```

### Task 2: Add policy and pending-state interfaces

**Files:**
- Modify: `src/diff_refresh/orchestrator.rs`
- Modify: `src/diff_refresh/mod.rs`

**Interfaces:**
- `LongLivedPersistPolicy { dirty_node_threshold: usize, max_pending_rounds: usize }` with defaults `100` and `10` plus environment overrides `METADATA_CHECKER_PERSIST_DIRTY_THRESHOLD` and `METADATA_CHECKER_PERSIST_MAX_ROUNDS`.
- `DiffRefreshOrchestrator::set_persist_policy(policy)` configures deterministic tests and callers without changing the existing constructor.
- `DiffRefreshReport` adds serialized `persisted: bool` and `pending_dirty_total: usize`.

- [ ] **Step 1: Implement only the public policy/report types and compile-facing fields.**
- [ ] **Step 2: Run the report-contract tests and confirm only behavior assertions remain red.**
- [ ] **Step 3: Commit the interface slice.**

  ```bash
  git add src/diff_refresh/orchestrator.rs src/diff_refresh/mod.rs
  git commit -m "feat: add m57 long-lived persist policy contract"
  ```

### Task 3: Implement install-first deferred persist and retry

**Files:**
- Modify: `src/diff_refresh/orchestrator.rs`
- Test: `tests/m57_phase3_long_lived_persist_tests.rs`

**Interfaces:**
- The orchestrator keeps `pending_checkpoint`, `pending_dirty_node_ids`, `pending_commit`, and `pending_rounds` private; `refresh_once` remains the public entry point.
- `effective_checkpoint()` selects the pending in-memory checkpoint before the durable graph checkpoint.
- `persist_pending()` persists the currently installed runtime graph with the retained `IndexCommit`, clears pending state only after success, and leaves it intact on error.

- [ ] **Step 1: Change non-empty refresh ordering to install the prepared graph/read model before deciding persistence.**
- [ ] **Step 2: Merge dirty and deleted IDs into the pending set, retain the latest checkpoint/commit, and increment the pending round for each successful refresh round.
- [ ] **Step 3: Trigger persist only when `pending_dirty_node_ids.len() > policy.dirty_node_threshold` or `pending_rounds >= policy.max_pending_rounds`; otherwise return a deferred report.
- [ ] **Step 4: Use pending watermark for later polls and allow empty polls to advance the pending-round fallback without re-installing a graph.
- [ ] **Step 5: On persist error, keep the installed runtime and pending commit, return the original error, and make the next refresh retry without requiring another install.
- [ ] **Step 6: Run the Phase 3 tests until GREEN.**

  ```bash
  cargo test --features cli-local --test m57_phase3_long_lived_persist_tests
  ```

- [ ] **Step 7: Commit the implementation.**

  ```bash
  git add src/diff_refresh/orchestrator.rs src/diff_refresh/mod.rs tests/m57_phase3_long_lived_persist_tests.rs tests/m57_ai_contract_tests.rs tests/m57_refresh_report_scope_tests.rs tests/m57_tick_loop_tests.rs
  git commit -m "feat: add long-lived deferred persist and retry"
  ```

### Task 4: Regression, docs, and acceptance evidence

**Files:**
- Modify: `docs/milestones/performance/m57-diff-refresh-closeout.md`
- Modify: `docs/plans/2026-07-26-m57-ai-contract-foundation-plan.md`
- Modify: `docs/milestones/performance/performance-baseline.md`

**Interfaces:**
- Documentation records the default policy, report semantics, install-before-persist ordering, and failure recovery without claiming that unpersisted rounds survive process crashes.

- [ ] **Step 1: Run M54 orchestrator, M56 persistence, M57 Phase 3, stdio, and report-contract tests.**

  ```bash
  cargo test --features cli-local --test m54_diff_refresh_orchestrator_tests --test m56_incremental_persist_tests --test m57_phase3_long_lived_persist_tests --test m57_ai_contract_tests --test m57_refresh_report_scope_tests --test stdio_server_tests
  ```

- [ ] **Step 2: Run `cargo fmt --check`, `cargo check --features cli-local`, and `git diff --check`.**
- [ ] **Step 3: Update M57 Phase 3 acceptance entries with observed test counts and failure-retry evidence.**
- [ ] **Step 4: Run the full `cargo test --features cli-local` suite before claiming completion.**
- [ ] **Step 5: Commit docs and final verification evidence.**

  ```bash
  git add docs/milestones/performance/m57-diff-refresh-closeout.md docs/plans/2026-07-26-m57-ai-contract-foundation-plan.md docs/milestones/performance/performance-baseline.md
  git commit -m "docs: record m57 phase3 acceptance"
  ```
