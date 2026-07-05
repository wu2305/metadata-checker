# M27：Stdio 查询命令面扩展

| milestone | M27 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M27：Stdio 查询命令面扩展

### 目标

把当前 AI 高频项目级查询接入 stdio server，让 function calling 在同一进程内复用 `GraphRuntime`，不再因为 `context` / `query-model` / `query-page-logic` 退回 CLI 而重复冷启动。

M27 只扩展 stdio command surface，不改变各查询本身的业务语义和输出结构。

### 非目标

- 不重写 pathfinder。
- 不改变 `--explain-condition` 主链路选择规则。
- 不引入 Lazy GraphDB。
- 不把 stdio server 升级成 MCP server。
- 不为了 stdio 扩展重写 CLI 输出 schema。

### 工作清单

- 新增 stdio command：`context`
  - 对齐 CLI：`--context <ID> --depth <N> --budget <compact|normal|full>`。
  - request 字段：`target`、`depth`、`budget`、`human`、`check_reload`。
  - 默认 `depth=1`，默认 `budget=normal`，AI 使用建议优先 `compact` / `normal`。
- 新增 stdio command：`query_model`
  - 对齐 CLI：`--query-model <MODEL>`。
  - request 字段：`target`、`budget`、`human`、`check_reload`。
  - 真实项目验收目标必须包含 `model:fact_qwSidebar`。
- 新增 stdio command：`query_page_logic`
  - 对齐 CLI：`--query-page-logic <PAGE>`。
  - request 字段：`target`、`budget`、`human`、`check_reload`。
  - compact 输出必须保留 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- action1/action4` 主链路。
- 新增 stdio command：`explain`
  - 对齐 CLI：`--explain <ID>`。
  - 用于普通对象解释，不替代 `explain_condition`。
- 抽出统一 stdio command registry。
  - 建议放在 `src/stdio.rs` 或 `src/runtime/stdio.rs`。
  - 每个 command 必须声明 required fields、默认参数、输出 kind、错误码。
  - 避免在 `main.rs` 中继续堆叠大 match 分支。

### 落地任务拆解

#### M27.1：Stdio command registry

- 抽出 `StdioCommand` / `StdioRequest` / `StdioResponse` 处理入口。
- 保留既有 `explain_condition` / `status` / `reload` 行为不变。
- unknown command 返回单行 JSON 错误，不 panic。
- missing required field 返回结构化错误。

验收：

- 既有 `stdio_server_tests` 全部通过。
- unknown command / missing target 负例测试通过。

#### M27.2：接入 `context`

- 在 runtime 层复用当前 graph 调用 context 查询。
- 支持 `depth`、`budget`、`human`。
- `check_reload=true` 时沿用 M25 reload 检测逻辑。

验收：

- stdio `context` 输出结构与 CLI JSON 结构一致。
- 第二次 stdio `context` 查询 `timing.graph_load_ms == 0`。
- malformed depth / budget 有稳定错误码。

#### M27.3：接入 `query_model`

- 在 runtime 层复用当前 graph 调用 model 查询。
- 支持 physical table target 和普通 model target。
- 不降低 M19/M22 主链路与 field alias 回归质量。

验收：

- 真实项目 `model:fact_qwSidebar` 能看到 `潜客信息跟进.spg|button1|action1` / `action4` 写入。
- 第二次 stdio `query_model` 查询 `timing.graph_load_ms == 0`。

#### M27.4：接入 `query_page_logic`

- 在 runtime 层复用当前 graph 调用页面逻辑查询。
- 支持 compact/normal/full budget。
- 输出过大时仍遵守已有截断和 diagnostics 规则。

验收：

- 真实项目 `page:app/销售.app/销售/合同协议.spg` compact 输出包含 input3 主链路。
- 第二次 stdio `query_page_logic` 查询 `timing.graph_load_ms == 0`。

#### M27.5：接入 `explain`

- 在 runtime 层复用当前 graph 调用 explain 查询。
- 保持普通 explain 与 explain_condition 的用途区分。

验收：

- `explain comp:...|input3` 与 CLI JSON 输出结构一致。
- `explain_condition` 既有测试不回退。

### 验收目标

- stdio 支持 `explain_condition`、`explain`、`context`、`query_model`、`query_page_logic`、`status`、`reload`。
- 所有新增 command 的第二次查询 `graph_load_ms == 0`。
- 新增 command 错误响应保持单行 JSON。
- 真实项目 `input3` 与 `fact_qwSidebar` 主链路不回退。
- skill 文档不再要求 AI 为这些查询退回 CLI，除非 graphdb 不可用。
