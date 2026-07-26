Task 2 Fix 报告（2026-07-26）

- 修复点：将 `DiffRefreshOrchestrator::refresh_once` 中「首次 bootstrap 空变更集 checkpoint-only」路径的
  `persisted` 从 `persist_report.is_some()` 改为固定 `false`，与 `durable graph` 提交语义对齐。
- 未改行为：`Graph + checkpoint` 正常提交分支仍返回 `persisted = true`；已有 checkpoint 的空 Poll 仍返回
  `persisted = false`。
- 覆盖：补充了 `m54_diff_refresh_orchestrator_tests` 中空变更集分支与首次 bootstrap 空变更集测试中的
  `persisted` 断言（最小范围内）。

验证：
- `cargo fmt`
- `cargo test --test m57_ai_contract_tests --features cli-local`（通过）
- `cargo test --test m57_refresh_report_scope_tests --features cli-local`（通过）
- `cargo test --test m57_tick_loop_tests --features cli-local`（通过）
- `cargo test --test m54_diff_refresh_orchestrator_tests --features cli-local` 目前受现有测试文件未修复的
  `set_persist_fn` 编译错误阻断（与本次修改无关）。
