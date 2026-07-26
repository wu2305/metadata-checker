# M57：Diff refresh 产品化与 tick 成本收口

> 状态：**active**（spec/plan approved；Phase 1 进行中）
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
| 1 | PersistReport、tick+Backoff、stdio 真机冒烟；**SKILL/`--help`/README**；**RefreshScope 自动探索+申明**；**凭证失效/重登/换绑**；M46 边界 | in progress |
| 2 | Dense/Facts/PageDep 增量更新 + 规模曲线 | pending |
| 3 | LongLived 内存优先 persist（阈值 100 + 10 轮兜底）、失败恢复 e2e | pending |

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
| M57-4 DiffRefresh scope TDD | in progress |
| Phase 1–3 代码 | pending |

## 设计决策（2026-07-24）

- **路径确定性**：不做；e2e 继续 `stable_semantic_view`
- **零写盘 open**：降级；主线改为 LongLived `install` 优先 + 批量 persist
- **LongLived 可见性**：`install` 后立即可查；崩溃可丢未 persist 轮次
- **批量 persist**：`dirty∪deleted > 100` 或连续 10 轮未落盘；one-shot 每轮同步

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
- M57-4：已先写主 `DiffRefreshReport` scope contract TDD；当前红灯等待共享报告暴露 `RefreshScope`。
- 真实项目 stdio 冒烟：已尝试运行既有 ignored 用例；首次索引报告 `1329 files / 78127 dirty`，超过约 5 分钟未产生终态，已中止，暂不计入通过证据。
