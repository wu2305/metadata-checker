# AGENTS.md — metadata-checker

> 本文件指导其他 AI Agent 在此仓库中工作。所有 Agent 必须遵守以下规范。

## 项目简介

`metadata-checker` 是一个 Rust CLI 工具，用于解析低代码平台的 SuperPage（`.spg`）和 Table（`.tbl`）元数据文件。核心能力包括组件树提取、表达式解析、依赖图构建、值来源追溯、计算优先级分析和跨文件图数据库查询。

## 技术约束

- **核心语言**：Rust 2024 Edition。Rust/WASM core、CLI、stdio、MCP、图查询和解析能力必须保持 Rust 实现
- **浏览器接入层**：M40 起允许在 `browser/` 下维护纯 JS glue / provider / runtime launcher / test harness。JS 只负责平台接入、消息桥、Service Worker 注册、DOM marker 和测试桩，不得承载解析、图查询、业务推理等核心能力
- **二进制大小**：Release 版本控制在 10MB 以内（当前约 1.6MB）
- **跨平台**：需支持 `windows_amd64`、`macos_arm64`、`linux_amd64`、`linux_arm64`
- **依赖管理**：新增依赖须使用 Cargo，优先选择轻量级 crate
- **前端依赖**：`browser/` 下默认不引入 bundler；新增 npm 依赖必须先说明必要性、运行环境和测试命令
- **禁止**：
  - `#[allow(dead_code)]` — 必须正确消费字段或删除
  - 深拷贝优化不当时不做 — 能引用则引用，能 move 则 move
  - 使用 `panic!` 处理可恢复错误 — 使用 `anyhow::Result`

## 代码规范

### 注释
- 所有模块级、类型级、函数级注释使用**中文**
- 文档注释（`///`）必须放在 `#[derive(...)]` 之前
- 复杂算法需添加行内注释说明意图

### 命名
- 函数/变量：`snake_case`
- 类型/枚举：`PascalCase`
- 常量：`SCREAMING_SNAKE_CASE`
- 避免单字母变量名（循环索引除外）

### 测试
- 使用 `assert_eq!` 进行断言
- 测试用例必须说明测试目的（通过函数名或注释）
- 新增功能必须配套测试，覆盖正常和边界场景
- 真实场景测试数据放在 `tests/fixtures/`

### 错误处理
- 使用 `anyhow::Result` 作为返回类型
- 使用 `.with_context()` 添加上下文信息
- 不要在库代码中直接 `println!`/`eprintln!`，由调用方决定输出方式

### Browser JS 规范
- `Plugin Core` 不得 import 平台 glue、runtime launcher、provider 或 DOM API
- `Designer Glue` 只负责把真实设计器对象转换为 selection，不直接 fetch metadata、不直接调 runtime
- `RemoteMetadataProvider` 只负责读取 raw metadata text，不解析 `.spg/.tbl`、不建图、不缓存 token/cookie/password
- `Integration Controller` 负责组合 `selection -> provider -> runtime -> renderer`，不得写入 Plugin Core
- `Service Worker` 注册入口必须是 JS 文件；WASM 只能由 SW JS lazy init 加载，不能把 `.wasm` 直接注册为 Service Worker
- selection payload 只能包含轻量字段，不得包含 `.spg` raw text 或完整 component JSON
- 真实环境可用 `console.log` 辅助人工观察，但自动化验收必须有 DOM marker 或结构化事件；涉及可见 UI 的真实验收还必须补充截图验证，不能只凭 marker 判定 UI 可见

## 模块职责

| 模块 | 职责 | 修改前必读 |
|---|---|---|
| `superpage.rs` | JSON 反序列化、组件树递归提取、表达式引用正则解析 | 理解 `RawComponent` 和 `SpgComponent` 的映射关系 |
| `dependency.rs` | 依赖图构建、拓扑排序、循环检测、值来源递归追溯 | 了解 `RefType` 分类和 `ValueTrace` 结构 |
| `priority.rs` | `defaultValue`/`exp`/`calcCondition` 优先级判定 | 优先级规则：`calcCondition` > `exp` > `defaultValue` |
| `output.rs` | human/JSON 双模式输出、交互式查询循环 | 修改输出格式需同步更新两种模式 |
| `parser.rs` | 文件类型识别、统一元数据入口 | 新增文件类型需在此注册 |
| `graph.rs` | 图节点/边等共享数据结构与通用 helper | 节点/边类型变更需同步 graph store、scanner、query 和序列化逻辑 |
| `graph_redb.rs` | redb 图数据库实现（`cli-local`）与 graphdb lock | 修改 redb 表、锁、持久化行为需覆盖 native 回归 |
| `graph_store.rs` | GraphReadStore / GraphWriteStore / IndexStateStore 抽象 | query 层不得重新绑定具体 redb 实现 |
| `scanner.rs` | 目录扫描、增量更新（mtime+size+hash）、SPG/TBL 处理 | 增量逻辑涉及文件状态比较，改动需谨慎 |
| `query.rs` | 图查询接口：`query_model`/`query_page`/`query_cross`/`query_dataflow` | DataFlow 子图展开涉及字段级追溯，较复杂 |
| `remote_metadata.rs` | 远程元数据 provider contract、WASM fetch 骨架、测试 provider | provider 只返回 raw text，不解析、不建图 |
| `persistence/` | Memory / redb / IndexedDB stub 持久化 provider | redb 不得另建第二套 graph snapshot；browser-wasm 不得引入 redb |
| `visualization/` | VisualGraph、Mermaid、ECharts option 生成 | 输出必须脱敏，renderer 不依赖 DOM |
| `browser/` | JS 插件接入、provider、runtime launcher、Service Worker/harness | 不得承载 Rust core 的解析/查询能力 |
| `cli.rs` | clap 参数定义 | 新增 CLI 参数需同步更新 help 文本 |
| `main.rs` | 子命令分发 | 新增子命令需在此注册并调用对应模块 |

## Agent 协作与验收循环

当用户要求“冷脸验收”“复用 workers”“循环这一段逻辑”时，必须按以下通用流程执行：

1. 主线程先确认当前看似完成的提交或工作区状态。
2. 拉起或复用一个**独立验收者**做冷脸验收。验收者只读不改，输出 P0/P1/P2 阻塞项、测试覆盖缺口和方向性偏差。
3. 主线程把验收问题拆成小任务包，按文件/职责边界分给 scoped workers。每个 worker 必须有明确写入范围，不得修改其他 worker 范围。
4. worker 完成后，主线程复核 diff、运行影响面测试并提交。
5. 再次让独立验收者复验新提交。
6. 循环 2-5，直到独立验收者明确给出“可验收”且无 P0/P1/P2 阻塞项。

执行该循环时：

- 不要让验收者直接修代码。
- 不要把阻塞项一次性塞给一个上下文较小或职责不匹配的 worker；拆成互不重叠的小包。
- 不要全量 `cargo test` 当默认动作；优先按影响面运行目标测试。只有跨模块核心行为或用户明确要求时才跑全量。
- 每轮可验收修复必须提交。

## M40 Browser 接入边界

M40.9 / M40.10 相关实现必须遵守以下边界：

- `RemoteMetadataProvider` 的接口能力在 M40.6 已有；M40.9 通过 Integration Controller 编排使用；M40.10 在真实 BI 环境验收。
- Plugin Core 永远不直接获取远程元数据。
- M40.9 负责 standalone harness 和组合测试，必须证明 `selection -> provider -> runtime load/build/analyze -> renderer` 链路。
- M40.10 必须注册 Service Worker，不得推迟到后续里程碑。
- Service Worker 可以加载 WASM，但注册入口仍是 JS；WASM 应 lazy init，并支持 `instantiateStreaming` 与 `arrayBuffer + instantiate` fallback。
- 若 CSP 禁止 wasm 编译或 WASM URL/MIME 不满足要求，必须返回稳定 diagnostic，并允许 page runtime fallback。
- M40.10 不实现 IndexedDB 真实持久化、不实现复杂离线缓存、不实现正式 UI 设计。
- 真实 BI 验收必须记录 DOM marker，例如 `data-metadata-checker-sw`、`data-metadata-checker-runtime`、`data-metadata-checker-wasm`、`data-metadata-checker-analysis-status`。

## 常见修改场景

### 新增表达式字段
1. 在 `superpage.rs` 的 `always_expr_fields` 或 `conditional_expr_fields` 中添加字段名
2. 在 `RawComponent` 中添加对应的反序列化字段（如需要）
3. 在 `extract_components` 中处理该字段
4. 补充测试用例到 `tests/superpage_tests.rs` 和 `tests/core_feature_tests.rs`

### 新增跨文件关系类型
1. 在 `graph.rs` 的 `EdgeType` 枚举中添加新边类型
2. 在 `scanner.rs` 中识别并创建对应的边
3. 在 `query.rs` 中处理新边类型的查询输出
4. 更新 `tests/scanner_tests.rs`

### 新增 CLI 子命令/参数
1. 在 `cli.rs` 的 `Commands` 枚举或 `Cli` 结构体中添加
2. 在 `main.rs` 中匹配并调用处理函数
3. 更新 `README.md` 和 `--help` 输出

### 优化性能（减少 clone）
1. 先 `cargo check` 确认编译通过
2. 识别 `.clone()` 热点（`grep -n '\.clone()' src/*.rs`）
3. 优先尝试：返回引用（`&T`）、使用 `Cow<str>`、预分配局部变量复用
4. 修改后 `cargo test` 确保全部通过
5. `git diff` 确认变更范围合理

## 提交规范

- **每次工作进度必须 `git commit`**
- 提交信息格式：`type: description`
- 常用 type：
  - `feat:` 新功能
  - `fix:` 修复
  - `perf:` 性能优化
  - `test:` 测试补充
  - `docs:` 文档/注释
  - `refactor:` 重构

## 测试命令

```bash
# 快速检查
cargo check

# 运行全部测试
cargo test

# Release 编译（验证二进制大小）
cargo build --release
ls -lh target/release/metadata-checker

# 在真实项目上测试
./target/release/metadata-checker --project-dir /path/to/project --build-graph
```

## 外部参考

- 低代码平台代码仓库：`/Users/wuhaocheng/Downloads/bi`
- 真实测试项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- metadata-checker 使用说明：若运行环境提供对应 skill / tool 文档，应以其当前版本为准；不要在仓库规范中依赖某个工具的私有本地路径。

## 常见问题

**Q: 编译出现 dead_code 警告怎么办？**  
A: 不要加 `#[allow(dead_code)]`。要么使用这个字段，要么删除它。

**Q: 需要新增外部依赖怎么办？**  
A: 编辑 `Cargo.toml`，优先选择下载量高、体积小的 crate。添加后需评估对二进制大小的影响。

**Q: 修改了输出格式但测试失败？**  
A: `output.rs` 的 human 和 JSON 模式需同步更新。检查 `tests/output_parser_tests.rs` 和 `tests/core_feature_tests.rs`。

**Q: 如何验证图数据库增量更新？**  
A: 先 `--build-graph`，修改某个 fixture 文件，再次 `--build-graph`，检查日志中是否只处理变更文件。
