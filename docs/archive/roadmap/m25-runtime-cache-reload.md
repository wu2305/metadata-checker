# M25：Runtime Cache 与 Reload ✅

| milestone | M25 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M25：Runtime Cache 与 Reload ✅

### 目标

让长驻 runtime 能识别 graphdb 文件变化，并在不崩溃的前提下刷新内存图，避免 AI/function calling 使用过期图。

M25 只解决 runtime 生命周期可靠性：状态查询、变更检测、手动 reload、失败降级。

### 非目标

- 不做自动后台文件监听。
- 不做多项目 graph runtime 池。
- 不做 MCP server。
- 不更新 skill 默认策略。
- 不做 Lazy GraphDB。

### 工作清单

- 扩展 `GraphRuntime`：
  - `reload_count: usize`
  - `last_reload_error: Option<String>`
  - `graph_fingerprint: GraphFingerprint`
- 新增 `GraphFingerprint`：
  - `path: PathBuf`
  - `mtime: Option<SystemTime>`
  - `size: u64`
- 实现 `GraphRuntime::current_fingerprint() -> Result<GraphFingerprint>`。
- 实现 `GraphRuntime::is_graph_changed() -> Result<bool>`。
- 实现 `GraphRuntime::reload_if_changed() -> Result<bool>`。
- 实现 `GraphRuntime::reload() -> Result<()>`。
- reload 策略：
  - 新图加载成功后再替换旧 graph。
  - 新图加载失败时保留旧 graph。
  - 记录 `last_reload_error`。
  - 返回 diagnostic，不能静默失败。
- stdio server 增加命令：
  - `status`
  - `reload`
- stdio server 每次普通查询前可选调用 `reload_if_changed()`；如果担心开销，M25 可先只在请求显式带 `check_reload: true` 时执行，但协议必须明确。
- `status` 输出：
  - graphdb path
  - loaded_at
  - load_count
  - reload_count
  - node_count
  - edge_count
  - graph_file_mtime
  - graph_file_size
  - last_reload_error
- 增加测试：
  - graphdb 未变化时 `reload_if_changed()` 返回 false。
  - graphdb 替换后 `reload_if_changed()` 返回 true。
  - reload 失败时旧 runtime 仍可查询。
  - stdio `status` 返回当前 runtime 状态。
  - stdio `reload` 成功后 `reload_count` 增加。

### 落地任务拆解

#### M25.1：Fingerprint 与状态输出

- 在 `src/runtime.rs` 中新增 `GraphFingerprint`。
- 为 `GraphRuntime` 增加 `status()` 方法，返回 JSON 或强类型 `RuntimeStatus`。
- `status` 不触发 reload。

验收：

- 单元测试能读取 node/edge 数和 graphdb 文件状态。

#### M25.2：安全 reload

- reload 不允许先清空当前 graph。
- 使用局部变量加载新 `GraphRuntime` 或新 `GraphDB`。
- 加载成功后再替换 `self.graph` 和 fingerprint。
- 加载失败时保留旧 graph，返回错误并记录。

验收：

- 损坏 graphdb reload 失败后，旧查询仍可返回结果。

#### M25.3：stdio 命令扩展

- `status` 不要求 target。
- `reload` 不要求 target。
- 普通查询响应 diagnostics 中可以包含 `"GRAPH_RELOADED"` 或 `"GRAPH_RELOAD_FAILED"`。
- 错误响应仍保持单行 JSON。

验收：

- 连续请求：`status` -> `explain_condition` -> `reload` -> `status` 都可解析。

### 验收目标

- 长驻进程不会静默使用过期 graphdb。
- reload 失败不会导致服务崩溃或丢失旧图。
- `status` 能让 AI 判断当前 runtime 是否加载了预期 graphdb。
- M25 完成后，才能进入 M26 的 skill/function calling 接入。
