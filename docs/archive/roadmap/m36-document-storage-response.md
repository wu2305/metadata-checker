# M36：Document / Storage / Response 边界纠偏

| milestone | M36 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M36：Document / Storage / Response 边界纠偏

目标：把已启动的输入与响应抽象纠偏为稳定边界：`source_path` 必须是项目内逻辑路径，`ParsedContent` 负责复用反序列化结果，当前“读取文件内容”的 provider 应与未来 redb/IndexedDB 存储后端抽象区分开；`ResponseProcessor` 定位为 renderer facade 的前置骨架，但不改现有 JSON 输出 schema。

范围约束：

- 不引入远程 API。
- 不实现 IndexedDB / redb 新后端。
- 不实现 MCP adapter。
- 不实现 Mermaid / REPL renderer 迁移。
- 不改 graphdb 文件格式。
- 不改现有 CLI/stdio JSON 输出 schema。
- 不改现有 graph node id；跨项目只先在结构化 `SourceId` 中预留 project scope。

任务清单：

- [x] M36.1：补 `SourceId` / `ProjectRef` / 路径语义
  - `source_path` 定义：
    - 必须是项目内逻辑路径，例如 `app/.../*.spg` 或 `data/tables/.../*.tbl`。
    - 不能是本地绝对路径。
    - session local path、remote path、本地绝对路径只能存在于 provider/session adapter。
  - `ProjectRef` 字段建议：
    - `namespace: Option<String>`
    - `project_id: String`
  - `SourceId` 字段建议：
    - `project_ref: ProjectRef`
    - `source_path: String`
    - `source_kind: spg|tbl|unknown`
    - `origin: local|session|remote|memory`
    - `revision: Option<String>`
  - 要求：
    - 先不改现有 graph node id。
    - evidence/meta 可逐步补 `project_ref + source_path`，为跨项目链路预留结构化身份。
    - 保留现有 `parse_file(path)` 兼容。

- [x] M36.2：新增 `ParsedContent`，替代重型 `MetadataDocument`
  - 定位：
    - 只负责持有原始内容、`SourceId` 和懒解析缓存。
    - 不负责判断 `.spg/.tbl`。
    - 不负责建图、查询、输出或远程拉取。
  - 结构建议：
    - `source: SourceId`
    - `content: MetadataContent`
    - `content_hash: Option<String>`
    - `parsed_json: OnceLock<Arc<serde_json::Value>>`
  - `MetadataContent` 建议：
    - `Bytes(Arc<[u8]>)`
    - `Text(Arc<str>)`
    - `Json(Arc<serde_json::Value>)`
    - 可预留 `CompressedBytes { encoding, bytes }`，但 M36 不实现完整压缩流解析。
  - 要求：
    - `ParsedContent::json()` 连续调用必须复用同一个 `Arc<Value>`。
    - 后续需要 `serde_json::Value` 的 parser/scanner 入口优先从 `ParsedContent::json()` 获取。
    - M36 可保留少量兼容 clone，但新入口必须避免重复 `serde_json::from_slice/from_str`。

- [x] M36.3：纠偏当前 `StorageProvider` 命名边界
  - 背景：
    - 当前已落地的 `StorageProvider` 实际是内容读取 provider。
    - 长期意义上的 `StorageProvider` 应表示 redb / IndexedDB / memory index 等存储后端抽象。
  - 任务：
    - 将当前只读内容接口重命名或标记为 `DocumentProvider` / `ContentProvider`。
    - 保留 `LocalDocumentProvider` 读取本地文件内容。
    - 为真正数据库后端抽象预留 `GraphStore` / `IndexStore`，但 M36 不大规模替换 redb。
  - 只读原则：
    - M36 的 document provider 只负责 `read_bytes` / `read_to_string` / `metadata`。
    - 不增加 `write_bytes`；写入 session/cache 放到 M40。
    - 不抽目录遍历；`discover files` 放到 M39 `ProjectIndexer`。
  - 测试：
    - memory provider fixture。
    - local provider error context。

- [x] M36.4：明确 `ResponseProcessor` 为 renderer facade 前置骨架
  - 定位：
    - 负责把内部结果渲染成机器可读或人类可读输出的统一入口。
    - 未来支持 MCP、STDIO、REPL、Mermaid 等 renderer。
    - M36 只收敛已有 runtime/stdio timing 与 response 后处理，不迁移 human/mermaid。
  - 结构建议：
    - `ResponseProcessor` 作为 facade。
    - 预留 `ResponseRenderer` trait。
    - 预留 renderer 类型：`AiJsonRenderer`、`StdioRenderer`、`McpRenderer`、`HumanRenderer`、`MermaidRenderer`。
  - 当前行为：
    - 继续复用现有 `RuntimeQueryResponse` / `RuntimeTiming`。
    - 保持 `AiOutput` schema 不变。
    - 保持 CLI/stdio 现有 JSON 字段不变。
  - 需要澄清并记录：
    - runtime 内部 result 预估大小与 stdio envelope 最终 stdout JSON 行大小不是同一个概念。
    - M36 不改字段名，但文档中要写清当前 `timing.output_size_bytes` 在 stdio 中表示最终 envelope 行大小。

- [x] M36.5：ParserFacade 最小入口
  - 新增入口建议：
    - `parse_content(&ParsedContent)`
    - `parse_file(path)` 内部构造 `ParsedContent` 后调用 `parse_content`
  - 要求：
    - 不改变 `PageMetadata` 返回结构。
    - 不引入 `ParsedMetadata` enum；parsed JSON 由 `ParsedContent::json()` 负责。
    - 不改 CLI 输出快照。

- [x] M36.6：文档与边界断言
  - 在 `docs/schema.md` 或 architecture 文档中声明：
    - `source_path` 是项目内逻辑路径。
    - `ParsedContent` 是反序列化缓存，不是 graph node，不进入 query 层。
    - document provider 与真正的 graph/index storage provider 的区别。
    - core response
    - adapter envelope
    - response renderer 责任边界
  - 测试新增：
    - `ParsedContent::json()` 连续调用复用同一个 `Arc<Value>`。
    - invalid JSON 只在 `json()` 时返回错误。
    - `parse_file` 输出兼容旧行为。
    - document provider 不依赖 graphdb。
    - `response_processor` 不依赖 stdio。

验收标准：

- `cargo test` 全量通过。
- `parser::parse_file` 兼容旧调用。
- 新增纯内存 document provider / `ParsedContent` 测试证明解析可脱离真实文件系统。
- 现有 CLI/stdio JSON schema 不变。
- `ParsedContent` 能证明同一内容不会被多次反序列化。
- M36 结束时不得出现远程 API、IndexedDB、MCP adapter、Mermaid renderer 的实现。

M36 验收后遗留项：

- [ ] M36-FOLLOW-1：真实项目 AI eval graphdb 缓存隔离
  - 问题：
    - `tests/ai_eval_tests.rs` 中真实项目 case 使用 `std::env::temp_dir()` 下的固定 graphdb 路径。
    - 现有逻辑是“graphdb 文件存在就复用”，不会校验当前二进制版本、元数据 hash、schema version 或构建参数。
    - 这会让旧 graphdb 污染后续验收，出现 `writer_facts.paths` 为空、`summary.primary_paths_count=0` 等假失败。
  - 要求：
    - 后续测试基础设施应保证真实项目 eval 每次使用干净 graphdb，或至少按二进制 hash / schema version / project metadata hash 失效。
    - 不应依赖手工清理 macOS `TMPDIR`。
    - 失败信息应能区分“当前代码输出不满足断言”和“测试复用了 stale graphdb”。
  - 建议归属：
    - 可放入 M38/M39 之前的测试基础设施修复，也可独立作为 `TEST-INFRA` 小任务。

- [ ] M36-FOLLOW-2：正式拆分 `source_path` 与本地展示/来源路径
  - 当前状态：
    - `SourceId::from_local_path` 在缺少 `project_dir` 且传入项目外绝对路径时，会把 `source_path` 退化为 basename。
    - 这保护了“`source_path` 不得是绝对路径”，也保留了单文件 CLI 绝对路径输入兼容。
  - 风险：
    - basename 不是完整项目内逻辑路径，只能作为单文件模式下的兼容降级。
    - 后续远程会话、跨项目链路、浏览器上传若继续复用这个降级语义，会丢失来源身份。
  - 要求：
    - 后续应新增正式字段或结构表达本地/远程展示路径，例如 `display_path` / `origin_path`。
    - `source_path` 继续只表示项目内逻辑路径或单文件兼容名称。
    - 不应把 basename fallback 扩展为跨文件、跨项目或 graph node identity 的依据。
  - 建议归属：
    - 已并入 M37.4，M37 直接实现 `display_path` 与 `origin_path`。
    - M40 Session Manager 或 M43 WASM 单文件阅读前继续补远程/session 场景测试。
