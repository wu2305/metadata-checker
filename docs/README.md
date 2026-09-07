# metadata-checker 文档入口

> 所有文档、里程碑与计划的单一导航。`docs/` 根目录旧路径保留 stub 一周期；以本页链接为准。

## 治理与协作

| 文档 | 用途 |
|------|------|
| [workflow.md](governance/workflow.md) | 分支、PR、`main` 只接受 merge |
| [planning.md](governance/planning.md) | 里程碑 INDEX、spec/plan 流程 |
| [documentation.md](governance/documentation.md) | 四类文档放哪、如何归档 |
| [pr-template.md](governance/pr-template.md) | PR 描述模板 |

## 当前活跃里程碑

| ID | 区域 | 状态 | Journal |
|----|------|------|---------|
| M57 | performance（产品化 + tick 成本；含 M46 handoff） | done | [m57](milestones/performance/m57-diff-refresh-closeout.md) / [design](specs/2026-07-23-m57-diff-refresh-closeout-design.md) / [plan](plans/2026-07-26-m57-ai-contract-foundation-plan.md) |
| M54–M56 | performance（差量刷新） | done | [m54-diff-refresh-pipeline.md](milestones/performance/m54-diff-refresh-pipeline.md) |
| patch | performance（产品路径/init/warm） | active | [design](specs/2026-07-13-perf-product-path-init-warm-patch-design.md) / [plan](plans/2026-07-13-perf-product-path-init-warm-patch-plan.md) |
| M53 | performance | done | [m53-performance-continuation.md](milestones/performance/m53-performance-continuation.md) |
| M28–M30 | stdio / FC | planned | [INDEX.md](milestones/INDEX.md) |
| M46 | browser（CLI 包装；depends M57） | planned | [archive](archive/roadmap/m46-local-cli-remote-metadata-index-and.md) |
| M47–M48 | browser | planned | [INDEX.md](milestones/INDEX.md) |
| M58 | ai-eval（理解力评测闭环） | closed（范围收口） | [m58](milestones/ai-eval/m58-cheap-model-comprehension-eval.md) |
| M59 | performance（Grafeo 后端迁移） | active | [m59](milestones/performance/m59-grafeo-backend-migration.md) / [plan](plans/2026-09-06-m59-grafeo-implementation-plan.md) |
| 待排期 | formats（rpt/dash，原 M59 占位） | planned | [INDEX.md](milestones/INDEX.md) |

完整表见 [milestones/INDEX.md](milestones/INDEX.md)。

## 目录（终态 / 迁移中）

| 目录 | 内容 |
|------|------|
| [reference/](reference/) | 活契约：schema、stdio、ai-eval |
| [milestones/](milestones/) | 里程碑 journal 与 [performance-baseline](milestones/performance/performance-baseline.md) |
| [specs/](specs/) | 实现前设计（须 `approved` 再编码） |
| [plans/](plans/) | 实现计划 |
| [runbooks/](runbooks/) | 真实 BI / 环境操作手册，含 [CNB 远程编译与排错测试](runbooks/cnb-remote-dev-env.md) |
| [archive/](archive/) | 已完成 roadmap 章节（[roadmap 索引](archive/roadmap/README.md)） |
| [knowledge/](knowledge/) | 知识库语料（唯一被索引的白名单目录，[说明](knowledge/README.md)） |
| [knowledge-acceptance/](knowledge-acceptance/) | 知识库验收集（问题与答案键，**不入库**，[说明](knowledge-acceptance/README.md)） |

## 设计与计划（Phase C）

- Spec：[specs/2026-07-05-docs-governance-reorg-design.md](specs/2026-07-05-docs-governance-reorg-design.md)
- Plan：[plans/2026-07-05-docs-governance-reorg-plan.md](plans/2026-07-05-docs-governance-reorg-plan.md)

## 知识库

CNB 知识库只索引 `docs/knowledge/**`、`AGENTS.md`、`SKILL.md`（白名单，关闭 Issue 同步）。
写入 `docs/knowledge/` 前先读 [knowledge/README.md](knowledge/README.md) 的语料规范
（状态分层、源码依据、自包含、否定结论、脱敏）。

- 设计规范：[specs/2026-09-06-knowledge-base-corpus-design.md](specs/2026-09-06-knowledge-base-corpus-design.md)
- 验收问题集（不入库）：[knowledge-acceptance/knowledge-acceptance-questions.md](knowledge-acceptance/knowledge-acceptance-questions.md)

## 根目录其它文档

- [AGENTS.md](../AGENTS.md) — Agent 代码规范
- [CHECKLIST.md](../CHECKLIST.md) — 指向 INDEX 的简短指针
- [SKILL.md](../SKILL.md) — AI 使用协议
