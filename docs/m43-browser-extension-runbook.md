# M43.7 / M43.9 Browser Extension 安装与真实验收 Runbook

适用范围：用于验证 M43 的真实接入目标（真实产物为 **Browser Extension + 固定 onInitDesigner bridge**），不覆盖 standalone reader，也不用于 TamperMonkey/userscript 验收。

关键前置结论：

- M43 的关键运行链路是：`custom.js onInitDesigner -> onInitDesigner bridge -> extension page script -> content bridge -> extension popup/diagnostic -> analyze request`。
- `custom.js` 不再承载 runtime launcher、WASM 装载、持久化、图渲染逻辑。
- `custom.js` 仅提供轻量、幂等、固定的 bridge stub。
- 内置浏览器（如编辑器自带 WebView）不能安装 extension，不能作为 M43 主要验收对象；只能用于 marker/console 可见性检查。
- Chrome/Edge 验收必须走 unpacked extension 加载与复核。
- 真实 BI 当前自定义脚本加载器按 AMD `exports` 对象合并脚本；固定 bridge 必须使用 `define(["require", "exports"], function (_require, exports) { ... })` 并同时导出 `exports.onInitDesigner` 与 `exports.CustomJS["*"/"spg"/"SuperPage"]`。

## 0. 文件与工具约定

本文默认使用当前仓库中的：

- `browser/bridge/metadata-checker-bridge.js`（协议定义）
- `browser/bridge/custom-bridge.js`（固定 `onInitDesigner` stub）
- `browser/tools/prepare-extension-package.mjs`（生成 unpacked extension 包）
- BI 自身的 `custom.js` 上传通道（例如 `remote-metadata-uploader.mjs`）

## 1. 安装/合并 `custom-bridge.js`

`custom-bridge.js` 不能覆盖业务原始逻辑，必须与原 `custom.js` 合并。

### 1.1 上传 bridge 资产

将下面文件放到同一 BI 项目 hook 目录（示例路径）：

- `/analyzer/public/hooks/metadata-checker-bridge.js`
- `/analyzer/public/hooks/custom-bridge.js`

建议使用现有远端上传工具：

```bash
node browser/tools/remote-metadata-uploader.mjs \
  --base-url 'https://<BI_HOST>' \
  --remote-path /analyzer/public/hooks/metadata-checker-bridge.js \
  --file /path/to/metadata-checker-bridge.js

node browser/tools/remote-metadata-uploader.mjs \
  --base-url 'https://<BI_HOST>' \
  --remote-path /analyzer/public/hooks/custom-bridge.js \
  --file /path/to/custom-bridge.js
```

> 凭证请通过环境变量传递，不在文档中明文写入。

### 1.2 合并到已有 `custom.js`（推荐可复用写法）

在原有 `custom.js` 的 `define` 或初始化逻辑里保留已有 `onInitDesigner` 行为，并在其中拼接 bridge，示例原则如下：

1. 保留原有 `onInitDesigner` 的返回值/副作用，不破坏业务逻辑。
2. 优先检查 `window.__metadata_checker_designer_bridge__` 是否已存在，避免重复注册。
3. 加载后只写 `custom.js` 级别的 bridge marker，不做运行时计算/渲染。
4. 对重复初始化做幂等（重复进入只更新设计器上下文，不重复 bind 监听）。

如果平台允许在 `custom.ts` 中引用 hook 模块，推荐把 bridge 作为独立 AMD 模块上传，例如：

```text
/{projectName}/public/hooks/metadata-checker-designer-hook.js
```

然后在现有 `custom.ts` 中使用具名导入并合并到已有 hook：

```ts
import { designerHook } from "/xiaoshouyi/public/hooks/metadata-checker-designer-hook";

const existingOnInitDesigner = CustomJS["*"]?.onInitDesigner;

CustomJS["*"] = {
  ...CustomJS["*"],
  onInitDesigner(designer, args) {
    existingOnInitDesigner?.(designer, args);
    return designerHook.onInitDesigner(designer, args);
  },
};
```

注意：不要覆盖业务已有 `custom.js`，不要把 bridge 逻辑复制到 `.spg` / `.tbl` 元数据中。

示意逻辑（仅结构示意，按项目 `custom.js` 风格适配）：

```js
let installed = false;
let bridgeOnInit = null;
const existingOnInitDesigner = null; // 在你的现有 custom.js 中按实际可执行上下文引用原有 onInitDesigner

function installBridgeIfNeeded() {
  if (installed) return;
  if (typeof bridgeOnInit !== "function") {
    // 若未预置 bridge loader，可在此动态加载 metadata-checker-bridge.js / custom-bridge.js
    // 并从模块导出中拿到 onInitDesigner 回调
  }
  installed = true;
}

function onInitDesigner(designer, args) {
  if (typeof existingOnInitDesigner === "function") {
    existingOnInitDesigner(designer, args);
  }
  if (!installed) {
    installBridgeIfNeeded();
  }
  // 保持原业务 custom.js 的后续逻辑不变
  if (typeof bridgeOnInit === "function") {
    return bridgeOnInit(designer, args);
  }
  return { installed };
}
```

## 2. 构建/加载 Extension（Chrome/Edge）

### 2.1 生成 unpacked 目录

```bash
node browser/tools/prepare-extension-package.mjs \
  --out_dir browser/artifacts/metadata-checker-extension-chromium \
  --version 0.1.0 \
  --host_match https://autocrm-test.xiaoshouyi.com/*
```

如果有 WASM 产物：

```bash
cargo build --release --no-default-features --features browser-wasm --target wasm32-unknown-unknown
wasm-bindgen --target web \
  --out-dir /private/tmp/metadata-checker-wasm-chromium \
  --out-name metadata_checker \
  target/wasm32-unknown-unknown/release/metadata_checker.wasm
node browser/tools/prepare-extension-package.mjs \
  --out_dir browser/artifacts/metadata-checker-extension-chromium \
  --version 0.1.0 \
  --host_match https://autocrm-test.xiaoshouyi.com/* \
  --wasm_bindgen_js /private/tmp/metadata-checker-wasm-chromium/metadata_checker.js \
  --wasm_file /private/tmp/metadata-checker-wasm-chromium/metadata_checker_bg.wasm
```

Chromium MV3 extension 的 background 是 module service worker，M45 起通过动态 `import(chrome.runtime.getURL("metadata_checker.js"))` 加载 WASM glue，因此这里必须使用 `wasm-bindgen --target web`。`--target no-modules` 仅用于 BI hook / `importScripts()` 形态，不要混用于 Chromium extension 包。

### 2.2 在 Chrome/Edge 安装

1. 打开 `chrome://extensions` / `edge://extensions`。
2. 开启「开发者模式」。
3. 点击「加载已解压的扩展程序」并选仓库内的 `browser/artifacts/metadata-checker-extension-chromium`。
4. 启用扩展，确认图标可见。
5. **不在内置浏览器中验证 extension 安装**；该步骤仅限用于浏览器内核为真但不支持外部扩展时的 marker/console 复核。

## 3. 打开真实 BI SuperPage Designer

示例（按真实项目实际可用路径替换）：

- `https://<BI_HOST>/analyzer/app/<SmokeApp>.app/<SmokePage>.spg?:edit=true`

打开页面后执行：

```js
function marker(name) {
  const key = `data-metadata-checker-${name}`;
  return document.querySelector(`[${key}]`)?.getAttribute(key) ?? null;
}

console.table({
  bridgeModule: marker("bridge-module"),
  bridge: marker("bridge"),
  bridgeProtocol: marker("bridge-protocol"),
  bridgeStatus: marker("bridge-status"),
  onInitDesigner: marker("on-init-designer"),
  lastEvent: marker("last-render-status"),
});
```

并监听 bridge ready 事件：

```js
window.__metadata_checker_last_ready_payload = null;
window.addEventListener("__metadata_checker_designer_ready__", (event) => {
  window.__metadata_checker_last_ready_payload = event.detail;
  console.log("bridge ready", event.detail);
});
```

期望：

- `marker("bridge-module")` 为 `loaded`
- `marker("bridge")` 为 `installed` 或 `updated`
- `marker("bridge-protocol")` 为 `m43-protocol-v1`
- `marker("bridge-status")` 为 `ready` 或 `updated`
- `window.__metadata_checker_designer_bridge__` 可见（若页面上下文可访问）
- ready 事件触发后，`event.detail.selection.selected_component_ids` 与 designer 当前选区一致

注意：M43 bridge marker 是隐藏节点属性（例如 `<span data-metadata-checker-bridge="installed">`），不是写在 `document.documentElement` 上。自动化验收必须使用 `document.querySelector("[data-metadata-checker-bridge]")` 读取。

## 4. 验证 extension popup / diagnostic / Analyze current selection

### 4.1 popup 验证

1. 在扩展图标上点击并打开 popup。
2. 重点确认：
   - Bridge 状态（应为 installed / ready）
   - 当前页面上下文（project / page / source path）
   - 当前 selection 摘要（selected count / active id）
   - 最近一次 diagnostic（若存在）

### 4.2 触发分析

1. 在 BI 页面确保已选中组件（非空选择）。
2. 在 popup 点击 `Analyze current selection`。
3. 复核：
   - popup 显示分析结果/错误码；
   - 页面侧如果有 analysis marker，`data-metadata-checker-last-render-status` 或等价字段为 `ready`；
   - 若 runtime 无法执行，扩展应返回稳定可读的 `diagnostic`（而不是静默失败）。

### 4.3 与 extension 交互的最小链路检查清单

- `custom.js onInitDesigner` 有调用并成功写入 bridge marker。
- `custom.js` 仅写轻量 payload；selection/page_context 不应包含 `raw_text` / `components` / `raw_component` 等大 payload。
- extension popup 能读取到 selection snapshot（至少显示 source path 与 selected ids）。
- 点击 `Analyze current selection` 至少能触发一次 request/诊断回执。

## 5. Safari（M43.9）兼容与预验收

- Safari 使用同一套 `extension-core`，不单独维护一套业务逻辑。
- Safari 打包流程分为：先生成 staging 包（或转档目录），再由 Xcode / Safari Web Extension 转换工具手工导入。
- **自动化脚本只要求能生成 staging 包，不自动启动 Safari 或执行自动点击回放。**

建议流程：

1. 使用同一份核心资源（bridge/core）生成 staging 目录。
2. 通过 `browser/extension-safari/manifest.template.json` 生成 Safari 兼容 manifest。
3. 在 macOS 使用 Xcode/Safari extension 工具链完成转换、网站访问许可、签名/加载（手工步骤）。
4. 在 Safari 中手工打开真实 BI 页面并复核 marker + popup 诊断。

## 6. 失败与记录

以下任何项失败都记录为阻塞：

- bridge marker 未出现或不一致；
- 无法触发 `__metadata_checker_designer_ready__`；
- popup 无法显示 page context / selection；
- `Analyze current selection` 不触发、或只出现不可复现错误且无稳定 diagnostic；
- 真实 BI 页面报错阻塞设计器本身；
- 内置浏览器误判为扩展已安装（这是环境限制，不作为 extension 验收依据）。

每次验收保留：

- 选页 URL
- 页面 marker 快照
- popup 关键字段截图
- 页面内 panel 展开态截图：必须能肉眼看到 panel 背景、边框/阴影、文本内容和 `Metadata` trigger；如果截图里只剩 trigger button，应判定为 UI 可见性未通过。
- console 与 extension 调试日志
- 如出现 unsupported，记录稳定 code 与触发操作
