Task 2 Interface Scope Report

- 已完成：仅在 `src/diff_refresh/orchestrator.rs` 与 `src/diff_refresh/mod.rs` 实现 `LongLivedPersistPolicy` 与 `DiffRefreshReport` 报告字段接口公开化，不触及持久化行为分支。
- 新增项：
  - `LongLivedPersistPolicy`：`dirty_node_threshold`、`max_pending_rounds`，默认值通过环境变量读取，回退至 `100` / `10`。
  - `DiffRefreshOrchestrator::set_persist_policy(policy)`：新增配置入口，保持构造函数签名不变。
  - `DiffRefreshReport`：新增 `persisted`、`pending_dirty_total`，并通过 `new_machine_report` 填充。
  - `diff_refresh::mod` 中导出 `LongLivedPersistPolicy`。
- 未实现项（符合 Task 2 范围）：
  - 未添加 `set_persist_fn` 或 pending 回放/延迟落盘决策逻辑。
  - `persist_policy` 当前仅存储配置，不影响现有提交路径。

测试与验证：
- `cargo fmt --check` 通过。
- 报告契约测试通过：
  - `cargo test --features cli-local --test m57_ai_contract_tests -- --nocapture`
  - `cargo test --features cli-local --test m57_refresh_report_scope_tests -- --nocapture`
  - `cargo test --features cli-local --test m57_tick_loop_tests -- --nocapture`
- 行为类测试未执行（按要求留给 Task 3），`m54` 中与长生命周期 pending 持久化相关断言预期仍为后续实现目标。
