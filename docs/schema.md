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
  "kind": "SuperPage | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic",
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
   - `Explain`：节点存在 evidence
   - `Context`：closure 范围 + upstream/downstream 独立 evidence
   - `PageLogic`：entrypoints/data_sources/write_targets 独立 evidence

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
| `importance` | string | 稳定分类：`entrypoint` / `data_source` / `write_target` / `navigation` / `calculated_display` / `container` / `unknown` |
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
| `writes` | array | 目标写入的对象列表 |
| `triggered_by` | array | 触发目标的源对象 |
| `affects` | array | 目标影响的下游对象 |
| `lineage` | array | 字段级血缘（M6 已实现）：每项包含 target_field / source_fields / source_expr / transform / confidence / evidence |
| `inputs` | array | DataFlow 输入源列表（DataFlow 有） |
| `outputs` | array | DataFlow 输出目标列表（DataFlow 有） |
| `internal_topology` | object | DataFlow 内部节点拓扑：nodes + edges（DataFlow 有） |
| `produced_by` | array | 字段产生者列表（Field 有） |
| `action_category` | string | 动作语义分类（Action 有）：data_write / data_read / navigation / param_mutation / ui_control / validation / data_refresh / data_initialization / unknown |
| `semantic_summary` | string | 动作自然语言摘要（Action 有） |
| `blocks_on` | object / null | 等待前置动作结构化解析（Action 有） |
| `condition` | object / null | 条件执行表达式结构化解析（Action 有） |
| `trigger_type` | string | 触发类型：click / hover / focus / ...（Action 有） |

### 允许为空的字段

- `details.lineage`：空数组表示无 traceable 来源，必须附带 `LINEAGE_SOURCE_MISSING` diagnostic；M6 已实现从 dimensions[].inputField / dimensions[].exp / submitField / fieldValues[] 追溯
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
  "code": "EVIDENCE_INCOMPLETE | CYCLE_DEPENDENCY | OUTPUT_TRUNCATED | NO_WRITE_TARGETS | NO_ENTRYPOINTS | ...",
  "message": "人类可读描述",
  "location": {
    "source_file": "...",
    "node_id": "...",
    "json_path": "..."
  },
  "suggestion": "建议的修复或下一步查询"
}
```

## 输出体积控制（Budget）

| Budget | 说明 |
|--------|------|
| `compact` | 仅 summary + 关键 evidence，details 深度截断到 1 层。 |
| `normal` | summary + 完整 details + 前 20 条 evidence。 |
| `full` | 不截断，包含所有 evidence 和原始元数据引用。 |

非法 budget 产生错误：CLI 层校验 `compact|normal|full`，非法值直接错误退出，不静默按 normal 处理。

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
| `high_value_relations` | object | 各类关系数量统计：upstream/downstream/related_actions/related_models/related_pages/related_components |

### details 字段

| 字段 | 类型 | 说明 |
|------|------|------|
| `upstream` | array | 谁影响我（按语义归类：读取来源、触发来源、父页面/父模型、DataFlow 输入） |
| `downstream` | array | 我影响谁（被哪些组件/action 使用、写入哪些模型/字段、打开哪些页面） |
| `related_actions` | array | 与目标强相关的 action |
| `related_models` | array | 相关模型和字段（保留 field_path） |
| `related_pages` | array | 所属页面、打开关系页面、嵌入页面、引用页面 |
| `related_components` | array | 相关组件依赖闭包 |

### diagnostics

- `OUTPUT_TRUNCATED`：当 `truncated=true` 时出现，说明被截断的类别和原始数量

### next_queries

根据目标类型生成真实可运行命令：
- component/action → `--explain <ID>`、`--query-page-logic <PAGE>`
- model/field → `--explain <ID>`、`--query-model <MODEL>`
- page → `--explain <PAGE>`、`--query-page-logic <PAGE>`
- dataflow → `--query-dataflow <MODEL>`、`--explain <MODEL>`

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
| `data_sources` | array | 读取的模型字段，包含 source_component / source_action / model / field_path / target_id |
| `write_targets` | array | 写入的模型字段，包含 source_component / source_action / model / field_path / target_id |
| `entrypoints` | array | 用户可触发组件（button、link 等），input 即使有 submitField 也不计入 |
| `action_flows` | array | 动作链，每项包含 action_id / action_type / component_id / trigger_type / reads / writes / navigation / sets_params |
| `visibility_rules` | array | 组件可见性规则（visible / hidden / disabled / readonly） |
| `navigation` | array | 跳转/嵌入关系，包含 from / to / type / field_path |
| `risk_diagnostics` | array | 风险诊断列表 |

### risk_diagnostics code

| Code | Severity | 说明 |
|------|----------|------|
| `NO_WRITE_TARGETS` | Info | 页面无写入目标（可能是只读页面） |
| `NO_ENTRYPOINTS` | Warning | 页面无用户可触发入口 |
| `ACTION_FLOW_INCOMPLETE` | Info | Action 读取但未写入（可能是查询动作） |
| `PAGE_INPUTS_DEFERRED` | Info | 未能在文件系统中读取原始页面文件，page_inputs 为空 |

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
| `action_category` | string | 动作语义分类：data_write / data_read / navigation / param_mutation / ui_control / validation / data_refresh / data_initialization / unknown |
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
| `VISIBILITY_RULE_UNRESOLVED` | Info | visibility 规则中的表达式可能包含未解析引用 |


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
| `LINEAGE_DEFERRED_TO_M8` | Info | 表达式诊断和证据体系需要完整化，当前解析器未输出完整 evidence（预留） |

## M7 表达式结构化解析补充

### 表达式统一结构（condition / visibility_rules / ComponentExpr）

所有表达式字段统一输出以下结构：

| 字段 | 类型 | 说明 |
|------|------|------|
| raw_expr | string / null | 原始表达式字符串 |
| refs | array | 提取的引用 ID 列表（去重） |
| resolved_refs | array | 结构化引用对象列表，每项含 type / id / field / confidence / reason |
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

