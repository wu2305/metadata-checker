# 里程碑 INDEX

> 权威状态表。根目录 [CHECKLIST.md](../../CHECKLIST.md) 仅作指针。  
> M10–M47 历史章节见 [archive/roadmap/README.md](../archive/roadmap/README.md)。

## AI / Stdio 线（M10–M30）

| id | title | status | area | journal | spec | plan | depends |
|----|-------|--------|------|---------|------|------|---------|
| M10 | 表与 DataFlow 单文件输出可理解 | done | ai-eval | [archive](../archive/roadmap/m10-dataflow.md) | | | |
| M11 | 图数据库路径、只读与并发 | done | ai-eval | [archive](../archive/roadmap/m11-archive.md) | | | |
| M12 | Explain 语义摘要与证据质量 | done | ai-eval | [archive](../archive/roadmap/m12-explain.md) | | | |
| M13 | 面向 AI 的低噪声 brief 输出 | done | ai-eval | [archive](../archive/roadmap/m13-ai-brief.md) | | | |
| M14 | 目标定位与命令规范防错 | done | ai-eval | [archive](../archive/roadmap/m14-archive.md) | | | |
| M15 | 真实项目 AI 评测集扩展 | done | ai-eval | [archive](../archive/roadmap/m15-ai.md) | | | |
| M16 | SKILL.md 真实使用协议收敛 | done | ai-eval | [archive](../archive/roadmap/m16-skill-md.md) | | | |
| M17 | 条件抽取基础层 | done | ai-eval | [archive](../archive/roadmap/m17-archive.md) | | | |
| M18 | 条件依赖图 | done | ai-eval | [archive](../archive/roadmap/m18-archive.md) | | | |
| M19 | 页面数据可用性摘要 | done | ai-eval | [archive](../archive/roadmap/m19-archive.md) | | | |
| M19-FIX | 可扩展主链路计算框架 | done | ai-eval | [archive](../archive/roadmap/m19-fix-archive.md) | | | |
| M20 | Why 条件查询 | done | ai-eval | [archive](../archive/roadmap/m20-why.md) | | | |
| M21 | 未知 Action 语义归类 | done | ai-eval | [archive](../archive/roadmap/m21-action.md) | | | |
| M22 | 条件类 AI 评测与协议收敛 | done | ai-eval | [archive](../archive/roadmap/m22-ai.md) | | | |
| M23 | Hot Graph Runtime 基座 | done | ai-eval | [archive](../archive/roadmap/m23-hot-graph-runtime.md) | | | |
| M24 | Stdio Function Calling Server | done | ai-eval | [archive](../archive/roadmap/m24-stdio-function-calling-server.md) | | | |
| M25 | Runtime Cache 与 Reload | done | ai-eval | [archive](../archive/roadmap/m25-runtime-cache-reload.md) | | | |
| M26 | Skill / FC 接入与性能验收 | done | ai-eval | [archive](../archive/roadmap/m26-skill-function-calling.md) | | | |
| M27 | Stdio 查询命令面扩展 | done | ai-eval | [archive](../archive/roadmap/m27-stdio.md) | | | |
| M28 | Stdio 请求/响应契约收敛 | planned | ai-eval | [archive](../archive/roadmap/m28-stdio.md) | | | M27 |
| M29 | Function Calling 工具层优化 | planned | ai-eval | [archive](../archive/roadmap/m29-function-calling.md) | | | M28 |
| M30 | Stdio 性能与容量治理 | planned | ai-eval | [archive](../archive/roadmap/m30-stdio.md) | | | M28 |

## Browser 线（M40–M48）

| id | title | status | area | journal | spec | plan | depends |
|----|-------|--------|------|---------|------|------|---------|
| M40 | 真实 BI 环境测试 | done | browser | [runbook](../runbooks/m40-real-bi-environment-runbook.md) | | | |
| M41 | 远程元数据与 Session | done | browser | [m41-plan](browser/m41-plan.md) | | [m41-plan](browser/m41-plan.md) | M40 |
| M42 | Real BI Smoke 与性能记录 | done | browser | [runbook](../runbooks/m42-real-bi-smoke-and-performance-runbook.md) | | | M41 |
| M43 | Browser Extension 验收 | done | browser | [runbook](../runbooks/m43-browser-extension-runbook.md) | | | M42 |
| M45 | Remote Metadata 自动拉取与后台分析 | done | browser | [handoff](browser/m45-real-bi-test-handoff.md) | | | M43 |
| M46 | Local CLI Remote Metadata Index | planned | browser | [archive](../archive/roadmap/m46-local-cli-remote-metadata-index-and.md) | | | M45, M57 |
| M47 | Browser UI 正式化 | planned | browser | [popup 设计](browser/m47-embedded-local-graph-popup-design.md) | | [pixi spike](browser/m47-pixi-d3-force-spike-plan.md) | M46 |
| M48 | Local Graph 交互优化 | planned | browser | [m48](browser/m48-local-graph-interaction-optimization.md) | | | M47 |

## Performance 线（M50–M57）

| id | title | status | area | journal | spec | plan | depends |
|----|-------|--------|------|---------|------|------|---------|
| M50 | 性能基线与 Criterion / Bencher 契约 | done | performance | [contract](performance/m50-performance-bench-contract.md) | | [offscreen plan](performance/m50-browser-offscreen-bench-plan.md) | |
| M51 | 性能堵点归因 | done | performance | [diagnosis](performance/m51-performance-diagnosis.md) | | | M50 |
| M52 | 性能优化落地（cost model / warm / fragment） | done | performance | [m52](performance/m52-performance-optimization.md) | | | M51 |
| M53 | persist / facts / path + redb v2 hydrate | done | performance | [m53](performance/m53-performance-continuation.md) | [design](../specs/2026-07-06-m53-performance-design.md) | [plan](../plans/2026-07-06-m53-performance-plan.md) | M52 |
| M54 | Diff refresh 通路（fixture META_FILES → rewarm） | done | performance | [m54](performance/m54-diff-refresh-pipeline.md) | [design](../specs/2026-07-12-diff-refresh-pipeline-design.md) | [plan](../plans/2026-07-12-diff-refresh-pipeline-plan.md) | M53 |
| M55 | Diff refresh 正确性（真 META_FILES + 依赖索引） | done | performance | [m54](performance/m54-diff-refresh-pipeline.md) | [design](../specs/2026-07-12-diff-refresh-pipeline-design.md) | [plan](../plans/2026-07-12-diff-refresh-pipeline-plan.md) | M54 |
| M56 | Diff refresh persist（redb_commit / commit batch） | done | performance | [m54](performance/m54-diff-refresh-pipeline.md) | [design](../specs/2026-07-12-diff-refresh-pipeline-design.md) | [plan](../plans/2026-07-12-diff-refresh-pipeline-plan.md) | M54 |
| M57 | Diff refresh 产品化 + tick 成本 + 范围/凭证/SKILL | active | performance | [m57](performance/m57-diff-refresh-closeout.md) | [design](../specs/2026-07-23-m57-diff-refresh-closeout-design.md)（approved） | [plan](../plans/2026-07-26-m57-ai-contract-foundation-plan.md)（approved） | M56 |

CI 基线：[performance-baseline.md](performance/performance-baseline.md)（`release-fast` + `cnb-078`）。

> M54–M56 共用同一 spec/plan/journal，已合入 `main`。**M57** 做产品壳 / 派生索引增量 / 输出打磨，并为 **M46** 提供可包装编排面（M46 不另起拉取栈）。M57 可与 **M58** 并行。

## Formats / AI-eval（M58–M59）

| id | title | status | area | journal | spec | plan | depends |
|----|-------|--------|------|---------|------|------|---------|
| M58 | 廉价模型理解力评测 | active | ai-eval | [m58](ai-eval/m58-cheap-model-comprehension-eval.md) | [design](../specs/2026-07-17-cheap-model-comprehension-eval-design.md)（approved；JSON-command runner 已冻结）<br>[design M58.2](../specs/2026-08-09-slm-eval-harness-design.md)（draft；agent harness，当前路径） | [plan](../plans/2026-07-27-m58-cnb-ai-chat-runner-plan.md)（done）<br>[runbook](../runbooks/m58-cnb-shell-dry-run.md)（CNB stage 本地 dash 干跑） | M22 |
| M59 | Dashboard / Report 格式支持 | planned | formats | — | （待建） | | |

> 原登记为 M56 的 Dashboard/Report（rpt/dash）已 **后推为 M59**。  
> 路线（2026-07-23 更新）：M54–M56 done → **M57**（产品化+tick 成本，含 M46 handoff）与 **M58** 并行 → M28–M30 → MCP / token / M59 / 语义层。

## 文档治理（Phase C）

| id | title | status | area | journal | spec | plan | depends |
|----|-------|--------|------|---------|------|------|---------|
| Phase C | 文档目录重组与协作规范 | done | governance | [docs README](../README.md) | [design](../specs/2026-07-05-docs-governance-reorg-design.md) | [plan](../plans/2026-07-05-docs-governance-reorg-plan.md) | |
