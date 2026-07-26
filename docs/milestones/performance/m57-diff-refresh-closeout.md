# M57：Diff refresh 产品化与 tick 成本收口

> 状态：**active**（spec/plan approved；M57-0 TDD 进行中）  
> Spec：[2026-07-23-m57-diff-refresh-closeout-design.md](../../specs/2026-07-23-m57-diff-refresh-closeout-design.md)  
> Plan：[2026-07-26-m57-ai-contract-foundation-plan.md](../../plans/2026-07-26-m57-ai-contract-foundation-plan.md)（approved）  
> Depends：M56（`cnb/main` @ `b4ec419` 已合入）

## 目标

在 M54–M56 正确性基座上完成：tick/观测产品壳、**SKILL/帮助同步**、**页级刷新范围**、**凭证流**、派生索引增量、输出/打开打磨；并为 **M46** 固定「只包装不重写栈」的 handoff。

### M57-0：SLM 测评基础契约（先行）

- [ ] `DiffRefreshReport` 暴露稳定 `schema_version`、`kind`、`persist_report`。
- [ ] one-shot 与 stdio 输出共享报告字段；机器模式严格单行 JSON。
- [ ] 登录 401/403 保留脱敏后的 `message` / `error.message`，稳定错误码不变。
- [ ] TDD 覆盖非空刷新、空 bootstrap、失败回滚、单行输出和敏感信息边界。
- [ ] 与 M58 明确边界：M57 不实现 AnswerJudge、ModelAdapter、RunReport 或模型基线。

## 三阶段

| Phase | 焦点 | 状态 |
|-------|------|------|
| 1 | PersistReport、tick+Backoff、stdio 真机冒烟；**SKILL/`--help`/README**；**RefreshScope 自动探索+申明**；**凭证失效/重登/换绑**；M46 边界 | pending |
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
| M57-0 TDD | in progress |
| M57-0 code | pending |
| Phase 1–3 代码 | pending |

## 设计决策（2026-07-24）

- **路径确定性**：不做；e2e 继续 `stable_semantic_view`
- **零写盘 open**：降级；主线改为 LongLived `install` 优先 + 批量 persist
- **LongLived 可见性**：`install` 后立即可查；崩溃可丢未 persist 轮次
- **批量 persist**：`dirty∪deleted > 100` 或连续 10 轮未落盘；one-shot 每轮同步

## 验收记录

（实现后按 phase 追加；每个相关 commit 更新本节或上表）
