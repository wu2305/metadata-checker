# M99：远期架构规划 - WASM 解析库、远程元数据会话与 MCP

| milestone | M99 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M99：远期架构规划 - WASM 解析库、远程元数据会话与 MCP

> 记录日期：2026-05-19。  
> M99 是 planning-only 的远期占位，不纳入 M35 当前验收范围。后续实施时应从 M99 拆出独立近期里程碑，避免把浏览器端、远程拉取和 MCP 协议同时塞进现有 CLI。

### 背景判断

当前仓库已经有 `src/lib.rs`，但它只是把 CLI 内部模块整体公开，并不是稳定的嵌入式核心库。长期要支持浏览器端人工阅读分析、远程拉取元数据、MCP 工具化，需要先把解析、分析、存储、会话、协议适配拆出清晰边界。

当前主要耦合点：

- `src/parser.rs` 的 `parse_file(path)` 同时做文件 IO 和解析。
- `src/tbl_single.rs` 解析依赖 `Path`，并混有 human `println!` 输出。
- `src/scanner/mod.rs` 同时负责目录遍历、hash、增量判断、SPG/TBL 处理和写 redb。
- `src/graph.rs` 把 `petgraph` 内存图和 `redb` 持久化放在同一 `GraphDB` 类型里。
- `src/runtime.rs` 与 `src/stdio_server.rs` 的 command surface 仍未完全统一，MCP 不应在这个状态下另起第二套查询协议。
- WASM 端不能直接依赖 `redb`、本地文件系统扫描、stdio server、CLI `clap` 分支。

### 目标分层

远期目标不是重写算法，而是拆分 crate / module 边界：

- `metadata-parse-core`
  - 纯解析库，可编译到 WASM。
  - 输入：`&str` / `serde_json::Value` / `SourceId`。
  - 输出：SPG/TBL AST、组件树、表达式引用、条件记录、DataFlow 原始结构。
  - 禁止依赖 fs、redb、clap、stdio、petgraph。

- `metadata-analysis-core`
  - 纯内存分析层。
  - 包含 dependency、priority、condition extraction、轻量 graph builder、answer contract。
  - 可作为 WASM 第二阶段能力，但需要控制输出大小和运行耗时。

- `metadata-graph-runtime`
  - 查询引擎层：`explain_condition`、`query_page_logic`、`query_model`、`context`。
  - 面向 `GraphStore` / `MetadataStore` trait，而不是直接依赖 redb。
  - CLI、stdio、MCP 都只能适配这一层，不重复实现分析逻辑。

- `metadata-storage-redb`
  - native 持久化层。
  - 负责 redb 文件格式、增量索引、锁、GraphDB 落盘。
  - 不进入 WASM。

- `metadata-session`
  - 会话目录管理。
  - 本地项目和远程项目最终都落到同一种 session manifest。
  - 记录 remote project id、metafile id、etag/hash、local path、last fetched、auth scope。

- `metadata-remote`
  - 远程元数据获取层。
  - 通过 provider trait 拉取 projects/metafiles。
  - 不参与解析，只负责把远程内容写入 session。

- `metadata-cli`
  - 当前二进制入口。

- `metadata-mcp`
  - MCP server adapter。
  - 只暴露任务型工具，不重新实现查询和输出 contract。

### 方向 1：抽离 WASM 可用解析库

优先级：最高。  
改造量：中等。  
风险：低。

第一阶段只支持单文件 `.spg/.tbl` 人工阅读分析，不做全项目 graphdb：

- 新增或迁移纯函数：
  - `parse_superpage_from_str(source_id, content)`
  - `parse_superpage_from_value(source_id, raw)`
  - `parse_tbl_from_str(source_id, content)`
  - `parse_tbl_from_value(source_id, raw)`
  - `parse_metadata_from_str(source_id, content)`
  - `parse_metadata_from_value(source_id, raw)`
- `parse_file(path)` 保留在 native adapter，不进入 core。
- `tbl_single` 中的 human 输出移出解析模块。
- 使用 feature gate 控制 native-only 能力：
  - `default = ["native"]`
  - `wasm` 不启用 `redb` / `clap` / stdio。
- WASM 第一阶段输出：
  - 组件树
  - 表达式引用
  - 条件记录
  - DataFlow 节点和字段映射
  - 单文件 value trace / priority summary

验收边界：

- core crate 能在 native test 中只用字符串输入解析 fixture。
- core crate 不出现 `std::fs`、`redb`、`clap`、`stdio` 依赖。
- 浏览器端可以把用户粘贴或上传的 `.spg/.tbl` 转成结构化 JSON 供人工阅读。

### 方向 2：远程获取元数据并建立会话目录

优先级：中。  
改造量：中大型。  
风险：中高，主要在认证、缓存一致性和远程增量。

设计原则：

- parser 不直接懂远程。
- remote provider 只负责拉取远程内容。
- session 目录模拟本地项目结构，scanner 继续扫描 session dir。
- token/cookie 不进入 graphdb，不进入普通日志。

建议 session 结构：

```text
.metadata-checker/
  sessions/
    <session_id>/
      manifest.json
      projects/
        <project_id>/
          app/...
          data/tables/...
```

`manifest.json` 需要记录：

- `session_id`
- `created_at`
- `remote_base_url`
- `account_id` / `tenant_id`（不含敏感 token）
- `projects[]`
- `metafiles[]`
  - `remote_project_id`
  - `remote_file_id`
  - `remote_path`
  - `local_path`
  - `file_type`
  - `etag` / `version` / `hash`
  - `fetched_at`
  - `deleted_remote`

远程接口建议：

```rust
trait RemoteMetadataProvider {
    fn list_projects(&self) -> Result<Vec<RemoteProject>>;
    fn list_metafiles(&self, project_id: &str) -> Result<Vec<RemoteMetafile>>;
    fn fetch_metafile(&self, file_id: &str) -> Result<FetchedMetafile>;
}
```

验收边界：

- 能创建 session 并同步一个远程 project 到本地 session 目录。
- 对 session 目录运行现有 `--build-graph` 与查询命令。
- 远程增量优先使用 etag/version/hash，避免每次全量拉取。
- token 不写入 manifest、graphdb、stdout/stderr。

### 方向 3：MCP 改造

优先级：中低，应在 runtime command 统一之后再做。  
改造量：中等。  
风险：中，主要是协议重复和输出 contract 漂移。

前置要求：

- 把 stdio 中支持的 `explain_condition`、`explain`、`context`、`query_model`、`query_page_logic`、`advise_query`、`status`、`reload` 统一下沉到 `RuntimeCommand`。
- CLI、stdio、MCP 共用同一套 request/response schema。
- `answer_contract` / `thinking_frame` / `required_followups` / `truncation_guard` 作为 runtime 输出 contract，不由各 adapter 自行拼装。

MCP tools 建议：

- `metadata_open_session`
- `metadata_list_projects`
- `metadata_fetch_project`
- `metadata_build_graph`
- `metadata_advise_query`
- `metadata_explain_condition`
- `metadata_query_page_logic`
- `metadata_query_model`
- `metadata_context`
- `metadata_resolve_model`
- `metadata_status`
- `metadata_reload`

MCP resources 建议：

- `metadata://sessions`
- `metadata://session/{id}/manifest`
- `metadata://session/{id}/schema`
- `metadata://session/{id}/pages`
- `metadata://session/{id}/models`

验收边界：

- MCP tool 输出与 CLI/stdio 在关键 contract 字段上等价。
- MCP 不暴露任意 shell 命令。
- MCP 不让模型绕过 `answer_contract.primary_fact_path` 直接消费 raw graph。
- 同一 session 内连续查询复用 runtime，不重复冷启动加载 graphdb。

### 推荐实施顺序

1. 先做 `metadata-parse-core`，把 WASM 能用的纯解析 API 抽出来。
2. 再统一 `RuntimeCommand` / `GraphStore` / `MetadataStore` 边界，为 MCP 和 future WASM query 铺路。
3. 做 `metadata-session`，让本地项目和远程项目变成同一种输入形态。
4. 做 `metadata-remote` provider 和远程增量同步。
5. 最后做 `metadata-mcp` adapter。

### 明确不建议

- 不要先做 MCP。现在直接上 MCP，会把 stdio/runtime 的历史分叉复制成第三套协议。
- 不要让 parser 直接访问远程接口。
- 不要让 WASM 版依赖 redb 或本地文件扫描。
- 不要把 token/cookie 放进 graphdb 或 AI 输出。
- 不要为了远程功能改写现有 `.spg/.tbl` 解析语义；远程只是另一种输入来源。

### 已可先行启动的基础抽象

这些调整不等于开始实现远程、WASM 或 MCP，只是为后续拆分降低耦合：

- [x] `StorageProvider` 基础接口
  - 当前范围：
    - 抽象 `read_bytes`
    - 抽象 `read_to_string`
    - 抽象 `metadata`
    - 提供 `LocalStorageProvider`
  - 现有消费点：
    - `parser::parse_file_with_storage`
    - `parser::parse_file`
    - `scanner::scan_project`
  - 后续扩展：
    - `SessionStorageProvider`
    - `RemoteCachedStorageProvider`
    - WASM 内存存储 provider
  - 当前不做：
    - 不改变 graphdb 格式
    - 不改变 scanner 的目录遍历策略
    - 不直接接远程 API

- [x] `ResponseProcessor` 基础接口
  - 当前范围：
    - 统一 `RuntimeQueryResponse`
    - 统一 `RuntimeTiming`
    - 统一 runtime response 的序列化大小统计
    - 统一 stdio 响应写出前的 timing 更新
  - 现有消费点：
    - `runtime::GraphRuntime::query`
    - `stdio_server` 的 error/status/query timing
  - 后续扩展：
    - CLI JSON 输出统一走 response processor
    - stdio / MCP 共用同一 response envelope adapter
    - function calling wrapper 复用同一 output size / truncation policy
  - 当前不做：
    - 不修改 `AiOutput` schema
    - 不重写 CLI 输出路径
    - 不实现 MCP adapter

### 仍需抽象的边界

以下抽象不应一次性落地，但需要作为 M99 后续里程碑的技术边界：

- `SourceId` / `ParsedContent`
  - 问题：当前很多函数仍用 `Path` 表示来源，浏览器、远程 session、内存上传都不是稳定文件路径。
  - 目标：用 `SourceId { project_ref, source_path, source_kind }` 表达项目内逻辑路径；用 `ParsedContent` 持有原始内容与懒反序列化后的 `serde_json::Value` 缓存。
  - 价值：解析层不关心内容来自本地、远程缓存还是浏览器上传，并避免同一元数据被多次反序列化。

- `ParserFacade`
  - 问题：`parser.rs` 仍是文件解析入口，不是 crate 级纯解析 API。
  - 目标：统一 `parse_metadata_from_str` / `parse_metadata_from_value` / `parse_document`。
  - 价值：WASM、remote session、CLI 单文件解析共用同一入口。

- `GraphStore`
  - 问题：`GraphDB` 同时代表内存图和 redb 持久化。
  - 目标：拆出 `GraphStore` trait，native redb 只是一个实现；内存图和 WASM 图可以是其他实现。
  - 价值：query/runtime 不直接绑定 redb。

- `ProjectIndexer`
  - 问题：`scanner::scan_project` 同时做目录扫描、增量判断、解析、建图、持久化。
  - 目标：把 `discover -> diff -> parse -> index -> persist` 分成可替换阶段。
  - 价值：远程 session 和本地目录能共用增量建图流程。

- `RuntimeCommandRegistry`
  - 问题：CLI、runtime、stdio 的 command surface 仍有分叉。
  - 目标：所有 command 先落成统一 `RuntimeCommand` / `RuntimeRequest` / `RuntimeResponse`。
  - 价值：MCP 只做 adapter，不重新实现查询分发。

- `ResponseEnvelopeAdapter`
  - 问题：`AiOutput`、stdio envelope、未来 MCP tool result 的包装边界还分散。
  - 目标：核心 response 只表达语义，CLI/stdio/MCP adapter 决定外层 envelope。
  - 价值：避免 CLI 有 contract、stdio/MCP 缺字段。

- `SessionManager`
  - 问题：远程项目需要会话目录、manifest、cache、graphdb 生命周期。
  - 目标：提供 session 创建、打开、同步、清理、状态查询。
  - 价值：本地项目和远程项目统一成 session 输入。

- `RemoteMetadataProvider` / `AuthProvider` / `SecretStore`
  - 问题：远程拉取不能把登录态、token、cookie 混进 parser/scanner/graphdb。
  - 目标：认证、远程 API、密钥存储各自分层。
  - 价值：安全边界清楚，后续可替换不同服务端实现。

- `PathResolver`
  - 问题：当前 page path、graph node id、remote path、session local path、DataFlow path 标准化散落在多个模块。
  - 目标：集中处理路径规范化、node id 构造、page-scoped target 构造。
  - 价值：减少 `model:PAGE|MODEL`、`$DATA:/...`、session path 的重复分支。

### M99 后续里程碑推进计划
