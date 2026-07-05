# M24：Stdio Function Calling Server ✅

| milestone | M24 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M24：Stdio Function Calling Server ✅

### 目标

把 M23 的 `GraphRuntime` 暴露为长驻 JSONL stdio 服务，让 AI/function calling 包装层可以在一个进程内连续执行多个项目级查询。

M24 只解决协议和进程生命周期：启动一次、加载一次 graphdb、stdin 每行一个请求、stdout 每行一个响应。

### 非目标

- 不做 MCP server。
- 不做后台 daemon。
- 不做 graphdb 自动 reload。
- 不更新 `metadata-checker` skill 默认调用策略。
- 不新增 Lazy GraphDB。
- 不改变单次 CLI 查询输出。

### 工作清单

- 在 `src/cli.rs` 新增参数：
  - `--serve-stdio`
  - `--serve-graph-db-path <PATH>` 或复用现有 `--graph-db-path`
- 在 `src/main.rs` 中增加 `--serve-stdio` 分支。
- 新增服务模块，建议命名：
  - `src/stdio_server.rs`
- 定义 JSONL 请求结构：
  - `request_id: String`
  - `command: String`
  - `target: String`
  - `budget: Option<String>`
  - `human: Option<bool>`
- 定义 JSONL 响应结构：
  - `request_id: String`
  - `ok: bool`
  - `result: Option<serde_json::Value>`
  - `error: Option<String>`
  - `diagnostics: Vec<String>`
  - `timing: RuntimeTiming`
- 服务启动时：
  - 解析 graphdb path。
  - 调用 `GraphRuntime::load`。
  - stderr 输出启动日志。
  - stdout 不输出启动日志，避免污染 JSONL 协议。
- 服务循环：
  - 从 stdin 逐行读取。
  - 空行跳过。
  - 非法 JSON 返回 `ok=false`，不能 panic。
  - 未知 command 返回 `ok=false` 和明确错误。
  - 支持 `explain_condition` 命令，映射到 `RuntimeQueryCommand::ExplainCondition`。
  - 每个响应必须单行 JSON。
  - stdout 每次响应后 flush。
- 错误处理：
  - 可恢复错误转为 JSONL error。
  - 不在库代码中 `println!` / `eprintln!`。
  - server 层可以用 stderr 输出运行日志。
- 增加测试：
  - 启动 `metadata-checker --serve-stdio --graph-db-path <fixture_db>` 子进程。
  - 连续写入两个 `explain_condition` 请求。
  - 读取两行响应。
  - 断言两个响应 `ok=true`。
  - 断言第二个响应 `timing.graph_load_ms == 0`。
  - 断言两次响应不包含非 JSON 日志。
- 增加负例测试：
  - 非法 JSON 行。
  - 未知 command。
  - 缺 target。

### 落地任务拆解

#### M24.1：CLI 参数和入口

- 修改 `src/cli.rs`，增加 `serve_stdio: bool`。
- 确认 help 文本说明这是机器协议，不是人类 REPL。
- 修改 `src/main.rs`：
  - 在项目级查询分支前处理 `--serve-stdio`。
  - 缺少 graphdb path 时返回结构化错误或 `anyhow::bail!`。
  - 不要求 `<FILE>` input。

验收：

- `metadata-checker --help` 能看到 `--serve-stdio`。
- 不影响现有 `--project-dir --query-*` 分支。

#### M24.2：JSONL 协议实现

- 新增 `StdioRequest` / `StdioResponse` 类型。
- 类型字段使用 snake_case。
- command 先用字符串解析，不急于暴露复杂 enum 到外部协议。
- 内部再转换到 `RuntimeQueryRequest`。
- 所有响应必须包含原始 `request_id`；若请求 JSON 都无法解析，则使用空字符串或生成 `"unknown"`。

验收：

- 一行输入只产生一行输出。
- stdout 只包含 JSONL。
- stderr 才允许包含启动和错误日志。

#### M24.3：子进程集成测试

- 新增 `tests/stdio_server_tests.rs`。
- 使用 `std::process::Command` 启动当前测试 binary 对应的 `metadata-checker` 可执行文件；如果现有测试已有 helper，复用 helper。
- 子进程 stdin/stdout 使用 pipe。
- 测试结束必须 kill/wait 子进程，避免遗留进程。
- 测试 fixture graphdb 在临时目录中构建，避免污染仓库。

验收：

- `cargo test --test stdio_server_tests` 通过。
- 测试失败时不会挂住。

#### M24.4：协议文档

- 新增或更新 docs，建议：
  - `docs/stdio-server.md`
- 文档包含：
  - 启动命令。
  - 请求示例。
  - 响应示例。
  - 错误响应示例。
  - stdout/stderr 边界。

验收：

- 文档示例可以直接复制执行。

### 验收目标

- `cargo check` 无 warning。
- `cargo test --test stdio_server_tests` 通过。
- stdio server 连续两个请求只加载一次 graphdb。
- stdout 协议对 function calling 包装层稳定可解析。
- M24 完成后，才能进入 M25 的 reload/cache。
