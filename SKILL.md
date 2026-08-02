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

## 使用边界

`metadata-checker` 是 Rust CLI 工具，用于解析低代码平台 `.spg` SuperPage 元数据和 `.tbl` Table/DataFlow 元数据。AI 使用本 skill 的目标是低噪声、可核验地选择查询路径并消费机器输出，不是背诵 CLI 参数列表。

优先级：
1. 有 Function Calling 工具时，优先调用工具层；工具层通过 stdio server 复用已加载 graph。
2. 没有工具层时，使用 CLI 命令；CLI 示例中的单引号只服务于 shell 转义。
3. 只做单文件分析时，直接解析 `.spg` 或 `.tbl`；需要跨文件关系时必须使用项目级 graphdb。

调用层边界：
| 场景 | target 写法 | 示例 |
|---|---|---|
| Shell / CLI | 含 `|`、中文、`$`、空格、括号时用单引号 | `--explain-condition 'comp:app/售后.app/首页.spg|button1'` |
| stdio JSON | 不带 shell 单引号 | `"target":"comp:app/售后.app/首页.spg|button1"` |
| Function Calling | 不带 shell 单引号，由 wrapper 传 JSON payload | `{ "target": "comp:app/售后.app/首页.spg|button1" }` |

`--human` 和 `--interactive` 只用于人工探索，不用于自动化/机器消费。

## 快速分流

先按用户意图选工具或命令；如果不确定，先用 `--advise-query TARGET --question-kind ...` 获取结构化推荐。

| 用户问题 | 首选工具或命令 | 首读字段 | 禁止混淆 |
|---|---|---|---|
| 这个组件什么时候显示/为什么不显示？ | `metadata_explain_condition` 或 `--explain-condition 'comp:PAGE|ID' --intent display --budget compact` | `summary.intent`、`details.answer_contract`、`details.answer_facts.display_facts` | `value` / `exp` 不是显示条件 |
| 这个组件显示什么值/值来自哪张表？ | `metadata_explain_condition` 或 `--explain-condition 'comp:PAGE|ID' --intent value-source --budget compact` | `details.answer_facts.value_source_facts`、必要时读 `details.value_source_context` | 裸字段 `${FIELD}` 不是表名 |
| 这个字段被谁写入/生成？ | `metadata_explain_condition` 或 `--explain-condition 'field:MODEL.FIELD' --intent writer --budget compact` | `details.answer_facts.writer_facts`、`details.primary_path` | `related_context` 不是必要条件 |
| 为什么数据源可能为空？ | `metadata_explain_condition` 或 `--explain-condition 'model:ID' --intent availability --budget compact` | `details.answer_facts.availability_facts` | DataFlow filter 不是组件显示门禁 |
| 这个按钮/动作做什么/为什么点不动？ | 先 `--explain-condition 'comp:PAGE|ID' --intent action --budget compact` 拿到 action target，再 `--explain 'action:PAGE|button|action'` | `details.answer_facts.action_facts.actions[].action_target`、`gate_conditions`；动作级再读 `summary.what_is_it`、`details.triggers`、`details.writes_models` | `action_facts.related_actions` 是别的组件的动作，只是门禁引用了本目标 |
| 这个页面做什么/有哪些逻辑？ | `metadata_query_page_logic` 或 `--query-page-logic 'page:PAGE' --budget compact` | `summary.key_findings`、`details.action_flows` | compact 数组可能截断 |
| 谁读写这个模型/表？ | `metadata_query_model` 或 `--query-model MODEL --budget compact` | `summary`、`details.read_by`、`details.write_by`、`details.consumed_by_dataflows` | `read_by_count=0` 不等于未使用 |
| 周围还有什么关系？ | `metadata_context` 或 `--context 'ID' --depth 2 --budget normal` | upstream/downstream 摘要 | context 是补充，不是主答案 |

## Single Thinking Flow

所有机器输出都按同一条阅读流消费：
1. 判断 intent / question kind：从 `summary.intent`、用户问题和命令类型确认 display、value-source、writer、availability、action、page-logic、model-relationships 等意图。
2. 先读 `summary`：优先看 `summary.what_is_it`、`summary.primary_reason`、`summary.key_findings`、`summary.evidence_summary` 和计数字段。
3. 对 explain-condition 和 advise-query，读 `details.answer_contract`：确认 `primary_fact_path`、`forbidden_fact_paths[]`、`must_read_summary_first`。
4. 只深入主证据块：按 `primary_fact_path` 读取 `details.answer_facts.<fact_block>`，不要把 forbidden fact block 混成同一结论。
5. 查容量和后续动作：若 `details.truncation_guard.safe_to_answer_full_relationships=false` 或有 `OUTPUT_TRUNCATED`，按 `required_budget_for_complete_answer` 从 `--budget compact` 升级到 `normal` 或 `--budget full`；若 `details.required_followups[]` 中 `must_run_for_complete_answer=true`，先执行 followup。
6. 需要核验时再读 `evidence`：优先使用 `confidence=high` 且有真实 `source_file` / `json_path` 的证据。
7. 有 `diagnostics` 时降级回答：按 Diagnostics & Fallback Matrix 说明限制、下一步和不确定性。
8. 最终回答声明使用的 fact block 或证据类型，例如“根据 display_facts...”。

默认不要读取 raw JSON 大对象，不要默认使用 full budget。

## 目标定位协议

目标不确定时先定位，禁止猜测 `model1`、`model5`、`model74` 这类局部 ID。

| 目标类型 | 格式 | 说明 |
|---|---|---|
| component | `comp:app/page.spg|button1` | 单文件模式可用裸 ID，如 `button1` |
| action | `action:app/page.spg|button1|action1` | 指向具体组件动作 |
| model | `model:model1` 或 `modelName` | CLI 的 `--query-model` 兼容裸模型名和 `model:` 前缀 |
| field | `field:model1.fieldA` | 字段级追溯 |
| page | `page:app/page.spg` | 页面节点使用规范化相对路径 |
| dataflow | `model:dataflow_output` | DataFlow 在图中也是 Model 类型 |

定位步骤：
1. 不知道精确目标：先用 `--find-page <KEYWORD>`、`--find-model <KEYWORD>`、`--find-component <KEYWORD>`。
2. 页面内局部 model ID：用 `--resolve-model-page 'page:...' --resolve-model model5` 映射到真实全局模型。
3. 拼写错误或不存在：读取 `TARGET_NOT_FOUND` 和 `candidate_targets`，从候选确认，不要自动替用户选。
4. 目标含特殊字符时，CLI 用单引号；stdio / function calling JSON payload 不加单引号。

## 项目级图数据库流程

项目级查询都需要 `--project-dir <DIR>`，并通常依赖 graphdb。

```bash
# 检查 graphdb 状态
metadata-checker --check-graph --graph-db-path /tmp/project.graphdb

# 构建 graphdb
metadata-checker --project-dir /path/to/project --build-graph --graph-db-path /tmp/project.graphdb

# 查询时复用 graphdb
metadata-checker --project-dir /path/to/project --query-page-logic 'page:app/销售.app/销售/合同协议.spg' --graph-db-path /tmp/project.graphdb --budget compact
```

默认 graphdb 路径是 `<project-dir>/.metadata-checker.graphdb`。项目目录只读或沙箱限制写入时，把 `--graph-db-path` 指向 `/tmp/...`。

可用项目级查询：
- `--query-model <MODEL>`：模型读写关系。
- `--query-page <PAGE>`：页面依赖。
- `--query-cross <A> <B>`：跨页关系。
- `--query-dataflow <MODEL>`：DataFlow 子图。
- `--query-page-logic <PAGE>`：页面入口、动作、读写、跳转、可见性。
- `--explain <ID>`：解释任意节点。
- `--context <ID> --depth 2 --budget normal`：补查上下游。

## Session 差量刷新路径

已有远端 session 需要更新时，先执行一轮 one-shot 差量刷新：

```bash
metadata-checker --session-dir ~/.metadata-checker/sessions \
  --session-diff-refresh <SESSION_ID> \
  --remote-username '<USER>' --remote-password '<PASSWORD>'
```

需要在同一进程内连续查询时，启动 stdio 并绑定 session：

```bash
metadata-checker --session-dir ~/.metadata-checker/sessions \
  --serve-stdio --runtime-session-id <SESSION_ID> \
  --remote-username '<USER>' --remote-password '<PASSWORD>'
```

随后发送 JSONL 请求：

```json
{"request_id":"refresh-1","command":"diff_refresh"}
```

`diff_refresh` 是一次刷新操作，不是普通查询；成功结果中的 `persist_report` 来自实际 graph persist，`checkpoint` 表示本轮消费水位。one-shot 和 stdio 均输出单行 JSON，并带 `schema_version="1.0"`、`kind="DiffRefresh"`。不要读取 `timing` 作为业务证据。

凭证边界：密码不写入 session manifest；401/403 返回稳定错误码和脱敏后的服务端 message，不默认静默重登。遇到鉴权失败，重新以用户名/密码绑定 session；不要把密码、token、cookie、cipherPassport 或 Set-Cookie 复制进回答。

## explain-condition 协议

`--explain-condition` 用于回答显示、可用性、值来源、写入来源问题。

支持 target：
- `comp:PAGE|ID`
- `model:ID`
- `field:MODEL.FIELD`
- `page:PATH`

intent 到 fact block 映射：
| intent | 主证据块 |
|---|---|
| `display` | `details.answer_facts.display_facts` |
| `value-source` | `details.answer_facts.value_source_facts` |
| `writer` | `details.answer_facts.writer_facts` |
| `availability` | `details.answer_facts.availability_facts` |
| `action` | `details.answer_facts.action_facts` |

关键字段：
- `details.answer_contract.primary_fact_path`：本次唯一主证据路径。
- `details.answer_contract.forbidden_fact_paths[]`：不能混入主结论的证据块。
- `details.answer_facts.*.paths[].steps[].why_included`：每一步为什么被纳入链路，用它过滤普通图邻居。
- `details.thinking_frame`：缺什么、不能用什么、下一步怎么查。
- `details.truncation_guard`：当前 budget 是否足以回答全量关系。
- `details.required_followups[]`：完整答案必须执行或建议执行的后续查询。

### Display Logic Gating Hierarchy

显示/隐藏问题只使用显示门禁证据，不使用 `value`、`exp` 或 `primary_path` 作为显示条件。

按以下 if-else 读取 `condition_scope`：
1. If `condition_scope = direct`：这是目标组件自身的显示/禁用/只读条件。
2. Else if `condition_scope = inherited`：这是祖先容器条件，是目标组件能显示的必要门禁。
3. If direct 或 inherited 条件引用 `modelX.totalRowCount__`：继续读取 `details.data_empty_gates` 中 `condition_scope = expanded_from_total_row_count` 的展开规则。
4. Else if `condition_scope = referenced_by_model_filter`：这是其他模型 filter 反向引用目标组件，只能作为 supporting context，不是目标显示门禁。
5. 若存在 `deduped_condition_ids` / `deduped_owner_node_ids`，只在审计重复条件时使用。

### Bare Symbol Protocol

当组件值形如 `${FIELD}` 且 `FIELD` 没有模型前缀时，不要回答“来自 FIELD 表”或“读取 model:FIELD”。

读取链路：
1. `raw_expr` / `bare_symbol`
2. `nearest_data_context.component_id` 和 `nearest_data_context.dataSet`
3. `field_path = dataSet.FIELD`
4. `table_source_path` 指向页面 source 或 DataFlow 表
5. `dataflow_field_origin.module_table_path` 存在时，才可作为字段级物理输入证明

如果只有 `candidate_inputs[]` 或 `table_source_path`，只能说候选来源；不能把 `candidate_inputs` 当作 `proven_physical_input`。

### 语义边界

- `blocking_conditions` / `data_empty_gates` 回答“何时能显示/何时有数据”。
- `value_source_context` 回答“显示之后值从哪里来”。
- `primary_path` 回答“字段级读写因果链”。
- `related_context` 和 `supporting_context` 是相关上下文，不是必要条件，除非用户明确要求扩展范围。
- 普通链式组件引用不默认递归展开；例如 `A.visibleCondition = B.value != ''` 时，只有用户追问 B 的来源才继续查 B。

## Function Calling 工具层

同一项目连续追问时，优先让 function calling wrapper 通过 stdio server 复用已加载 graph。工具层只暴露任务型工具，不让 AI 直接拼任意 stdio request。

| Tool | stdio command | 何时使用 |
|---|---|---|
| `metadata_explain_condition` | `explain_condition` | 为什么不显示 / 为什么没数据 / 值从哪来 |
| `metadata_explain` | `explain` | 这个对象是什么 |
| `metadata_context` | `context` | 周围关系是什么 / 需要补查上下游 |
| `metadata_query_model` | `query_model` | 模型读写全貌 / 谁写了这个表 |
| `metadata_query_page` | `query_page` | 页面出边/入边关系 |
| `metadata_query_page_logic` | `query_page_logic` | 页面整体逻辑 / 入口 / 写入 / 跳转 / 可见性 |
| `metadata_advise_query` | `advise_query` | 为目标生成结构化查询建议 |
| `metadata_runtime_status` | `status` | 只检查 runtime/graph 状态 |
| `metadata_runtime_reload` | `reload_graph` | graphdb 更新后手动刷新 |
| `metadata_runtime_check_reload` | `check_reload` | graphdb 变更时自动刷新 |

工具层约束：
- `timing` 只能用于性能判断，不能作为业务证据。
- `timing.output_size_bytes` 用于容量治理，表示最终 stdout JSON 行字节数。
- 业务回答优先读取 `result.summary`。
- 证据核查读取 `result.details.primary_path`、`result.summary.key_primary_paths`、`result.evidence`。
- `related_context` 默认不是必要条件，只能作为相关上下文表述。
- `metadata_runtime_status` / `metadata_runtime_reload` / `metadata_runtime_check_reload` 不能回答业务来源链路。

## 输出结构

所有机器输出默认遵循统一顶层结构：

```json
{
  "schema_version": "1.0",
  "kind": "SuperPage | Table | PageQuery | ModelQuery | CrossPageQuery | DataFlowQuery | ComponentQuery | PriorityQuery | Explain | Context | PageLogic",
  "query_target": "...",
  "summary": {},
  "details": {},
  "evidence": [],
  "diagnostics": [],
  "next_queries": []
}
```

summary-first 规则：
- 先读 `summary.what_is_it`、`summary.page_role` / `importance`、`summary.key_findings`、`summary.evidence_summary`。
- 需要核验再读 `evidence`。
- summary + evidence 仍不足时才展开 `details`。
- 遇到 `diagnostics` 必须保守表达。

### summary 与 details 冲突

当 `summary` 的计数/角色字段与 `details` 数组看似不一致时，通常是 compact 截断。

处理优先级：
1. 优先采信 `summary.dataflow_role`、`summary.consumed_by_dataflow_count`、`summary.produced_by_count`、`summary.dataflow_input_count`、`summary.dataflow_output_count`。
2. `details.*` 在 compact 下可能只是 Top-N 样本，不代表全集。
3. 禁止只看 `read_by_count=0` 就断言“模型没有被使用”；必须同时检查 `consumed_by_dataflow_count` 和 `dataflow_role`。
4. 禁止只看 `details.upstream` 为空就断言“没有上游依赖”。

### Budget 升级

| Budget | 使用时机 | 说明 |
|---|---|---|
| `compact` | 第一轮默认 | 低噪声 summary + key_findings + evidence_summary + Top-N details |
| `normal` | 需要核查关键细节 | 展开主要 details，数组仍可能截断 |
| `full` | 深度审计、必须完整数组 | 输出完整 details，不作为第一轮默认 |

只有遇到 `OUTPUT_TRUNCATED`、`truncation_guard` 提示、或用户明确要求全量关系时，才从 `compact` 升级。

## 证据强弱分级

AI 引用 CLI 输出时必须区分证据可信度。

| 证据特征 | 可信度 | 表达要求 |
|---|---|---|
| `json_path` 为真实文件路径，`source_file` 存在，`node_id` 非 `?` | high | 可直接引用 |
| `json_path="<graph-edge-derived>"`，无原始 JSON 路径 | medium/low 弱证据 | 降级表达，标注“图推导” |
| `node_id="?"` 或 `source_file` 缺失 | low 弱证据 | 说明“证据位置缺失” |
| `raw_expr="n/a"` 且 `confidence=medium` | low 弱证据 | 不可作为独立依据 |
| 出现 `EVIDENCE_LOCATION_MISSING` | 需人工确认 | 说明“该结论缺少原始文件定位” |

## Diagnostics & Fallback Matrix

真实项目故障处理和输出降级统一按下表执行。

| code / severity | 含义 | AI 回答口径 | 下一步 |
|---|---|---|---|
| `GRAPH_DB_NOT_FOUND` | graphdb 不存在 | 不能做项目级关系结论 | 执行 `--build-graph` |
| `GRAPH_DB_LOCKED` | 锁冲突 | 当前查询受阻 | 等待、换 `--graph-db-path`，或加 `--graph-lock-timeout-ms 30000` |
| `GRAPH_DB_PERMISSION_DENIED` | 只读或无权限 | 当前路径不可写 | 使用 `/tmp/...` graphdb 路径 |
| `GRAPH_DB_OPEN_ERROR` | redb/IO 错误 | graphdb 状态不可信 | 重建 graphdb |
| `TARGET_NOT_FOUND` | 目标不存在或拼写错误 | 不要猜测目标 | 读取 `candidate_targets`，或先 `--find-model` / `--find-page` / `--find-component` |
| `OUTPUT_TRUNCATED` | 输出数组被截断 | 只能回答 Top-N 或摘要结论 | 按需升级 budget |
| `EVIDENCE_SAMPLED` | evidence 为控噪采样 | 可回答局部结论，不可声称全集 | 需要全集时升级 budget 或读 details 全量数组 |
| `EVIDENCE_INCOMPLETE` | 证据不完整 | 降级为“初步判断” | 补查 next_queries |
| `UNRESOLVED_REF` | 引用未解析 | 不把该引用当确定链路 | 先定位目标或补查上下文 |
| `LINEAGE_SOURCE_MISSING` | 血缘来源缺失 | 字段来源不完整 | 查 lineage / DataFlow |
| `LINEAGE_EXPR_UNPARSED` | 表达式无法解析 | 不能确定字段转换来源 | 说明表达式解析限制 |
| `UNKNOWN_ACTION_TYPE` | 动作类型未知 | 不强行解释动作为写入/跳转 | 查原始 evidence 或 next_queries |
| `severity=error` | 查询或解析失败 | “输出包含错误诊断，无法确定” | 修复后重试 |
| `severity=warning` | 结论可能不完整 | “存在警告诊断，结论可能不完整” | 补查证据 |
| `severity=info` | 提示信息 | 可继续使用主结论 | 仅在相关时说明 |

单文件 fallback：如果 graphdb 无法构建或查询失败，但只需要分析单个文件，可直接运行：

```bash
metadata-checker page.spg --budget compact
metadata-checker table.tbl --budget compact
```

此时无法获得跨文件关系，只能得到单文件组件语义、字段血缘或表结构。

## .tbl 与 DataFlow 路径

单文件 `.tbl` 和项目级 DataFlow 是两条不同路径。

| 场景 | 命令 | 可回答 | 不可回答 |
|---|---|---|---|
| 单文件 `.tbl` | `metadata-checker app_table.tbl --budget compact` | 表类型、字段列表、单文件 `field_lineage` | 全局读写关系、被哪些页面消费 |
| 项目级 DataFlow | `--query-dataflow model:dataflow_name --budget compact` | `details.inputs[]`、`details.outputs[]`、`details.consumed_by_dataflows[]` | 单个页面组件显示条件 |
| 项目级模型 | `--query-model model:fact_saleContract --budget compact` | 模型被哪些页面/组件/DataFlow 读写 | 单文件内部完整字段表达式 |
| 页面局部 DataFlow/model | `--resolve-model-page 'page:app/某页.spg' --resolve-model model5` | 局部 ID 到真实模型映射 | 未解析前不可直接解释 model5 |

单文件 `.tbl` 阅读顺序：
1. `summary.table_type`、`field_count`、`input_count`、`output_count`、`what_is_it`。
2. `details.field_lineage`：`target_field`、`source_fields`、`source_expr`、`transform`、`confidence`。
3. `details.dataflow_inputs` / `details.dataflow_outputs`。
4. `evidence.source_file` / `evidence.json_path`。
5. `diagnostics`，例如 `DATAFLOW_NO_OUTPUT`。

`.tbl kind=Table` 不能按 SuperPage 语义解释；没有 `field_lineage` 时不能编造字段来源。

## Page Logic 输出

当用户问“这个页面主要做什么、用户能触发哪些逻辑、会影响哪些数据”时，优先使用 `--query-page-logic`，不是 `--context` 或单点 `--explain`。

关键字段：
- `summary.what_is_it`：页面一句话摘要。
- `summary.page_role`：`form_submit_page` / `readonly_dashboard` / `navigation_page` / `data_maintenance_page` / `mixed_interaction_page` / `unknown`。
- `details.entrypoints`：用户可触发入口，不含普通 input。
- `details.action_flows`：动作链，含 `action_id`、`action_type`、`action_category`、`semantic_summary`、`component_id`、`trigger_type`、`blocks_on`、`condition`、`reads`、`writes`、`navigation`、`sets_params`、`passes_params`。
- `details.data_sources`：页面读取的模型和字段。
- `details.write_targets`：页面写入的模型和字段。
- `details.navigation`：跳转、嵌入、参数传递关系。
- `details.visibility_rules`：visible/hidden/disabled/readonly 规则。
- `details.risk_diagnostics`：`NO_WRITE_TARGETS`、`NO_ENTRYPOINTS`、`ACTION_FLOW_INCOMPLETE`、`EVIDENCE_SAMPLED`、`UNRESOLVED_PAGE_NAVIGATION`、`UNRESOLVED_MODEL_WRITE`、`VISIBILITY_RULE_UNRESOLVED`。

## 来源分类使用指南

当用户问“这个值从哪里来”“这个字段是用户输入的还是自动带出的”，读取 `details.value_trace[]` 或 `details.lineage[]` 中的 `source_type`。

| source_type | 含义 | 回答建议 |
|---|---|---|
| `Param` | 页面参数 | “来自页面参数” |
| `UserInput` | 用户可交互输入 | “用户输入” |
| `ModelAuto` | 数据模型自动绑定 | “从数据模型自动获取” |
| `System` | 系统变量 | “系统变量” |
| `Computed` | 表达式计算 | “由表达式计算产生” |
| `Constant` | 固定常量 | “固定常量” |
| `Unknown` | 无法分类 | “来源无法确定，保守回答” |

使用顺序：
1. 先看 `summary` 是否有 `source_type` 或 `value_trace_count`。
2. 需要追溯时看 `details.value_trace[]`。
3. 字段级 lineage 用 `details.lineage[]`：`target_field` -> `source_fields` -> `via_node` -> `transform`。
4. 用 evidence 验证 `claim`、`source_file`、`json_path`。

## DataFlow Projection

当目标字段或页面 model 指向 DataFlow 加工表时，`explain_condition` 会在 `details.answer_facts.<fact_block>` 下输出 DataFlow 内部投影短事实。

value-source intent：
- 读取 `details.answer_facts.value_source_facts.dataflow_table`、`dataflow_output_field`、`physical_source_fields[]`、`via`、`original_node`、`original_field`、`candidate_inputs[]`。
- `physical_source_fields[]` 非空时才可把 `proven_physical_input` 当作字段级物理来源证明。
- 只有 `candidate_inputs[]` 或 `table_source_path` 时，只能说候选来源。

availability intent：
- 页面局部 model 优先使用 page-scoped target，例如 `model:app/售后.app/绑定车辆/会员已注册.spg|model11`。
- 读取 `details.answer_facts.availability_facts.dataflow_availability`、`dataflow_table`、`physical_inputs[]`、`source_filters[]`、`output_filters[]`、`join_rules[]`、`union_rules[]`、`referenced_vars[]`。
- DataFlow filter 描述 model 数据可用性，不是组件自身 direct/inherited visibleCondition。

action intent：
- 用户问句里不会出现内部 action id，所以先在组件上查 `--intent action`，
  再用 `details.answer_facts.action_facts.actions[].action_target` 作为下一条命令的 target。
- `actions[]` 是目标自己挂的动作（图上的 Triggers 边）；`related_actions[]` 是别的组件的动作，
  只是它们的门禁条件引用了本目标——回答「为什么点不动」要用它，但不能说成本目标的动作。
- `gate_conditions[].raw_expr` 是动作门禁原文；组件自身的 visibleCondition/disableCondition 不是动作门禁。

compact 约束：
- compact 模式不展开完整 DataFlow 节点树；优先读取 `answer_facts` 中的短事实。
- 如果 compact availability 混入其他页面同名局部 model gates，应改用 page-scoped target 或视为输出回退。

## Legacy Compatibility Map

这些字段保留兼容，但不作为新回答的主语义来源。

| legacy 字段/旧语义 | 当前优先字段 |
|---|---|
| `triggered_by` 中的 Contains | `details.located_in` |
| `affects` 总包字段 | `details.triggers` / `details.writes_models` / `details.affects_components` |
| `upstream_dependencies` | `produced_by` / `consumed_by_dataflows` / `dataflow_inputs` |
| `downstream_outputs` | `dataflow_outputs` / `produced_by` |
| 旧 `importance=data_source/write_target/navigation` | `entrypoint` / `data_source_display` / `calculated_display` / `static_display` / `container` / `form_input` / `action_target` / `unknown` |

## 常用示例

```bash
# Parse single file, compact JSON output
metadata-checker page.spg

# Parse with full details
metadata-checker page.spg --detail

# Query specific component
metadata-checker page.spg --query input3

# Explain a component
metadata-checker page.spg --explain input1

# Build graph for project
metadata-checker --project-dir /path/to/project --build-graph --graph-db-path /tmp/project.graphdb

# Query model
metadata-checker --project-dir /path/to/project --query-model model1 --budget compact

# Get context around a button
metadata-checker --project-dir /path/to/project --context 'comp:app/page.spg|button1' --depth 2 --budget normal

# Page logic summary
metadata-checker --project-dir /path/to/project --query-page-logic 'page:app/合同管理/销售合同.spg' --budget compact
```

## Negative Constraints Checklist

- 禁止默认读取 raw JSON 大对象。
- 禁止跳过 `summary` 直接读取 `details` 或 `evidence`。
- 禁止默认使用 `--budget full`；第一轮默认 `--budget compact`，按需升级。
- 禁止用 compact sample 回答“全部 readers/writers/关系”。
- 禁止忽略 diagnostics 做空洞确定性结论。
- 禁止在 evidence 不足时编造来源。
- 禁止把 `<graph-edge-derived>` 当作可核验强证据。
- 禁止忽略 `node_id="?"` 或 `raw_expr="n/a"` 做确定性结论。
- 禁止在 `EVIDENCE_LOCATION_MISSING` 存在时假装证据完整。
- 禁止 value-source 回答 display 问题。
- 禁止把 `related_context` / `supporting_context` 当作必要条件。
- 禁止把 `candidate_inputs` 当作 `proven_physical_input`。
- 禁止把 `read_by_count=0` 当作“模型没有被使用”；必须同时检查 `consumed_by_dataflow_count` 和 `dataflow_role`。
- 禁止把单文件 `.tbl` 输出当作项目级 DataFlow 的全局拓扑。
- 禁止把 `.tbl` 当作 SuperPage 解析。
- 禁止在没有 `value_trace`、`lineage` 或 `field_lineage` 时凭空推断来源。
- 禁止将 `Computed` 误判为 `UserInput`。
- 禁止在目标 ID 不确定时猜测；必须先 `--find-*` 或 `--resolve-model`。
- 禁止把 CLI 单引号带进 stdio JSON 或 function calling payload。
