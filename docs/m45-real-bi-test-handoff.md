# M45 Real BI Test Handoff

记录时间：2026-05-29

## 当前目标

闭合 M45 真实 BI 端到端验收：在真实 `autocrm-test.xiaoshouyi.com` 设计器页面中验证 extension 能安全完成 `selection -> token bootstrap -> SW session -> remote metadata fetch/index -> WASM analysis -> panel progress/diagnostic`。

真实环境验收不能用 Node 测试、本地构建或 marker 单点成功替代。必须保留真实页面状态、panel 展开态、关键 marker、后台进度和失败 diagnostic。

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

## 当前阻塞点

当前未闭合项是：真实页面当前 selection 的单文件分析失败原因还未知。

已知状态：

- reload `2d0f27a` 后，selection 不再停在 `waiting_for_metadata`。
- 已进入当前页处理：
  - `Background progress: 1/1`
  - `Background failed: 1`
  - `Cache misses: 1`
  - `Retry available: yes`
- 需要 reload 包含 `3688df1` 的扩展后，重新触发 selection，读取 panel 新增的 `First diagnostic` 和 `Diagnostic message`。

不要把当前状态写成 M45 验收完成。真实环境端到端仍未闭合。

## 下一轮恢复步骤

1. 确认 unpacked 包已是最新：

```bash
node browser/tools/prepare-extension-package.mjs \
  --out_dir browser/artifacts/metadata-checker-extension-chromium \
  --version 0.1.0 \
  --host_match 'https://autocrm-test.xiaoshouyi.com/*' \
  --wasm_bindgen_js /private/tmp/metadata-checker-wasm-m45/metadata_checker.js \
  --wasm_file /private/tmp/metadata-checker-wasm-m45/metadata_checker_bg.wasm
```

如果 `/private/tmp/metadata-checker-wasm-m45` 不存在，先重新生成：

```bash
cargo build --release --no-default-features --features browser-wasm --target wasm32-unknown-unknown
mkdir -p /private/tmp/metadata-checker-wasm-m45
wasm-bindgen --target no-modules \
  --out-dir /private/tmp/metadata-checker-wasm-m45 \
  --out-name metadata_checker \
  target/wasm32-unknown-unknown/release/metadata_checker.wasm
```

2. 在 `chrome://extensions` reload `browser/artifacts/metadata-checker-extension-chromium`。

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

7. 根据 `First diagnostic` 分流：

- `REMOTE_METADATA_NOT_FOUND` / `REMOTE_METADATA_FETCH_FAILED`：优先查 SW metadata content path 与真实 BI content API。
- `BACKGROUND_ANALYSIS_FAILED`：继续看 diagnostic message，区分 raw metadata 获取失败、WASM load/build 失败、selection analyze 失败。
- `BACKGROUND_ANALYSIS_RUNTIME_UNAVAILABLE` 或 WASM 相关 code：查 `metadata_checker.js` / `metadata_checker_bg.wasm` 是否被扩展成功加载。

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

