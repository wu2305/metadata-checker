# M45 Real BI Test Handoff

记录时间：2026-05-31

## 当前目标

闭合 M45 真实 BI 端到端验收：在真实 `autocrm-test.xiaoshouyi.com` 设计器页面中验证 extension 能安全完成 `selection -> token bootstrap -> SW session -> remote metadata fetch/index -> WASM analysis -> panel progress/diagnostic`。

真实环境验收不能用 Node 测试、本地构建、假 BI 服务或 marker 单点成功替代。必须保留真实页面状态、panel 展开态、关键 marker、后台进度和失败 diagnostic。

## 已验证进度

真实页面：

```text
https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/价审.app?:edit=true&:file=销售订单价格审批-信息补充.spg
```

已确认：

- Chromium unpacked extension 已从仓库内路径加载：
  `browser/artifacts/metadata-checker-extension-chromium`
- 真实页面标题为 `销售订单价格审批-信息补充.spg`，设计器能打开。
- extension content script、page script、runtime adapter 已注入。
- BI `custom.js` bridge 已加载：
  - `data-metadata-checker-bridge-module=loaded`
  - `data-metadata-checker-bridge-protocol=m43-protocol-v1`
  - `data-metadata-checker-on-init-designer=called`
- selection bridge 已安装：
  - `data-metadata-checker-selection-bridge=installed`
  - source path 为 `app/价审.app/demo/销售订单价格审批-信息补充.spg`
- content script 获取一次性 token 后，SW session bootstrap 成功：
  - `data-metadata-checker-extension-session=ready`
  - `data-metadata-checker-extension-token-source=content-script`
  - `data-metadata-checker-extension-session-diagnostic-code=` 为空
- 未在 DOM marker / console 中看到 token 明文。
- panel 已挂载在 shadow DOM，可展开，能显示当前 selection、后台进度、cache 统计和 retry 状态。
- 2026-05-31 已重新生成 Chromium unpacked extension 到 `browser/artifacts/metadata-checker-extension-chromium`，本次 WASM glue 使用 `wasm-bindgen --target web`，并通过 MV3 offscreen document 承载 WASM runtime。

2026-05-31 最终真实验收结果：

```text
extensionOrigin=chrome-extension://jmfmjedknganfgjhhnpeelfkpbdokbcl
page=销售订单价格审批-信息补充.spg
token_status=200
token_length=36
project_count=1
file_count=2177
analyzable_count=1300
current_file=app/价审.app/demo/销售订单价格审批-信息补充.spg
file_id=nl84gB4KWkMN4vHWEsBBRE
revision=1524
artifact_ready=true
artifact.status=ready
artifact.result_status=ready
artifact.target=canvas
artifact.item_kinds=component, visual_graph, dependencies, expressions, reads
background.processed=1
background.failed=0
offscreen target=chrome-extension://.../offscreen.html
service_worker version=0.1.13
```

运行中 SW 源码确认包含：

- `createOffscreenWasmAnalysisClient`
- `file_id: item.file_id ?? ""`
- `include_priority/include_conditions/include_dataflow`
- `analysisClient.initRuntime({ project_ref })`
- `requestText(...)` raw metadata 读取
- `tryCacheSet("visible-index|...")` 非阻塞 visible index cache 写入

## 已修复问题

### 1. SW 被回收后 selection 卡在 waiting_for_metadata

现象：

- 页面 DOM 上仍保留旧 `session=ready` marker。
- Chrome MV3 service worker 被回收后，新 selection 消息进入新 SW 实例，内存 session/index 丢失。
- panel 显示：
  - `Status: analyzing`
  - `Indexing: waiting_for_metadata`
  - `Retry available: yes`

修复：

- `browser/extension-chromium/content-script.js`
- selection 返回 `waiting_for_metadata` 或 `indexing_current_page` 且没有 artifact 时，content script 自动重新执行 token bootstrap，然后重放当前 selection。

提交：

```text
2d0f27a fix: rebootstrap M45 selection after worker reset
```

验证：

```bash
node --test browser/test/*.test.mjs
cargo check
```

### 2. panel 不显示具体失败码

现象：

- rebootstrap 后 selection 已进入后台队列。
- panel 显示：
  - `Background progress: 1/1`
  - `Background failed: 1`
  - `Diagnostics: 1`
- 但 panel 只显示 diagnostic 数量，不显示 code/message，无法判断失败点。

修复：

- `browser/extension-core/panel-host.js`
- panel 增加：
  - `First diagnostic: <code>`
  - `Diagnostic message: <message>`
- diagnostic message 做敏感字段脱敏。

提交：

```text
3688df1 fix: surface M45 panel diagnostics
281a4e9 docs: record M45 real retest blocker
```

验证：

```bash
node --test browser/test/panel-host-smoke.test.mjs browser/test/extension-panel-lifecycle-smoke.test.mjs browser/test/extension-popup-panel-smoke.test.mjs browser/test/m45-background-controller.test.mjs
```

### 3. 真实 BI `getPermissionInfo` 为 encoded string 时无法建立项目索引

现象：

- Playwright Chrome for Testing 真实登录后，`/api/auth/getAccessToken` 返回 200，`/api/me/whoami` 返回已登录用户。
- `/api/me/getPermissionInfo` 返回 200，但 body 不是 JSON，而是长度约 9246 的 encoded string。
- `/api/meta/services/getFileChildren/xiaoshouyi` 和 `/api/meta/services/getFileDescendant/xiaoshouyi/app` 返回真实目录 JSON，可定位当前页面：
  - `source_path=app/价审.app/demo/销售订单价格审批-信息补充.spg`
  - `file_id=nl84gB4KWkMN4vHWEsBBRE`
- 旧逻辑只从 permission JSON 推导 project，遇到 encoded string 时 project 列表为空，后台索引无法推进到真实 descendant 结果。

修复：

- `browser/extension-chromium/content-script.js`
- `browser/extension-chromium/background.js`
- content script 在 bootstrap payload 中传递当前页面 project name。
- SW 在 permission info 无法解析出 project 时，使用 page context 中的 project name 作为 fallback，再通过真实 `getFileChildren/getFileDescendant/getFileContent/<file_id>` 链路索引当前页。

回归测试：

- 新增脱敏 fixture：`browser/test/fixtures/m45-real-bi-index-shape.json`
- fixture 只保留真实 API 结构、目录字段、目标文件 id/revision，不包含 token、cookie、password 或 raw metadata。
- `browser/test/m45-background-controller.test.mjs` 覆盖两类场景：
  - permissionInfo 为 encoded string 时使用 page project fallback。
  - 按真实 BI descendant shape 命中 `getFileContent/nl84gB4KWkMN4vHWEsBBRE`，而不是退回 path 404。

验证：

```bash
node --test browser/test/m45-background-controller.test.mjs browser/test/extension-panel-lifecycle-smoke.test.mjs
```

### 4. MV3 Service Worker 中 WASM 编译被 CSP 阻止

现象：

- 真实 Chrome 中 SW 直接 `WebAssembly.instantiateStreaming()` 时失败：
  `script-src 'self'` 下没有 `wasm-eval` / `unsafe-eval`。
- extension page / offscreen document 可以在 `wasm-unsafe-eval` 下编译 WASM。

修复：

- SW 降级为 bootstrap/router/session/API 编排层。
- 新增 `browser/extension-chromium/offscreen.html` 和 `offscreen-runtime.js`。
- `createWasmAnalysisClient()` 在 MV3 extension 环境默认走 offscreen document。
- offscreen document import `metadata_checker.js`，lazy init `metadata_checker_bg.wasm`，通过 `chrome.runtime.sendMessage` 接收 SW 的 runtime method call。

### 5. 真实 WASM 调用缺少 Rust contract 必填字段

现象：

- `analyzeSuperpageSelection` 首次返回：
  - `INVALID_SELECTION: missing field file_id`
  - `INVALID_OPTIONS: missing field include_priority`
  - `RUNTIME_NOT_INITIALIZED`

修复：

- background 从 visible index 中补齐当前 selection 的 `file_id/revision`，传给 WASM 的 selection 仍只包含轻量字段，不携带 raw metadata 或完整组件 JSON。
- analysis options 显式传：
  - `include_priority=false`
  - `include_conditions=true`
  - `include_dataflow=false`
- analysis 前先调用 `initRuntime({ project_ref })`，client 内部缓存 init promise。

### 6. 真实 `getFileContent` 返回 raw `.spg` 文本

现象：

- 真实 `/api/meta/services/getFileContent/nl84gB4KWkMN4vHWEsBBRE` 返回 `application/octet-stream;charset=UTF-8`。
- body 是 7.25MB 的 `.spg` JSON 文本，不是 `{ raw_text: ... }` wrapper。
- 旧 `request()` 会尝试 JSON parse，导致传给 WASM 的 raw text 为空，后续 `DOCUMENT_NOT_FOUND`。

修复：

- metadata content path 改用专用 `requestText()`，保持 raw `.spg/.tbl` 原文。
- visible index cache 写入改为 `tryCacheSet`，避免测试中清 IndexedDB 时的 cache 写失败阻断真实 session/index 主链路。

## 复现步骤

1. 确认 unpacked 包已是最新：

```bash
node browser/tools/prepare-extension-package.mjs \
  --out_dir browser/artifacts/metadata-checker-extension-chromium \
  --version 0.1.13 \
  --host_match 'https://autocrm-test.xiaoshouyi.com/*' \
  --wasm_bindgen_js /private/tmp/metadata-checker-wasm-m45-web/metadata_checker.js \
  --wasm_file /private/tmp/metadata-checker-wasm-m45-web/metadata_checker_bg.wasm
```

如果 `/private/tmp/metadata-checker-wasm-m45-web` 不存在，先重新生成 Chromium extension 使用的 ESM glue：

```bash
cargo build --release --no-default-features --features browser-wasm --target wasm32-unknown-unknown
mkdir -p /private/tmp/metadata-checker-wasm-m45-web
wasm-bindgen --target web \
  --out-dir /private/tmp/metadata-checker-wasm-m45-web \
  --out-name metadata_checker \
  target/wasm32-unknown-unknown/release/metadata_checker.wasm
```

`--target no-modules` 只适用于 BI hook / `importScripts()` 场景；Chromium MV3 extension 当前通过 offscreen document import ESM glue，必须使用 `--target web`。

2. 在 `chrome://extensions` reload `browser/artifacts/metadata-checker-extension-chromium`。如果 Chrome profile 对同一路径的 unpacked extension 缓存旧 `background.js`，使用新的 versioned artifact 目录验证，例如 `browser/artifacts/metadata-checker-extension-chromium-0.1.13`。

3. 刷新真实 BI 页面：

```text
https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/价审.app?:edit=true&:file=销售订单价格审批-信息补充.spg
```

4. 等待设计器加载后，确认 marker：

```text
extension-content=loaded
extension-page-script=loaded
extension-runtime-adapter=loaded
bridge=installed/updated
bridge-protocol=m43-protocol-v1
on-init-designer=called
selection-bridge=installed
extension-session=ready
extension-token-source=content-script
```

5. 触发一次真实 selection 变化。

6. 展开 panel，读取并记录：

```text
Status
Background
Indexing
Background progress
Background failed
Last failed
First diagnostic
Diagnostic message
Cache hits/misses
Retry available
```

7. 若出现 `First diagnostic`，按 code 分流：

- `REMOTE_METADATA_NOT_FOUND` / `REMOTE_METADATA_FETCH_FAILED`：优先查 SW metadata content path 与真实 BI content API。
- `BACKGROUND_ANALYSIS_FAILED`：继续看 diagnostic message，区分 raw metadata 获取失败、WASM load/build 失败、selection analyze 失败。
- `BACKGROUND_ANALYSIS_RUNTIME_UNAVAILABLE` 或 WASM 相关 code：查 `metadata_checker.js` / `metadata_checker_bg.wasm` 是否被扩展成功加载。
- `INVALID_SELECTION`：检查 background 是否把 `file_id` 补入 WASM selection payload。
- `INVALID_OPTIONS`：检查 `include_priority/include_conditions/include_dataflow` 是否显式传入。
- `RUNTIME_NOT_INITIALIZED`：检查 background 是否先调用 `initRuntime`。
- `DOCUMENT_NOT_FOUND`：检查 `getFileContent` 是否以 raw text 原样传给 `loadSuperpageDocument`。

## 安全边界

- 不修改真实 `.spg` / `.tbl`。
- 不读取 cookie/localStorage/sessionStorage。
- 不记录 token/cookie/password。
- 不把 raw metadata 贴到日志或文档。
- 自动化验收只读取 DOM marker、panel 文本、截图和脱敏 console。

## 当前工作区注意事项

工作区仍有进入 M45 测试前就存在的两个 Rust 格式化变更，未纳入 M45 提交：

```text
src/session/reqwest_provider.rs
tests/session_reqwest_provider_tests.rs
```

不要误提交到 M45 browser 真实环境修复里，除非下一轮明确处理它们。
