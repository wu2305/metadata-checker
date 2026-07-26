# M57 Phase 3 Task 4 合约同步报告

## 完成范围

- 更新 stdio 合约测试：`tests/stdio_server_tests.rs`
  - `test_stdio_diff_refresh_with_bound_context` 现已断言：
    - `persisted = false`
    - `pending_dirty_total > 0`
    - `persist_report = null`
  - 第二轮空轮询仍断言 `persisted = false`、`pending_dirty_total > 0`，并维持 `persist_report = null`，以确保 deferred pending 语义连续性。
  - 保留既有 `checkpoint/timing/invalidated_pages/warm_failures` 字段断言。

- 更新 one-shot 回归测试：`tests/regression_tests.rs`
  - `test_cli_runtime_session_diff_refresh_empty_bootstrap_ok` 补充：
    - `persisted = false`
    - `pending_dirty_total = 0`
  - 新增 `test_cli_runtime_session_diff_refresh_non_empty_bootstrap_sync_persisted`：
    - 在非空变更场景下验证 one-shot 返回 `persisted = true`
    - `pending_dirty_total = 0`
    - `persist_report` 存在、`change_count = 1`、`checkpoint` 存在

## 测试执行

- `cargo test --features cli-local --test stdio_server_tests -- --exact test_stdio_diff_refresh_with_bound_context --nocapture`
- `cargo test --features cli-local --test regression_tests -- --exact test_cli_runtime_session_diff_refresh_empty_bootstrap_ok --nocapture`
- `cargo test --features cli-local --test regression_tests -- --exact test_cli_runtime_session_diff_refresh_non_empty_bootstrap_sync_persisted --nocapture`
- `cargo test --features cli-local --test regression_tests -- --test-threads=1 --nocapture test_cli_runtime_session_diff_refresh`

均通过（1 + 3 + 1 = 5/5 + 额外筛选共6/6）。

## 结果

- 仅修改了测试与合约报告文件，未改动源实现、文档其它文件。
- 结论：LongLived stdio 与 one-shot 契约字段已对齐为：
  - 首轮非空 LongLived deferred：`persisted=false`、`pending>0`、`persist_report=null`
  - 空轮询保留 deferred pending 语义：`persisted=false`、`pending>0`、`persist_report=null`
  - one-shot 非空：`persisted=true`、`pending=0`
