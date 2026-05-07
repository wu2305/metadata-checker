---
name: metadata-checker
description: |
  Use the metadata-checker CLI tool to parse and analyze SuperPage (.spg) and Table (.tbl) metadata files from a low-code platform.
  This skill guides you on when and how to invoke the tool to extract component trees, expressions, dependencies,
  value source traces, and calculation priority analysis from page metadata JSON files.

  Use this skill when:
  1. You need to parse or analyze a .spg file (low-code platform SuperPage metadata).
  2. You need to trace the source of a component's value (e.g., which model/param/component it comes from).
  3. You need to build a dependency graph of components and detect cycles.
  4. You need to analyze calculation priority rules (defaultValue vs exp vs calcCondition).
  5. You need to extract structured metadata from a low-code platform page for downstream processing.
---

# metadata-checker Skill

## Overview

The `metadata-checker` is a Rust CLI tool that parses `.spg` files (SuperPage metadata JSON) and `.tbl` files (Table/DataFlow metadata) from a low-code platform.

## Task Decision Tree

When working with metadata-checker, follow this decision tree to choose the right command:

### Q1: Do you have a single `.spg` file to analyze?

**Yes** → Use `metadata-checker <FILE.spg>` (default JSON output)

- Need a compact overview? → Default output (no flags)
- Need full details (components, expressions, dependency_order)? → Add `--detail`
- Need priority analysis (defaultValue vs exp vs calcCondition)? → Add `--priority`
- Need to query a specific component? → Add `--query <COMPONENT_ID>`
- Need human-readable text? → Add `--human` (expert exploration mode)

### Q2: Do you need to understand what a specific ID does?

**Yes** → Use `metadata-checker --explain <ID>`

- Component in a single file → `--explain input1` (with `--project-dir` for cross-file context)
- Model/field/page/dataflow in project graph → `--explain model:physical_x`

Supported target types and ID formats:
- `component` → `comp:app/page.spg|button1` 或单文件模式下裸 ID `button1`
- `action` → `action:app/page.spg|button1|action1`
- `model` → `model:model1`
- `field` → `field:model1.fieldA`
- `page` → `page:app/page.spg`
- `dataflow` → `model:dataflow_output`（DataFlow 也是 Model 类型）

Output structure (kind = Explain):
- `summary.what_is_it`: 自然语言短句，例如"按钮 button1，位于页面 page_relations，具有1个动作"
- `summary.importance`: 稳定分类（M12）：`entrypoint` / `data_source_display` / `calculated_display` / `static_display` / `container` / `form_input` / `action_target` / `unknown`。旧值 `data_source` / `write_target` / `navigation` 已废弃，保留时视为 legacy。
- `details.reads`: 目标读取的模型字段、参数、组件值
- `details.writes`: 目标写入的模型字段、参数、页面状态
- `details.triggered_by`: 真实触发关系（action 被组件触发、页面被 action 打开）。**M12 后不再包含 Contains（页面包含），Contains 已移入 `located_in`。**
- `details.affects`: 下游影响对象（legacy 兼容对象，含 `components` / `models` / `pages` 子数组）
- `details.located_in`: 父页面 Contains 关系（M12 从 `triggered_by` 中拆分）
- `details.triggers`: 组件触发的 action 列表（M12 从 `affects` 中拆分）
- `details.navigates_to`: 跳转/嵌入的目标页面
- `details.writes_models`: 写入的模型/字段（含 action 聚合）
- `details.affects_components`: 受影响的组件（ActionControlsComponent / SetsParam）
- `details.lineage`: 字段级血缘（M6 已实现）
  - DataFlow 字段：从 `dimensions[].inputField` 和 `dimensions[].exp` 追溯
  - 页面写入字段：从 `submitData.submitFields[]`、`insertData/updateData/deleteData.fieldValues[]` 追溯
  - 每项包含：`target_field`、`source_fields`、`source_expr`、`transform`、`confidence`、`evidence`
  - 表达式无法解析时产生 `LINEAGE_EXPR_UNPARSED` diagnostic
  - **M12 后，普通 button/text/dialog 无 lineage 时不输出 `LINEAGE_SOURCE_MISSING` 噪声，仅当组件/字段确实存在写入/字段映射但无法追溯时才输出**
- **Action 特有字段**:
  - `action_category`: 动作语义分类（data_write / data_read / navigation / param_mutation / ui_control / validation / data_initialization / data_refresh / unknown）
  - `semantic_summary`: 动作自然语言摘要，例如"点击 button1 后提交数据到 model1.name"
  - `blocks_on`: 结构化等待前置动作（替代旧 `wait_prev`）
  - `condition`: 结构化条件执行表达式（condition 或 conditionExp）
  - `trigger_type`: click / hover / focus / ...
- `evidence`: 每条结论的具体证据，包含 `source_file`、`node_id`、`edge_type`
  - 当 `diagnostics` 出现 `EVIDENCE_SAMPLED` 时，表示 evidence 为控噪采样，需结合 `details` 全量数组判断
- `next_queries`: 建议的后续命令

**When explain is not enough**:
- 需要了解周围关联 → `--context <ID> --depth 2`
- 需要页面整体逻辑 → `--query-page-logic <PAGE>`
- 需要模型全量读写 → `--query-model <MODEL>`

### Q3: Do you need the surrounding context of an ID?

**Yes** → Use `metadata-checker --context <ID> --depth <N> --budget <compact|normal|full>`

- **定位**：`--context` 用于补充 `--explain`，不是替代。当 `--explain` 给出的摘要不够理解周围依赖/影响时，再用 `--context`。
- Default depth is 1, default budget is `normal`.
- **AI 不要默认读取 `full` budget**，优先使用 `normal` 或 `compact`；只有遇到 `OUTPUT_TRUNCATED` diagnostic 且确实需要更多关系时，才升级到 `full`。
- `--context` 输出包含：upstream（谁影响我）、downstream（我影响谁）、related_actions、related_models、related_pages、related_nodes（全类型）、related_components（仅组件）。
  - `--context` evidence 会附带 upstream/downstream 的边级采样证据；若缺原始 JSON 路径，`json_path` 为 `<graph-edge-derived>`。

### Q4: Do you need project-level analysis?

**Yes** → You need `--project-dir <DIR>`

**Step 1: 检查 graphdb 状态**
```bash
metadata-checker --check-graph --graph-db-path /tmp/project.graphdb
```
- 若返回 `GRAPH_DB_NOT_FOUND` → 进入 Step 2 构建。
- 若返回 `GRAPH_DB_LOCKED` → 等待、换一个 `--graph-db-path`，或在真实项目/大图场景增加 `--graph-lock-timeout-ms 30000`。
- 若返回 `GRAPH_DB_PERMISSION_DENIED` → 将 `--graph-db-path` 指向 `/tmp` 等可写目录。

**Step 2: 构建图数据库**
```bash
metadata-checker --project-dir /path/to/project --build-graph --graph-db-path /tmp/project.graphdb
```
- 默认路径是 `<project-dir>/.metadata-checker.graphdb`。
- 当项目目录只读或沙箱限制写入时，**必须**指定 `--graph-db-path` 到可写位置（如 `/tmp`）。

**Step 3: 查询**
所有项目级查询都可以附加 `--graph-db-path`：
- Model read/write relationships → `--query-model <MODEL>`
- Page dependencies → `--query-page <PAGE>`
- Cross-file relations between two pages → `--query-cross <A> <B>`
- DataFlow subgraph → `--query-dataflow <MODEL>`
- Page-level logic summary → `--query-page-logic <PAGE>`
- Explain any node → `--explain <ID>`
- Context around a node → `--context <ID> --depth 2 --budget normal`

### Q5: Do you need page-level logic summary?

**Yes** → Use `metadata-checker --project-dir /path/to/project --query-page-logic <page>`

**定位**：当 AI 被问到“这个页面主要做什么、用户能触发哪些逻辑、会影响哪些数据”时，优先使用 `--query-page-logic`，而不是 `--context` 或 `--explain`。

Output structure (kind = PageLogic):
- `summary.what_is_it`: 自然语言短句，例如“页面 actions_test，5 个用户入口，读取 2 个模型，写入 2 个模型，存在 1 个跳转”
- `summary.page_role`: 稳定分类：`form_submit_page` / `readonly_dashboard` / `navigation_page` / `data_maintenance_page` / `mixed_interaction_page` / `unknown`
- `details.entrypoints`: 用户可触发入口（button、link 等），不含普通 input
- `details.action_flows`: 按组件触发链输出 action，包含：
  - `action_id` / `action_type` / `action_category` / `semantic_summary` / `component_id` / `trigger_type`
  - `blocks_on`: 结构化等待前置动作（替代旧 `wait_prev` 字符串）
  - `condition`: 结构化条件执行表达式（condition 或 conditionExp）
  - `reads` / `writes` / `navigation` / `sets_params` / `passes_params`
- `details.data_sources`: 页面读取的模型和字段
- `details.write_targets`: 页面写入的模型和字段
- `details.navigation`: 页面跳转/嵌入关系（OpensPage、EmbedsPage、SetsParam、PassesParam）
- `details.visibility_rules`: 组件可见性规则（visible/hidden/disabled/readonly），递归扫描 canvas/components/panels/steps/comps
- `details.risk_diagnostics`: 风险诊断，包括：
  - `NO_WRITE_TARGETS`：只读页面
  - `NO_ENTRYPOINTS`：无用户可触发入口
  - `ACTION_FLOW_INCOMPLETE`：仅对按语义应有副作用（如 data_write/param_mutation）却“读取但无写入且无导航”的 action 提示
  - `EVIDENCE_SAMPLED`：evidence 为低噪声采样，不代表 details 全集
  - `UNRESOLVED_PAGE_NAVIGATION`：导航目标页面不存在
  - `UNRESOLVED_MODEL_WRITE`：写入目标模型不存在
  - `VISIBILITY_RULE_UNRESOLVED`：visibility 规则包含未解析或歧义引用（warning）
- `next_queries`: 建议后续命令，例如 `--explain <PAGE>`、`--context <PAGE> --depth 2`

## Machine JSON Output Schema

All machine outputs (`--non-human`, default) follow a unified top-level structure:

```json
{
  "schema_version": "1.0",
  "kind": "SuperPage | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic",
  "query_target": "...",
  "summary": { /* Low-noise summary, AI should read this first */ },
  "details": { /* Detailed data, only when needed */ },
  "evidence": [ /* Evidence chain for every conclusion */ ],
  "diagnostics": [ /* Warnings, errors, unresolved refs */ ],
  "next_queries": [ /* Suggested follow-up CLI commands */ ]
}
```

**AI Usage Rule**: Always read `summary` first. Only read `details` or `evidence` when you need to verify a specific claim. Never read raw JSON by default. If `diagnostics` contains entries, you must give a conservative answer.

## Important Constraints

1. `--project-dir` is **required** for all project-level queries (`--query-model`, `--query-page`, `--query-cross`, `--query-dataflow`, `--explain`, `--context`, `--query-page-logic`). Without it, the tool exits with an error.
2. `--human` and `--interactive` both enter REPL mode for expert exploration. Do not use them for automated/machine consumption.
3. Component IDs in expressions use the full path format: `comp:app/page.spg|component_id`.
4. Page node IDs use normalized relative paths: `page:app/page.spg`.
5. Legacy compatibility fields exist but should not be used as primary semantics:
   - `upstream_dependencies` → use `produced_by` / `consumed_by_dataflows` / `dataflow_inputs`
   - `downstream_outputs` → use `dataflow_outputs` / `produced_by`

## Examples

```bash
# Parse single file, compact JSON output (default)
metadata-checker page.spg

# Parse with full details
metadata-checker page.spg --detail

# Query specific component
metadata-checker page.spg --query input3

# Explain a component
metadata-checker page.spg --explain input1

# Build graph for project
metadata-checker --project-dir /path/to/project --build-graph

# Query model (JSON output)
metadata-checker --project-dir /path/to/project --query-model model1

# Query model (human output)
metadata-checker --project-dir /path/to/project --query-model model1 --human

# Get context around a button
metadata-checker --project-dir /path/to/project --context button1 --depth 2 --budget compact

# Page logic summary
metadata-checker --project-dir /path/to/project --query-page-logic "page:app/合同管理/销售合同.spg"
```


### .tbl 单文件使用指南

当没有项目图数据库时，可直接解析单个 `.tbl` 文件获取表/DataFlow 语义。

**何时使用**
- 需要快速判断一个 `.tbl` 是物理表还是 DataFlow。
- 需要查看 DataFlow 的输入、输出和字段加工链。
- 项目图数据库尚未构建或目标文件不在项目中。

**命令**
```bash
metadata-checker app_table.tbl
metadata-checker dataflow_output.tbl
```

**AI 阅读顺序**
1. **summary**：先看 `table_type`（AppTable / DataFlow）、`field_count`、`input_count`、`output_count`、`what_is_it`。
2. **details.field_lineage**：字段级来源链，包含 `target_field`、`source_fields`、`source_expr`、`transform`、`confidence`。
3. **details.dataflow_inputs / dataflow_outputs**：DataFlow 的输入输出拓扑。
4. **evidence**：验证字段来源的 `source_file` 和 `json_path`。
5. **diagnostics**：若含 `DATAFLOW_NO_OUTPUT`，说明该 DataFlow 未指定输出物理表。

**禁止行为**
- 禁止把 `.tbl` 当作 SuperPage 解析（`kind=SuperPage` 属于语义错误）。
- 禁止在没有 `field_lineage` 时编造字段来源。
## AI 回答协议（M9-D）

当使用 metadata-checker CLI 回答业务问题时，必须遵守以下协议，禁止默认读取 raw JSON 或凭空推断。

### 1. 先选命令

按问题类型优先选择以下命令：

| 问题类型 | 优先命令 |
|----------|----------|
| 这个页面主要做什么？ | `--query-page-logic page:...` |
| 这个按钮/组件做什么？ | `--explain comp:...\|component` |
| 这个字段值从哪里来？ | `--explain field:model.field` |
| 这个模型在哪里被读写？ | `--query-model model` / `--explain model:...` |
| 这个动作在什么条件下执行？ | `--explain action:...\|...\|action` |
| DataFlow 怎么来的？ | `--query-dataflow model` / `--explain model:dataflow_model` |
| 周围还有哪些关键依赖？ | `--context \u003cID\u003e --depth 1 --budget normal` |

### 2. 再读 summary

执行命令后，**必须首先读取 `summary`**：
- `summary.what_is_it` 给出自然语言一句话定义。
- `summary.page_role` / `importance` 给出语义分类。
- 计数字段（`entrypoint_count`、`write_target_count`、`read_by_count` 等）建立数量级认知。
- **禁止跳过 summary 直接读取 details 或 evidence**。

### 3. 需要核查时读 evidence

当 summary 中的结论需要验证时，读取 `evidence`：
- 每条 evidence 包含 `claim`、`source_file`、`node_id`、`edge_type`、`raw_expr`、`json_path`、`confidence`、`reason`。
- 优先读取 `confidence=high` 的证据。
- `confidence=medium/low` 的结论必须降级表达（"可能..."、"初步判断..."）。

### 4. 需要细节时读 details

只有在 summary + evidence 仍无法回答问题时，才展开 `details`：
- `details.action_flows[]` 查看动作链详情。
- `details.lineage[]` 查看字段来源链。
- `details.upstream/downstream` 查看图邻居。

### 5. 遇到 diagnostics 必须保守回答

如果输出含 `diagnostics`，必须遵守：
- `severity=error`：该结论不可信，必须说明"输出包含错误诊断，无法确定"。
- `severity=warning`：结论可能不完整，必须说明"存在警告诊断，结论可能不完整"。
- `severity=info`：仅作提示，不影响主要结论。
- 常见需要降级的诊断码：`UNRESOLVED_REF`、`EVIDENCE_INCOMPLETE`、`EVIDENCE_SAMPLED`、`LINEAGE_SOURCE_MISSING`、`LINEAGE_EXPR_UNPARSED`、`UNKNOWN_ACTION_TYPE`。

### 6. 输出体积控制

- 默认使用 `--budget normal`；只有需要最小上下文时才用 `--budget compact`。
- `truncated=true` 时说明输出被截断，还有未展示的关系。
- 禁止无差别读取 `--budget full` 的完整 details 大数组。

### 7. 禁止行为

- 禁止默认读取 raw JSON 大对象。
- 禁止忽略 diagnostics 做空洞确定性结论。
- 禁止在 evidence 不足时编造来源。
- 禁止重复使用错误的命令格式（例如 `--query-model model:model1` 会被 CLI 二次加前缀变成 `model:model:model1`，应写 `--query-model model1`）。

## 来源分类使用指南（值追溯）

当 AI 被问"这个值从哪里来""这个字段是用户输入的还是自动带出的"时，应结合 CLI 输出的来源分类信息。

### source_type 快速判定

CLI 在 `details.value_trace[]` 或 `details.lineage[]` 中提供 `source_type`：

| source_type | 含义 | AI 回答建议 |
|-------------|------|------------|
| `Param` | 页面参数 | "来自页面参数" |
| `UserInput` | 用户可交互输入 | "用户输入" |
| `ModelAuto` | 数据模型自动绑定 | "从数据模型自动获取" |
| `System` | 系统变量 | "系统变量" |
| `Computed` | 表达式计算 | "由表达式计算产生" |
| `Constant` | 固定常量 | "固定常量" |
| `Unknown` | 无法分类 | "来源无法确定，保守回答" |

### 使用顺序

1. **先看 summary**：是否有 `source_type` 或 `value_trace_count` 等高层信息。
2. **需要追溯时看 details.value_trace[]**：从目标组件开始，逐节点展开来源链。
3. **字段级 lineage 用 details.lineage[]**：`target_field` → `source_fields` → `via_node` → `transform`。
4. **每个节点有 evidence**：验证 `claim`、`source_file`、`json_path`。

### 回答模板

- 单级来源："组件 X 的值来源于页面参数 param1（source_type=Param）。"
- 多级来源："组件 X 的值由 input2 计算产生（Computed），而 input2 又来源于页面参数 param1（Param）。"
- 多分支来源："组件 X 的值由 input1 + input2 计算产生，其中 input1 来源于参数 param1，input2 来源于模型 model1.A。"
- 不确定时："组件 X 的表达式包含未解析引用，source_type=Unknown，不能确定最终来源。"

### 禁止行为

- 禁止在没有 `value_trace` 或 `lineage` 时凭空推断来源。
- 禁止将 `Computed` 误判为 `UserInput`（例如 input 组件有表达式时是计算产生，不是用户输入）。
- 禁止忽略 `Unknown` 标记做空洞确定性结论。
