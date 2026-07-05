# M13：面向 AI 的低噪声 brief 输出模式

| milestone | M13 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M13：面向 AI 的低噪声 brief 输出模式

### 目标

降低大页面、大模型查询输出体积，让小模型优先抓住结论、风险和下一步，而不是陷入 details/evidence 大数组。

### 问题清单

- `query-page-logic` 在真实页面上同时展开 `action_flows`、`visibility_rules`、`data_sources`、`evidence`，输出过长。
- `EVIDENCE_SAMPLED` 提示存在，但 AI 仍需在 details 与 evidence 之间自行判断覆盖范围。
- `query-model fact_saleContract` 输出 52 个 `consumed_by_dataflows`，summary 只说读写为 0，模型容易忽略 DataFlow 消费方。
- `details` 中同类长列表缺少分组、排序依据、Top-N 摘要与剩余计数。
- M10 遗留观察：单文件 `.tbl` / DataFlow 的 `details.fields`、`details.field_lineage`、`evidence` 在真实 DataFlow 上仍可能很长，低噪声预算不能只覆盖项目级查询，也要覆盖单文件 Table/DataFlow 输出。
- `--budget compact|normal|full` 已存在，但不是所有查询都能体现清晰预算语义。
- 默认 `SuperPage` 输出没有 `page_role/what_is_it`，AI 要从数据源和表达式中自行归纳页面用途。
- `--priority` 在真实页面上对 AI 的增量信息不明显，容易让模型白跑命令。

### 验收目标（M15 已收敛）

- CLI 增加或强化 `--budget compact`/`--ai-brief`，输出只包含 `summary`、`key_findings`、`risk_diagnostics`、`evidence_summary`、`next_queries`。
- 长列表输出必须包含 `total_count`、`shown_count`、`truncated`、`remaining_count`。
- 单文件 `.tbl` / DataFlow 输出必须支持低噪声摘要，默认避免让 AI 直接读取完整字段和 evidence 大数组。
- `query-model` summary 增加 DataFlow 角色计数，如 `consumed_by_dataflow_count`、`produced_by_count`。
- `query-page-logic` summary 增加 `top_entrypoints`、`top_writes`、`top_navigation` 的低噪声摘要。
- 默认单页输出增加 `what_is_it` 与 `page_role`，不要求项目图也能给出保守页面意图。
- `--priority` 若无有效优先级发现，应输出明确 `NO_PRIORITY_RULES` 或 `priority_rule_count=0`。
