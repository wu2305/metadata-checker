# M37：Parse Core 纯解析入口

| milestone | M37 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M37：Parse Core 纯解析入口

目标：把 `.spg/.tbl` 的“结构化解析”稳定成不依赖本地文件系统的 core API。M37 只收敛解析入口、返回类型和 native wrapper 边界，为后续 WASM/浏览器端单文件阅读做准备；不拆 workspace、不引入 wasm target、不改变 graphdb、query、CLI/stdio 输出 schema。

M37 的核心目的：

- 让调用方可以直接传入 `ParsedContent` / `serde_json::Value` / `&str` 完成 `.spg` 和 `.tbl` 解析。
- 让解析层只返回结构化元数据对象，不承担文件读取、graph 构建、query、AI 输出、人类输出。
- 把本地文件路径相关逻辑留在 native adapter wrapper，例如 `parse_file(path)`。
- 明确单文件输出中的 `query_target` 只是“本次回答目标的展示/短标识”，不是跨项目、多远程服务器下的全局身份。
- 为未来 `metadata-parse-core` crate 提供最小可迁移 API，但 M37 不实际拆 crate。

M37 需要实现的功能点：

- `ParsedContent` 使用 `OnceLock<Arc<serde_json::Value>>` 管理 JSON 缓存。
- 新增不依赖文件系统的 `.spg/.tbl` 解析入口。
- `.tbl` 解析新增 source-based 入口，逐步移除 core API 对 `Path` 的依赖。
- `SourceId` 一步到位拆分 `source_path`、`display_path`、`origin_path`：
  - `source_path`：项目内逻辑路径或单文件兼容名称，参与逻辑身份。
  - `display_path`：面向人类/AI 的安全展示路径，不能包含 token/cookie。
  - `origin_path`：provider 内部定位用，可为本地绝对路径、session key 或远程资源 locator；不进入 graph node id，不默认进入 AI 输出。
- `tbl_single` 中的 human 输出迁移到 output / adapter 层。
- `AiOutput.query_target` 的语义写清楚：
  - 它是用户可读的短目标或当前输出目标。
  - 单项目内可使用表 ID / dbTableName。
  - 单远程服务器多项目时需要配合 `ProjectRef`。
  - 多远程服务器场景不能只依赖表 ID。
- 为未来结构化目标身份预留设计口径，但 M37 不实现 schema 变更：
  - `RemoteRef { remote_id, server_fingerprint, tenant_id, environment }`
  - `ProjectRef { remote_ref, namespace, project_id, revision }`
  - `target_ref { remote_id, project_id, source_path, table_id, db_table_name }`

范围约束：

- 不拆分 Cargo workspace。
- 不新增 WASM 构建配置。
- 不迁移 graphdb / scanner / query 语义。
- 不修改现有 CLI、stdio、MCP 预备 schema。
- 不实现远程 metadata 获取。
- 不实现 `RemoteRef` / `target_ref` 输出字段。
- 不把 `origin_path` 写入 graph node id 或默认 AI 输出。
- 不解决 `GraphStore` / `IndexStore`，留给 M39。
- 不把 `basename` fallback 当作长期 source identity 方案。

任务清单：

- [x] M37.1：定义 Parse Core 返回类型边界
  - 不新增 `ParsedMetadata` enum。
  - M37 中“parsed metadata”指的是已经反序列化并缓存的 `serde_json::Value`，由 `ParsedContent::json() -> Arc<Value>` 提供。
  - 领域解析结果继续使用现有结构：
    - `.spg`：`superpage::SuperPageMetadata`
    - `.tbl`：`tbl_single::TblMetadata`
    - 兼容聚合入口：`PageMetadata`
  - 决策要求：
    - 不让解析 API 返回 AI 输出结构。
    - 不让解析 API 依赖 graph node/edge 类型。
    - 不让解析 API 依赖本地文件系统路径。

- [x] M37.2：新增纯解析 API
  - 建议入口：
    - `parse_content(&ParsedContent) -> Result<PageMetadata>` 保持兼容。
    - `parse_metadata_from_value(source: SourceId, raw: Arc<Value>) -> Result<PageMetadata>`。
    - `parse_metadata_from_str(source: SourceId, content: &str) -> Result<PageMetadata>`。
    - `parse_superpage_from_value(source, raw)`。
    - `parse_tbl_from_value_with_source(source, raw)`。
  - 要求：
    - 不读取文件。
    - 不创建 graphdb。
    - 不输出 JSON/human 文本。
    - `ParsedContent::json()` 仍是反序列化缓存入口。

- [x] M37.3：清理 `tbl_single` 的 `Path` 强依赖
  - 当前 `tbl_single::parse_tbl(path, raw)` 把 `Path` 同时用于 query target、source file 和解析上下文。
  - 新增 source-based 入口：
    - `parse_tbl_from_value_with_source(source: &SourceId, raw: Value)`。
  - `parse_tbl(path, raw)` 降级为 native wrapper：
    - 只负责把 `Path` 转成 `SourceId` 或 display/source 信息。
    - 不在 core API 中传播 `Path`。
  - 保持现有 `.tbl` 输出快照不变。

- [x] M37.4：扩展 `SourceId` 路径字段
  - 增加字段：
    - `display_path: Option<String>`
    - `origin_path: Option<String>`，语义必须是 provider 内部定位。
  - 构造规则：
    - `from_local_path(project_ref, path, project_dir)`：
      - 有 `project_dir`：`source_path` 为相对项目路径，`display_path` 优先同 `source_path`，`origin_path` 可保存本地绝对路径。
      - 无 `project_dir` 且传入相对路径：`source_path` 和 `display_path` 使用该相对路径。
      - 无 `project_dir` 且传入绝对路径：`source_path` 使用 basename 兼容单文件模式，`display_path` 使用安全展示路径，`origin_path` 保存本地绝对路径。
    - `from_memory(project_ref, source_path)`：`origin_path` 为空，`display_path` 默认同 `source_path`。
  - 约束：
    - `source_path` 继续必须通过 `is_project_internal_path`。
    - `display_path` 不参与身份判断。
    - `origin_path` 不进入 graph node id，不作为 `query_target` 默认值，不进入默认 AI 输出。
  - 测试：
    - 绝对路径输入时 `source_path` 不是绝对路径。
    - 绝对路径输入时 `origin_path` 能保留原始本地路径。
    - `display_path` 在单文件模式下可用于展示但不污染 `source_path`。

- [x] M37.5：明确 human 输出和解析模块关系
  - 检查 `tbl_single` 中是否仍有 human 输出函数混在解析模块。
  - M37 必须迁出解析模块中的 human 输出函数，不保留 legacy/native adapter 注释作为长期解释。
  - 要求：
    - 解析模块只返回结构化对象。
    - human 输出属于 `output` / adapter 层。
    - `main.rs` 调用点随迁移同步更新。
    - 模型面对模块职责时不应看到“解析模块内仍可负责人类输出”的双重口径。

- [x] M37.6：梳理 native-only 依赖清单
  - 标记解析 core 中不能进入 WASM 的依赖：
    - `std::fs`
    - 本地 `Path` 强依赖
    - graphdb/redb
    - CLI/clap
    - stdout/stderr 输出
  - M37 不配置 feature gate，只在代码注释和文档中明确 native wrapper 边界。

- [x] M37.7：明确 `query_target` 与未来全局目标身份
  - 这里的 `query_target` 特指 `AiOutput.query_target` 顶层字段，不是 graph 查询入口。
  - 当前单文件 `.tbl` 输出链路：
    - `tbl_single::build_tbl_output` 将 `output.query_target = meta.input_path.clone()`。
    - `tbl_single::parse_tbl(path, raw)` 当前用 `path.display()` 初始化 `TblMetadata.input_path`。
    - M36 后 `parser::parse_content` 会把 `PageMetadata.input_path` 设为 `SourceId.source_path`，再传给 `parse_tbl`。
  - M37 需要给出稳定口径：
    - 单文件 `.tbl` 的 `AiOutput.query_target` 可以继续使用 `source_path` 或改为表 ID / dbTableName，但必须是展示/短目标。
    - 表 ID / dbTableName 只能作为项目内局部身份。
    - 多项目身份需要 `ProjectRef + table_id`。
    - 多远程服务器身份需要 `RemoteRef + ProjectRef + table_id`。
    - M37 不新增 `target_ref` 字段，但文档要说明后续结构化身份应放在 `target_ref` / node meta / session manifest，而不是复用 `query_target`。
  - 验收：
    - 单文件 `.tbl` 输出不再把绝对路径作为 `query_target`。
    - 输出 schema 不变。
    - docs/schema.md 说明 `query_target` 不是全局唯一身份。

- [x] M37.8：补测试
  - 纯字符串 `.spg` 解析。
  - 纯字符串 `.tbl` 解析。
  - `ParsedContent::from_json` 解析。
  - `parse_file(path)` 旧行为兼容。
  - 绝对路径单文件 CLI 仍输出合法 JSON。
  - `source_path` 不包含绝对路径。
  - `origin_path` 保留绝对路径输入的定位信息。
  - `display_path` 不参与身份判断。
  - 单文件 `.tbl` 的 `AiOutput.query_target` 不包含本地绝对路径。
  - CLI/stdio 输出快照不变。

M37 已确认决策：

- `ParsedContent` 应使用 `OnceLock<Arc<Value>>` 替代 `Mutex<Option<Arc<Value>>>`。
- 不新增或修改 `ParsedMetadata` 类型。
- `SourceId` 在 M37 增加 `display_path` 与 `origin_path`，一步到位解决单文件绝对路径的展示/定位边界。

验收标准：

- 单文件 `.spg/.tbl` fixture 可以只用字符串解析。
- 纯解析路径不需要 `std::fs`。
- 不改变 CLI 输出快照。
- `cargo fmt --check`、`cargo check`、`cargo test` 全部通过。
- 默认 `cargo test` 不受 stale graphdb 影响；若仍依赖真实项目 graphdb，必须使用隔离/失效策略。
