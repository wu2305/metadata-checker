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
| `lineage` | array | 字段级血缘（M2 保留空数组） |

### 允许为空的字段

- `details.lineage`：M2 保留空数组，必须附带 `LINEAGE_DEFERRED_TO_M6` diagnostic
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
