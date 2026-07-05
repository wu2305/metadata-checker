# M19：页面数据可用性摘要

| milestone | M19 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M19：页面数据可用性摘要

### 目标

从条件依赖图生成低噪声页面级摘要，让 AI 不读 raw `.spg` 也能理解页面数据正常显示的主要前置条件。

### 问题清单

- `5.4-mini` 需要手动归纳 `param5 + text41 + input3 + $user.dept_id + model*.totalRowCount__`。
- 当前 PageLogic 的 `data_source_count`、`entrypoint_count`、`write_target_count` 不能直接说明“页面为什么没数据”。
- `visibleCondition`、`disableCondition` 和数据源过滤条件没有按影响范围排序。
- M18 已能建出条件图和物理表写入归并，但 `--context comp:...|input3 --depth 2` 会访问数百个邻近节点，混入同名局部模型、旁路 DataFlow、其他页面组件等低相关关系，容易让小模型从主链路漂移。

### 工作清单

- 聚合页面级关键输入：入口参数、系统变量、隐藏计算字段、用户输入字段。
- 聚合主要数据源过滤条件。
- 聚合主要显示门控：`visibleCondition`。
- 聚合主要交互门控：`disableCondition` 和 enabled condition。
- 按影响范围排序：影响模型数量、影响组件数量、是否影响入口/面板/按钮。
- 输出 `summary.display_prerequisites`、`summary.data_prerequisites`、`summary.action_prerequisites`。
- 从 M18 的条件图中抽取 `primary_paths` / `key_paths`，优先展示与目标页面、目标组件、目标物理表直接相关的链路，例如 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- 对跨页面或同名局部模型扩散出的旁路关系降级到 `related_context`，并在 summary 中只给计数和升级建议，不默认展开。

### 验收目标

- AI 只读 summary 就能说出页面数据正常显示的主要条件。
- compact 模式下输出 Top-N 条件，normal/full 模式可展开完整条件列表。
- 对 `OUTPUT_TRUNCATED` 场景给出明确升级建议。
- 小模型默认读取 summary 时能优先复述主链路，不会把同名局部模型的其他页面引用、DataFlow 旁路或低相关组件误认为页面正常显示的必要条件。
