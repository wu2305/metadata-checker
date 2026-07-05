# M43：Browser Extension Distribution & onInitDesigner Bridge

| milestone | M43 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M43：Browser Extension Distribution & onInitDesigner Bridge

目标：把浏览器端能力从“覆盖项目级 `custom.js`”调整为“浏览器插件 + 固定 `onInitDesigner` bridge”。平台侧只安装一次极薄、长期稳定的 `onInitDesigner` 函数，用来把真实 Designer 对象转换成轻量 bridge；浏览器插件负责分发、版本、运行时加载、分析调用、状态展示和诊断。M43 暂不继续推进 standalone upload/paste `.spg/.tbl` reader。

核心判断：

- 主产物改为 Browser Extension，而不是 TamperMonkey/userscript。
- `custom.js` 不再作为 runtime launcher 主线；它只保留一个固定 bridge stub。
- bridge stub 不加载 WASM、不 fetch 元数据、不建图、不渲染图关系、不承载业务推理。
- 浏览器插件不能依赖内置浏览器完成安装验收；内置浏览器只用于页面 marker/console/真实 BI 行为检查。Extension 安装验收需要 Chrome/Edge 的 unpacked extension 或人工半自动流程。
- M43 暂不把图关系嵌入设计器页面；首轮只证明 extension 能通过 bridge 获取页面上下文与当前 selection，并能触发现有 runtime 分析链路。
- Safari 插件不另写一套业务逻辑；M43 必须从第一天开始按“共享 WebExtension core + 平台 packaging 差异”组织代码。
- Chrome/Edge 与 Safari 的差异只允许出现在 manifest、权限声明、打包脚本和 runbook，不允许复制 page bridge、content bridge 或 runtime message protocol。

推荐架构：

```text
BI custom.js / onInitDesigner
  -> window.__metadata_checker_designer_bridge__
  -> CustomEvent("__metadata_checker_designer_ready__")
  -> Extension injected page script
  -> content script message bridge
  -> extension runtime/background/offscreen
  -> existing browser runtime client / WASM
  -> popup/side panel/DOM marker diagnostic
```

预验证矩阵：

```text
Node fake page tests
  -> 验证 bridge protocol、page script、content bridge、message envelope

Static package tests
  -> 验证 manifest、web_accessible_resources、WASM binary copy、secret/path leak

Chromium unpacked extension
  -> 使用 Chrome/Edge 加载 dist/metadata-checker-extension-chromium
  -> 验证 fake harness 与真实 BI 页面

Safari Web Extension package
  -> 复用 extension-core
  -> 通过 Safari Web Extension 转换/包装流程生成 Safari 工程或包
  -> 人工启用后验证同一 bridge protocol
```

边界要求：

- 平台 bridge：
  - 只导出固定 `onInitDesigner(designer, args)`。
  - 保存 Designer 引用到闭包或 page context bridge，不暴露完整 raw metadata/component JSON。
  - 暴露轻量 API：`getPageContext()`、`getSelectionSnapshot()`、`getBridgeStatus()`。
  - 通过 `CustomEvent` 通知插件，不主动调用插件私有 API。
  - `onInitDesigner` 是页面侧检查插件是否已加载的稳定入口：检查 extension 注入 marker / page-script 状态，并写出 `data-metadata-checker-plugin-state` 与 `data-metadata-checker-plugin-ready`。
  - 可与业务已有 `custom.js` 手工合并；不得要求覆盖业务 monkey patch。
- Extension：
  - content script 默认隔离环境，不直接假设能访问 page context 对象。
  - 必须注入 page script，由 page script 读取 bridge，再通过 `window.postMessage` 或 DOM event 与 content script 通信。
  - background/offscreen/runtime 负责加载 WASM 或复用现有 runtime launcher。
  - popup/side panel 只显示状态、诊断、当前 selection 摘要和触发分析按钮。
  - 不把 BI 特化对象、raw `.spg/.tbl`、完整组件 JSON 传入 extension UI。
- Runtime：
  - 继续复用 M40-M42 的 Plugin Core、runtime client、remote provider、browser analysis envelope。
  - 不新增 JS parser/query 逻辑；Rust/WASM core 仍是分析能力来源。
  - 首轮可用现有 page/SW runtime path；若 extension background 无法注册目标站同源 SW，则改用 extension 自己的 runtime/offscreen worker。

建议目录：

```text
browser/bridge/
  custom-bridge.js
  metadata-checker-bridge.js

browser/extension-core/
  bridge-protocol.js
  page-script.js
  content-bridge.js
  runtime-adapter.js

browser/extension-chromium/
  manifest.json
  background.js
  popup.html
  popup.js

browser/extension-safari/
  manifest.template.json
  safari-notes.md

browser/tools/
  prepare-extension-package.mjs
  prepare-safari-extension-package.mjs
```

任务清单：

- [ ] M43.1：定义 Bridge Protocol v1
  - 文件建议：
    - `browser/bridge/metadata-checker-bridge.js`
    - `browser/test/bridge-protocol-smoke.test.mjs`
  - 明确事件名：
    - `__metadata_checker_designer_ready__`
    - `__metadata_checker_selection_requested__`
    - `__metadata_checker_selection_response__`
  - 明确全局 bridge 名：
    - `window.__metadata_checker_designer_bridge__`
  - 明确返回结构：
    - `protocol`
    - `page_context`
    - `selection`
    - `diagnostics`
  - 正例测试：
    - `onInitDesigner` 调用后写入 bridge。
    - ready 事件只包含轻量字段。
    - `getPageContext()` 返回项目内逻辑路径、页面类型、project/app/page 信息。
    - `getSelectionSnapshot()` 返回 M40.3 snake_case selection contract。
  - 反例测试：
    - 不返回 raw `.spg/.tbl` 文本。
    - 不返回完整 component JSON。
    - 无 selection 时返回 stable diagnostic。

- [ ] M43.2：固定 onInitDesigner bridge stub
  - 文件建议：
    - `browser/bridge/custom-bridge.js`
    - `browser/test/custom-bridge-smoke.test.mjs`
  - `custom-bridge.js` 使用真实 BI 可加载和可合并的 AMD `define(["require", "exports"], function (_require, exports) { ... })` 形态。
  - 必须同时导出 `exports.onInitDesigner` 与 `exports.CustomJS["*"/"spg"/"SuperPage"]`，避免只支持直接导出时在 `getCustomJS(...)` 合并路径中失效。
  - 只做 bridge 初始化和事件发送。
  - 不 import sidecar、不动态加载 WASM、不注册 Service Worker。
  - 对重复初始化幂等：已有 bridge 时更新 designer 引用和状态，不重复挂多个监听。
  - 写稳定 DOM marker：
    - `data-metadata-checker-bridge-module`
    - `data-metadata-checker-bridge`
    - `data-metadata-checker-bridge-protocol`
    - `data-metadata-checker-bridge-status`
    - `data-metadata-checker-on-init-designer`
  - marker 以隐藏节点属性形式写入，验收脚本必须使用 `document.querySelector("[data-metadata-checker-bridge]")` 读取，不要只读 `document.documentElement` 属性。

- [ ] M43.3：Extension manifest 与目录骨架
  - 文件建议：
    - `browser/extension-core/bridge-protocol.js`
    - `browser/extension-core/page-script.js`
    - `browser/extension-core/content-bridge.js`
    - `browser/extension-chromium/manifest.json`
    - `browser/extension-chromium/background.js` 或 `service-worker.js`
    - `browser/extension-chromium/popup.html`
    - `browser/extension-chromium/popup.js`
  - `manifest_version` 使用 MV3。
  - `host_permissions` 支持可配置域名，测试环境先覆盖 `https://autocrm-test.xiaoshouyi.com/*`。
  - content script 只负责注入 page script 和 message bridge。
  - 不在 content script 里直接访问 `window.__metadata_checker_designer_bridge__`。
  - popup 显示：
    - bridge detected / missing
    - current page context
    - current selection summary
    - last diagnostic

- [ ] M43.4：Page script 与 content script 双向通信
  - page script 监听 bridge ready 事件。
  - content script 可发送请求：
    - `getBridgeStatus`
    - `getPageContext`
    - `getSelectionSnapshot`
    - `analyzeCurrentSelection`
  - page script 返回结构化 response envelope。
  - 所有 message 带：
    - `protocol`
    - `request_id`
    - `type`
    - `payload`
    - `diagnostics`
  - 反例测试：
    - 非 metadata-checker 消息被忽略。
    - protocol 不匹配返回 stable diagnostic。
    - bridge missing 不抛异常。

- [ ] M43.5：Extension runtime 接入策略
  - 首轮不假设 extension 可以注册目标站同源 Service Worker。
  - 优先复用现有 runtime client contract：
    - background/offscreen 内加载 WASM。
    - 或 content script/page runtime 通过现有 page/SW runtime fallback。
  - 若无法在 extension worker 中直接使用 wasm-bindgen no-modules 产物，记录 diagnostic 并切 page runtime fallback。
  - 不复制 BI metadata URL 拼接、响应解析和 query 逻辑到 extension JS。

- [ ] M43.6：打包工具
  - 文件建议：
    - `browser/tools/prepare-extension-package.mjs`
    - `browser/tools/prepare-safari-extension-package.mjs`
    - `browser/test/prepare-extension-package-smoke.test.mjs`
  - 输入：
    - extension source
    - wasm-bindgen JS
    - wasm bytes
    - version
    - host match 配置
  - 输出：
    - unpacked extension 目录
    - zip 包
  - 验收：
    - `manifest.json` 存在且 JSON 可解析。
    - required files 全部存在。
    - `.wasm` 以二进制复制，不做 UTF-8 写入。
    - 不包含 repo 私有路径、密码、测试账号。
    - Chromium 包与 Safari staging 包复用同一份 `extension-core` 文件。
    - Safari staging 包不引入 Chrome-only API 的硬编码依赖；无法自动验证的 API 写入 `safari-notes.md`。

- [ ] M43.7：真实 BI 安装与验证 runbook
  - 文件建议：
    - `docs/m43-browser-extension-runbook.md`
  - 记录：
    - 如何上传/安装固定 `custom-bridge.js`。
    - 如何在 Chrome/Edge 加载 unpacked extension。
    - 如何打开真实 SuperPage 设计器。
    - 如何确认 bridge marker。
    - 如何确认 extension popup/diagnostic。
    - 如何触发 `Analyze current selection`。
    - 如何生成 Safari Web Extension staging 包，以及哪些步骤必须在 Xcode/Safari 中人工执行。
  - 明确内置浏览器不能安装 extension，只能复核页面侧 marker 和真实 BI 行为。

- [ ] M43.8：真实环境 smoke
  - 使用真实 BI 测试环境验证：
    - `custom-bridge.js` 被加载。
    - `onInitDesigner` 被调用。
    - extension 检测到 bridge ready。
    - extension 能读取 page context。
    - extension 能读取当前 selection snapshot。
    - extension 能触发一次分析请求或返回明确 unsupported diagnostic。
  - 不要求 M43 首轮完成正式图面板。
  - console 不得出现未捕获错误。

- [ ] M43.9：Safari Web Extension 兼容预验收
  - 不要求 M43 在 CI 中自动启动 Safari。
  - 必须证明：
    - Safari 包使用同一份 `extension-core`。
    - manifest/template 不包含 Chrome Web Store 专属字段。
    - runbook 描述 Safari 转换、启用、调试和限制。
    - 如果某个 API 只能 Chromium 使用，必须有 adapter 或 stable diagnostic。
  - 后续真实 Safari 验收以人工执行记录为准。

验收标准：

- Node 测试覆盖 bridge、extension message contract、package builder 正反例。
- Extension 目录可作为 unpacked extension 加载。
- Safari staging 包可生成，且与 Chromium 包共享 core。
- 平台侧固定 bridge 不覆盖业务 `custom.js`，可以被手工合并。
- 真实 BI 页面能证明 `onInitDesigner -> bridge -> extension` 主链路。
- 首轮不要求设计器内图关系嵌入，但必须保留后续接入 Graph renderer 的扩展点。

暂不实施：

- TamperMonkey/userscript 主线。
- standalone upload/paste reader。
- 正式 UI 设计。
- extension store 发布。
- 多平台自动适配。
- IndexedDB 真实持久化。
