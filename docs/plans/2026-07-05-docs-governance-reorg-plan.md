# 文档治理与目录重组实施计划

> **For agentic workers:** 按 PR-C1 → PR-C2 → PR-C3 顺序执行；每 PR 独立 merge，禁止直推 `main`。

**Goal:** 统一文档入口、治理规范与里程碑 INDEX；拆分巨型 roadmap；记录 `release-fast` CI 基线。

**Architecture:** 三 PR 渐进迁移（骨架 → 文件搬迁+stub → roadmap 拆分）；旧路径 stub 保留一周期。

**Tech Stack:** Markdown、git mv、rg 链接检查、CNB PR。

**Spec:** [docs/specs/2026-07-05-docs-governance-reorg-design.md](../specs/2026-07-05-docs-governance-reorg-design.md)

---

## PR-C1：骨架 + 治理

- [x] `docs/README.md`、`docs/governance/*`、`docs/milestones/INDEX.md`
- [x] `docs/plans/` 与本计划
- [x] `AGENTS.md` 文档与计划节
- [x] `performance-baseline.md` 记录 `cnb-078`
- [x] `CHECKLIST.md` 瘦身 + `archive/checklist-p0-p4.md`
- [x] PR merge + 删来源分支

## PR-C2：reference + milestones 搬迁

- [x] `git mv` → `docs/reference/`、`docs/milestones/performance/`、`docs/milestones/browser/`、`docs/runbooks/`
- [x] 旧路径 stub
- [x] 更新测试与文档链接（`regression_tests.rs`、`INDEX`、`AGENTS`、`README`）
- [x] PR merge + 删来源分支

## PR-C3：roadmap 拆分

- [x] `tools/split-roadmap-archive.py` 拆到 `docs/archive/roadmap/`（43 节 + README）
- [x] `real-project-optimization-roadmap.md` stub
- [x] `INDEX` / `CHECKLIST` / `AGENTS` 链接更新
- [ ] PR merge + 删来源分支
