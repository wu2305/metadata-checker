# M47 设计器内嵌 Local Graph Popup 设计稿

## 定位

M47 的设计器内嵌 popup 是一个跟随当前选中组件自动更新的局部关系图辅助浮层。它不是浏览器扩展 popup，不承载插件全局控制台能力，也不展示 runtime 健康检查。

目标是在设计器右下角约九分之一页面空间内，帮助用户快速理解当前组件的上下游关系，并把复杂操作引导到完整 panel。

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

WASM/Rust 输出当前组件 2-hop 关系，但 popup 默认视觉只突出 1-hop：1-hop 节点和边为正常权重，2-hop 节点和边保留位置但半透明虚化，用于提示还有外围关系，不提供 Depth 切换控件。

节点类型：

- `component`：设计器组件。
- `model`：数据模型或字段。
- `param`：页面参数、上下文参数。
- `expression`：表达式或计算规则。
- `event`：事件/action 关系。
- `collapsed`：被截断的更多节点。

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
- 被截断节点：中性灰。

形状语义：

- 组件：圆角矩形。
- 模型/参数：胶囊节点。
- 表达式：细边框节点。
- collapsed：虚线边框节点。

关系线：

- 默认边使用 60% opacity。
- hover/click 聚焦边提升为 100% opacity。
- 非相关节点降到 25%-35% opacity。
- 2-hop 节点和边默认降到 18%-28% opacity，且不抢占中心区域。
- 不依赖颜色作为唯一状态；边 label 或 tooltip 必须补充关系类型。

## 尺寸约束

浮层固定在设计器右下角：

- `position: fixed; right: 16px; bottom: 16px;`
- 宽度目标：`min(350px, 33vw)`，与低代码平台右侧属性栏 `230px-350px` 宽度范围对齐。
- 高度目标：`min(320px, 33vh)`。
- 最小可用尺寸：`280px x 220px`。
- Graph 区域高度优先占 60%-70%。
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

涉及可见 UI 的验收需要截图确认浮层位于右下角、尺寸未超过九分之一页面、节点和文字无重叠。

M47 必须做真实 BI 环境复测，至少覆盖：

- 设计器 selection changed 自动更新。
- 1-hop 高亮和 2-hop 半透明虚化。
- 点击邻居节点只显示详情，不切换设计器选中项。
- 点击边在有 evidence 时显示具体字段，在无 evidence 时降级显示边类型。
- 浮层在右侧属性栏展开和收起两种状态下均不遮挡关键设计器控件。

## 当前仍需验证

以下事项不是产品方向不确定，而是实现前必须用代码或真实环境证实：

- WASM/Rust 当前是否已经能按 `active_component_id` 输出 2-hop VisualGraph；如果不能，M47 需要补 Rust API。
- WASM/Rust 是否在 edge metadata 中提供字段级 evidence；如果不提供，M47 首版只展示边类型、方向和源/目标节点。
- 真实 BI 设计器 selection changed 事件在单选、多选、取消选择、页面切换场景下是否稳定。
- ECharts graph/canvas 在 extension content-script/页面注入场景下的加载、CSP 和 resize 行为是否稳定。
- 右侧属性栏展开时浮层最终锚点应在页面右下角，还是避让到属性栏左侧。

## 实现归属

建议把设计器内嵌 popup 落在现有 browser renderer 边界内：

- `browser/renderer/graph-panel-host.mjs`：宿主、右下角定位、收起/固定、marker。
- `browser/renderer/graph-panel-renderer.mjs`：状态归一化、事件上报、renderer 编排。
- `browser/renderer/graph-dom.mjs`：非核心图区域 DOM、详情面板、hover/click/focus 状态容器。
- `browser/renderer/graph-layout.mjs`：局部图布局、节点/边截断。
- 图主体优先使用 ECharts graph 或 canvas 渲染；不再把 DOM/SVG 关系图作为 M47 主渲染路径。

不要在 `browser/extension-chromium/popup.html` 中实现这个设计；那里属于浏览器扩展 popup。

## 第一版验收标准

- 切换设计器组件后，popup 自动进入 loading 并更新到 ready/empty/error。
- WASM/Rust 输出 2-hop 局部关系，popup 默认突出 1-hop，2-hop 半透明虚化。
- hover 和 click 节点会高亮相关关系，不相关节点降透明。
- 不提供 Depth 切换控件；节点数量超过上限时必须截断。
- `Pin` 后 selection changed 不覆盖当前图。
- `Open detail` 能发出结构化事件。
- `Collapse` 后浮层不遮挡设计器主体。
- 自动化测试覆盖 DOM marker、状态转换、hover/click 事件、1-hop/2-hop 视觉权重、pin/collapse。
- Playwright 截图和真实 BI 复测共同验证右下角九分之一尺寸、无文字重叠、graph 可见、不遮挡右侧属性栏关键控件。
