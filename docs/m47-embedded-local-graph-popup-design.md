# M47 设计器内嵌 Local Graph Popup 设计稿

## 当前结论

M47 正式落地方向确定为：设计器内嵌 Local Graph popup 使用 Rust/WASM 输出 2-hop VisualGraph，浏览器侧在 Chrome extension content script 隔离执行世界中运行 PixiJS + `d3-force-3d` renderer。页面主上下文只保留设计器 selection bridge，不运行 Pixi renderer。

该决策来自 M47 spike 真实环境验证：

- 扩展内 ESM vendor bundle 可以被真实 BI 页面加载。
- 真实 BI 主页面上下文污染了 `Array.prototype.pushAll`，且该属性可枚举，会破坏 Pixi 8 内部 systems 遍历。
- Chrome isolated world 中没有该污染，同一 bundle 可正常渲染 `renderer=pixi`。
- 因此正式实现必须保持 renderer、vendor bundle、DOM host 编排在 content script / extension isolated world；page-script 只传轻量 selection message。

## 定位

M47 的设计器内嵌 popup 是一个跟随当前选中组件自动更新的局部关系图辅助浮层。它不是浏览器扩展 popup，不承载插件全局控制台能力，也不展示 runtime 健康检查。

目标是在设计器右下角约 `280px x 260px` 的辅助浮层内，帮助用户快速理解当前组件的上下游关系，并把复杂操作引导到完整 panel。

M47 不再把该能力推迟到后续里程碑：设计器内嵌 Local Graph、插件弹出页设置化归位、真实环境复测都属于 M47 的完成边界。

## 设计输入

- `ui-ux-pro-max` 设计系统建议偏向 developer tool 的 dark/code/run-green 语义，适合表达运行和关系状态。
- `ui-ux-pro-max` chart 建议表明关系数据适合 Network Graph，但紧凑场景必须提供文字摘要或邻接列表替代。
- `ui-ux-pro-max` UX 建议强调 hover 只能做增强，click/tap 必须是主要交互；错误状态需要 `aria-live` 或结构化状态。
- Obsidian Graph View 官方参考：https://obsidian.md/help/plugins/graph。参考其 Graph/Local Graph 语义：节点代表对象、连线代表关系；hover 高亮连接、click 打开对象；Local Graph 围绕 active note 展示关联节点，并通过 depth 控制邻接展开层级。
- 低代码平台设计器右侧属性栏宽度参考：
  - `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/app/superpage/superpagedsn.ts` 中 `right` panel 使用 `flexBasis: 230`、`minSize: 230`、`maxSize: 350`。
  - `/Users/wuhaocheng/Downloads/bi/com.succez.bi/web/static-file/fapp/dsn/fappdsn.ts` 中右侧属性栏同样使用 `flexBasis: 230`、`minSize: 230`、`maxSize: 350`，并注释“适合属性栏的最小宽度为230”。

## 明确不属于本 popup 的能力

以下能力属于浏览器扩展 popup 或完整 panel，不进入设计器内嵌 popup：

- Refresh / Sync remote metadata。
- 登录、session、runtime、WASM、offscreen、IndexedDB 健康检查。
- 后台索引开始、暂停、继续。
- 缓存统计、全局诊断、设置。
- 完整 graph 搜索、过滤、路径追踪。
- raw metadata、完整组件 JSON、token/cookie/password 等敏感内容。
- 手动 Analyze 按钮。分析由设计器 selection changed 事件自动触发。

## 用户任务

内嵌 popup 只服务当前组件上下文：

1. 看清当前选中的组件。
2. 看清当前组件依赖谁。
3. 看清谁依赖当前组件。
4. 识别当前组件附近是否存在异常关系。
5. 快速跳到完整 panel 查看细节。
6. 在不遮挡设计器工作的前提下收起或固定浮层。

## 信息架构

首屏固定包含四个区域：

```text
+--------------------------------------------------+
| Component name/type                    Pin Close |
| id: cmp_xxx               3 in / 5 out / 1 warn  |
+--------------------------------------------------+
|                                                  |
|        upstream nodes                            |
|             \                                    |
|              [ current component ] -> downstream |
|             /                                    |
|        model / param / expression                |
|                                                  |
+--------------------------------------------------+
| Pin                      Open detail      Copy id |
+--------------------------------------------------+
```

折叠态只保留一个小型入口：

```text
[ Graph ] 3 in / 5 out / 1 warn
```

## Graph 语义

中心节点永远是当前选中组件。

WASM/Rust 输出当前组件 2-hop 关系，但 popup 默认视觉只突出 1-hop：当前组件固定在视口中心附近，1-hop 高价值链路优先显示，2-hop 只作为低透明上下文。图面采用 Obsidian Local Graph 风格的自然 force 子图，不使用环形布局、不绘制全量点云、不提供 Depth 切换控件。

节点类型：

- `component`：设计器组件。
- `model`：数据模型或字段。
- `param`：页面参数、上下文参数。
- `expression`：表达式或计算规则。
- `event`：事件/action 关系。
- `aggregate`：渲染层聚合节点，只表示被隐藏节点计数，不回写 Rust/WASM graph contract。

边类型：

- `reads`：当前组件读取上游值。
- `writes`：当前组件写入或影响下游。
- `condition`：条件关系，例如 `calcCondition`。
- `action`：事件触发关系。
- `dataflow`：跨字段或模型的数据流。
- `warning`：异常或不确定关系。

## 视觉编码

使用低饱和开发工具风格，不做 marketing hero，不做装饰性渐变。

颜色语义：

- 当前组件：深色实心节点，最高视觉权重。
- 上游来源：蓝/青色边框。
- 下游影响：绿色边框。
- 条件/事件：紫或琥珀色边框。
- 异常关系：红/橙色边和 warning badge。
- 聚合节点：低饱和中性灰，显示 `+N`，不能伪装成真实元数据节点。

形状语义：

- 组件：圆角矩形。
- 模型/参数：胶囊节点。
- 表达式：细边框节点。
- collapsed：虚线边框节点。

关系线：

- 默认边使用 60% opacity。
- hover/click 聚焦边提升为 100% opacity。
- 非相关节点降到 25%-35% opacity。
- 2-hop 节点和边默认低透明，且不抢占中心区域。
- Pixi 主路径不直接把完整 force layout 点云画出来；真实图面只渲染高价值子图，默认最多 `36` 个真实节点和 `90` 条真实边。
- 稠密图优先绘制 focus、高优先 1-hop、普通 1-hop、高优先 2-hop；剩余节点聚合为最多 `4` 个 `+N` 节点。
- 聚合维度固定为 `hidden-1hop`、`hidden-2hop-filter-condition`、`hidden-2hop-source`、`hidden-2hop-other`。
- 聚合节点和聚合边只属于 renderer view model，不改变 `data-metadata-checker-graph-node-count` 和 `data-metadata-checker-graph-edge-count` 的完整 graph 计数。
- 不依赖颜色作为唯一状态；边 label 或 tooltip 必须补充关系类型。

## 尺寸约束

浮层固定在设计器右下角：

- `position: fixed; right: 16px; bottom: 16px;`
- 宽度上限：`min(280px, calc(100vw - 32px))`。
- 高度上限：`min(260px, calc(100vh - 32px))`。
- 该浮层只能作为辅助图关系 inspector，完整详情进入 `Open detail`。
- Graph 区域优先占用剩余主空间，详情摘要以图面底部单行 overlay 呈现，不能撑开浮层。
- 内容超出时只在浮层内部滚动，不扩大浮层。
- 字号使用固定 token，不随 viewport 等比缩放。
- 真实环境复测时必须确认该浮层是否遮挡右侧属性栏关键控件；如果遮挡，需要在右侧属性栏展开时锚定到属性栏左侧，而不是页面物理右边缘。

## 交互模型

自动更新：

```text
designer selection changed
-> glue emits lightweight selection
-> integration controller requests local graph analysis
-> renderer updates embedded popup
```

节点交互：

- hover 节点：高亮该节点相关边和邻居。
- click 节点：固定聚焦；再次点击取消。
- click 当前组件：显示当前组件详情。
- click 组件邻居：显示邻居节点详情，不反向切换设计器选中组件。
- click model/param/expression/event：显示关系详情，不直接切换设计器选中项。

边交互：

- hover 边：显示关系原因，例如 `exp`、`defaultValue`、`calcCondition`。
- click 边：固定关系说明。若 WASM 输出了具体字段 evidence，则显示来源字段、目标字段和规则摘要；若未输出，则只显示边类型、方向和稳定 diagnostic。

控制交互：

- `Pin`：固定当前图，不随 selection 改变。
- `Open detail`：进入完整 panel。
- `Collapse`：收起成小入口。
- `Copy id`：复制当前组件 id。

## 状态模型

```text
idle        未选中组件
loading     selection 已变化，正在分析
ready       有局部关系图
empty       当前组件无可展示关系
warning     有截断、未知引用或低置信度关系
error       分析失败，显示稳定 diagnostic code
pinned      用户固定当前图
collapsed   用户收起浮层
```

状态呈现：

- `loading`：轻量 skeleton，不显示旧图误导用户。
- `empty`：显示当前组件和空关系文案。
- `warning`：保留图，同时显示 warning count。
- `error`：显示 diagnostic code 和 `Open detail`。
- `pinned`：selection 变化时显示 “Pinned to previous component”。

## 数据契约

renderer 消费 Rust/WASM 输出的脱敏 VisualGraph envelope。JS 不解析 `.spg/.tbl`，不建图，不做业务推理。

最小字段：

```json
{
  "status": "ready",
  "target": "component:Button_1",
  "focus_node": "component:Button_1",
  "depth": 2,
  "visible_hop": 1,
  "nodes": [],
  "edges": [],
  "source_summary": {
    "total_nodes": 8,
    "total_edges": 9,
    "node_kinds": {},
    "edge_kinds": {}
  },
  "diagnostics": []
}
```

selection payload 仍保持轻量，只包含 source path、file id、revision、active component id 和少量 selected ids，不包含 raw metadata 或完整组件 JSON。

边的 detail 面板优先消费 WASM/Rust 输出的 evidence 字段。若当前 WASM API 暂不提供具体字段，JS 只展示已脱敏的边类型、方向、源节点、目标节点和 diagnostic，不在 JS 中补推理。

## DOM 与验收 marker

真实环境验收不能只看 console。内嵌 popup 至少提供：

- `data-metadata-checker-embedded-popup="mounted|collapsed|hidden"`
- `data-metadata-checker-analysis-status="idle|loading|ready|empty|warning|error"`
- `data-metadata-checker-focus-component="<sanitized-id>"`
- `data-metadata-checker-graph-depth="2"`
- `data-metadata-checker-graph-visible-hop="1"`
- `data-metadata-checker-graph-node-count="<number>"`
- `data-metadata-checker-graph-edge-count="<number>"`
- `data-metadata-checker-graph-visible-node-count="<number>"`
- `data-metadata-checker-graph-visible-edge-count="<number>"`
- `data-metadata-checker-graph-hidden-node-count="<number>"`
- `data-metadata-checker-graph-aggregate-node-count="<number>"`

涉及可见 UI 的验收需要截图确认浮层位于右下角、尺寸符合辅助 inspector 定位、节点和文字无重叠。

M47 必须做真实 BI 环境复测，至少覆盖：

- 设计器 selection changed 自动更新。
- 1-hop 高亮和 2-hop 半透明虚化。
- 稠密关系只显示可读子图，隐藏节点通过 `+N` 聚合节点表达。
- 点击邻居节点只显示详情，不切换设计器选中项。
- 点击边在有 evidence 时显示具体字段，在无 evidence 时降级显示边类型。
- 浮层在右侧属性栏展开和收起两种状态下均不遮挡关键设计器控件。

## 当前仍需验证

以下事项不是产品方向不确定，而是实现前必须用代码或真实环境证实：

- WASM/Rust 当前是否已经能按 `active_component_id` 输出 2-hop VisualGraph；如果不能，M47 需要补 Rust API。
- WASM/Rust 是否在 edge metadata 中提供字段级 evidence；如果不提供，M47 首版只展示边类型、方向和源/目标节点。
- 真实 BI 设计器 selection changed 事件在单选、多选、取消选择、页面切换场景下是否稳定。
- PixiJS/canvas/WebGL 在 extension content-script isolated world 中的加载、CSP 和 resize 行为是否稳定；page-script 主上下文只验证 selection bridge。
- 右侧属性栏展开时浮层最终锚点应在页面右下角，还是避让到属性栏左侧。

## 实现归属

建议把设计器内嵌 popup 落在现有 browser renderer 边界内：

- `browser/renderer/graph-panel-host.mjs`：宿主、右下角定位、收起/固定、marker。
- `browser/renderer/graph-panel-renderer.mjs`：状态归一化、事件上报、renderer 编排。
- `browser/renderer/graph-dom.mjs`：非核心图区域 DOM、详情面板、hover/click/focus 状态容器。
- `browser/renderer/force-local-graph-layout.mjs`：`d3-force-3d` 局部图布局、节点/边截断。
- 图主体优先使用 PixiJS canvas/WebGL renderer；DOM/canvas fallback 只作为稳定降级路径，不作为 M47 主渲染路径。

不要在 `browser/extension-chromium/popup.html` 中实现这个设计；那里属于浏览器扩展 popup。

## 正式落地任务清单

### M47.1 Rust/WASM Local Graph 契约

写入范围：

- `src/visualization/`
- `src/query/`
- `src/browser.rs`
- `tests/browser_wasm_bindgen_tests.rs`
- `tests/browser_runtime_api_tests.rs`
- `tests/fixtures/`

任务：

- 增加或确认 WASM/Rust API 能按 `source_path + active_component_id` 返回当前组件 2-hop `VisualGraph`。
- 输出字段保持脱敏，包含 `status`、`target`、`focus_node`、`depth=2`、`visible_hop=1`、`nodes`、`edges`、`diagnostics`。
- 节点必须包含 `id`、`kind`、`label`、`depth`、必要的脱敏摘要；不得输出完整 component JSON。
- 边必须包含 `id`、`from`、`to`、`kind`、`label`、`fromDepth`、`toDepth`；如果 Rust 已有字段级 evidence，则作为脱敏 `evidence` 附加。
- 如果 Rust 当前无法提供字段级 evidence，返回稳定 diagnostic，例如 `EDGE_EVIDENCE_UNAVAILABLE`，JS 不补推理。
- 没有关系时返回 `status=empty`，而不是报错。

验收：

- 覆盖正常 2-hop、无关系、未知 component、边 evidence 缺失四类测试。
- JS 测试 fixture 只能消费 Rust/WASM 风格 VisualGraph，不得另造业务推理字段。

### M47.2 Selection 到分析编排链路

写入范围：

- `browser/platform-glue/`
- `browser/integration/`
- `browser/extension-chromium/content-script.js`
- `browser/extension-core/page-script.js`
- `browser/test/*selection*`
- `browser/test/plugin-integration-smoke.test.mjs`

任务：

- page-script 只负责监听/包装真实设计器 selection，输出轻量 payload。
- selection payload 只包含 `source_path`、`file_id/revision`、`active_component_id`、少量 `selected_component_ids/types`、timestamp。
- content script / integration controller 负责 `selection -> runtime analyze local graph -> renderer`。
- 增加 selection changed 防抖与过期响应丢弃，避免快速切换组件时旧图覆盖新图。
- `Pin` 状态下 selection changed 只更新提示，不自动换图。
- 多选、取消选择、页面切换需要明确状态：`ready`、`idle`、`loading` 或 `error`。

验收：

- Node smoke 覆盖单选、多选、取消选择、快速连续 selection、pinned 状态。
- 真实 BI 验收必须证明 `onInitDesigner` 后 selection changed 能触发局部图更新。

### M47.3 Pixi Renderer 正式接入

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/renderer/force-local-graph-layout.mjs`
- `browser/renderer/graph-panel-host.mjs`
- `browser/renderer/graph-panel-renderer.mjs`
- `browser/spike-vendor/`
- `browser/tools/build-spike-vendor-bundle.mjs`
- `browser/tools/prepare-extension-package.mjs`
- `browser/test/pixi-local-graph-renderer.test.mjs`
- `browser/test/m47-pixi-graph-screenshot.test.mjs`

任务：

- 保留 `pixi.js` / `d3-force-3d` 为扩展内置 bundle，不用 CDN。
- Pixi bundle 必须由 `browser/spike-vendor/pixi-entry.mjs` 显式导入 side-effect 初始化入口。
- Renderer 只能在 content script / extension isolated world 中运行；不得在 page-script 主上下文运行。
- `d3-force-3d` 只负责布局，不承担解析、图查询、业务推理。
- 当前组件固定中心附近；1-hop 正常权重；2-hop 半透明虚化；不提供 depth 切换控件。
- Pixi 图面使用自然 force 子图，不使用环形布局或引导环。
- 默认最多绘制 `36` 个真实节点、`90` 条真实边；隐藏节点合并为最多 `4` 个 renderer-only aggregate node。
- full graph marker 保留 Rust/WASM 原始节点/边数量，visible/hidden/aggregate marker 单独记录渲染视图数量。
- 节点 hover 高亮邻接关系；节点 click 固定详情，不切换设计器选中组件。
- 边按类型着色；边 click 显示关系详情，字段级信息只显示 Rust/WASM evidence。
- Canvas 渲染失败时降级为稳定 DOM/canvas fallback，并写 `data-metadata-checker-local-graph-renderer="fallback"`。

验收：

- `pnpm run build:spike-vendor -- --out-dir <extension-out>/spike-vendor` 可重复生成 bundle。
- Node 测试覆盖 Pixi runtime 注入、fallback、节点/边事件、2-hop opacity。
- 真实环境 marker 必须显示 `data-metadata-checker-local-graph-renderer="pixi"`，除非测试明确验证 fallback。

### M47.4 内嵌 Popup UI 与交互

写入范围：

- `browser/renderer/graph-dom.mjs`
- `browser/renderer/graph-panel-host.mjs`
- `browser/renderer/graph-panel-renderer.mjs`
- `browser/test/graph-panel-host.test.mjs`
- `browser/test/m47-pixi-graph-screenshot.test.mjs`

任务：

- 浮层固定右下角，默认 `right: 16px; bottom: 16px; width: min(280px, calc(100vw - 32px)); height: min(260px, calc(100vh - 32px))`。
- UI 占用页面右下角辅助区域，不扩成 dashboard。
- Header 只展示状态点、`Graph` 和关系计数，避免长路径压缩图面。
- Graph 区域占主体 70% 左右，内部保持低密度自然网络，不把 200 节点/500 边全部画成点云。
- 底部动作只保留 icon-only 的 `Pin`、`Open detail`、`Copy id`、`Collapse`，完整说明放入 tooltip/aria-label。
- 邻居节点详情和边详情显示为图面底部单行摘要；完整 JSON、长 evidence 和路径追踪进入 `Open detail`。
- 右侧属性栏展开时必须评估遮挡；如遮挡关键控件，浮层锚定到属性栏左侧。
- 文本、按钮、节点 label 在 280px 宽度下不得溢出或互相遮挡。

验收：

- 截图验证右下角位置、尺寸、可见 graph surface、详情区文字不重叠。
- DOM marker 覆盖 mounted/collapsed/hidden、analysis status、focus、depth、visible hop、node/edge count。

### M47.5 浏览器扩展 Popup 设置化归位

写入范围：

- `browser/extension-chromium/popup.html`
- `browser/extension-chromium/popup.js`
- `browser/test/extension-popup-panel-smoke.test.mjs`

任务：

- 浏览器扩展 popup 作为设置/状态页，不承载设计器内嵌 Local Graph。
- 可以展示登录/session/runtime/offscreen/index 状态、refresh/sync 控制、diagnostic 和设置项。
- 不展示当前组件局部关系图，不放 Analyze 当前组件入口。
- 设计语言可以与内嵌 popup 一致，但信息密度和目的不同：插件 popup 是全局设置页，设计器 popup 是当前组件辅助浮层。

验收：

- popup smoke 测试证明全局状态字段仍可读、敏感字段不泄漏。
- 与内嵌 popup 的 DOM marker 和 CSS class 不冲突。

### M47.6 打包与扩展 artifact

写入范围：

- `browser/tools/prepare-extension-package.mjs`
- `browser/test/prepare-extension-package-smoke.test.mjs`
- `browser/package.json`
- `browser/pnpm-lock.yaml`
- `browser/pnpm-workspace.yaml`

任务：

- `prepare-extension-package` 必须支持 `--spike-renderer-artifacts` 与 `--spike-vendor-artifacts`。
- manifest 必须把 `spike-renderer/*.mjs`、`spike-renderer/*.js`、`spike-vendor/*.mjs`、`spike-vendor/*.js` 加入 `web_accessible_resources`。
- unpacked extension 必须位于 `browser/artifacts/` 下。
- 不提交 `browser/artifacts/`、`browser/node_modules/`、`.pnpm-store/`。
- 新增 npm 依赖必须说明必要性、运行环境、测试命令。

验收：

- 打包 smoke 测试检查 vendor bundle 被复制且 manifest 可访问。
- 真实 Chrome 只能加载 `browser/artifacts/metadata-checker-extension-chromium*` 下的 unpacked extension。

### M47.7 自动化测试矩阵

影响面测试优先级：

- `node --test browser/test/prepare-extension-package-smoke.test.mjs`
- `node --test browser/test/pixi-local-graph-renderer.test.mjs`
- `node --test browser/test/m47-pixi-graph-screenshot.test.mjs`
- `node --test browser/test/graph-panel-host.test.mjs`
- `node --test browser/test/plugin-integration-smoke.test.mjs`
- `node --test browser/test/extension-selection-bridge-smoke.test.mjs`
- Rust/WASM contract 改动时运行对应 `cargo test` 目标测试，不默认全量。

测试要求：

- Node fixture 只模拟 VisualGraph envelope 与轻量 selection，不模拟 BI 业务逻辑。
- SW / WASM 解析可用 mock fixture，但真实环境验证必须使用真实 BI 页面和真实 extension artifact。
- 对 UI 可见性不能只看 marker，必须补截图。

### M47.8 真实 BI 环境复测

复测入口：

- Chrome for Testing + unpacked extension。
- 真实 BI 页面：`autocrm-test.xiaoshouyi.com` 设计器页面。
- 登录态由用户完成，自动化脚本只读取 marker 和截图，不读取 cookie/token/session store。

必须覆盖：

- extension content、page-script、runtime-adapter、session、selection-bridge、onInitDesigner marker。
- selection changed 自动更新局部图。
- `renderer=pixi` 在 isolated world 成立。
- 主页面 `Array.prototype` 污染不影响正式 renderer。
- 1-hop 高亮、2-hop 半透明。
- 稠密图有 `+N` 聚合节点，截图中不得出现错误环形结构或全量点云。
- 邻居 click 只显示详情，不切换设计器选中项。
- 边 click 有 evidence 时显示字段；无 evidence 时显示边类型和 diagnostic。
- 右侧属性栏展开/收起时浮层不遮挡关键控件。
- collapse/pin/copy/open detail 基础交互。

### M47.8.1 最后验收命令（只读）

以下命令用于复现验收过程，均不在此处预设通过结果：

- `node browser/tools/m47-real-bi-cdp-acceptance.mjs --out-dir browser/artifacts/m47-real-bi-acceptance`
- `node --test browser/test/m47-real-bi-cdp-acceptance.test.mjs`
- `node --test browser/test/prepare-extension-package-smoke.test.mjs`
- `node --test browser/test/extension-selection-bridge-smoke.test.mjs`
- `node --test browser/test/plugin-integration-smoke.test.mjs`

执行后用上述脚本输出中的 `manifest_diff`、`performance_timings` 与 `failed_categories` 来判定是否触发 `renderer / canvas / geometry / selection / interaction / manifest / timing` 相关失败。

证据要求：

- JSON evidence 写入 `browser/artifacts/m47-real-bi-spike-evidence/` 或后续正式 `browser/artifacts/m47-real-bi-acceptance/`。
- 截图写入同目录。
- 记录 extension origin、service worker URL、页面 title/url、markers、geometry、renderer、node/edge count。

### M47.9 冷脸验收与修复循环

验收者只读不改，重点看：

- 落地方向是否偏离：是否把 renderer 放进 page-script 主上下文，是否把 Refresh/Analyze 放进内嵌 popup，是否让 JS 做解析/推理。
- 测试覆盖是否完整：Rust/WASM contract、browser integration、renderer interaction、packaging、真实环境截图。
- 命令与 artifact 是否可复现：pnpm install、vendor build、extension package、Chrome real-env script。
- UI 是否仍超出辅助浮层定位或遮挡右侧属性栏。
- 是否有敏感信息泄漏：raw metadata、component JSON、token/cookie/password。

每轮修复必须：

- 主线程拆成不重叠 worker 包。
- worker 只写自己的范围。
- 主线程复核 diff、运行影响面测试、提交。
- 复验到无 P0/P1/P2 阻塞项后才可标记 M47 可验收。

## M47 完成定义

M47 可验收必须同时满足：

- Rust/WASM 能输出当前组件 2-hop VisualGraph，字段不足时有稳定 diagnostic。
- 设计器 selection changed 自动驱动 Local Graph 更新。
- 内嵌 popup 默认显示当前组件高价值 1-hop，2-hop 半透明虚化，隐藏节点以聚合节点计数。
- Pixi renderer 在真实 BI Chrome isolated world 中可见且 marker 为 `pixi`。
- 浏览器扩展 popup 已回归设置/状态页，不混入当前组件分析能力。
- 所有必要 marker、截图、JSON evidence 齐全。
- 影响面自动化测试通过。
- 独立冷脸验收无 P0/P1/P2 阻塞项。

## M47 非目标

- 不实现完整 graph 搜索、过滤、路径追踪。
- 不提供 depth 切换。
- 不在 JS 中解析 `.spg/.tbl`。
- 不在 JS 中推断字段级 evidence。
- 不把完整 raw metadata、完整 component JSON 或敏感凭据放入 selection payload。
- 不把内嵌 popup 做成插件全局控制台。

## 基础体验验收标准

- 切换设计器组件后，popup 自动进入 loading 并更新到 ready/empty/error。
- WASM/Rust 输出 2-hop 局部关系，popup 默认突出 1-hop，2-hop 半透明虚化。
- hover 和 click 节点会高亮相关关系，不相关节点降透明。
- 不提供 Depth 切换控件；节点数量超过上限时必须截断。
- `Pin` 后 selection changed 不覆盖当前图。
- `Open detail` 能发出结构化事件。
- `Collapse` 后浮层不遮挡设计器主体。
- 自动化测试覆盖 DOM marker、状态转换、hover/click 事件、1-hop/2-hop 视觉权重、pin/collapse。
- Playwright 截图和真实 BI 复测共同验证右下角辅助浮层尺寸、无文字重叠、graph 可见、不遮挡右侧属性栏关键控件。
