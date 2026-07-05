# M18：条件依赖图

| milestone | M18 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M18：条件依赖图

### 目标

把条件表达式与上游数据来源、下游组件状态串起来，形成可查询的条件依赖路径。

### 问题清单

- 当前工具无法直接表达 `param5 -> model10.filter -> model10.totalRowCount__ -> panel13.visibleCondition` 这类链路。
- 隐藏计算字段如 `input3.exp = model22.phoneNumber` 会成为条件中间层，但 PageLogic 摘要不会显式串联。
- 模型过滤条件与 `model*.totalRowCount__` 的显示门控之间缺少结构化关系。
- 页面局部模型写入没有归并到物理表字段：例如 `潜客信息跟进.spg` 的 `model6.phoneNumber` 实际写入 `$DATA:/主数据/fact_qwSidebar.tbl` 的 `phoneNumber`，但 `query-model model:fact_qwSidebar` 无法发现该跨页面 writer，导致 `合同协议.spg` 中 `input3 = model22.phoneNumber` 的生成链断在物理表读取处。

### 工作清单

- 建立 `param/input/text/model/system_user` 到 `condition` 的依赖边。
- 将隐藏计算字段纳入依赖链，例如 `input3.exp`、`text41.value`、`input8.exp`。
- 将 `model.filter` 与 `model.totalRowCount__` 建立关系。
- 将 `visibleCondition/disableCondition` 与组件显示/禁用状态建立关系。
- 输出条件依赖路径，保留每段 path 的 evidence。
- 建立 `.spg` 页面内 `sources[]` 局部模型到真实 `$DATA:/.../*.tbl` 物理表的别名映射，覆盖大小写、相对路径和中文目录。
- 在 `submitData/updateData/insertData/deleteData` 等数据写入 action 中，同时保留页面局部模型写入边，并补充归并后的物理表字段写入边。
- 物理表字段写入边必须携带原始 action 证据，包括 `source_file`、`json_path`、`component_id`、`action_id`、局部 `dataSet` 和归并后的物理表字段。
- `query-model <physical_table>`、`--explain action:...`、`--query-page-logic <page>` 需要能从不同入口看到同一条写入事实，且不能把 `<graph-edge-derived>` 当作唯一强证据。
- 增加最小 fixture：一个页面通过局部 `model6` 写入 `$DATA:/主数据/fact_qwSidebar.tbl.phoneNumber`，另一个页面通过局部 `model22` 读取同一物理字段，验证跨页面 writer 可被物理表查询发现。

### 验收目标

- 能回答“某个组件显示依赖哪些参数/模型/系统变量”。
- 能回答“某个参数影响哪些模型和组件”。
- 条件链中每个节点都能追溯到 `ConditionRecord` 或原始组件/模型定义。
- 给定物理表 `model:fact_qwSidebar`，`query-model` 能列出来自 `潜客信息跟进.spg` 的 `button1.action1/action4` 对 `phoneNumber` 的写入。
- 给定 `合同协议.spg` 的 `input3`，工具能串联 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.model6.phoneNumber`，并标明读取边与写入边各自证据位置。
