## Task 2 fix report (Bootstrap empty ChangeSet persisted flag)

已修复 `m54_diff_refresh_orchestrator` 中 bootstrap 空变更集语义：
`refresh_once` 在 `changeset.change_count == 0` 时，仅当无现有 checkpoint（首次 bootstrap）才会执行 checkpoint-only 持久化，因此报告里的 `persisted` 应始终为 `false`；已有 checkpoint 的空 poll 仍保持 `persisted = false` 且不写 graph。

- 修改文件：
  - `src/diff_refresh/orchestrator.rs`
- 回归断言：
  - `tests/m54_diff_refresh_orchestrator_tests.rs`（空 ChangeSet 断言补充）
  - `tests/m57_bootstrap_checkpoint_only_report_tests.rs`（新增 bootstrap 空变更集回归用例）

已执行：
- `cargo fmt --check`
- `cargo test --features cli-local --test m57_ai_contract_tests m57_diff_refresh_report_has_machine_contract_fields`
- `cargo test --features cli-local --test m57_refresh_report_scope_tests m57_diff_refresh_report_declares_project_scope`
- `cargo test --features cli-local --test m57_bootstrap_checkpoint_only_report_tests m57_bootstrap_empty_changeset_still_reports_not_durable_persist`

说明：`tests/m54_diff_refresh_orchestrator_tests.rs` 当前主文件存在历史性
`set_persist_fn` 方法引用编译问题，未在本次任务内修复（按要求避免改 Task 3 行为）。
