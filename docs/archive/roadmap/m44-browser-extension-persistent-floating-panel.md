# M44：Browser Extension Persistent Floating Panel

| milestone | M44 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M44：Browser Extension Persistent Floating Panel

目标：把 M43 已验证的 Browser Extension + `onInitDesigner` bridge 从“popup/diagnostic smoke”推进为“页面内常驻悬浮窗”。popup 只作为入口、状态页和设置页；真正持续显示分析结果、图关系、诊断和交互操作的 UI 由 content script 在当前 BI 页面内挂载，不依赖浏览器 toolbar popup 的生命周期。

核心判断：

- Chrome/Edge/Safari 的 extension popup 会在失焦、点击页面或切换 tab 时关闭，不能作为常驻工作面板。
- 页面内悬浮窗可以常驻，但会随页面刷新/SPA 重建而销毁，需要 content script 重新挂载。
- 悬浮窗属于 extension 接入层，不承载解析、建图、查询、业务推理；核心能力仍走 Rust/WASM runtime 与既有 analysis envelope。
- M44 不追求正式设计器内嵌图面板的完整体验，先证明“可常驻、可恢复、可与 bridge 通信、可显示分析结果/诊断”。
- UI 形态参考 eruda：页面右下角固定一个小型触发 button，初始只显示 button；点击 button 展开 panel，再点击 button 收起 panel。button 始终存在，panel 只做 show/hide。
- SuperPage selection 不通过 DOM click 猜测。源码确认 `onInitDesigner(designer, args)` 先拿到 Workbench 级 designer，SuperPage 选择状态最终进入 `SuperPageBuilder.selectComponents(...)`、`deselectComponents(...)`、`deselectAll(...)` 并统一触发 `doSelectedChange(selectIds, deselectIds)`。M44 应优先在 page script adapter 中定位当前 SuperPage builder，并 patch `doSelectedChange` 或相关选择方法，触发后用 `builder.getSelectedComponents()` 重新读取完整当前 selection。

推荐架构：

```text
BI onInitDesigner
  -> plugin-ready marker
  -> page script bridgeReady
  -> content script
  -> floating panel host (Shadow DOM or iframe)
  -> runtime/background/offscreen
  -> analysis envelope / visual graph summary
```

边界要求：

- popup：
  - 只做入口、设置、权限提示、最近状态摘要。
  - 不作为常驻分析结果 UI。
  - 可以提供按钮：打开/隐藏页面悬浮窗、重新检测 bridge、触发一次分析。
- content script panel：
  - 在页面 DOM 中创建固定定位容器。
  - 首选 Shadow DOM 隔离样式；如 BI 样式冲突严重，再评估 iframe。
  - 初始只挂载右下角 trigger button；panel 默认隐藏，不自动弹出。
  - trigger button 与 panel 必须幂等：重复注入、刷新、SPA 重入不能产生多个 button/panel。
  - panel 关闭后 selection 事件仍继续更新内部状态；再次打开时展示最新 selection。
  - 不读取 raw `.spg/.tbl`、不保存密码/token/cookie。
  - 只消费轻量 selection/page_context、analysis envelope、visual graph option。
  - 必须幂等挂载：重复 content script、SPA 切换、设计器重入不重复创建多个 panel。
- `onInitDesigner`：
  - 每次调用检查 extension 是否注入，写出 `plugin-state/plugin-ready`。
  - 不直接调用 `chrome.runtime`；只通过 DOM marker / page script 状态判断插件加载情况。
  - 插件未加载时仍保持 bridge 可用，后续 extension 注入后由 page script 补发 ready。
  - 只安装设计器 bridge / selection bridge，不负责渲染 panel，也不直接 fetch metadata、调 runtime 或读取 raw metadata。
- selection bridge：
  - patch 点优先级：
    - 首选 `SuperPageBuilder.doSelectedChange(selectIds, deselectIds)`，因为它是选择状态统一出口。
    - 如真实对象不可直接稳定定位，再 fallback patch `selectComponents`、`deselectComponents`、`deselectAll`。
  - 每次事件必须重新读取完整 selection，而不是只使用增量参数：
    - `builder.getSelectedComponents()`
    - `component.getId()`
    - `component.getType()`
    - 可选 `component.getName()`，但不得发送完整 component JSON。
  - selection payload 只允许轻量字段：
    - `source_path`
    - `selected_component_ids`
    - `selected_component_types`
    - `selected_count`
    - `selection_source`
    - `changed_at`
  - 高频 selection 变化需要 debounce，首轮建议 100-200ms。

任务清单：

- [x] M44.1：Floating Panel Host contract
  - 文件建议：
    - `browser/extension-core/panel-host.js`
    - `browser/test/panel-host-smoke.test.mjs`
  - 定义 API：
    - `mountPanel({ rootDocument, initialState })`
    - `unmountPanel()`
    - `updatePanel(envelope)`
    - `setPanelStatus(status)`
    - `togglePanel(forceVisible?)`
    - `updateSelection(selection)`
  - 写稳定 DOM marker：
    - `data-metadata-checker-panel="mounted|hidden|error"`
    - `data-metadata-checker-panel-trigger="mounted"`
    - `data-metadata-checker-panel-source-path`
    - `data-metadata-checker-panel-selection-count`
    - `data-metadata-checker-panel-last-status`
  - 正反例测试：
    - 重复 mount 不创建多个节点。
    - 初始只显示 trigger button，panel 处于 hidden。
    - trigger button 点击后 panel show/hide 状态切换。
    - panel hidden 时 selection 更新仍进入状态，重新打开后展示最新 selection。
    - 没有 `document.body` 时返回 stable diagnostic。
    - 不泄漏 raw metadata / raw component JSON。

- [x] M44.2：SuperPage Selection Bridge
  - 文件建议：
    - `browser/extension-core/page-script.js`
    - `browser/test/extension-selection-bridge-smoke.test.mjs`
  - 通过 `onInitDesigner` / page script 中保存的 designer context 定位当前 SuperPage builder。
  - patch `doSelectedChange` 或 fallback patch `selectComponents`、`deselectComponents`、`deselectAll`。
  - 触发后发送 `metadata-checker-selection-changed` page message，由 content script 转成 extension 内部状态。
  - 正反例测试：
    - patch 后调用 `selectComponents` 能发出当前完整 selection。
    - 调用 `deselectComponents` / `deselectAll` 后 selection 可更新到 canvas 或空状态。
    - 重复 patch 不重复包装方法、不重复发事件。
    - payload 不包含 raw metadata、raw component JSON、cookie、token、password。
    - 找不到 SuperPage builder 时返回 stable diagnostic，不抛未捕获异常。

- [x] M44.3：Content Script Panel Lifecycle
  - content script 负责：
    - 接收 popup/background 的 open/close/toggle panel 请求。
    - 在 bridge ready 后自动刷新 panel 状态。
    - 接收 page script selection changed message，更新 panel host。
    - 页面刷新或重新进入设计器后重新挂载。
  - 反例：
    - extension 未注入时 popup 显示 stable diagnostic。
    - bridge missing 时 panel 显示缺失诊断，不抛未捕获异常。

- [x] M44.4：Panel Rendering MVP
  - 首屏显示：
    - bridge 状态
    - source_path
    - selected component ids/count
    - last diagnostic
    - analyze 按钮
  - 分析结果显示：
    - summary / items / diagnostics
    - visual graph summary 或 ECharts option 的简化预览入口
  - 暂不要求完整图关系交互；先保证可持续展示和更新。

- [x] M44.5：Popup 与 Panel 协作
  - popup 不再承载主结果 UI。
  - popup 按钮：
    - Open panel
    - Hide panel
    - Refresh bridge
    - Analyze current selection
  - popup 通过 `chrome.tabs.sendMessage` 与 content script 通信。
  - 不使用 `chrome.scripting.executeScript` 直接读取页面对象。

- [ ] M44.6：真实 BI 验收（验收步骤 / 待执行真实环境验证）
  - 在 `https://autocrm-test.xiaoshouyi.com` 验证：
  - 状态：本项为真实环境待执行闭环，单元测试、Node 测试通过不等同闭环完成。
    - `onInitDesigner` 写出 `plugin-ready=true`。
    - extension content script 注入成功。
    - 页面右下角固定 trigger button 存在，初始 panel 隐藏。
    - 点击 trigger button 后 panel 展开，再次点击后 panel 收起。
    - 点击 trigger button 后必须截图验证展开态：截图中应能看到明确的 panel 背景、边框/阴影、文本内容和 `Metadata` trigger，不允许只凭 DOM marker 判定 UI 可见。
    - panel 挂载后点击页面不会关闭。
    - 切换组件后 panel 可刷新 selection。
    - panel 收起时切换组件，再展开后显示最新 selection。
    - 点击 Analyze 后 panel 显示结果或 stable diagnostic。
    - 刷新页面后 panel 可恢复或明确显示 idle 状态。

验收标准：

- Node 测试覆盖 panel host、content script lifecycle、popup-panel message contract。
- 真实 BI 页面能看到常驻 panel marker。
- 真实 BI 页面必须保留 panel 展开态截图；截图证据优先于“只看到 marker”的验收结论。
- 页面点击、设计器内交互不会关闭 panel。
- popup 关闭不影响 panel 常驻。
- panel 不承载核心解析/查询逻辑，不复制 Rust/WASM 能力。
