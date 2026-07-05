# M23：Hot Graph Runtime 基座 ✅

| milestone | M23 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M23：Hot Graph Runtime 基座 ✅

### 目标

让项目级查询能够在同一进程内复用已加载的 `GraphDB`，为后续 function calling / stdio server / MCP 长驻工具形态打基础。

M23 只解决一件事：把“加载图”和“执行查询”从 CLI 分支中解耦，证明同一个 runtime 连续执行多次查询时，第二次不再全量加载 graphdb。

### 非目标

- 不实现 Lazy GraphDB。
- 不新增 redb 邻接索引。
- 不实现 MCP server。
- 不改 `metadata-checker` skill 调用协议。
- 不做 pathfinder 深度性能优化。
- 不改变现有 CLI 输出协议。

### 背景问题

- AI 通过 skill 调用 CLI 时，每次都是新进程冷启动。
- 当前项目级查询会在 `src/main.rs` 中先打开 graphdb，再通过 `GraphDB::load_from_db` 全量反序列化 nodes/edges 并构建 `petgraph`。
- `samply` 对真实项目 `xiaoshouyi` 的 `input3 --explain-condition` 采样显示，冷查询的主要成本集中在 graphdb 全量读取、redb range 迭代、`serde_json::from_slice`、`serde_json::Value::deserialize` 和内存图构建。
- 如果目标转向 function calling / 长驻进程，全量加载并不是错误方向；问题是不能每次 function call 都重复加载。

### 设计原则

- 保留现有 `GraphDB` 全量加载模型，作为热查询和复杂关系遍历的主路径。
- 先建立运行时复用边界，再考虑服务协议。
- 查询逻辑应能被 CLI 和 runtime 共同调用，避免继续把业务查询逻辑写死在 `main.rs` 分支里。
- 阶段耗时必须可观测，否则后续无法判断优化是否真正命中冷启动问题。

### 工作清单

- 新增运行时模块，例如 `src/runtime.rs`。
- 新增 `GraphRuntime` 结构，至少包含：
  - `graph: GraphDB`
  - `graph_db_path: PathBuf`
  - `loaded_at: SystemTime`
  - `graph_file_mtime: Option<SystemTime>`
  - `graph_file_size: u64`
  - `load_count: usize`
- 实现 `GraphRuntime::load(graph_db_path: impl AsRef<Path>) -> Result<Self>`。
- 实现 `GraphRuntime::query(request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse>`。
- 定义 `RuntimeQueryRequest`，至少支持：
  - `command: RuntimeQueryCommand`
  - `target: String`
  - `budget: String`
  - `human: bool`
- 定义 `RuntimeQueryCommand`，M23 最小范围只要求：
  - `ExplainCondition`
- 定义 `RuntimeQueryResponse`，至少包含：
  - `result: serde_json::Value`
  - `timing: RuntimeTiming`
  - `diagnostics: Vec<String>`
- 定义 `RuntimeTiming`，至少包含：
  - `graph_load_ms`
  - `query_compute_ms`
  - `serialize_ms`
  - `total_ms`
- 将 `explain_condition_target` 的核心逻辑拆出为“返回 `serde_json::Value`”的函数。
  - CLI 仍负责 `println!("{}", serde_json::to_string_pretty(...))`。
  - runtime 直接复用该函数，避免通过 stdout 解析结果。
- 保留当前 `explain_condition_target` 作为 CLI 包装函数，避免破坏现有调用。
- 增加 runtime 单元测试或集成测试：
  - 加载 fixture graphdb。
  - 连续执行两次同一 `ExplainCondition` 查询。
  - 断言 `load_count == 1`。
  - 断言两次查询结果的关键字段一致。
- 增加真实项目 ignored 测试或手动验收命令：
  - 目标：`comp:app/销售.app/销售/合同协议.spg|input3`
  - 要求结果仍包含 `model22.phoneNumber`、`fact_qwSidebar.phoneNumber`、`action1`、`action4`。
- 增加阶段耗时日志或测试可读输出，用于区分：
  - 第一次 runtime 查询：包含 graph load。
  - 第二次 runtime 查询：`graph_load_ms == 0` 或接近 0。
- 更新 `main.rs` 内部调用路径时保持 CLI 行为兼容：
  - 单次 CLI 项目级查询仍可继续冷启动。
  - M23 不要求 CLI 自动进入长驻模式。

### 落地任务拆解

#### M23.1：确认当前查询入口和可复用边界

- 阅读 `src/main.rs` 中项目级查询分支，确认 `--explain-condition` 当前调用链。
- 阅读 `src/explain.rs` 中 `explain_condition_target`，标记所有直接 `println!`、`serde_json::to_string_pretty`、`human` 输出分支。
- 阅读 `src/graph.rs` 中 `GraphDB::open_or_diagnostic`、`GraphDB::open_readonly`、`GraphDB::load_from_db`，确认全量加载发生点。
- 记录 M23 不改动的范围：
  - 不改 `GraphDB` 序列化格式。
  - 不改 redb 表结构。
  - 不改 `PathFinder` 算法。
  - 不改 CLI JSON 输出 shape。
- 输出一份简短实现备注，说明 M23 的最小代码改动路径。

验收：

- 能指出 `GraphRuntime` 应复用的最小查询函数。
- 能指出哪些 CLI 输出逻辑必须留在 wrapper 层。

#### M23.2：拆出 ExplainCondition 纯查询函数

- 在 `src/explain.rs` 中新增返回结构化结果的函数，建议命名：
  - `build_explain_condition_output(graph: &GraphDB, target_id: &str, budget: &str) -> Result<serde_json::Value>`
- 将 `explain_condition_target` 中构建 JSON 的主体逻辑迁移到新函数。
- `build_explain_condition_output` 不允许直接 `println!` / `eprintln!`。
- `build_explain_condition_output` 不处理 human 输出。
- 原 `explain_condition_target` 保留为 CLI wrapper：
  - 调用 `build_explain_condition_output`。
  - `human == false` 时继续 pretty JSON 输出。
  - `human == true` 时保持现有行为或显式沿用当前 JSON 输出，不能破坏编译。
- target 不存在时仍返回当前 `TARGET_NOT_FOUND` 结构，不能退化为 error。
- 保持 `details.primary_path`、`details.related_context`、`details.data_empty_gates` 等字段不变。

验收：

- 现有 `--explain-condition` CLI 输出与重构前等价。
- `cargo test --test regression_tests test_real_project_query_page_logic_input3_chain -- --ignored --nocapture` 仍通过，若该测试名调整则运行现有 input3 真实项目回归。
- 不新增 `#[allow(dead_code)]`。

#### M23.3：新增 Runtime 类型与请求响应协议

- 新增 `src/runtime.rs`。
- 在 `src/lib.rs` 或模块入口注册 `pub mod runtime;`，若当前项目没有 `lib.rs`，按现有模块组织在 `main.rs` 中声明 `mod runtime;`。
- 定义 `GraphRuntime`：
  - `graph: GraphDB`
  - `graph_db_path: PathBuf`
  - `loaded_at: SystemTime`
  - `graph_file_mtime: Option<SystemTime>`
  - `graph_file_size: u64`
  - `load_count: usize`
- 定义 `RuntimeQueryCommand`：
  - `ExplainCondition`
- 定义 `RuntimeQueryRequest`：
  - `command: RuntimeQueryCommand`
  - `target: String`
  - `budget: String`
  - `human: bool`
- 定义 `RuntimeQueryResponse`：
  - `result: serde_json::Value`
  - `timing: RuntimeTiming`
  - `diagnostics: Vec<String>`
- 定义 `RuntimeTiming`：
  - `graph_load_ms: u128`
  - `query_compute_ms: u128`
  - `serialize_ms: u128`
  - `total_ms: u128`
- 为上述类型添加中文文档注释，文档注释放在 `#[derive(...)]` 之前。
- 只为确实需要序列化的请求/响应类型派生 `Serialize` / `Deserialize`。

验收：

- `cargo check` 无 warning。
- 类型命名和字段命名符合 Rust 规范。
- 没有为了消除 warning 添加 `#[allow(dead_code)]`。

#### M23.4：实现 GraphRuntime 加载与查询

- 实现 `GraphRuntime::load(graph_db_path: impl AsRef<Path>) -> Result<Self>`。
- `load` 内部调用当前稳定的 graphdb 打开方式，优先使用现有 CLI 项目级查询同路径。
- `load` 记录 graphdb 文件 metadata：
  - `mtime`
  - `size`
  - `loaded_at`
- `load_count` 初始化为 `1`。
- 实现 `GraphRuntime::query(&self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse>`。
- M23 只支持 `RuntimeQueryCommand::ExplainCondition`。
- `query` 调用 `build_explain_condition_output(&self.graph, &request.target, &request.budget)`。
- `query` 的 `graph_load_ms` 必须为 `0`，因为 runtime 已经加载。
- `query_compute_ms` 只统计查询构建耗时。
- `serialize_ms` 可以先统计 `serde_json::to_vec(&result)` 的耗时，但不能改变返回的 `result`。
- `total_ms` 覆盖整个 `query` 调用。
- 对不支持的命令不预留空分支；M23 只有一个 enum variant。

验收：

- 同一个 `GraphRuntime` 调用多次 `query`，`load_count` 保持 `1`。
- `query` 不调用 `GraphDB::open_or_diagnostic` / `GraphDB::load_from_db`。
- `query` 返回的 `result.kind == "Explain"`。

#### M23.5：补测试项目与 runtime 回归

- 新增测试文件或扩展现有 regression 测试，建议命名：
  - `tests/runtime_tests.rs`
- 测试 fixture graphdb 可以在测试内基于 `tests/fixtures/test_project` 构建到 `/tmp` 或临时目录。
- 增加测试：`test_graph_runtime_reuses_loaded_graph_for_explain_condition`。
- 测试步骤：
  - 构建 fixture graphdb。
  - `GraphRuntime::load(&graph_db_path)`。
  - 第一次执行 `ExplainCondition`。
  - 第二次执行同一 `ExplainCondition`。
  - 断言 `runtime.load_count == 1`。
  - 断言两次结果的 `kind`、`query_target`、关键 `summary` 字段一致。
  - 断言第二次响应的 `timing.graph_load_ms == 0`。
- 增加真实项目 ignored 测试，建议命名：
  - `test_real_project_runtime_input3_explain_condition_reuses_graph`
- 真实项目测试目标：
  - project：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
  - target：`comp:app/销售.app/销售/合同协议.spg|input3`
- 真实项目测试断言：
  - `details.primary_path` 中包含 `model22.phoneNumber`
  - 包含 `fact_qwSidebar.phoneNumber`
  - 包含 `action1`
  - 包含 `action4`
  - 连续两次查询 `load_count == 1`

验收：

- fixture 测试默认运行。
- 真实项目测试标记 `#[ignore]`，避免 CI 依赖本机路径。
- 测试断言使用 `assert_eq!` / 明确 helper，避免只 `assert!(json.to_string().contains(...))` 的弱断言；真实项目长 JSON 可使用已有 `array_any_contains_substring` 类 helper。

#### M23.6：保留 CLI 兼容并避免范围外扩散

- 不新增 CLI 参数。
- 不新增 `--serve-stdio`。
- 不更新 `SKILL.md`。
- 不改变 `--explain-condition` 当前命令输出。
- 如必须调整 `main.rs`，只做调用函数名变化，不改变分支顺序和参数语义。
- 不把 `GraphRuntime` 接入默认 CLI 单次查询路径；M23 只提供 runtime 能力和测试证明。

验收：

- 现有命令仍可运行：
  - `./target/release/metadata-checker --project-dir <PROJECT> --graph-db-path <DB> --explain-condition '<TARGET>' --budget compact`
- 输出字段兼容 M22 eval。

#### M23.7：性能与验证记录

- 运行 `cargo check`。
- 运行受影响测试：
  - `cargo test --test runtime_tests`
  - `cargo test --test regression_tests`
  - 如改动 explain 输出，再运行 `cargo test --test explain_tests` 或仓库中对应 explain 测试文件。
- 运行真实项目 ignored 测试或手动命令，记录结果。
- 可选：用 `samply` 对 runtime 测试或后续 M24 stdio 查询再采样；M23 不强制。
- 在提交信息或交付说明中明确：
  - 第一次 runtime 查询包含 graph load。
  - 第二次 runtime 查询复用内存 graph。
  - CLI 单次查询仍是冷启动，留给 M24/M26 接入解决。

验收：

- `cargo check` 0 warning。
- 默认测试通过。
- 真实项目 input3 主链路不回退。
- git diff 只包含 runtime、explain 查询函数拆分、测试和必要模块注册。

### 验收目标

- `cargo check` 无 warning。
- 现有测试不回退。
- `GraphRuntime` 可以在一个进程内复用同一份内存 `GraphDB`。
- 同一个 runtime 连续执行两次 `ExplainCondition`，第二次不调用 `GraphDB::load_from_db`。
- `input3` 真实项目查询的主链路不回退，仍能输出 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- M23 完成后，才能进入 M24 的 stdio function calling server。

### 后续拆分

- M24：Stdio Function Calling Server。把 `GraphRuntime` 暴露为 JSONL stdin/stdout 长驻服务。
- M25：Runtime Cache 与 Reload。处理 graphdb 文件变更检测、手动 reload、reload 失败降级。
- M26：Skill / Function Calling 接入与性能验收。更新 skill 决策树，固化冷 CLI、首次 stdio、第二次 stdio 的真实项目性能基线。
