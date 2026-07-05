# M15：真实项目 AI 评测集扩展

| milestone | M15 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M15：真实项目 AI 评测集扩展

### 目标

把真实项目中已暴露的误判模式纳入自动评测，确保后续优化不是只在人工检查中成立。

### 问题清单

- 当前 AI eval 主要覆盖 fixtures，真实项目问题没有形成长期回归。
- 缺少空上下文小模型对真实项目的固定任务验证。
- 缺少文本统计/仪表盘组件误判为入口的测试。
- 缺少 `.tbl` 单文件真实输出测试。
- 缺少 graphdb 不存在、只读、锁冲突的行为测试。
- 缺少 `next_queries` shell 安全测试，无法防止未加引号的 `|` 回归。
- 缺少针对 `fact_saleContract` 这类高扇出模型的摘要质量测试。

### 验收目标（M15 已收敛）

- 在不复制整个真实项目的前提下，建立真实项目选择清单，引用 `xiaoshouyi` 中固定文件和目标。
- 每个真实项目评测 case 包含问题、命令计划、期望摘要、禁止误判、证据要求。
- 空上下文 `5.4-mini` eval 至少覆盖：页面用途、按钮行为、文本统计、模型被 DataFlow 消费、`.tbl` 单文件理解。
- 自动测试覆盖 graphdb 缺失和锁冲突的 JSON diagnostic。
- 自动测试覆盖 `next_queries` 中 target 引号。
- 高扇出模型输出必须验证 `consumed_by_dataflow_count` 出现在 summary。
