# M47 PixiJS + d3-force-3d Spike Plan

## 目标

验证设计器内嵌 Local Graph popup 是否可以使用 PixiJS 渲染，并用 `d3-force-3d` 计算 2-hop 局部图布局。

本 spike 只证明技术可行性，不把完整 M47 一次性做完。

## 产品约束

- 当前组件为中心节点。
- Rust/WASM 输出 2-hop VisualGraph。
- UI 默认突出 1-hop，2-hop 半透明虚化。
- 不提供 Depth 切换。
- 点击邻居节点只显示详情，不切换设计器选中组件。
- 点击边显示关系详情；字段级 detail 只消费 WASM/Rust 提供的 edge evidence。
- 图主体使用 PixiJS canvas 渲染；DOM 负责外框、详情、marker 和可访问性文本。
- JS 不解析 `.spg/.tbl`，不建图，不做业务推理。

## Spike 交付物

1. 一个可独立运行的 PixiJS + `d3-force-3d` renderer 原型。
2. 一个 2-hop VisualGraph fixture。
3. Node smoke 测试覆盖布局 contract、节点/边样式权重、事件回调。
4. Playwright 或等价截图 harness，验证右下角浮层尺寸和 graph 可见。
5. extension 打包策略说明，确认依赖如何进入 `browser/artifacts/metadata-checker-extension-chromium`。

### Worker A 当前结论（M47-Pixi）

- `browser/` 下原本没有 npm/pnpm 配置，新增了最小 `browser/package.json` 作为 spike 依赖清单。
- `pixi.js` 与 `d3-force-3d@^3.0.6` 结论先评估为：
  - 优先采用「扩展内置 artifact」而不是 CDN；
  - 真实 Chrome 验证显示直接加载 raw `pixi.min.js` 会在 `Application.init()` 阶段失败，错误为 renderer 初始化类未注册；
  - 因此 spike 改为用 esbuild 从 `browser/spike-vendor/*-entry.mjs` 打包 ESM vendor bundle，显式导入 Pixi side-effect 初始化入口。
- 真实 BI 主页面上下文存在可枚举 `Array.prototype.pushAll`，会触发 Pixi 8 内部 `for...in` systems 遍历失败；在 Chrome 隔离执行世界中同一 bundle 可正常渲染为 `renderer=pixi`，因此正式集成必须保持 renderer 运行在 content script / extension isolated world，不要放到 page-script 主上下文执行。
- `prepare-extension-package` 增加 `--spike-renderer-artifacts` 和 `--spike-vendor-artifacts`，用于把 renderer glue 与预构建 vendor bundle 分别打入 `spike-renderer/`、`spike-vendor/`，并自动补充 `web_accessible_resources`。
- 当前最小可行环境是：
  - 运行环境：CI/开发机在有网络时执行 `pnpm install`，在扩展内测时使用 `pnpm exec node tools/prepare-extension-package.mjs ... --spike-renderer-artifacts <path>`。
  - vendor 构建命令：`pnpm run build:spike-vendor -- --out-dir <extension-out>/spike-vendor`。
  - 复测命令：`node --test browser/test/prepare-extension-package-smoke.test.mjs`，同时确认 `result.spike_renderer_artifacts`、`result.spike_vendor_artifacts` 与输出目录中的 `spike-renderer/*`、`spike-vendor/*` 文件。
  - `pnpm-workspace.yaml` 显式允许 `esbuild` 安装脚本，避免 pnpm 阻止平台二进制安装。

## Worker 拆分

### Worker A：依赖与打包方案

写入范围：

- `browser/package.json`
- `browser/pnpm-lock.yaml`（仅在联网环境生成完整解析树后提交）
- `browser/tools/prepare-extension-package.mjs`
- `browser/test/prepare-extension-package-smoke.test.mjs`
- `docs/m47-pixi-d3-force-spike-plan.md`

任务：

- 评估 `pixi.js` 和 `d3-force-3d` 作为 `browser/` 下 npm 依赖的最小引入方式。
- 明确是否需要 bundler。如果需要，说明必要性、运行环境和测试命令。
- 让 extension package 能携带 renderer 依赖或构建产物。
- 不改 Rust core，不改 renderer 业务实现。

### Worker B：PixiJS + d3-force-3d renderer 原型

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/renderer/force-local-graph-layout.mjs`
- `browser/test/fixtures/m47-local-graph-2hop.json`
- `browser/test/pixi-local-graph-renderer.test.mjs`

任务：

- 用 2-hop fixture 生成确定性布局。
- current component 固定中心。
- 1-hop 正常 opacity，2-hop 半透明 opacity。
- 提供 node click、edge click、hover/focus 的回调 contract。
- 不接真实 BI，不改 graph-panel-host。

### Worker C：Host 集成与截图 harness

写入范围：

- `browser/renderer/graph-panel-host.mjs`
- `browser/renderer/graph-panel-renderer.mjs`
- `browser/test/graph-panel-host.test.mjs`
- `browser/test/m47-pixi-graph-screenshot.test.mjs`
- `browser/test/fixtures/m47-local-graph-2hop.json`

任务：

- 评估现有 graph panel host 如何挂载 canvas renderer。
- 增加或验证 marker：embedded popup、analysis status、focus component、depth、visible hop、node count、edge count。
- 增加截图 harness 验证右下角浮层尺寸和 graph 非空。
- 不改 package/dependency 打包逻辑，不改 Rust core。

## 主线程集成检查

- workers 不能互相覆盖写入范围。
- 若同一 fixture 需要共同使用，由主线程最终合并。
- 每轮合并后运行影响面测试，不默认全量 `cargo test`。
- spike 成功后再决定是否把 M47 正式技术方案从 “ECharts/canvas” 改为 “PixiJS + d3-force-3d”。
