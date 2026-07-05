# M41：统一远程元数据获取与 Session

| milestone | M41 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M41：统一远程元数据获取与 Session

目标：在 Rust core 中收敛远程元数据获取语义，使用 `reqwest` 作为统一 HTTP client：native/CLI 使用 reqwest 原生 HTTP client，browser-wasm 使用 reqwest 的 WASM/fetch backend 并借用浏览器登录 session。M41 同时覆盖本地 CLI/stdio session 同步，以及浏览器 Service Worker 通过 WASM 后台获取远程元数据的能力；Service Worker JS 仍只是注册、message routing 和 WASM lifecycle 胶水，不在 JS 中复制低代码平台元数据 URL 拼接、响应解析和错误码映射。

边界调整：

- M41 允许引入 `reqwest`，但必须通过 feature/target 控制依赖，避免 CLI 制品携带无效 wasm 依赖，避免 browser-wasm 制品携带 native-only 能力。
- M41 不实现 IndexedDB 真实持久化，不实现正式 UI 面板，不实现复杂离线缓存。
- M41 不把 `window.SZ.rc` 作为主链路；`window.SZ.rc` 只保留为浏览器原生 fetch 或 WASM fetch 不可用时的 page fallback。
- M41 的 Service Worker 能力是“页面 message 触发后台 fetch”，不是拦截 `/analyzer/app/...` 页面请求。
- M41 native session 仍需要保证 token/secret 不进入 stdout/stderr/graphdb/manifest。

任务清单：

- [ ] M41.1：收敛远程元数据 core contract
  - 保留并扩展 `RemoteMetadataProvider` / `AsyncRemoteMetadataProvider`。
  - 统一 `RemoteFileRef`，至少包含：
    - `remote_ref`
    - `project_ref`
    - `source_path`
    - `file_id`
    - `revision`
  - 统一响应类型：
    - `RemoteFileInfo`
    - `RemoteFileContent`
    - `RemoteProjectInfo`
    - `RemoteMetafileEntry`
  - 统一错误码：
    - `REMOTE_AUTH_REQUIRED`
    - `REMOTE_SESSION_EXPIRED`
    - `REMOTE_FETCH_UNAUTHORIZED`
    - `REMOTE_FETCH_FORBIDDEN`
    - `REMOTE_FETCH_NOT_FOUND`
    - `REMOTE_FETCH_CORS_BLOCKED`
    - `REMOTE_FETCH_FAILED`
    - `REMOTE_RESPONSE_INVALID`
  - URL 构造、fileRef 校验、响应解析、错误码映射必须在 Rust 中实现。
  - JS 侧不得再新增低代码平台 URL 拼接逻辑；现有 JS URL helper 后续应迁移为调用 WASM 或只作为 fallback。

- [ ] M41.2：引入 reqwest transport
  - `Cargo.toml` 中新增可选 `reqwest` 依赖。
  - native/CLI feature 使用 reqwest native async client。
  - browser-wasm feature 使用 reqwest wasm/fetch backend。
  - wasm 请求必须等价于 `fetch(..., credentials: include)`，借用浏览器登录 session。
  - 明确 wasm 下不依赖 reqwest 的 native cookie jar；cookies 由浏览器环境提供。
  - 验证 `cargo check`、`cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown`。
  - 使用 `cargo tree` 证明 feature/target 依赖没有明显串线。

- [ ] M41.3：实现 Rust/WASM browser remote provider
  - 不再写死 `web_sys::window().fetch`；应支持 page / Service Worker 运行环境。
  - 暴露 WASM API：
    - `fetchRemoteFileInfo(fileRefJson)`
    - `fetchRemoteFileContent(fileRefJson)`
    - `loadRemoteSuperpageDocument(fileRefJson)`
    - 后续可扩展 `analyzeRemoteSelection(selectionJson)`
  - 返回统一 envelope，不直接抛裸 JS Error。
  - 覆盖同源 cookie/session 借用路径。
  - 覆盖 401/403/404/CORS/network/invalid JSON/empty body。

- [x] M41.4：Service Worker message API 接入远程元数据
  - 在 `browser/service-worker/metadata-checker-sw.js` 中新增 message method：
    - `fetchRemoteFileInfo`
    - `fetchRemoteFileContent`
    - `loadRemoteSuperpageDocument`
  - SW JS 只调用 WASM export，不复制元数据 URL 构造和响应解析。
  - 即使 WASM runtime 初次未初始化，也能 lazy init 后执行远程 fetch。
  - WASM 初始化失败时返回稳定 diagnostic，允许 page fallback。
  - 测试覆盖 SW fetch 成功、401、404、WASM_FETCH_FAILED、重复请求复用初始化 Promise。

- [x] M41.5：Controller 支持远程加载双模式
  - Page provider 模式：页面 provider 拉 rawText，再调用 `loadSuperpageDocument(sourcePath, rawText)`。
  - Runtime/SW provider 模式：页面只传 `fileRef/selection`，runtime/SW 自行远程加载。
  - 默认优先 runtime/SW provider；失败时可 fallback 到 page provider。
  - selection payload 仍禁止携带 `.spg` raw text 或完整组件 JSON。
  - 大型 `.spg` 不通过 selection bridge 传输。

- [x] M41.6：定义 native session manifest schema
  - `session_id`
  - `created_at`
  - `updated_at`
  - `remote_server`
  - `project_ref`
  - `project_name`
  - `source_origin`
  - `files[]`
  - `graph_db_path`
  - `schema_version`
  - 不记录 token/secret。

- [x] M41.7：实现 session 创建/打开
  - 从 remote server + project 创建 session。
  - session 内保存可扫描的项目镜像。
  - 保持原 `--project-dir` 行为不变。
  - session-aware build graph 复用现有 scanner/query。

- [x] M41.8：实现 native CLI remote provider
  - 当前已完成 native remote session provider contract、in-memory/test provider、基于 reqwest 的 BI provider、permissionInfo JSON / lz-string 压缩响应解析、gzip HTTP body 解压、真实登录 cookie jar 复用和真实服务器端到端验收。
  - `list_projects`
  - `list_metafiles`
  - `fetch_metafile_info`
  - `fetch_metafile_content`
  - `fetch_changed_since`（远端支持时启用）
  - 登录流程使用 reqwest native client。
  - cookie/session 使用 native cookie jar 或显式 session store。
  - 不把密码、cookie、token 写入日志、manifest 或 graphdb。

- [x] M41.9：定义认证与安全边界
  - `AuthProvider`
  - `SecretStore`
  - 凭证读取与刷新策略。
  - 日志脱敏策略。
  - token 不进入 stdout/stderr/graphdb/manifest。
  - AuthContext 至少区分：
    - `browser_session`
    - `cookie_jar`
    - `explicit_credentials`
    - `none`

- [x] M41.10：远程同步到 session
  - 当前已完成远程内容写入 session project mirror 的底座：logical path 校验、临时文件写入、manifest 更新、未变化跳过、删除标记、deleted 条目不拉取内容、失败时保留旧 session。
  - 已提供 `build_session_graph` 复用现有 `ProjectIndexer::scan`。
  - 已接入真实 CLI refresh 命令，并在真实 BI 测试环境验证 `analyzer` 项目可同步到 session 后构建 graphdb。
  - 根据 remote logical path 写入 session project。
  - manifest 记录 etag/version/hash/mtime/size。
  - 支持只同步 SuperPage 及其关联 `.tbl` 的局部模式。
  - 支持全项目同步模式。

- [x] M41.11：远程增量
  - 当前已有 partial sync 不删除未出现文件、deleted entry 标记并删除 mirror 文件、fetch 失败不污染旧 session 的测试。
  - 尚未完成真实远端 changed_since / etag diff 的端到端验收。
  - 未变化文件不重新下载。
  - 删除文件在 manifest 中标记并从 session index 移除。
  - 失败时保留上一轮可用 session。

- [x] M41.12：session 管理命令
  - [x] list sessions
  - [x] show manifest
  - [x] delete session
  - [x] status
  - [x] refresh session：已接入真实 provider/login/session sync/build graph

验收标准：

- 远程元数据 URL 构造、响应解析、错误码映射在 Rust 中完成，JS 不再新增第二套业务实现。
- browser-wasm 通过 reqwest/fetch 借用浏览器登录 session 获取远程元数据。
- Service Worker 可以通过 message 调用 WASM 远程 fetch，并返回统一 envelope。
- 没有真实 WASM artifact 时仍有稳定 fallback diagnostic，不阻断 page fallback 验收。
- 本地可以从远程 server 拉取项目元数据并建立 session。
- session 同步后可用现有 build graph/query。
- 增量同步不会全量重拉。
- token/secret 不进入 stdout/stderr/graphdb/manifest。
- 远程失败不破坏已有可用 session。
- M41 不引入 rexie/IndexedDB 真实持久化依赖。
