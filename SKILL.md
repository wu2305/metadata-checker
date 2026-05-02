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
- `summary.importance`: 稳定分类：`entrypoint` / `data_source` / `write_target` / `navigation` / `calculated_display` / `container` / `unknown`
- `details.reads`: 目标读取的模型字段、参数、组件值
- `details.writes`: 目标写入的模型字段、参数、页面状态
- `details.triggered_by`: 真实触发关系（组件被页面包含、action 被组件触发、页面被 action 打开）
- `details.affects`: 下游影响对象（被哪些组件引用、影响哪些模型字段、打开哪些页面）
- `details.lineage`: 字段级血缘（M6 已实现）
  - DataFlow 字段：从 `dimensions[].inputField` 和 `dimensions[].exp` 追溯
  - 页面写入字段：从 `submitData.submitFields[]`、`insertData/updateData/deleteData.fieldValues[]` 追溯
  - 每项包含：`target_field`、`source_fields`、`source_expr`、`transform`、`confidence`、`evidence`
  - 表达式无法解析时产生 `LINEAGE_EXPR_UNPARSED` diagnostic
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

First, build the graph database:
```bash
metadata-checker --project-dir /path/to/project --build-graph
```

Then query:
- Model read/write relationships → `--query-model <MODEL>`
- Page dependencies → `--query-page <PAGE>`
- Cross-file relations between two pages → `--query-cross <A> <B>`
- DataFlow subgraph → `--query-dataflow <MODEL>`
- Page-level logic summary → `--query-page-logic <PAGE>`

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
