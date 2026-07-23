# 里程碑与计划规范

## 里程碑 INDEX

权威表：[milestones/INDEX.md](../milestones/INDEX.md)

根目录 [CHECKLIST.md](../../CHECKLIST.md) 仅作指针，checkbox 以 INDEX 为准。

### INDEX 列

| 列 | 说明 |
|----|------|
| `id` | 如 M53 |
| `title` | 一行摘要 |
| `status` | `planned` / `active` / `done` / `cancelled` |
| `area` | performance / browser / formats / ai-eval |
| `journal` | journal 文件链接 |
| `spec` | 设计 spec（可空） |
| `plan` | 实施 plan（可空） |
| `depends` | 依赖的 M id |

### 编号

- 沿用 `MNN`；新工作优先续写当前 **active** journal（性能线现为 M53）
- 不在 INDEX 登记的 `mXX-*.md` 视为草稿，不得作为验收依据

## Spec 与 Plan

| 阶段 | 路径 | 状态 |
|------|------|------|
| 设计 | `docs/specs/YYYY-MM-DD-<topic>-design.md` | `draft` → `approved` |
| 计划 | `docs/plans/YYYY-MM-DD-<topic>-plan.md` | `draft` → `approved` → `done` |

**硬规则（M53 起）**：

- 新模块、性能主路径切换、CI 契约变更 → 须 `approved` spec
- 多 PR 工作 → 须 `approved` plan
- trivial 修复（typo、单测、注释）可豁免，但须在 PR 中说明

## 当前路线

地基优先（2026-07-17 确认）：

```
地基：M54 / M55 / M56 差量刷新一条龙（正确性基座）
    ＋ M58 廉价模型理解力评测 spec + 自动化 runner（验收门，并行启动）
收敛：M28 / M29 / M30 stdio/FC 契约收敛（接口面稳定后再接 MCP）
高楼：MCP adapter、token 级输出预算、M59 rpt/dash、语义层体系化
```

- M54–M56 保证 LongLived 会话中图数据新鲜正确（LLM 不被陈旧图误导）；formats rpt/dash 后推 M59。
- M58 是「支撑小端侧 LLM 理解元数据逻辑」的验收门：没有量化评测，任何理解力优化不可测。
- M46 复用 M54+ change-detect / orchestrator，不另起拉取栈。
- 性能线打到「正确、可验收」即停，不在已 warm 查询上继续恋战。

差量刷新 spec/plan：

- [2026-07-12-diff-refresh-pipeline-design.md](../specs/2026-07-12-diff-refresh-pipeline-design.md)
- [2026-07-12-diff-refresh-pipeline-plan.md](../plans/2026-07-12-diff-refresh-pipeline-plan.md)
- journal：[m54-diff-refresh-pipeline.md](../milestones/performance/m54-diff-refresh-pipeline.md)

M58 spec：

- [2026-07-17-cheap-model-comprehension-eval-design.md](../specs/2026-07-17-cheap-model-comprehension-eval-design.md)（draft）
- journal：[m58-cheap-model-comprehension-eval.md](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)
