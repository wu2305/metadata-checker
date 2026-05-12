# metadata-checker JSON Schema 规范

> 版本：1.0  
> 适用范围：所有 `--non-human`（默认）机器输出  
> 目标读者：AI / LLM / 下游自动化工具

## 设计原则

1. **稳定顶层结构**：任何机器输出都包含相同的顶层字段，方便模型建立稳定解析预期。
2. **summary 优先**：模型默认读取 `summary`，不应默认读取 `details` 或 raw 数据。
3. **evidence 可核查**：任何结论都必须附带 `evidence`，指向具体元数据位置。
4. **diagnostics 显式化**：不确定的关系、缺失的目标、不支持的语义必须进入 `diagnostics`，禁止静默丢弃。
5. **兼容字段标注**：旧字段保留但不作为主语义字段，需在文档中标注 `legacy`。

## 统一顶层结构

所有机器输出（`--non-human`，默认）通过 `AiOutput` Rust struct 序列化，禁止手写 `json!`。

```json
{
  "schema_version": "1.0",
  "kind": "SuperPage | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic | Table | DataFlow | GraphDbCheck",
  "query_target": "可选，被查询对象的 ID 或标识",
  "summary": { /* 低噪声摘要，模型优先读取 */ },
  "details": { /* 可选的详细信息，按需展开 */ },
  "evidence": [ /* 支撑结论的证据链 */ ],
  "diagnostics": [ /* 异常、不确定、风险提示 */ ],
  "next_queries": [ /* 建议的下一步查询命令 */ ]
}
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `schema_version` | string | 是 | 固定为 `"1.0"`，未来 schema 升级时递增。 |
| `kind` | string | 是 | 输出类别，见上枚举。禁止随意字符串。 |
| `query_target` | string \| null | 否 | 被查询对象的 ID，如 `"input1"`、`"model:physical_x"`。 |
| `summary` | object | 是 | 核心摘要，控制在 50 行以内。 |
| `details` | object \| null | 否 | 详细数据，仅在 `--detail` 或相关查询中展开。 |
| `evidence` | array | 是 | 每项结论的证据。只要 summary 有实质内容，evidence 不能为空。 |
| `diagnostics` | array | 是 | 可为空数组，但不能省略。 |
| `next_queries` | array[string] | 是 | 建议的 CLI 命令。如果 summary 有实质内容，不能为空。 |

### Evidence 的两种状态

当前实现中，evidence 分为两类：

1. **真实证据（Real Evidence）**：从解析或图遍历中直接提取的具体结论，每条 evidence 对应 summary 中的一个计数或 details 中的一个主要数组。
   已覆盖的查询：
   - `SuperPage` 默认输出：解析结果（components/expressions 计数）
   - `ModelQuery`：每个 reader/writer/dataflow_input/dataflow_output/produced_by/consumed_by_dataflows 数组独立 evidence
   - `PageQuery`：outgoing/incoming 边数独立 evidence
   - `CrossPageQuery`：path 总数 + 每条具体 path 的 evidence（最多 5 条）
   - `DataFlowQuery`：inputs/outputs 独立 evidence
   - `ComponentQuery`：组件定义 evidence
   - `Explain`：节点定义 + reads/writes/triggered_by/affects/lineage 关系 evidence
   - `Context`：closure 范围 + upstream/downstream/related_nodes evidence
   - `PageLogic`：entrypoints/data_sources/write_targets/action_flows/navigation/visibility_rules evidence

   这类证据带 `confidence: high` 或 `medium`，并有 `node_id`/`source_file` 等定位信息。

2. **兜底证据（Fallback Evidence）**：仅当某个查询路径**尚未**产出真实证据时，`AiOutput::validate()` 自动添加：
   - claim: `"Output generated from parsed metadata"`
   - confidence: `low`
   - 同时 diagnostics 追加 `EVIDENCE_INCOMPLETE`

   M1 完成后，兜底证据仅在以下情况出现：
   - 新增查询类型尚未实现独立 evidence 收集
   - 代码路径遗漏（应视为 bug，需修复）

**AI 使用规则**：
- 优先读取 summary 中的计数，然后到 evidence 中找对应的具体来源
- 遇到 `EVIDENCE_INCOMPLETE` diagnostic 时，应保守回答，不基于 summary 做强断言
- 若 evidence 完整且 confidence 为 high/medium，可基于 evidence 做确定性结论

## Rust 实现

统一输出结构定义在 `src/output/schema.rs`：

```rust
pub struct AiOutput {
    pub schema_version: String,
    pub kind: OutputKind,
    pub query_target: Option<String>,
    pub summary: serde_json::Value,
    pub details: Option<serde_json::Value>,
    pub evidence: Vec<Evidence>,
    pub diagnostics: Vec<Diagnostic>,
    pub next_queries: Vec<String>,
}
```

所有 machine 输出必须通过 `AiOutput` 序列化，调用 `.validate()` 自动检查契约：
- summary 有实质内容但 evidence 为空时，自动添加兜底 evidence 和 `EVIDENCE_INCOMPLETE` diagnostic

## kind: Explain 字段约束

`--explain <ID>` 输出，按目标 `NodeType` 分发语义：

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `what_is_it` | string | 自然语言短句，直接说明目标含义 |
| `type` | string | 目标类型：`component` / `action` / `model` / `field` / `page` / `dataflow` |
| `type_detail` | string | 具体名称或子类型 |
| `importance` | string | 稳定分类：`entrypoint` / `data_source_display` / `calculated_display` / `static_display` / `container` / `form_input` / `action_target` / `unknown` |
| `page` / `page_id` | string | 所属页面（Component/Action 有） |
| `parent_component` / `parent_component_id` | string | 父组件（Action 有） |
| `read_by_count` / `written_by_count` | number | 读写者计数（Model/Field 有） |
| `action_count` | number | 动作数量（Component 有） |
| `child_count` / `entrypoint_count` / `data_source_count` / `write_target_count` | number | 页面统计（Page 有） |
| `input_count` / `output_count` / `internal_node_count` | number | DataFlow 统计 |

### details 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `reads` | array | 目标读取的对象列表 |
| `writes` | array | 目标写入的对象列表（Component 的 legacy 字段，等价于 `writes_models`） |
| `writes_models` | array | 组件写入的模型/字段列表（含 action 聚合） |
| `located_in` | array | 父页面 Contains 关系（含 page 名称/ID） |
| `triggers` | array | 组件触发的 action 列表 |
| `navigates_to` | array | 跳转/嵌入的目标页面列表 |
| `affects_components` | array | 受影响的组件列表（ActionControlsComponent / SetsParam） |
| `triggered_by` | array | 触发目标的源对象（incoming Reads），不再包含 Contains |
| `affects` | object | legacy 字段，包含 `components` / `models` / `pages` 子数组 |
| `lineage` | array | 字段级血缘（M6 已实现）：每项包含 target_field / source_fields / source_expr / transform / confidence / evidence |
| `inputs` | array | DataFlow 输入源列表（DataFlow 有） |
| `outputs` | array | DataFlow 输出目标列表（DataFlow 有） |
| `internal_topology` | object | DataFlow 内部节点拓扑：nodes + edges（DataFlow 有） |
| `produced_by` | array | 字段产生者列表（Field 有） |
| `action_category` | string | 动作语义分类（Action 有）：data_write / data_read / navigation / param_mutation / ui_control / validation / data_initialization / data_refresh / unknown |
| `semantic_summary` | string | 动作自然语言摘要（Action 有） |
| `blocks_on` | object / null | 等待前置动作结构化解析（Action 有） |
| `condition` | object / null | 条件执行表达式结构化解析（Action 有） |
| `trigger_type` | string | 触发类型：click / hover / focus / ...（Action 有） |

### 允许为空的字段

- `details.lineage`：空数组表示无 traceable 来源。**M12 收敛后，仅当组件/字段确实存在写入/字段映射但无法追溯时才附带 `LINEAGE_SOURCE_MISSING` diagnostic；普通 button、text、无写入的 form_input 不再输出此诊断。** M6 已实现从 dimensions[].inputField / dimensions[].exp / submitField / fieldValues[] 追溯
- `details.inputs` / `details.outputs`：DataFlow 确实无输入/输出时可为空
- `details.reads` / `details.writes`：目标确实无读写关系时可为空
- `summary.page` / `summary.page_id`：非页面上下文的目标可为 null

## 兼容字段策略（Legacy）
## 兼容字段策略（Legacy）

以下字段为兼容旧版本保留，**不应作为 AI 主语义字段**：

- `upstream_dependencies` → 请使用 `produced_by` / `consumed_by_dataflows` / `dataflow_inputs`
- `downstream_outputs` → 请使用 `dataflow_outputs` / `produced_by`
- `truncated` → 已由 `details` 的存在性替代
- `version` / `theme`（顶层）→ 已下沉到 `summary.page_info`

## Evidence 结构

每条 evidence 至少包含：

```json
{
  "claim": "结论描述",
  "source_file": "来源文件相对路径或 null",
  "node_id": "图节点 ID 或 null",
  "edge_type": "关系边类型或 null",
  "raw_expr": "原始表达式或 null",
  "json_path": "JSON 路径或 null",
  "confidence": "high | medium | low",
  "reason": "置信度说明"
}
```

## Diagnostics 结构

每条 diagnostic 包含：

```json
{
  "severity": "error | warning | info",
  "code": "EVIDENCE_INCOMPLETE | EVIDENCE_SAMPLED | CYCLE_DEPENDENCY | OUTPUT_TRUNCATED | NO_WRITE_TARGETS | NO_ENTRYPOINTS | ...",
  "message": "人类可读描述",
  "location": {
    "source_file": "...",
    "node_id": "...",
    "json_path": "..."
  },
  "suggestion": "建议的修复或下一步查询"
}
```

## Brief 结构与截断数组

### 截断数组包装

所有长数组在 `compact` / `normal` 模式下统一包装为：

```json
{
  "total_count": 42,
  "shown_count": 5,
  "truncated": true,
  "remaining_count": 37,
  "items": [ ... ]
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `total_count` | number | 原始数组总长度 |
| `shown_count` | number | 当前展示长度 |
| `truncated` | boolean | 是否被截断 |
| `remaining_count` | number | 剩余未展示数量 |
| `items` | array | 实际展示的子数组 |

### evidence_summary 结构

`compact` 模式下自动注入 `summary.evidence_summary`：

```json
{
  "total_count": 25,
  "shown_count": 5,
  "sampled": true,
  "confidence_counts": {
    "high": 18,
    "medium": 5,
    "low": 2
  },
  "source_file_count": 3,
  "has_graph_derived": true
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `total_count` | number | evidence 总数 |
| `shown_count` | number | 当前展示数（可能因采样而 < total） |
| `sampled` | boolean | 是否为采样子集 |
| `confidence_counts` | object | high / medium / low 分布 |
| `source_file_count` | number | 有 source_file 的证据数量 |
| `has_graph_derived` | boolean | 是否包含 graph-derived 证据 |

### key_findings 结构

`compact` 模式下自动注入 `summary.key_findings`，每项包含：

```json
{
  "claim": "页面有 3 个入口和 2 个写入目标",
  "category": "summary",
  "evidence_level": "high"
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `claim` | string | 一句话结论 |
| `category` | string | `summary` / `risk` / `truncation` |
| `evidence_level` | string | `high` / `medium` / `low` / `sampled` |
| `code` | string | 可选，关联的 diagnostic code |

## 输出体积控制（Budget）

| Budget | 说明 |
|--------|------|
| `compact` | 只输出 brief 结构与少量 Top-N，details 中所有长数组使用截断包装，自动注入 `evidence_summary` 和 `key_findings`。适合第一轮读取。 |
| `normal` | 输出 brief + 主要 details（数组仍可能截断），保留 `evidence_summary` 和 `key_findings`。适合需要核查时的第二轮读取。 |
| `full` | 输出完整 details（不截断），但仍保留 `summary`、`key_findings`、`evidence_summary`。适合深度审计。 |

非法 budget 产生错误：CLI 层校验 `compact|normal|full`，非法值直接错误退出，不静默降级。

## 查询不存在目标的行为

以下查询在目标不存在时返回明确错误（`Result::Err`），不静默输出半截 JSON：

- `--query-page <PAGE>`（目标不存在时）
- `--query-dataflow <MODEL>`（目标不存在时）

## kind: Context 字段约束

`--context <ID> --depth <N> --budget <compact|normal|full>` 输出目标节点周围的最小闭包上下文。

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `center_node` | string | 中心节点 ID |
| `center_type` | string | 中心节点类型：`component` / `action` / `model` / `field` / `page` / `dataflow` |
| `depth` | number | BFS 深度 |
| `budget` | string | `compact` / `normal` / `full` |
| `related_nodes_count` | number | 访问到的邻居节点数（不含中心节点） |
| `truncated` | boolean | 是否因 budget 被截断 |
| `high_value_relations` | object | 各类关系数量统计：upstream/downstream/related_actions/related_models/related_pages/related_nodes/related_components |

### details 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `upstream` | array | 谁影响我（按语义归类：读取来源、触发来源、父页面/父模型、DataFlow 输入） |
| `downstream` | array | 我影响谁（被哪些组件/action 使用、写入哪些模型/字段、打开哪些页面） |
| `related_actions` | array | 与目标强相关的 action |
| `related_models` | array | 相关模型和字段（保留 field_path） |
| `related_pages` | array | 所属页面、打开关系页面、嵌入页面、引用页面 |
| `related_nodes` | array | 相关闭包节点（可包含 Component/Action/Model/Field/Page） |
| `related_components` | array | 仅组件节点（`type=Component`）的子集，兼容旧字段 |

### context evidence 采样说明

- `evidence` 中会包含 upstream/downstream 的**边级采样证据**（claim 为 `Upstream edge ...` / `Downstream edge ...`）
- 采样上限随 budget 变化：`compact=3`、`normal=5`、`full=20`
- 当边缺少原始 JSON 路径时，`json_path` 固定为 `<graph-edge-derived>`

### diagnostics

- `OUTPUT_TRUNCATED`：当 `truncated=true` 时出现，说明被截断的类别和原始数量

### next_queries

根据目标类型生成可直接复制执行的 shell-safe 命令片段。所有 target 自动用单引号包裹，避免 `|`、中文路径、`$` 等特殊字符被 shell 解析：
- component/action → `--explain '<ID>'`、`--query-page-logic '<PAGE>'`
- model/field → `--explain '<ID>'`、`--query-model '<MODEL>'`
- page → `--explain '<PAGE>'`、`--query-page-logic '<PAGE>'`
- dataflow → `--query-dataflow '<MODEL>'`、`--explain '<MODEL>'`
- 目标不存在时 → `--find-page '<KEYWORD>'`、`--find-model '<KEYWORD>'`、`--find-component '<KEYWORD>'`

### AI 使用规则

- `--context` 用于补充 `--explain`，不是替代
- 默认不要读取 `full` budget，优先 `normal` 或 `compact`
- 遇到 `OUTPUT_TRUNCATED` 时，可按需用 `--budget full` 重新查询

## kind: PageLogic 字段约束

`--query-page-logic <PAGE>` 输出页面级业务逻辑摘要。

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `page_id` | string | 页面完整 ID |
| `page_name` | string | 页面名称 |
| `what_is_it` | string | 自然语言短句，说明页面主要功能 |
| `page_role` | string | 稳定分类：`form_submit_page` / `readonly_dashboard` / `navigation_page` / `data_maintenance_page` / `mixed_interaction_page` / `unknown` |
| `entrypoint_count` | number | 用户可触发入口数量 |
| `data_source_count` | number | 读取的数据源数量 |
| `write_target_count` | number | 写入目标数量 |
| `navigation_count` | number | 跳转/嵌入数量 |
| `risk_count` | number | 风险诊断数量 |

### details 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `page_inputs` | array | 页面参数（params） |
| `data_sources` | array | 读取的模型字段，包含 source_component/source_action/model/field_path/target_id + source_file/edge_type/raw_expr/json_path |
| `write_targets` | array | 写入的模型字段，包含 source_component/source_action/model/field_path/target_id + source_file/edge_type/raw_expr/json_path |
| `entrypoints` | array | 用户可触发组件（button、link 等），并包含 source_file/node_id/edge_type/json_path |
| `action_flows` | array | 动作链，每项包含 action_id/action_type/component_id/trigger_type/reads/writes/navigation/sets_params，并带 source_file/node_id/json_path |
| `visibility_rules` | array | 组件可见性规则（visible / hidden / disabled / readonly） |
| `navigation` | array | 跳转/嵌入关系，包含 from/to/type/field_path + source_file/edge_type/raw_expr/json_path |
| `risk_diagnostics` | array | 风险诊断列表（包含 severity/code/message/location/suggestion） |

### risk_diagnostics code

| Code | Severity | 说明 |
|------|----------|------|
| `NO_WRITE_TARGETS` | Info | 页面无写入目标（可能是只读页面） |
| `NO_ENTRYPOINTS` | Warning | 页面无用户可触发入口 |
| `ACTION_FLOW_INCOMPLETE` | Info | 对 `data_write/param_mutation` 等应有副作用的动作，检测到“读取但无写入且无导航” |
| `EVIDENCE_SAMPLED` | Info | evidence 为控噪采样，不代表 details 全集 |
| `PAGE_INPUTS_DEFERRED` | Warning | 未能读取原始页面文件，page_inputs / visibility_rules 可能不完整 |

### M19 页面可用性摘要字段（summary）

| 字段 | 类型 | 说明 |
|------|------|------|
| `display_prerequisites_count` | number | 显示门控条件数量（visibleCondition / enableCondition / disableCondition） |
| `data_prerequisites_count` | number | 数据源过滤条件数量（sourceFilter / filter clause / totalRowCount__） |
| `action_prerequisites_count` | number | 动作执行条件数量（action condition / conditionExp） |
| `primary_paths_count` | number | 主链路数量（从关键字段出发的端到端路径） |
| `related_context_count` | number | 旁路关系数量（非必要但相关的跨页/跨模型关系） |

### M19 页面可用性摘要字段（details）

| 字段 | 类型 | 说明 |
|------|------|------|
| `display_prerequisites` | array | 显示门控条件列表，每项为 Prerequisite 结构 |
| `data_prerequisites` | array | 数据源过滤条件列表，每项为 Prerequisite 结构 |
| `action_prerequisites` | array | 动作执行条件列表，每项为 Prerequisite 结构 |
| `primary_paths` | array | 主链路列表，每项为 PathSegment 结构 |
| `related_context` | array | 旁路关系列表（compact 模式默认截断，normal/full 展开） |
| `related_context_summary` | object | 旁路关系统计，含 total_count / by_type / note |

### M19 Prerequisite 项结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `kind` | string | 条件类型：VisibleCondition / EnableCondition / SourceFilterExp / ActionConditionExp / CalcCondition / DefaultValueExp / ... |
| `target` | string | 条件作用对象 ID（组件 ID / action ID / model ID） |
| `owner_type` | string | 所属对象类型：Component / Action / ModelSource / FieldDefault / Page |
| `subject_type` | string | 语义主体：component / action / model_source / field / page |
| `effect_type` | string | 业务效果：show / hide / enable / disable / execute / filter / compute / ... |
| `raw_expr` | string | 原始表达式字符串 |
| `normalized_expr` | string | 去空白后的规范化表达式 |
| `json_path` | string | 在原始 JSON 中的精确路径 |
| `source_file` | string | 来源文件路径 |
| `depends_on` | array | 表达式引用的符号列表 |
| `impact` | object | 影响范围对象，含 affected_component_count / affected_model_count / is_entrypoint / is_main_panel / affects_row_count / impact_score |
| `evidence` | object | 证据对象，含 node_id / edge_type / json_path / raw_expr / source_file |
| `confidence` | string | high（有 json_path 和 raw_expr）/ medium（缺 json_path 或 raw_expr）/ low（证据缺失） |
| `diagnostics` | array | 诊断列表，每项含 code / message |

### M19 PathSegment 结构（primary_paths）

| 字段 | 类型 | 说明 |
|------|------|------|
| `from` | string | 起点节点 ID |
| `to` | string | 终点节点 ID |
| `edge_type` | string | 边类型：Reads / Writes / ActionWrites / DependsOn / Contains / ... |
| `field_path` | string | 字段路径，如 model1.fieldA |
| `source_file` | string | 来源文件路径 |
| `json_path` | string | 原始 JSON 路径（若有） |
| `raw_expr` | string | 原始表达式（若有） |
| `confidence` | string | high（有 json_path）/ medium（图推导）/ low（推断） |

### M19 注意力漂移治理约定

- `related_context` 默认只输出计数和类型分布，不展开明细。
- compact 模式不展开 `related_context` 明细。
- normal 模式最多展示 Top-N 旁路。
- full 模式才展开完整旁路关系。
- 输出中明确标记：`related_context` 不是必要条件。
- 当邻居数量过大时，应生成 `ATTENTION_DRIFT` diagnostic，建议使用 `primary_paths` 优先回答。

### next_queries

- `--explain <PAGE> for page semantic summary`
- `--context <PAGE> --depth 2 --budget normal for surrounding context`
- `--query-model <MODEL> for model details`（对涉及的模型）

**注意**：next_queries 中的 `--query-model` 不带 `model:` 前缀，CLI 会自动添加。

## M4 补充字段（PageLogic）

### action_flows 详细字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `action_id` | string | action 裸 ID（如 action1） |
| `action_type` | string | submitData / updateData / insertData / deleteData / link / setParamValue / ... |
| `component_id` | string | 触发该 action 的组件完整 ID |
| `trigger_type` | string | click / hover / focus / ... |
| `action_category` | string | 动作语义分类：data_write / data_read / navigation / param_mutation / ui_control / validation / data_initialization / data_refresh / unknown |
| `semantic_summary` | string | 动作自然语言摘要 |
| `blocks_on` | object / null | 等待前置动作结构化解析，替代旧 `wait_prev` 字符串 |
| `condition` | object / null | 条件执行表达式结构化解析（condition 或 conditionExp） |
| `reads` | array | action 读取的模型字段 |
| `writes` | array | action 写入的模型字段 |
| `navigation` | array | action 跳转/嵌入的目标页面 |
| `sets_params` | array | action 设置的页面参数（setParamValue） |
| `passes_params` | array | action 传递的页面参数（link） |

### 风险诊断 code 补充

| Code | Severity | 说明 |
|------|----------|------|
| `UNRESOLVED_PAGE_NAVIGATION` | Warning | 导航目标页面不存在于图中 |
| `UNRESOLVED_MODEL_WRITE` | Warning | 写入目标模型不存在于图中 |
| `VISIBILITY_RULE_UNRESOLVED` | Warning | visibility 规则中的表达式包含未解析或歧义引用 |


## M6 字段级血缘（Lineage）补充

### lineage 项结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `target_field` | string | 目标字段 ID，如 `field:dataflow_output.工单号` |
| `source_fields` | array | 来源字段列表，如 `["field:dataflow_output.workNo"]` |
| `source_expr` | string / null | 原始表达式，如 `IF(status=="completed","已完成","处理中")` |
| `transform` | string | 转换类型：`inputField mapping` / `expression calculation` / `submitField mapping` / `fieldValues mapping` / `unknown` |
| `via_node` | string / null | 经过的中间节点 ID，如 `model:dataflow_output` |
| `confidence` | string | `high`（inputField / submitField 直接映射） / `medium`（表达式解析出 ModelField） / `low`（表达式无法解析） |
| `evidence` | object | 包含 `source_file`、`node_id`、`edge_type`、`json_path`、`raw_expr` |

### lineage diagnostics code

| Code | Severity | 说明 |
|------|----------|------|
| `LINEAGE_SOURCE_MISSING` | Info | 维度/字段无 inputField 或 exp，无法追溯来源 |
| `LINEAGE_EXPR_UNPARSED` | Info | 表达式存在但解析器无法提取具体字段引用（如 `appointmentNo` 裸标识符） |
| `LINEAGE_AMBIGUOUS_MODEL` | Warning | 字段名无法唯一定位到具体模型（M6 暂不支持，预留） |
| `LINEAGE_CHAIN_TRUNCATED` | Info | 因 budget/depth 限制，血缘链在展开时被截断 |

## M7 表达式结构化解析补充

### 表达式统一结构（condition / visibility_rules / ComponentExpr）

所有表达式字段统一输出以下结构：

| 字段 | 类型 | 说明 |
|------|------|------|
| component_id | string | 表达式所属组件 ID（仅 ComponentExpr 输出） |
| field | string | 表达式所属字段名（仅 ComponentExpr 输出） |
| source_file | string / null | 来源文件（单文件解析时尽量为真实输入路径；无法确定时为 null） |
| json_path | string | 稳定近似 JSON 路径（可直接定位到组件字段） |
| raw_expr | string / null | 原始表达式字符串 |
| refs | array | 提取的引用 ID 列表（去重集合） |
| resolved_refs | array | 结构化引用对象列表（出现级列表，允许重复），每项含 type / id / field / confidence / reason |
| refs_count | number | `refs` 去重后数量 |
| resolved_occurrence_count | number | `resolved_refs` 出现次数总量 |
| unresolved_refs | array | 无法确认类型的引用对象列表，每项含 type / id / reason |
| ambiguous_refs | array | 被分类为 Other 的模糊引用列表，每项含 raw |
| diagnostics | array | 表达式诊断列表，每项含 code / message / position |
| confidence | string | high（无 unresolved 无 diagnostic） / medium（少量 unresolved/diagnostic） / low（大量问题） |

### 引用类型枚举（resolved_refs[].type）

| 类型 | 说明 |
|------|------|
| ComponentValue | 组件值引用，如 input1.value |
| ComponentProperty | 组件属性引用，如 input1.checked.value |
| ModelField | 模型字段引用，如 model1.fieldA 或 ${model1.fieldA} |
| Param | 页面参数引用，如 param1 |
| UserProperty | 用户属性引用，如 $user.name |
| SystemVar | 系统变量引用，如 $currentDate |
| Literal | 字面量或其他无法分类的引用 |

### 表达式诊断 code

| Code | Severity | 说明 |
|------|----------|------|
| EXPR_PARSE_ERROR | Warning | 表达式存在语法错误，无法构建完整 AST |
| EXPR_UNRESOLVED_REF | Info | 标识符或成员访问无法被分类为已知引用类型 |
| EXPR_UNSUPPORTED_FUNCTION | Info | 使用了当前未列入支持列表的函数 |
| EXPR_AMBIGUOUS_REF | Info | 引用存在歧义，可能属于多种类型 |

## 来源分类（SourceType）

组件值的来源类型，用于回答"这个值是用户输入、参数、模型自动带出还是计算得来"。

### 稳定枚举

| 值 | 说明 | 判断规则 |
|----|------|----------|
| `Param` | 页面参数 | 表达式引用 `paramX` 或直接以 `param` 开头 |
| `UserInput` | 用户输入 | 无表达式、可交互输入组件 |
| `ModelAuto` | 模型自动获取 | 表达式引用 `modelX.field` 或直接以 `model` 开头 |
| `System` | 系统变量 | 表达式包含 `$user`、`$project` 等系统变量 |
| `Computed` | 计算产生 | 表达式引用其他组件值（`.value` / `.checked`） |
| `Constant` | 常量 | 纯常量表达式或静态默认值（不以 `=` 开头） |
| `Unknown` | 未知 | 无法分类时保守标记 |

### 字段位置

- `details.value_trace[].source_type`：追溯链中每个节点的来源类型
- `details.lineage[].source_type`：字段 lineage 中每个步骤的来源类型

### 使用指南

AI 被问"这个值从哪里来"时，应优先查看 `source_type`：
- `Param` → "来自页面参数"
- `UserInput` → "用户输入"
- `ModelAuto` → "从数据模型自动获取"
- `System` → "系统变量"
- `Computed` → "由表达式计算产生"
- `Constant` → "固定常量"
- `Unknown` → "来源无法确定，保守回答"

## kind: Table 字段约束

单文件 `.tbl` 物理表/应用表解析输出。

### summary 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `table_id` | string | 是 | 表标识，通常为文件名或 `dbTableName`。 |
| `table_name` | string | 是 | 表显示名或文件名。 |
| `table_type` | string | 是 | `"PhysicalTable" | "AppTable" | "DataFlow"`。单文件 .tbl 无 `dataFlow` 节点时为 `AppTable` 或 `PhysicalTable`。 |
| `field_count` | int | 是 | `dimensions[]` 字段数量。 |
| `input_count` | int | 是 | DataFlow 输入节点数，物理表为 0。 |
| `output_count` | int | 是 | DataFlow 输出目标数，物理表为 0。 |
| `what_is_it` | string | 是 | 自然语言短句，说明表类型和字段数。 |

### details 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `fields` | array | 字段列表，含 `name`、`dbfield`、`data_type`、`length`、`is_dimension`、`input_field`、`exp`、`original_field`、`original_node`。 |
| `dataflow_inputs` | array | DataFlow 输入节点（`ModelTable` 类型）。物理表为空数组。 |
| `dataflow_outputs` | array | DataFlow 输出物理表。物理表为空数组。 |
| `internal_nodes` | array | DataFlow 内部加工节点（`Select`、`AddField` 等）。物理表为空数组。 |
| `field_lineage` | array | 字段级来源追溯，含 `target_field`、`source_fields`、`source_expr`、`transform`、`confidence`。 |

### diagnostics

- `DATAFLOW_NO_OUTPUT`：DataFlow 缺少 `dbTableName`，无法确定输出目标。
- `DATAFLOW_NO_INPUTS`：DataFlow 没有 `ModelTable` 输入节点。
- `EXPR_UNPARSED`：字段表达式过长或包含换行，无法完全解析引用。

## kind: DataFlow 字段约束

单文件 `.tbl` DataFlow 加工表解析输出。结构与 `kind: Table` 相同，但 `table_type` 固定为 `"DataFlow"`。

额外语义：
- `summary.input_count` > 0 表示存在输入源。
- `summary.output_count` > 0 表示存在输出物理表。
- `details.internal_nodes` 展示加工链。
- `details.field_lineage` 反映 `inputField` / `exp` / `originalField` 来源。

### 使用指南

AI 被问"这个 DataFlow 从哪里来、输出到哪里"时：
1. 先看 `summary.what_is_it` 获取整体描述。
2. 查看 `details.dataflow_inputs` 了解输入源。
3. 查看 `details.dataflow_outputs` 了解输出目标。
4. 查看 `details.field_lineage` 了解字段加工逻辑。
5. 遇到 `DATAFLOW_NO_OUTPUT` 或 `EXPR_UNPARSED` 时保守回答。


## kind: GraphDbCheck 字段约束

图数据库状态检查输出，由 `--check-graph` 触发。

### summary 字段

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `db_path` | string | 是 | 检查的图数据库路径。 |
| `exists` | bool | 是 | 文件是否存在。 |
| `readable` | bool | 是 | 当前进程是否可读。 |
| `writable` | bool | 是 | 当前进程是否可写。 |
| `needs_rebuild` | bool | 是 | 是否需要重建（文件不存在、表损坏或打不开）。 |

### diagnostics

- `GRAPH_DB_NOT_FOUND`：图数据库文件不存在。必须运行 `--build-graph` 创建。
- `GRAPH_DB_LOCKED`：图数据库被其他进程占用（redb lock 冲突）。建议等待、使用不同的 `--graph-db-path`，或在真实项目场景增加 `--graph-lock-timeout-ms`（默认 10s，可延至 30s）。
- `GRAPH_DB_PERMISSION_DENIED`：当前进程对图数据库路径无读/写权限。建议更换 `--graph-db-path` 到可写目录。
- `GRAPH_DB_OPEN_ERROR`：其他打开错误。建议 `--build-graph` 重建。

### next_queries

- `--build-graph --graph-db-path <PATH>`：在指定路径重建图数据库。
- `--check-graph --graph-db-path <PATH>`：再次检查指定路径状态。

## GraphDB 相关通用诊断码

以下诊断码可能出现在任何项目级查询输出中（当 graphdb 打开失败时，查询本身返回 `kind=GraphDbCheck`）：

| 诊断码 | 级别 | 触发条件 | 建议 |
|--------|------|----------|------|
| `GRAPH_DB_NOT_FOUND` | error | graphdb 文件不存在 | `--build-graph` |
| `GRAPH_DB_LOCKED` | error | redb lock 冲突，多进程并发 | 等待、换 `--graph-db-path`，或在真实项目场景增加 `--graph-lock-timeout-ms 30000` |
| `GRAPH_DB_PERMISSION_DENIED` | error | 只读目录或无权限 | 换到 `/tmp` 等可写路径 |
| `GRAPH_DB_OPEN_ERROR` | error | 其他 redb/IO 错误 | `--build-graph` 重建 |

## kind: PageQuery / ModelQuery / ComponentQuery（find 命令复用） 字段约束

`--find-page <KEYWORD>`、`--find-model <KEYWORD>`、`--find-component <KEYWORD>` 输出统一 AI JSON。

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `keyword` | string | 查询关键词 |
| `match_count` | number | 匹配结果数量 |
| `what_is_it` | string | 自然语言说明匹配结果概况 |

### details.matches[] 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | string | 节点完整 ID |
| `name` | string | 节点名称 |
| `node_type` | string | 节点类型 |
| `source_file` | string | 来源文件路径 |
| `match_score` | number | 匹配分数 |
| `match_reason` | string | 匹配原因 |

### diagnostics

- `NO_MATCHES_FOUND`：未找到匹配节点，建议扩大关键词或检查拼写

### next_queries

- `--explain '<ID>'`：查看第一个匹配目标的语义摘要
- `--context '<ID>' --depth 2`：查看第一个匹配目标的上下文

## kind: ModelResolve 字段约束（内部 kind，可能复用 ModelQuery）

`--resolve-model-page <PAGE_ID> --resolve-model <LOCAL_MODEL_ID>` 在页面作用域内解析局部模型 ID。

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `page_id` | string | 页面完整 ID |
| `local_model_id` | string | 用户传入的局部模型名 |
| `resolved_count` | number | 解析成功数量 |
| `ambiguous` | boolean | 候选是否不唯一 |
| `what_is_it` | string | 自然语言说明解析结果 |

### details.candidates[] 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `model_id` | string | 候选节点完整 ID |
| `model_name` | string | 候选节点名称 |
| `confidence` | number | 匹配分数（0-100） |
| `match_reason` | string | 匹配原因（如 "exact match" / "substring match" / "global model match"） |

### diagnostics

- `TARGET_NOT_FOUND`：页面不存在时输出，附带 `candidate_targets`
- `AMBIGUOUS_RESOLUTION`：当解析出多个候选时输出，提示 AI 需要进一步确认

### next_queries

- `--explain '<ID>'`：查看候选目标语义摘要
- `--find-model '<KEYWORD>'`：当解析失败时，使用全局搜索

## ConditionRecord 字段定义

`--explain`、单文件 `.spg` 解析、以及项目级查询的 `details.conditions` 数组中，条件统一使用以下结构：

| 字段 | 类型 | 说明 |
|------|------|------|
| `condition_id` | string | 条件唯一标识，格式为 `{owner_id}#{field}#{index}` |
| `condition_type` | string | 条件技术来源，见下表 |
| `effect_type` | string | 条件业务效果：`show`/`hide`/`enable`/`disable`/`readonly`/`execute`/`filter`/`compute`/`validate`/`mask`/`submit`/`default_panel` |
| `subject_type` | string | 条件作用对象：`component`/`action`/`model_source`/`field`/`page` |
| `raw_expr` | string | 原始表达式字符串，保留多行和原始空白 |
| `normalized_expr` | string | 去首尾空白的规范化表达式 |
| `source_file` | string | 来源文件路径（绝对路径或相对路径） |
| `json_path` | string | 在原始 JSON 中的精确路径，如 `canvas.components[0].visibleCondition` |
| `owner_type` | string | 条件所属对象类型：`component`/`action`/`model_source`/`field`/`page` |
| `owner_id` | string | 所属对象 ID（组件 ID、action ID、source ID 等） |
| `referenced_symbols` | string[] | 表达式中引用的符号列表，格式为 `component:{id}`、`model:{model}.{field}`、`param:{name}`、`user:{prop}`、`system:{name}` |
| `diagnostics` | object[] | 解析诊断，每项包含 `code` 和 `message` |

### condition_type 枚举

| 值 | 来源字段 | 说明 |
|------|------|------|
| `visible_condition` | `visibleCondition` / `visible` | 组件显示/隐藏条件 |
| `enable_condition` | `enable` / `disableCondition` | 组件启用/禁用条件 |
| `action_condition_exp` | `action.conditionExp` | 动作执行条件（表达式形式） |
| `action_condition` | `action.condition` | 动作执行条件（字符串形式） |
| `item_filter` | `itemFilter` | 列表/表格项过滤条件 |
| `calc_condition` | `calcCondition` | 计算触发条件 |
| `valid_exp` | `validExp` | 校验表达式 |
| `calc_exp` | `calcExp` | 计算表达式 |
| `mask_condition` | `maskCondition` | 掩码/脱敏条件 |
| `submit_condition` | `submitCondition` | 提交条件 |
| `submit_page_condition` | `submitPageCondition` | 页面提交条件 |
| `default_panel_condition` | `defaultPanelCondition` | 默认面板条件 |
| `field_exp` | `exp` / `value` / `text` / `formula` / `html` 等 | 通用字段表达式 |
| `default_value_exp` | `defaultValue` | 默认值表达式 |
| `source_filter_exp` | `source.filter.clauses[].exp` | 数据源过滤表达式（完整表达式） |
| `source_filter_clause` | `source.filter.clauses[].leftExp` | 数据源过滤子句（拆分形式） |

### effect_type 与 subject_type 映射

| condition_type | effect_type | subject_type |
|------|------|------|
| `visible_condition` | `show` | `component` |
| `enable_condition` | `enable` | `component` |
| `action_condition_exp` / `action_condition` | `execute` | `action` |
| `source_filter_exp` / `source_filter_clause` | `filter` | `model_source` |
| `calc_condition` / `calc_exp` / `field_exp` / `default_value_exp` | `compute` | `component` 或 `field` |
| `valid_exp` | `validate` | `component` |
| `mask_condition` | `mask` | `component` |
| `submit_condition` / `submit_page_condition` | `submit` | `component` |
| `default_panel_condition` | `default_panel` | `component` |

### 单文件 .spg 输出中的 conditions

单文件解析时，`summary.page_info` 增加 `conditions_count` 字段；`details` 中增加 `conditions` 数组，元素为上述 `ConditionRecord`。

### 诊断码

- `EMPTY_CONDITION`：条件表达式为空字符串，不生成有效记录但保留诊断
- `EXPR_DIAGNOSTIC_*`：表达式 AST 解析产生的诊断（如未解析引用、不支持函数等）

## PageLogic 查询输出

### 概述

`--query-page-logic` 输出页面级数据可用性摘要，核心目标是让 AI 不读 raw `.spg` 也能理解：
- 页面正常显示数据的主要前置条件
- 关键字段值的来源链路
- 相关但非必要的旁路关系

### summary 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `page_id` | string | 页面节点 ID，如 `page:app/销售.app/销售/合同协议.spg` |
| `page_name` | string | 页面名称 |
| `what_is_it` | string | 一句话描述页面角色 |
| `page_role` | string | `readonly_dashboard` / `mixed_interaction_page` / `data_maintenance_page` / `navigation_page` |
| `entrypoint_count` | int | 用户可触发入口数量 |
| `data_source_count` | int | 数据源读取数量 |
| `write_target_count` | int | 写入目标数量 |
| `navigation_count` | int | 页面跳转关系数量 |
| `display_prerequisites_count` | int | 显示前置条件数量 |
| `data_prerequisites_count` | int | 数据前置条件数量 |
| `action_prerequisites_count` | int | 动作前置条件数量 |
| `primary_paths_count` | int | 主链路数量 |
| `related_context_count` | int | 旁路关系数量 |
| `top_display_prerequisites` | object[] | Top-3 显示前置条件（AI 优先读取） |
| `top_data_prerequisites` | object[] | Top-3 数据前置条件 |
| `top_action_prerequisites` | object[] | Top-3 动作前置条件 |
| `key_primary_paths` | object[] | Top-5 主链路（AI 优先读取） |

### details 字段（budget 差异）

**compact 模式**：
- 所有数组被截断到 Top-5/Top-3
- 输出 `related_context_summary`（计数 + 类型分布）
- 不输出 `candidate_paths`、`rejected_paths`、`path_selection_diagnostics`

**normal 模式**：
- 输出完整 `primary_paths`、`supporting_paths`、`related_context`
- 不输出 `candidate_paths`、`rejected_paths`

**full 模式**：
- 额外输出 `candidate_paths`（候选解释链路）
- 额外输出 `rejected_paths`（被过滤路径 + 过滤原因）
- 额外输出 `path_selection_diagnostics`（选择器诊断）

### PathCandidate 结构

每条路径候选包含：

| 字段 | 类型 | 说明 |
|------|------|------|
| `path_id` | string | 路径唯一标识，如 `comp:input1>model:model22>model:fact_qwSidebar` |
| `purpose` | string | 路径用途描述 |
| `terminals` | string[] | 路径终端类型：`TargetComponent`、`SinkPhysicalField`、`CrossPageWriter`、`DataFilter`、`Entrypoint`、`Unknown` |
| `segments` | object[] | 路径段数组，每段包含 `from`、`to`、`edge`、`evidence`、`confidence` |
| `evidence` | string[] | 路径证据文本 |
| `selection_reason` | string | 被选择/保留的理由 |
| `classification` | string | 路径分类：`PrimaryPath`、`CandidatePath`、`SupportingPath`、`RelatedContext`、`RejectedPath` |
| `classification_reason` | string | 分类理由 |
| `confidence` | string | `high` / `medium` / `low` |
| `diagnostics` | string[] | 路径级诊断 |
| `rank_features` | object | 排名特征（见下） |

### PathSegment 结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `from` | object | 起点节点：`node_id`、`node_type`、`path`、`name` |
| `to` | object | 终点节点：同上 |
| `edge` | object | 边信息：`edge_type`、`field_path`、`json_path`、`source_expr` |
| `evidence` | string | 段证据 |
| `confidence` | string | `high` / `medium` / `low` |

#### 字段级主链路标准结构

典型的跨页字段因果链路由三段 `PathSegment` 组成：

1. **组件读取局部模型字段**：`Component --Reads--> field:modelX.fieldY`
   - `edge_type`: `Reads`
   - `field_path`: `modelX.fieldY`
   - `source_expr`: 原始表达式，如 `${modelX.fieldY}`
2. **局部字段映射到物理表字段**：`field:modelX.fieldY --FieldAlias--> field:physicalModel.fieldY`
   - `edge_type`: `FieldAlias`
   - 保留局部模型来源和物理表目标
3. **物理表字段被跨页 action 写入**：`field:physicalModel.fieldY <--FieldWrite-- action:otherPage|button|actionN`
   - `edge_type`: `FieldWrite`
   - `field_path`: `physicalModel.fieldY`
   - `source_expr`: 写入来源，如 `input2` 或 `NULL`

该三段路径进入 `key_primary_paths` 的前提是 `rank_features` 同时满足：
- `contains_target_component = true`
- `contains_physical_field = true`
- `contains_field_alias = true`
- `contains_field_write = true`
- `contains_cross_page_writer = true`

### rank_features 结构

| 字段 | 类型 | 说明 |
|------|------|------|
| `contains_target_component` | bool | 是否包含目标组件 |
| `contains_physical_field` | bool | 是否包含物理表字段 |
| `contains_cross_page_writer` | bool | 是否包含跨页写入 |
| `field_name_match` | bool | 字段名是否匹配 |
| `same_page` | bool | 是否全路径在当前页面 |
| `has_condition_node` | bool | 是否包含条件节点 |
| `has_json_path` | bool | 是否有原始 JSON 路径证据 |
| `edge_type_sequence` | string[] | 边类型序列 |
| `path_length` | int | 路径段数 |
| `source_file_count` | int | 涉及的不同源文件数 |
| `contains_only_structural_edges` | bool | 是否仅结构边（Contains/Triggers） |
| `contains_dataflow_side_branch` | bool | 是否包含 DataFlow 旁路 |
| `contains_entrypoint` | bool | 是否包含入口组件 |
| `contains_model_filter` | bool | 是否包含模型过滤 |
| `contains_model_read` | bool | 是否包含模型读取 |
| `contains_model_write` | bool | 是否包含模型写入 |
| `contains_field_alias` | bool | 是否包含局部模型字段到物理表字段的 FieldAlias 映射 |
| `contains_field_write` | bool | 是否包含字段级写入（FieldWrite） |

### 设计原则

1. **rank_features 不等于最终打分**：特征供策略使用，但不直接决定真伪。
2. **分组保底**：key_primary_paths 采用分类保底策略，避免单一 Top-K 忽视问题。
3. **可替换选择器**：`PathSelector` trait 允许后续接入 `WeightedPathSelector`、`FutureLearningPathSelector` 等。
4. **related_context 不是必要条件**：明确标记为参考信息，不进入主结论。
