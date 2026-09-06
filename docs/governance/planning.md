# 里程碑与计划规范

## 里程碑 INDEX

权威表：[milestones/INDEX.md](../milestones/INDEX.md)

根目录 [CHECKLIST.md](../../CHECKLIST.md) 仅作指针，checkbox 以 INDEX 为准。

### INDEX 列

| 列 | 说明 |
|----|------|
| `id` | 如 M53 |
| `title` | 一行摘要 |
| `status` | `planned` / `active` / `done` / `closed`（范围收口，须列延期项） / `cancelled` |
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

地基优先（2026-09-06 更新）：

```
地基：M54 / M55 / M56 差量刷新（正确性基座，已合入）
收口：M57 产品化 + tick 成本 + SKILL/范围/凭证（含 M46 handoff；journal 随 commits 更新）
    ＋ M58 评测基础设施（已按范围关闭，延期验收移交 M59）
迁移：M59 身份/schema → 正确性契约 → Grafeo → 真实语料验收
收敛：M28 / M29 / M30 stdio/FC 契约收敛（接口面稳定后再接 MCP）
高楼：MCP adapter、token 级输出预算、rpt/dash（待排期）、语义层体系化
产品：M46 在 M57 Phase 1 面就绪后做 CLI 包装（depends M45, M57）
```

- M54–M56 保证 LongLived 会话中图数据新鲜正确；formats rpt/dash 待排期，M59 统一用于 Grafeo 迁移。
- M57：tick/观测壳、派生索引增量、输出打磨；**不**在本里程碑内做完 M46 CLI。
- M58：评测基础设施按范围关闭；模型业务理解力与扩充语料基线仍未获完整验收。
- M46：只包装 M54–M57 栈，不另起拉取栈；depends `M45, M57`。
- 性能线打到「正确、可验收」即停，不在已 warm 查询上继续恋战。

差量刷新：

- [2026-07-12-diff-refresh-pipeline-design.md](../specs/2026-07-12-diff-refresh-pipeline-design.md)
- [2026-07-12-diff-refresh-pipeline-plan.md](../plans/2026-07-12-diff-refresh-pipeline-plan.md)
- journal：[m54-diff-refresh-pipeline.md](../milestones/performance/m54-diff-refresh-pipeline.md)

M57：

- [2026-07-23-m57-diff-refresh-closeout-design.md](../specs/2026-07-23-m57-diff-refresh-closeout-design.md)（approved）
- [2026-07-26-m57-ai-contract-foundation-plan.md](../plans/2026-07-26-m57-ai-contract-foundation-plan.md)（approved）
- journal：[m57-diff-refresh-closeout.md](../milestones/performance/m57-diff-refresh-closeout.md)

M58：

- [2026-07-17-cheap-model-comprehension-eval-design.md](../specs/2026-07-17-cheap-model-comprehension-eval-design.md)（approved，历史 runner 已冻结）
- [2026-08-24-m58-3-command-surface-gap-fixes-design.md](../specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md)（closed，范围收口至 PR1/PR2）
- [2026-09-05-m58-3-condition-symbol-normalization-design.md](../specs/2026-09-05-m58-3-condition-symbol-normalization-design.md)（approved）
- [2026-09-04-local-query-plane-and-backend-research-plan.md](../plans/2026-09-04-local-query-plane-and-backend-research-plan.md)（approved）
- [2026-09-05-graph-backend-migration-plan-review.md](../plans/2026-09-05-graph-backend-migration-plan-review.md)（复核意见）

知识库（Issue #30）：

- [2026-09-06-knowledge-base-corpus-design.md](../specs/2026-09-06-knowledge-base-corpus-design.md)（draft）
- 语料目录：[knowledge/README.md](../knowledge/README.md)；验收：[knowledge-acceptance-questions.md](../knowledge/knowledge-acceptance-questions.md)

M59（redb → Grafeo 迁移）：

- [2026-09-05-grafeo-backend-migration-design.md](../specs/2026-09-05-grafeo-backend-migration-design.md)（approved）
- 实测：[2026-09-05-grafeo-spike-measurements.md](../ai-eval-runs/2026-09-05-grafeo-spike-measurements.md)
- plan：[2026-09-06-m59-grafeo-implementation-plan.md](../plans/2026-09-06-m59-grafeo-implementation-plan.md)（approved）
- journal：[m59-grafeo-backend-migration.md](../milestones/performance/m59-grafeo-backend-migration.md)
