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

```
Phase C  文档治理 + CI 基线收口
    ↓
Phase B  M53 persist / materialized facts / path finder
    ↓
Phase A  M53 redb v2 hydrate
```

Phase B/A 各需独立 spec，见 [m53-performance-continuation.md](../m53-performance-continuation.md)。
