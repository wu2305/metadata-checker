# M10：表与 DataFlow 单文件输出可理解

| milestone | M10 |
| status | done |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M10：表与 DataFlow 单文件输出可理解

状态：已收敛。剩余非阻塞观察项已归并到 M13 和 M14。

### 目标

让 AI 在没有项目图数据库时，也能从单个 `.tbl` 文件获得正确的表/DataFlow 语义，而不是被误导为 SuperPage。

### 问题清单

- 单文件解析真实 `.tbl` 时输出 `kind=SuperPage`，`component_count=0`，证据为 `Legacy page metadata parsed`，语义错误。
- `.tbl --detail` 只输出空 `components/data_bindings/settings`，无法支撑 AI 理解物理表字段、DataFlow 输入、DataFlow 输出或维度加工。
- 默认 `SuperPage` schema 与 `.tbl` 实际语义混用，会让模型以为表也是页面。
- 没有单文件 DataFlow 摘要，AI 无法在 graphdb 缺失时回答“这个表从哪里来”“输出到哪里”“字段怎么加工”。
- 缺少 `.tbl` 单文件 evidence，无法回到 `dimensions`、`inputField`、`exp`、`target/output` 等原始 JSON 路径。
- 缺少 `.tbl` 单文件诊断，解析不到输出节点时没有区分“确实无输出”和“当前解析能力不足”。

### 验收目标（M15 已收敛）

- 真实 `.tbl` 单文件输出 `kind=Table | DataFlow`，不再输出 `SuperPage`。
- `summary` 至少包含 `table_id/name`、`table_type`、`field_count`、`input_count`、`output_count`、`what_is_it`。
- `details` 至少包含 `fields`、`dataflow_inputs`、`dataflow_outputs`、`field_lineage`。
- `evidence` 能定位到真实 `source_file` 与 JSON 路径。
- 对无输出、无法识别输出、表达式无法解析分别产生明确 diagnostics。
- 以 `xiaoshouyi/data/tables/销售/fact_saleContract.tbl` 和至少 2 个真实 DataFlow `.tbl` 建立回归样例。
