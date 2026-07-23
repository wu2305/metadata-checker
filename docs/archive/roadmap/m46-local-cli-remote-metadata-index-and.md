# M46：Local CLI Remote Metadata Index and Analysis

| milestone | M46 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M46：Local CLI Remote Metadata Index and Analysis

目标：把 M45 在真实浏览器中验证过的远程元数据获取与 raw text 分析链路，沉淀成本地 CLI 可复用能力。用户可以在本地命令行用真实远程服务账号/session 拉取指定 project/module/file 的 `.spg/.tbl`，写入受控 session mirror，构建或增量更新 graphdb，并立即执行 query/analyze。M46 优先复用 M41 native session/provider 能力，不重新发明第二套远程协议。

边界要求：

- Rust/CLI 是主实现；不得把远程下载、index、graph build 的核心逻辑放到 JS。
- 不修改远程 `.spg` / `.tbl`。
- token、cookie、password 不进入 stdout/stderr、graphdb、session manifest、cache、diagnostics 或测试 fixture。
- remote provider 只返回 raw metadata text 和文件元信息，不解析 `.spg/.tbl`、不建图、不做业务推理。
- 本地 session mirror 可以保存 raw metadata 文件，但必须和凭证存储隔离；默认不持久化凭证。
- 继续复用已有 scanner、graph store、query/analyze 能力，不为 remote-index 另建一套 graph snapshot。
- **复用 M54–M56 差量刷新栈（2026-07-18 固定）**：change detection 用 `MetaFilesChangeSource`（含 `BiMetaFilesChangeSource` 真源实现）、文件差量用 `src/diff_refresh/mirror.rs`、编排用 `DiffRefreshOrchestrator`；M46 不实现第二套 change detection / 拉取栈，只做 CLI 产品化包装。

任务清单：

- [ ] M46.1：CLI 命令契约与参数收敛
  - 设计并落地一个清晰入口，例如 `remote-index` 或在现有 session 命令中扩展 `--session-refresh --build-graph --analyze`。
  - 参数至少覆盖：
    - `--base-url`
    - `--project`
    - `--module` / `--source-path` / `--file-id`
    - `--graph-db-path`
    - `--session-dir`
    - `--json`
    - `--dry-run`
  - 认证输入只允许通过 env、交互输入或显式一次性参数；输出必须脱敏。
  - `--help`、README 和错误示例同步更新。

- [ ] M46.2：远程发现与筛选策略
  - 复用 `/api/me/whoami`、`getPermissionInfo`、`getFileChildren`、`getFileDescendant`、`getFileContent/<file_id>`。
  - 支持 project/module/source_path/file_id 四种粒度。
  - 默认只下载 `.spg` / `.tbl`；其它文件只计入 discovered，不进入解析队列。
  - 支持 current page / explicit file first，避免全量项目下载阻塞单页分析。
  - 对 encoded permission、gzip body、octet-stream raw content 保持兼容。

- [ ] M46.3：Session mirror 与增量同步
  - session manifest 记录 remote server、project、file_id、revision/hash、logical source_path、local path、last fetched。
  - raw metadata 写入 session mirror 时保持项目内逻辑路径，不把 remote URL 或本地绝对路径混入 `source_path`。
  - 支持 revision/hash/mtime 级别的增量跳过。
  - 远程删除或不可见文件输出 stable diagnostic，不静默污染旧 graph。
  - 支持 `--clean-remote-mirror` 或同等显式清理入口，默认不破坏已有 session。

- [ ] M46.4：Graph build / index 主链路
  - 从 session mirror 调用现有 scanner/build graph 能力。
  - 支持显式 `--graph-db-path`，并沿用 graphdb lock/read-only 诊断。
  - 单文件失败不终止整批；输出 processed/failed/skipped/current_file。
  - graph node/edge 中不写入 token/cookie/password。
  - 对 `.spg` 和 `.tbl` 均覆盖，避免 CLI 只跑页面样例。

- [ ] M46.5：Analyze / Query 后处理
  - index 后可选择直接执行：
    - page summary
    - component selection analyze
    - model/table query
    - dataflow query
  - JSON 输出使用稳定 envelope：`status`、`summary`、`diagnostics`、`timing`、`graph_db_path`、`session_id`。
  - human 输出保持低噪声，默认不打印 raw metadata。

- [ ] M46.6：进度、诊断与安全审计
  - human 模式展示 discovered/downloaded/indexed/skipped/failed。
  - JSON 模式输出 machine-readable progress summary。
  - 错误码覆盖 unauthorized/forbidden/not_found/invalid_response/network/content_empty/graph_locked。
  - 增加敏感信息脱敏测试，覆盖 URL、headers、body、manifest、graphdb metadata、日志。

- [ ] M46.7：测试与真实环境验收
  - 单元测试：provider contract、URL 构造、raw text 保持、encoded permission、octet-stream content、错误码。
  - 集成测试：mock remote -> session mirror -> graph build -> query。
  - 真实 fixture：只保存脱敏目录 shape 和少量 synthetic raw metadata，不保存 token/cookie/raw 业务元数据。
  - 真实环境验收：使用 `autocrm-test.xiaoshouyi.com` 的已知项目，记录命令、脱敏输出、graphdb 路径、indexed/failed 数量和 query 样例。

验收标准：

- `cargo test` 或影响面 Rust 测试覆盖 remote provider、session mirror、graph build、query 输出和敏感信息脱敏。
- 一条命令可从真实远程 project 拉取至少一个 `.spg`，写入 session mirror，构建 graphdb，并执行至少一个 query/analyze。
- 真实验收记录不含 token/cookie/password/raw metadata。
- remote CLI 和 browser extension 共用同一 remote/session 语义，不出现两套互相矛盾的 source_path/file_id/revision 规则。
