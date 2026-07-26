# M57：Diff refresh 产品化与 tick 成本收口

> 状态：**done**（spec/plan approved；M57-6 LongLived persist 验收通过）
> Spec：[2026-07-23-m57-diff-refresh-closeout-design.md](../../specs/2026-07-23-m57-diff-refresh-closeout-design.md)  
> Plan：[2026-07-26-m57-ai-contract-foundation-plan.md](../../plans/2026-07-26-m57-ai-contract-foundation-plan.md)（approved）  
> Depends：M56（`cnb/main` @ `b4ec419` 已合入）

## 目标

在 M54–M56 正确性基座上完成：tick/观测产品壳、**SKILL/帮助同步**、**页级刷新范围**、**凭证流**、派生索引增量、输出/打开打磨；并为 **M46** 固定「只包装不重写栈」的 handoff。

### M57-0：SLM 测评基础契约（先行）

- [x] `DiffRefreshReport` 暴露稳定 `schema_version`、`kind`、`persist_report`。
- [x] one-shot 与 stdio 输出共享报告字段；机器模式严格单行 JSON。
- [x] 登录 401/403 保留脱敏后的 `message` / `error.message`，稳定错误码不变。
- [x] TDD 覆盖非空刷新、空 bootstrap、失败回滚、单行输出和敏感信息边界。
- [x] 与 M58 明确边界：M57 不实现 AnswerJudge、ModelAdapter、RunReport 或模型基线。

## 三阶段

| Phase | 焦点 | 状态 |
|-------|------|------|
| 1 | PersistReport、tick+Backoff、stdio 真机冒烟；**SKILL/`--help`/README**；**RefreshScope 自动探索+申明**；**凭证失效/重登/换绑**；M46 边界 | done |
| 2 | Dense/Facts/PageDep 增量更新 + 规模曲线 | done |
| 3 | LongLived 内存优先 persist（阈值 100 + 10 轮兜底）、失败恢复 e2e | done |

## 进度约定

- journal **随 commits** 滚动更新（本表、曲线、真机片段）
- INDEX 仅在三阶段全收完后改 `done`

## 明确不做

见 spec 非目标（含：不做完 M46 CLI 产品化、不做 keychain/浏览器活刷新/多项目并行 daemon）。

## 与 M46

- INDEX：`M46 depends = M45, M57`
- M57 交付编排 + 范围过滤内核 + 凭证流原语 + 文档同步  
- M46 交付聚合 CLI / 发现 UX / analyze 组合 / 进度打磨  
- Phase 1 完成即可开 M46（不必等 Phase 2/3）

## 实现进度

| 项 | 状态 |
|----|------|
| Spec draft（含 SKILL/范围/凭证/LongLived persist） | done（本分支） |
| INDEX / planning / M46 边界调整 | done（本分支） |
| Spec approved | done（2026-07-26） |
| Plan approved | done（2026-07-26） |
| M57-0 TDD | done（`d452e85`） |
| M57-0 code | done（`c9302fd`、`145e838`） |
| M57-1 tick loop TDD | done（`7763b20`） |
| M57-1 tick loop code | done（`8249d6f`、`2ec96a8`） |
| M57-2 RefreshScope TDD | done（`387871b`） |
| M57-2 RefreshScope code | done（`e980b37`、`dd231af`） |
| M57-3 runtime auth TDD | done（`55e5339`） |
| M57-3 runtime auth code | done（`6bb8364`） |
| M57-4 DiffRefresh scope TDD | done（`f0b03f1`） |
| M57-4 DiffRefresh scope code | done（`c3f9fda`） |
| Phase 1 rebind TDD | done（`3eed1af`） |
| M57-5 Phase 2 incremental read model TDD | done（`4eacb64`） |
| M57-5 Phase 2 incremental read model code | done（`c80e95c`） |
| M57-5 real release baseline harness | done（`a07fa2e`、`497a6bc`、`ef59de1`） |
| M57-6 Phase 3 LongLived persist TDD | done（`9784e2f`） |
| M57-6 Phase 3 LongLived persist 代码 | done（`63ab220`、`17186e1`、`c66e7f7`、`0349f0a`） |
| Phase 1–3 代码 | done |

## 设计决策（2026-07-24）

- **路径确定性**：不做；e2e 继续 `stable_semantic_view`
- **零写盘 open**：降级；主线改为 LongLived `install` 优先 + 批量 persist
- **LongLived 可见性**：`install` 后立即可查；崩溃可丢未 persist 轮次，重启后从 durable checkpoint 回放增量
- **批量 persist**：`dirty∪deleted > 100` 或连续 10 轮未落盘触发；one-shot 每轮同步

## 验收记录

- M57-0：`cargo fmt --check`、`cargo check --features cli-local` 通过。
- M57-0：M57 报告、stdio bound refresh、session provider、one-shot regression 目标测试通过；stdio 集成测试 `35 passed / 1 ignored`。
- M57-0：发现并修复已有 checkpoint 的空 poll 不应触发 `persist_with_checkpoint` 全量重写（`145e838`）；该路径现在返回 `persist_report=null` 且不写盘。
- M57-1：tick loop TDD 已完成；网络退避、数据错误不重试和尝试上限验收通过。
- M57-1：`m57_tick_loop_tests` 4 passed；`m54_diff_refresh_orchestrator_tests` 5 passed；`m55_meta_files_source_tests` 15 passed；M57 AI contract 与 stdio bound refresh 目标测试通过。
- M57-1：网络错误读取完整 `anyhow` error chain；401/403、`INVALID_*`、`DIFF_REFRESH_*` 和未分类错误不退避；最后一次可重试失败不 sleep，耗尽以 `exhausted=true` 统计返回。
- M57-2：scope declaration 4 个 TDD 通过；session CLI 17 个测试通过；显式 module/source/file 输出 applied scope，compound 保留全部 selectors，current-page 排序提示和 project fallback 均带稳定原因。
- M57-2：scope 枚举 JSON 固定为 snake_case（`dd231af`），供后续 SLM runner 直接消费。
- M57-3：runtime auth error mapping TDD 与实现已通过；401/403 在完整 anyhow chain 中映射为 `SESSION_AUTH_REQUIRED`，普通错误仍为 `DIFF_REFRESH_FAILED`，stdio/one-shot 均保持脱敏诊断。
- M57-3：专项认证测试 2 passed；stdio 全套 36 passed、1 ignored；M57-0/M57-1/M57-2 影响面测试继续通过。
- M57-4：主 `DiffRefreshReport`、stdio、one-shot、tick 共享 `RefreshScope`；无局部 selector 时统一输出 `project/fallback/applied=false`，scope contract 2 passed，stdio 全套回归仍为 36 passed、1 ignored。
- M57-6：M57-5 真实项目 long-lived + stdio 路径保持可见性：`runtime` 长驻启动后刷新，查询可见更新；`m57_phase3` 语义变更未改变 query 正确性。
- M57-6：`m57_phase3` 语义在 stdio 绑定上下文一轮 `diff_refresh` 首次执行后仍通过；`test_stdio_diff_refresh_with_bound_context` 通过。
- Phase 1 rebind：同一 session 两次绑定使用不同凭据，manifest 不保存 password/token/cookie；测试 1 passed。
- Phase 1 真实项目 graphdb：使用独立路径 `/private/tmp/m57-xiaoshouyi.graphdb` 完成 `1329 files / 78127 dirty / 0 deleted` 构建，避免与既有 graphdb 争用。
- Phase 1 真实项目 stdio：复用该 graphdb 启动 LongLived，报告 `78127 nodes, read_model=true, ready`；`m57-r1 query_model` 与 `m57-r2 query_page_logic` 均返回 `ok=true`，随后结束进程，计入通过证据。
- M57-5：`DenseGraphSnapshot`、`MaterializedAvailabilityFactsIndex`、`PageDependencyIndex` 已按 `dirty ∪ deleted` 接入 replacement；稳定 node set 走 incremental，新增/删除统一 full fallback，更新模式由 `ReadModelUpdateMode` 标记。
- M57-5 等价与曲线：`m57_incremental_read_model_tests` 3 passed；fixture full rebuild 样本 `7 ms`，增量 `dirty=1/10/100` 样本分别为 `0/2/5 ms`（wall `1/3/7 ms`）。
- M57-5 曲线复跑：同一测试再次得到 full `17 ms`、增量 read-model `2/6/17 ms`（wall `2/6/19 ms`）；毫秒值受机器与编译缓存影响，只作为可观测性样本，不替代 real-project release 基线。
- M57-5 针对性回归：M54 编排器 `5 passed`、M55 PageDep `5 passed`、M53 Facts `1 passed`、Dense `1 passed`、runtime `19 passed/1 ignored`、rebind `1 passed`。
- M57-5 真实项目 release 基线：`m57_real_project_release_dirty_curve` 使用候选图实际改名 mutation（node set/拓扑不变），`78127 nodes / 150164 edges`；cold full read-model `249906 ms`，dirty `1/10/100` 的 incremental read-model 分别为 `1653/2347/3052 ms`，wall 分别为 `1655/2348/3053 ms`，三档均为 `Incremental`。
- M57-5 性能门：相对 full 分别约 `151.2x/106.5x/81.9x` 加速；真实 release 曲线通过，Phase 2 关闭。Phase 3 复核后 release binary 为 `3781936 bytes`（约 `3.6M`，小于 10MB 约束）。
- M57-6：M54 orchestrator 全部通过：`cargo test --features cli-local --test m54_diff_refresh_orchestrator_tests`（12 passed）。
- M57-6：M57 AI 契约/Scope/tick 全部通过：`cargo test --features cli-local --test m57_ai_contract_tests`（1 passed）、`cargo test --features cli-local --test m57_refresh_report_scope_tests`（2 passed）、`cargo test --features cli-local --test m57_tick_loop_tests`（4 passed），合计 7 passed。
- M57-6：`cargo test --features cli-local --test stdio_server_tests -- --exact test_stdio_diff_refresh_with_bound_context --test-threads=1`（1 passed）。
- M57-6：持久化策略默认值更新（代码常量 + env）：
  - `METADATA_CHECKER_PERSIST_DIRTY_THRESHOLD`，默认 `100`
  - `METADATA_CHECKER_PERSIST_MAX_ROUNDS`，默认 `10`
- M57-6：标准语义与稳定字段：
  - LongLived 默认 defer；非空变更首次落地时 `persisted=false`，`pending_dirty_total>0`，`persist_report=null`。
  - `dirty∪deleted > 100` 或 `pending_rounds >= 10` 时触发持久化；`threshold` 为严格“ > ”边界。
  - one-shot 始终同步持久化（强制 `persist` 后 `install`），并通过 CLI one-shot 测试返回完整 `DiffRefreshReport`。
  - `pending` checkpoint 采用 `max(disk_checkpoint, pending_checkpoint)` 做 watermark，空轮次只在阈值/轮次未满足时跳过 durable 写入。
  - 持久化失败保留已 install 的 runtime 并保留 pending 状态，下一轮可重试；崩溃只能恢复 durable checkpoint，未持久化轮次可能丢失。
