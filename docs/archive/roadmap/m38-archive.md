# M38：运行期工具契约统一

| milestone | M38 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M38：运行期工具契约统一

目标：消除 CLI / stdio / 未来 MCP 在“可调用工具、参数校验、错误码、输出 envelope”上的分叉。M38 只统一已经加载 graphdb 后的运行期工具契约，不把项目扫描、建图初始化或 graphdb 文件格式改造并入本里程碑。

已确认决策：

- `build_graph` 不进入统一工具面。
  - 原因：它是项目索引初始化能力，会扫描项目、解析元数据并写 graphdb，不属于已加载 runtime 的查询工具。
  - 后续如果需要模型触发重新索引，应放入独立 `ProjectIndexer` / session 生命周期里程碑，而不是 M38。
- `reload_graph` / `check_reload` / `status` 进入统一工具面。
  - 这三者只管理“当前 runtime 是否重新读取已有 graphdb 文件”，不扫描项目、不重建索引、不写 graphdb。
  - stdio / MCP 这类长驻进程需要它们；CLI 是短进程，可返回 no-op 诊断。
- Rust 中不做继承式 adapter。
  - 采用 trait + 组合：各调用方式各自处理输入/输出，核心工具契约和执行器共享。
- M38 不改变 `AiOutput` 语义 schema。
  - 可以统一外层 envelope、错误码、tool spec 和 adapter 入口。
  - 不借 M38 重写 explain / query 的业务输出内容。

范围约束：

- 不实现 MCP server。
- 不实现远程项目拉取、登录态、session 目录。
- 不实现 redb / IndexedDB / memory graph store 抽象迁移。
- 不改变 graphdb 文件格式。
- 不改变扫描器、增量建图、Dataflow 展开语义。
- 不把 `build_graph`、`refresh_index`、`rebuild_graph` 伪装成 query tool。
- 不删除 CLI 原有子命令；先让它们复用统一工具契约，必要时保留薄包装。

统一工具清单：

- 查询/解释工具：
  - `explain_condition`
  - `explain`
  - `context`
  - `query_model`
  - `query_page`
  - `query_cross`
  - `query_dataflow`
  - `query_page_logic`
  - `find_page`
  - `find_model`
  - `find_component`
  - `advise_query`
- 运行期生命周期工具：
  - `status`
  - `reload_graph`
  - `check_reload`
- 明确排除：
  - `build_graph`
  - `refresh_index`
  - `rebuild_graph`

建议核心结构：

```rust
/// 运行期工具枚举，表示所有已加载 graphdb 后可以调用的能力。
pub enum ToolCommand {
    ExplainCondition,
    Explain,
    Context,
    QueryModel,
    QueryPage,
    QueryCross,
    QueryDataflow,
    QueryPageLogic,
    FindPage,
    FindModel,
    FindComponent,
    AdviseQuery,
    Status,
    ReloadGraph,
    CheckReload,
}

/// 工具声明，供 CLI / stdio / MCP 共享 help、校验和 tool list。
pub struct ToolSpec {
    pub command: ToolCommand,
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub requires_target: bool,
    pub requires_graph: bool,
    pub mutates_runtime: bool,
    pub supports_human: bool,
    pub supported_budgets: &'static [&'static str],
    pub supported_intents: &'static [&'static str],
    pub output_kind: ToolOutputKind,
}

/// adapter 解析后的标准调用对象。
pub struct ToolInvocation {
    pub command: ToolCommand,
    pub target: Option<String>,
    pub budget: Option<String>,
    pub intent: Option<String>,
    pub depth: Option<usize>,
    pub page_scope: Option<String>,
    pub human: bool,
    pub check_reload: bool,
}
```

实施任务清单：

- [x] M38.1：新增 `ToolCommand` / `ToolSpec` / `ToolRegistry`
  - 建议位置：
    - `src/tool_contract.rs` 或 `src/runtime/tool_contract.rs`。
  - 要求：
    - 工具名、别名、是否需要 target、是否需要 graph、是否会修改 runtime 状态、支持的 budget / intent 在一个 registry 中声明。
    - CLI、stdio、未来 MCP 的工具列表必须从同一 registry 获取。
    - `reload_graph` 与 `check_reload` 的 `mutates_runtime=true`；`status` 的 `requires_graph=false`。
    - `build_graph` 不在 registry 中出现。
  - 测试：
    - registry 包含统一工具清单中的所有工具。
    - registry 不包含 `build_graph` / `refresh_index` / `rebuild_graph`。
    - 工具名与别名查找稳定，未知工具返回统一错误码。

- [x] M38.2：定义标准 `ToolInvocation`
  - 字段覆盖：
    - `command`
    - `target`
    - `budget`
    - `intent`
    - `depth`
    - `page_scope`
    - `human`
    - `check_reload`
  - 要求：
    - 参数校验基于 `ToolSpec`，不要在 stdio / CLI 中各写一套条件。
    - target 缺失、budget 非法、intent 非法、depth 非法必须走统一错误码。
    - `status`、`reload_graph`、`check_reload` 不要求 target。
  - 测试：
    - `explain_condition` 缺 target 返回 `MISSING_TARGET`。
    - `status` 无 target 合法。
    - 非法 budget 返回 `INVALID_BUDGET`。
    - 非法 intent 返回 `INVALID_INTENT`。
    - 非法 depth 返回 `INVALID_DEPTH`。

- [x] M38.3：扩展 `GraphRuntime::query` 为统一工具执行入口
  - 当前状态：
    - `RuntimeQueryCommand` 只覆盖 `ExplainCondition` / `AdviseQuery`，stdio 仍直接分发 `query_model`、`context`、`explain`、`query_page_logic` 等。
  - 要求：
    - `GraphRuntime` 接收标准 `ToolInvocation` 或等价 request。
    - 查询类工具在 runtime 内部统一分发到现有模块实现。
    - `reload_graph` 强制重新读取当前 graphdb 文件。
    - `check_reload` 仅在 graphdb 文件变化时重载。
    - `status` 返回当前 graphdb 路径、是否已加载、节点/边数量、最近一次加载时间或可用诊断。
  - CLI 特殊语义：
    - CLI 默认每次启动重新加载 graphdb。
    - CLI 调用 `reload_graph` 可以返回成功 no-op，诊断码为 `RUNTIME_RELOAD_NOOP`，并说明 `stateless_cli_already_loads_latest_graph=true`。
  - stdio / MCP 语义：
    - 长驻进程必须真实支持 `reload_graph` / `check_reload`。
    - 不得把 reload 实现成重新扫描项目或写 graphdb。

- [x] M38.4：统一错误码
  - 建议新增：
    - `UNKNOWN_COMMAND`
    - `MISSING_TARGET`
    - `INVALID_TARGET`
    - `INVALID_BUDGET`
    - `INVALID_INTENT`
    - `INVALID_DEPTH`
    - `INVALID_ARGUMENT`
    - `TARGET_NOT_FOUND`
    - `GRAPH_DB_NOT_FOUND`
    - `GRAPH_RELOAD_FAILED`
    - `HUMAN_MODE_NOT_SUPPORTED`
    - `QUERY_FAILED`
    - `INTERNAL_ERROR`
  - 建议诊断码：
    - `RUNTIME_RELOAD_NOOP`
    - `GRAPH_UNCHANGED`
    - `GRAPH_RELOADED`
  - 要求：
    - 错误码由核心 contract 层定义。
    - CLI human 文本、CLI JSON、stdio JSON 使用同一 code。
    - no-op / unchanged 不应伪装成错误；放入 diagnostics。
  - 测试：
    - stdio 未知 command 返回 `UNKNOWN_COMMAND`。
    - CLI / stdio 同一非法参数返回同一 code。
    - `check_reload` 未变化返回成功并带 `GRAPH_UNCHANGED` 诊断。

- [x] M38.5：实现调用方式 adapter
  - 建议 trait：

```rust
pub trait InvocationAdapter {
    type RawInput;
    type RawOutput;

    fn parse_input(&self, raw: Self::RawInput) -> Result<ToolInvocation, ToolError>;
    fn render_output(&self, response: ToolResponse) -> Result<Self::RawOutput>;
}
```

  - `CliAdapter`：
    - 负责把 clap 子命令转换为 `ToolInvocation`。
    - 负责 human / JSON 输出选择。
    - 不负责工具语义分发。
  - `StdioAdapter`：
    - 负责把 JSON line envelope 转换为 `ToolInvocation`。
    - 负责输出 stdio envelope。
    - 不再维护独立 command enum 的业务分发大 match。
  - `McpAdapter`：
    - M38 只预留概念或 trait 边界，不实现 MCP server。
  - 测试：
    - CLI adapter 与 stdio adapter 对同一逻辑输入产出同一 `ToolInvocation`。
    - adapter 只处理输入/输出包装，不直接调用 `query_model` 等业务函数。

- [x] M38.6：统一输出 envelope 与 `ResponseProcessor` 使用方式
  - 要求：
    - 核心执行返回 `ToolResponse` 或复用 `RuntimeQueryResponse` 的等价结构。
    - adapter 决定外层 envelope：
      - CLI human：人类可读文本。
      - CLI JSON：机器可读 JSON。
      - stdio：逐行 JSON envelope。
      - 未来 MCP：MCP tool result。
    - `ResponseProcessor` 继续作为渲染 facade 前置骨架，不在 M38 大规模重写业务 JSON。
  - 验证重点：
    - `timing.output_size_bytes` 的含义在 CLI / stdio 中保持当前约定或文档同步说明。
    - `human=true` 仍在不支持的调用方式返回 `HUMAN_MODE_NOT_SUPPORTED`。

- [x] M38.7：CLI / stdio parity 回归
  - 覆盖命令：
    - `explain_condition`
    - `explain`
    - `context`
    - `query_model`
    - `query_page_logic`
    - `advise_query`
    - `status`
    - `reload_graph`
    - `check_reload`
  - 要求：
    - 同一命令的关键 contract 字段一致。
    - 同一错误输入的错误码一致。
    - stdio 工具列表与 registry 一致。
    - CLI help 或 docs 中的工具列表与 registry 一致。
  - 真实项目验证：
    - 用 xiaoshouyi graphdb 验证至少一个 `explain_condition` 与一个 `query_page_logic`。
    - 验证 `check_reload` 不会触发项目扫描。

- [x] M38.8：文档与 skill 同步
  - 更新：
    - `docs/schema.md`
    - `README.md` 或 CLI 使用文档
    - `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`
  - 要求：
    - 明确“运行期工具”和“初始化建图”区别。
    - 明确模型可调用 `reload_graph` / `check_reload` 刷新已存在 graphdb。
    - 明确模型不能用 M38 工具触发 `build_graph`。
    - 工具列表以 registry 为准，避免 skill 继续维护过期命令清单。

验收标准：

- `cargo fmt --check`、`cargo check`、`cargo test` 全部通过。
- `cargo test --test stdio_server_tests` 全部通过。
- 新增 registry / adapter / error-code 单元测试全部通过。
- stdio 支持的命令来自同一 registry，不再由独立业务分发大 match 决定工具全集。
- CLI 与 stdio 对同一 tool 的关键 contract 字段一致。
- `status` / `reload_graph` / `check_reload` 在 CLI 与 stdio 中都有明确行为。
- `build_graph` 不出现在 runtime tool list、stdio tool list 或未来 MCP tool list 中。
- 文档和 skill 中的工具清单与代码 registry 一致。
- 真实项目验证证明 `check_reload` / `reload_graph` 只重载 graphdb，不扫描项目、不写 graphdb。
