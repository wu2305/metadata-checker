# M17：条件抽取基础层

| milestone | M17 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M17：条件抽取基础层

### 目标

从 `.spg` 中稳定抽取条件表达式，形成可复用的条件记录，而不是让 AI 直接阅读大体积 raw metadata。

### 问题清单

- `5.4-mini` 分析 `合同协议.spg` 的“正常显示数据条件”时，需要直接读取 5MB `.spg` 才能定位 `visibleCondition`、`disableCondition` 和 `filter.clauses[]`。
- `--query-page-logic` 能给出页面规模和 Top-N 入口，但不能完整表达页面显示/禁用/数据过滤条件。
- 条件表达式分散在组件、动作、数据源、默认值中，缺少统一结构。
- 多行表达式、`IF`、`CASE`、`CONCAT`、系统变量和中文路径容易导致抽取遗漏。

### 工作清单

- 抽取 `visibleCondition`、`disableCondition`、`conditionExp`、`filter.clauses[].exp`、`defaultValueExp`、`exp`。
- 为每条条件记录 `source_file`、`json_path`、`component_id/model_id/action_id`、`condition_type`。
- 解析条件中引用的 `param*`、`$user.*`、`model*.field`、`input*.value`、`text*.value`、`model*.totalRowCount__`。
- 定义统一结构 `ConditionRecord`，并写入 `docs/schema.md`。
- 增加 fixture 覆盖空条件、多行条件、`IF/CASE/CONCAT`、中文路径、系统变量。

### 验收目标

- 给定 `.spg`，工具能列出所有条件表达式及引用对象。
- 每条条件都有可核验位置，不允许缺少 `source_file` 和 `json_path`。
- 本阶段只做抽取和定位，不输出业务解释。
