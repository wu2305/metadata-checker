# 文档治理与目录重组设计

> 状态：approved（用户确认 2026-07-05）  
> 范围：Phase C — 工程稳定、文档统一、协作规范；不含静态站点；不含 M53 B/A 实现。

## 背景

- `main` 近期为验证 CI 曾直接 push；**今后 `main` 只接受 PR**，merge 后删除来源分支。
- `docs/` 现有 25+ 个扁平 markdown，reference / milestone / runbook / design 混杂，无单一入口。
- `CHECKLIST.md` 与 `docs/real-project-optimization-roadmap.md`（5800+ 行）职责重叠。
- CI 已在 `f1e7285` / `cnb-078` 全绿；`release-fast` criterion 基线需正式记录。
- 后续路线：**C（本设计）→ M53 B（persist / facts / path）→ M53 A（redb v2 hydrate）**。

## 目标

1. 建立**单一文档入口**与分层目录，Agent/新人 30 秒内能找到 reference、milestone、spec、plan、runbook。
2. 建立**文档 + 计划规范**：四类文档、状态机、里程碑 INDEX、PR 必填项。
3. **拆分** `real-project-optimization-roadmap.md` 已完成章节到 archive，主文件退役为索引或 stub。
4. 收口分支卫生、CI 基线记录、`AGENTS.md` / `SKILL.md` 引用。
5. 为 M53 B/A 预留 `docs/specs/` + `docs/plans/` 工作流。

## 非目标

- 静态站点 / MkDocs / VitePress（后续里程碑）。
- M53 性能实现（属 Phase B/A，需独立 spec/plan）。
- 修改 CNB 平台分支保护规则本身（文档约定 + 人工遵守；若平台支持再补配置）。

## 终态目录结构

```
docs/
├── README.md                      # 唯一入口：导航 + 当前活跃里程碑
├── governance/
│   ├── workflow.md                # 分支、PR、禁止直推 main、删来源分支
│   ├── planning.md                # 里程碑编号、INDEX、spec/plan 流程
│   └── documentation.md           # 四类文档、命名、归档规则
├── reference/                     # 活契约（随代码变更同步）
│   ├── schema.md
│   ├── stdio-server.md
│   ├── function-calling-runtime.md
│   ├── action-semantics.md
│   ├── ai-eval.md
│   ├── ai-eval-run-template.md
│   └── corpus-manifest.md
├── milestones/
│   ├── INDEX.md                   # 全里程碑表（CHECKLIST 的权威替代）
│   ├── performance/
│   │   ├── performance-baseline.md
│   │   ├── m50-browser-offscreen-bench-plan.md
│   │   ├── m50-performance-bench-contract.md
│   │   ├── m51-performance-diagnosis.md
│   │   ├── m52-performance-optimization.md
│   │   └── m53-performance-continuation.md   # 当前活跃性能线
│   ├── browser/
│   │   └── m40–m48 系列（自原 docs/ 迁入）
│   ├── formats/
│   │   └── m56-dashboard-report-support-design.md
│   │   └── m58-cheap-model-understanding-eval-design.md（若存在）
│   └── ai-eval/                   # 预留；可与 reference/ai-eval 交叉链接
├── specs/                         # 实现前设计（brainstorming → approved）
│   └── YYYY-MM-DD-<topic>-design.md
├── plans/                         # 实现计划（writing-plans → approved）
│   └── YYYY-MM-DD-<topic>-plan.md
├── runbooks/                      # 人工操作手册
│   └── m40/m41/m42/m43/m45 等 runbook
└── archive/
    ├── roadmap/                   # 自 real-project-optimization-roadmap 拆出
    │   ├── README.md              # 原 roadmap 索引（M10–M47 链接表）
    │   └── mNN-<slug>.md          # 每个已完成里程碑一节
    └── superseded/                # 被新设计替代的旧 spec
```

根目录保留：

- `AGENTS.md` — 增加「文档与计划」节，指向 `docs/README.md`。
- `CHECKLIST.md` — **瘦身为指针**（≤30 行），checkbox 迁移到 `docs/milestones/INDEX.md` 或删除重复项仅留链接。
- `README.md` — 项目简介 + 链接 `docs/README.md`。

## 文档四类与状态机

| 类型 | 路径 | 何时创建 | 允许状态 |
|------|------|----------|----------|
| Reference | `docs/reference/` | 协议/对外契约变更 | 持续维护 |
| Milestone journal | `docs/milestones/<area>/` | 每个 M 一条工作线 | `active` → `done` → 迁入 `archive/` |
| Spec | `docs/specs/` | **编码前** | `draft` → `approved` → `superseded` |
| Plan | `docs/plans/` | spec `approved` 后 | `draft` → `approved` → `done` |
| Runbook | `docs/runbooks/` | 需人工步骤验收时 | 随环境更新 |

**硬规则（自 M53 起）**：

- 非 trivial 改动（新模块、性能路径切换、CI 契约变更）必须先有 `approved` spec；multi-PR 工作还需 `approved` plan。
- 每个 feature PR 在描述中列出：变更文档路径、测试命令、是否影响 `performance-baseline.md`。

## 里程碑 INDEX 字段

`docs/milestones/INDEX.md` 表格列：

| 列 | 说明 |
|----|------|
| `id` | 如 M53 |
| `title` | 一行摘要 |
| `status` | `planned` / `active` / `done` / `cancelled` |
| `area` | performance / browser / formats / ai-eval |
| `journal` | 链接到 journal 文件 |
| `spec` | 链接（可空） |
| `plan` | 链接（可空） |
| `depends` | 依赖的其他 M id |

`CHECKLIST.md` 中 M10+ 的 checkbox **迁移**到 INDEX；P0–P4 历史项可归档到 `archive/checklist-p0-p4.md` 或 INDEX 脚注。

## Git / PR 规范（`governance/workflow.md`）

```
分支命名：codex/<topic> 或 feat/<topic>
禁止：git push 到 main（含 CI 验证；用 PR + merge 触发）
合并：PR merge 后删除远程来源分支
本地：长期跟踪 main 的分支仅 main；feature 分支从 main 拉出
```

PR 描述模板（CNB PR body，可放 `docs/governance/pr-template.md`）：

- Summary（1–3 句）
- Test plan（命令列表）
- Docs（变更的 docs 路径）
- Baseline impact（yes/no；若 yes 说明哪条 CI build sn）

## `real-project-optimization-roadmap.md` 拆分策略

**原则**：不继续往单文件追加；已完成里程碑迁入 `docs/archive/roadmap/`。

**步骤**：

1. 解析原文件 `## MNN：` 标题，按 M 号切分为独立文件 `archive/roadmap/mNN-<slug>.md`。
2. 保留每节原文（含状态、问题清单、验收目标），文首加 YAML 或表格元数据：`milestone`, `status`, `archived_from`。
3. 新建 `archive/roadmap/README.md`：M10–M47（及已收敛项）索引表，链到各 `mNN-*.md`。
4. 原路径 `docs/real-project-optimization-roadmap.md` 替换为 **stub**（≤20 行）：

   ```markdown
   # 已迁移
   本文件已拆分至 docs/archive/roadmap/README.md
   ```

5. 全局 `grep docs/real-project-optimization-roadmap` 更新链接；`CHECKLIST.md` 指向 INDEX 或 archive README。

**不拆**：`milestones/performance/m5x-*.md` 已是独立 journal 的文件保持原位，仅在 INDEX 交叉引用。

## 迁移策略：两阶段（方案 B）

### PR-C1：骨架 + 治理（不搬大文件）

- 创建 `docs/README.md`、`governance/*`、`milestones/INDEX.md`（从 CHECKLIST 导入 M 表）。
- 创建 `specs/`、`plans/` 目录与本 design 文件。
- 更新 `AGENTS.md`（文档与计划 + PR 规则摘要）。
- 添加 `governance/pr-template.md`。
- 记录 `cnb-078` 基线到 `performance-baseline.md`（若尚未完整）。
- 删除/归档陈旧远程分支（文档记录命令，PR 描述列出已删分支名）。

### PR-C2：reference + milestones 搬迁

- `git mv` reference 类文件到 `docs/reference/`。
- `git mv` m50–m53、performance-baseline 到 `docs/milestones/performance/`。
- `git mv` browser / runbook 到 `docs/runbooks/` 或 `docs/milestones/browser/`（runbook 与 journal 分离：操作步骤 → runbooks，设计记录 → milestones）。
- 旧路径留 stub 一个 release 周期（指向新路径）。
- 更新 `SKILL.md`、测试、文档内链接。

### PR-C3：roadmap 拆分

- 脚本或半自动按 `## M` 标题拆分 `real-project-optimization-roadmap.md`。
- 生成 `archive/roadmap/*` + README 索引。
- 原文件改 stub；CHECKLIST 瘦身。
- `cargo test` / 文档链接检查（`rg` 无残留断链）。

每 PR 独立可 merge，顺序 C1 → C2 → C3。

## 分支清理（Phase C 附带）

| 分支 | 动作 |
|------|------|
| `cnb/codex/m52-redb-v2-and-ci` | merge 差异已在 main 则删除远程 |
| `cnb/codex/m52-performance-optimization` | 若已 superseded 则删除 |
| 本地 `master` | 对齐 `cnb/main` 或删除本地跟踪 |
| `[gone]` 本地分支 | `git branch -d` 清理 |

## Phase C 验收标准

- [ ] `docs/README.md` 存在且链到 governance / milestones / reference / specs / plans。
- [ ] `governance/workflow.md` 明确禁止直推 main。
- [ ] `milestones/INDEX.md` 含 M28–M58 与 performance M50–M53 状态。
- [ ] `real-project-optimization-roadmap.md` 已 stub；M10–M47 内容在 `archive/roadmap/` 可检索。
- [ ] `AGENTS.md` 指向新入口；无未解释的 `docs/m52-*.md` 裸路径（除 stub）。
- [ ] `cnb-078` 基线条目含 commit、wall time、五路 criterion 分项说明、`release-fast` 标注。
- [ ] 至少一次按新规范走完整 PR（可用 C1 自身示范）。

## Phase C → B → A 衔接

| 后续 | 前置 | 下一文档 |
|------|------|----------|
| **M53-B1** persist 增量或 materialized facts（择一先做） | C 完成 + approved spec | `docs/specs/2026-07-0x-m53-b1-*.md` |
| **M53-B2** path finder / 另一 B 项 | B1 merge | 更新 `m53-performance-continuation.md` |
| **M53-A** redb v2 hydrate | B1 persist 基础更稳 | 独立 spec + shadow 测试延续 |

## 风险与缓解

| 风险 | 缓解 |
|------|------|
| 大量断链 | stub 一周期 + PR-C3 前 `rg` 全仓 |
| PR-C3 diff 巨大 | 拆分纯移动与纯生成拆分两个 commit |
| Agent 仍读旧路径 | `AGENTS.md` + stub 顶部加粗警告 |
| CHECKLIST 与 INDEX 双维护 | CHECKLIST 仅保留指针 |

## 决策记录

| 日期 | 决策 |
|------|------|
| 2026-07-05 | 大力度目录重组；两阶段迁移（方案 B） |
| 2026-07-05 | `real-project-optimization-roadmap` **拆内容**归档，不做静态站点 |
| 2026-07-05 | `main` 仅 PR；merge 删来源分支 |
| 2026-07-05 | 路线 C → B → A |
