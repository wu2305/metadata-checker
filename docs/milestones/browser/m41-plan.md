# M41 实施计划：统一远程元数据获取与 Session

> 目标：在 Rust core 中收敛远程元数据获取语义，引入 reqwest 作为统一 HTTP client；建立本地 session 目录并复用现有 scanner/query。M41 不接浏览器页面 UI，不实现 Service Worker 拦截请求，不实现 IndexedDB 真实持久化。

## 1. 当前状态盘点

### 1.1 已有基础

| 模块 | 当前能力 | M41 关系 |
|---|---|---|
| `remote_metadata.rs` | `RemoteFileRef/Info/Content`、`RemoteMetadataErrorCode`、`AsyncRemoteMetadataProvider` trait、`WasmFetchMetadataProvider` 兼容类型 | 核心 contract 保留，兼容类型必须委托 reqwest provider，不得再直接调用 `web_sys::window().fetch` |
| `browser.rs` + `browser_wasm_bindgen.rs` | WASM runtime API：init/load/build/analyze/status，内存态 graph | 仅负责 runtime core；远端 fetch/load 语义不在此暴露 |
| `remote_metadata_provider.rs` | `wasm_bindings`：`fetchRemoteFileInfo` / `fetchRemoteFileContent` / `loadRemoteSuperpageDocument` | 远端文件信息、内容与远端文档加载的 wasm-export 在这里对外提供 |
| `scanner/indexer.rs` | 6 阶段本地扫描：discover → diff → parse → apply → delete → persist | 复用于 session 目录；新增远程 discover/diff |
| `storage_provider.rs` | `DocumentProvider` trait：`read_bytes`/`metadata` | 新增 `RemoteDocumentProvider` 实现 |
| `persistence/` | `GraphPersistenceProvider`：memory / redb / indexeddb stub | 新增 session manifest 持久化（本地文件/JSON） |
| `Cargo.toml` | `cli-local`（clap+redb）、`browser-wasm`（wasm-bindgen+web-sys） | 新增 reqwest，通过 feature/target 隔离 |

### 1.2 当前缺失

- 没有 reqwest 统一 HTTP transport。
- 没有 session 概念：远程 server + project → 本地目录映射。
- 没有远程元数据批量同步能力（list/list_changed/fetch）。
- 没有认证/secret 抽象。
- CLI 没有 session 管理命令。

---

## 2. 关键决策

### 2.1 reqwest 引入方式

```toml
# Cargo.toml 新增
[dependencies]
# 只在 cli-local 和 browser-wasm 都启用时引入 reqwest
# 注意：reqwest 默认 feature 包含 native tls / tokio，必须显式关闭
reqwest = { version = "0.12", default-features = false, optional = true }

[features]
default = ["cli-local"]
cli-local = ["dep:clap", "dep:redb", "dep:reqwest"]
browser-wasm = [
    "dep:wasm-bindgen", "dep:wasm-bindgen-futures", "dep:js-sys", "dep:web-sys",
    "dep:reqwest",  # request wasm backend
]
```

- **native**：`reqwest` + `rustls-tls` + `tokio`（已有 std async）。
- **wasm**：`reqwest` 的 `wasm-bindgen` backend，等价于 `fetch(..., credentials: include)`，不依赖 tokio。
- **验证**：`cargo check`、`cargo check --no-default-features`、`cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown`。

### 2.2 session 存储策略

- **native**：本地文件系统。session manifest 为 JSON 文件，session 内容为目录树（`.spg`/`.tbl`）。
- **wasm**：不实现 IndexedDB 真实持久化（里程碑约束）。session 为内存态，页面刷新后丢失；仅用于演示和测试。
- **graphdb**：native session 仍使用 redb；wasm session 使用 memory graph store。

### 2.3 远程同步复用 scanner

- 远程同步结果写入 session 目录后，直接复用 `ProjectIndexer::scan(session_dir, db_path)`。
- 新增 `RemoteSessionProvider`：负责远程 list/download，本地写入 session 目录。
- 增量逻辑在 `RemoteSessionProvider` 层完成（对比远程 etag/version 与本地 manifest），scanner 层只负责本地 diff。

---

## 3. 任务分解与文件规划

### Phase 1：基础设施（M41.1 + M41.2）

| 任务 | 文件 | 说明 |
|---|---|---|
| 调整 Cargo.toml | `Cargo.toml` | 引入 reqwest（optional, default-features=false），cli-local 和 browser-wasm 分别启用 |
| 验证编译 | — | `cargo check`、`cargo check --no-default-features`、`cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown` |
| 扩展 remote_metadata contract | `src/remote_metadata.rs` | 确认字段完整；新增 `RemoteProjectInfo`、`RemoteMetafileEntry`；URL 构造保留在 Rust 中 |
| 实现 `ReqwestRemoteMetadataProvider` | `src/remote_metadata_provider.rs`（新建） | 统一使用 reqwest；构造失败必须返回错误，不得在库代码中 panic |
| 移除 `web_sys::window().fetch` 主实现 | `src/remote_metadata.rs` | `WasmFetchMetadataProvider` 仅作为兼容 wrapper 委托 reqwest backend |

### Phase 2：WASM 远程 provider（M41.3）

| 任务 | 文件 | 说明 |
|---|---|---|
| 暴露远端 WASM API | `src/remote_metadata_provider.rs`（`wasm_bindings`） | 通过 wasm-bindgen 暴露 `fetchRemoteFileInfo`、`fetchRemoteFileContent`、`loadRemoteSuperpageDocument` |
| 统一 envelope | `src/browser.rs` | 新增远程 fetch 结果 envelope，不抛裸 JS Error |
| 覆盖错误场景 | `src/remote_metadata_provider.rs` | 401/403/404/CORS/network/invalid JSON/empty body |

### Phase 3：Service Worker message API（M41.4）

状态：已落地。`browser/service-worker/metadata-checker-sw.js` 已新增远程元数据 message method，并通过 runtime launcher contract 暴露给 Controller；SW JS 只做 WASM lifecycle/message routing，不新增 BI URL 拼接。

| 任务 | 文件 | 说明 |
|---|---|---|
| SW 新增 message method | `browser/service-worker/metadata-checker-sw.js` | `fetchRemoteFileInfo`、`fetchRemoteFileContent`、`loadRemoteSuperpageDocument` |
| SW 调用 WASM | `metadata_checker.js` + `metadata_checker_bg.wasm` | SW 通过 `self.wasm_bindgen` 调用 `fetchRemoteFileInfo`、`fetchRemoteFileContent`、`loadRemoteSuperpageDocument`，要求 glue 与 wasm 产物同目录发布 |
| lazy init 后 fetch | — | WASM runtime 未初始化时先 init，再 fetch |
| fallback diagnostic | — | WASM 初始化失败时返回稳定错误码，允许 page fallback |

### Phase 4：Controller 双模式（M41.5）

状态：已落地。Controller 默认优先 runtime/SW remote loader，失败时 fallback 到 page provider；selection payload 仍不携带 raw metadata。

| 任务 | 文件 | 说明 |
|---|---|---|
| page provider 模式 | `browser/integration/metadata-checker-controller.mjs` | 保留现有逻辑：页面 provider 拉 rawText → runtime.loadSuperpageDocument |
| runtime/SW provider 模式 | `browser/integration/metadata-checker-controller.mjs` | 新增：selection 只传 fileRef → runtime 自行远程加载 |
| 默认优先级 | — | 优先 runtime/SW provider；失败 fallback 到 page provider |
| selection payload 约束 | — | 仍禁止携带 `.spg` raw text 或完整 component JSON |

### Phase 5：Native Session 基座（M41.6 - M41.12）

状态：native session 主链路已落地。已完成 manifest schema、session manager、AuthContext/SecretStore、native remote session provider contract + in-memory/test provider、reqwest BI provider、远程内容写入 session project mirror、删除事件处理、失败不污染旧 session、session-aware build graph 辅助函数，以及 `--session-list/show/status/delete/refresh` 的 CLI 契约测试。2026-05-25 已用真实 BI 测试环境验证 `--session-refresh analyzer` 可登录、拉取远程元数据、写入 session mirror 并构建 graphdb。browser-wasm 的真实联调需按 `metadata_checker.js` 与 `metadata_checker_bg.wasm` 一并生成、上传并部署到同一路径，且 SW 调用对应 export。

| 任务 | 文件 | 说明 |
|---|---|---|
| session manifest schema | `src/session/manifest.rs`（新建目录） | `session_id`, `created_at`, `updated_at`, `remote_server`, `project_ref`, `project_name`, `source_origin`, `files[]`, `graph_db_path`, `schema_version` |
| session 创建/打开 | `src/session/manager.rs` | 从 remote_server + project 创建 session 目录；保持 `--project-dir` 行为不变 |
| remote session provider | `src/session/remote_provider.rs` | `list_projects`, `list_metafiles`, `fetch_metafile_info`, `fetch_metafile_content`, `fetch_changed_since` |
| 同步到 session | `src/session/sync.rs` | 远程 logical path → 本地 session 目录；局部模式（只同步 SPG+关联 TBL）/ 全量模式 |
| 增量同步 | `src/session/sync.rs` | manifest 记录 etag/version/hash/mtime/size；未变化不下载；删除文件标记并移除 |
| 认证边界 | `src/session/auth.rs` | `AuthProvider`、`SecretStore`、`AuthContext`（browser_session / cookie_jar / explicit_credentials / none） |
| session 管理命令 | `src/cli.rs` + `src/main.rs` | 已提供 `--session-list`, `--session-show`, `--session-refresh`, `--session-delete`, `--session-status`；`refresh` 已接入真实 provider/login/session sync/build graph |
| 脱敏策略 | — | token/secret 不进入 stdout/stderr/graphdb/manifest；日志脱敏 |

---

## 4. 新增文件清单

```
src/
  remote_metadata_provider.rs      # reqwest-based provider（native + wasm）
  session/
    mod.rs                          # 模块入口
    manifest.rs                     # SessionManifest schema + 读写
    manager.rs                      # session 创建/打开/删除/列表
    remote_provider.rs              # 远程元数据批量获取
    sync.rs                         # 远程 → 本地 session 同步（含增量）
    auth.rs                         # AuthProvider / SecretStore / AuthContext
  ...
browser/
  integration/
    metadata-checker-controller.mjs   # 修改：支持双模式
  service-worker/
    metadata-checker-sw.js          # 修改：新增远程 fetch message method
```

---

## 5. 验收标准

- [x] `cargo check` 通过（native）。
- [x] `cargo check --no-default-features` 通过。
- [x] `cargo check --no-default-features --features browser-wasm --target wasm32-unknown-unknown` 通过。
- [ ] `cargo tree` 证明 cli-local 不携带 wasm-only 依赖，browser-wasm 不携带 tokio/native-tls。
- [ ] browser tests 全量通过（`node --test browser/test/*.test.mjs`）。
- [ ] Rust tests 全量通过（`cargo test`）。
- [ ] 远程元数据 URL 构造、响应解析、错误码映射仅在 Rust 中实现，JS 不再新增第二套。
- [ ] browser-wasm 通过 reqwest/fetch 借用浏览器登录 session 获取远程元数据。
- [ ] Service Worker 可以通过 message 调用 WASM 远程 fetch，并返回统一 envelope。
- [ ] 本地可以从远程 server 拉取项目元数据并建立 session。（provider 和同步底座已有，CLI 登录/refresh 尚未接通）
- [x] session 同步后可用现有 `ProjectIndexer::scan` + `build_graph` + `query`。
- [ ] 增量同步不会全量重拉。（partial/delete/失败保留已有测试，真实远端 changed_since 尚未验收）
- [ ] token/secret 不进入 stdout/stderr/graphdb/manifest。
- [ ] 远程失败不破坏已有可用 session。
- [ ] 不引入 rexie/IndexedDB 真实持久化依赖。

---

## 6. 风险与缓解

| 风险 | 缓解措施 |
|---|---|
| reqwest wasm backend 编译失败或体积过大 | 先 `cargo check --target wasm32-unknown-unknown` 验证；若体积失控，回退到 `web_sys::fetch` 但封装在统一 provider trait 后 |
| reqwest 在 Service Worker 中无法获取浏览器 cookie | reqwest wasm backend 底层就是 `fetch`，`credentials: include` 由浏览器环境提供，不依赖 reqwest cookie jar |
| session 目录与现有 `--project-dir` 行为冲突 | session 目录放在独立路径（如 `~/.local/share/metadata-checker/sessions/`），`--project-dir` 保持原有语义 |
| 远程同步阻塞 CLI | 使用 async + tokio（native），WASM 使用浏览器 event loop |
| 认证 secret 泄露 | `SecretStore` trait 抽象，native 实现用 keyring / env var，wasm  stub 返回空；日志统一脱敏 |

---

## 7. 建议落地顺序

1. **Week 1**：Phase 1（Cargo.toml + reqwest provider + 编译验证）。
2. **Week 2**：Phase 2（WASM API 暴露）+ Phase 3（SW message API）。
3. **Week 3**：Phase 4（Controller 双模式）+ browser 测试更新。
4. **Week 4**：Phase 5（Native Session 基座：manifest + manager + auth）。
5. **Week 5**：Phase 5 续（Remote provider + Sync + CLI 命令）。
6. **Week 6**：验收（cargo test + node test + cargo tree + 文档更新）。
