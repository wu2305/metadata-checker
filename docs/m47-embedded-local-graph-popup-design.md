# M47 设计器内嵌 Local Graph Popup 设计稿

## 定位

M47 的设计器内嵌 popup 是一个跟随当前选中组件自动更新的局部关系图辅助浮层。它不是浏览器扩展 popup，不承载插件全局控制台能力，也不展示 runtime 健康检查。

目标是在设计器右下角约九分之一页面空间内，帮助用户快速理解当前组件的上下游关系，并把复杂操作引导到完整 panel。

## 设计输入

- `ui-ux-pro-max` 设计系统建议偏向 developer tool 的 dark/code/run-green 语义，适合表达运行和关系状态。
- `ui-ux-pro-max` chart 建议表明关系数据适合 Network Graph，但紧凑场景必须提供文字摘要或邻接列表替代。
- `ui-ux-pro-max` UX 建议强调 hover 只能做增强，click/tap 必须是主要交互；错误状态需要 `aria-live` 或结构化状态。
- Obsidian Graph View 官方参考：https://obsidian.md/help/plugins/graph。参考其 Graph/Local Graph 语义：节点代表对象、连线代表关系；hover 高亮连接、click 打开对象；Local Graph 围绕 active note 展示关联节点，并通过 depth 控制邻接展开层级。

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
| Depth 1  Depth 2       Open detail        Copy id |
+--------------------------------------------------+
```

折叠态只保留一个小型入口：

```text
[ Graph ] 3 in / 5 out / 1 warn
```

## Graph 语义

中心节点永远是当前选中组件。

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
- 不依赖颜色作为唯一状态；边 label 或 tooltip 必须补充关系类型。

## 尺寸约束

浮层固定在设计器右下角：

- `position: fixed; right: 16px; bottom: 16px;`
- 宽度目标：`min(420px, 33vw)`。
- 高度目标：`min(320px, 33vh)`。
- 最小可用尺寸：`280px x 220px`。
- Graph 区域高度优先占 60%-70%。
- 内容超出时只在浮层内部滚动，不扩大浮层。
- 字号使用固定 token，不随 viewport 等比缩放。

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
- click 当前组件：打开 detail panel 并定位到当前组件。
- click 组件邻居：如果设计器支持则选中该组件；否则打开 detail panel 并过滤到该组件。
- click model/param/expression/event：打开关系详情，不直接切换设计器选中项。

边交互：

- hover 边：显示关系原因，例如 `exp`、`defaultValue`、`calcCondition`。
- click 边：固定关系说明，显示来源字段、目标字段和规则摘要。

控制交互：

- `Depth 1 / 2`：切换局部图深度。默认 1-hop。
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
  "depth": 1,
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

## DOM 与验收 marker

真实环境验收不能只看 console。内嵌 popup 至少提供：

- `data-metadata-checker-embedded-popup="mounted|collapsed|hidden"`
- `data-metadata-checker-analysis-status="idle|loading|ready|empty|warning|error"`
- `data-metadata-checker-focus-component="<sanitized-id>"`
- `data-metadata-checker-graph-depth="1|2"`
- `data-metadata-checker-graph-node-count="<number>"`
- `data-metadata-checker-graph-edge-count="<number>"`

涉及可见 UI 的验收需要截图确认浮层位于右下角、尺寸未超过九分之一页面、节点和文字无重叠。

## 实现归属

建议把设计器内嵌 popup 落在现有 browser renderer 边界内：

- `browser/renderer/graph-panel-host.mjs`：宿主、右下角定位、收起/固定、marker。
- `browser/renderer/graph-panel-renderer.mjs`：状态归一化、事件上报、renderer 编排。
- `browser/renderer/graph-dom.mjs`：DOM/SVG 渲染、hover/click/focus 样式。
- `browser/renderer/graph-layout.mjs`：局部图布局、节点/边截断。

不要在 `browser/extension-chromium/popup.html` 中实现这个设计；那里属于浏览器扩展 popup。

## 第一版验收标准

- 切换设计器组件后，popup 自动进入 loading 并更新到 ready/empty/error。
- 默认只显示当前组件 1-hop 局部图。
- hover 和 click 节点会高亮相关关系，不相关节点降透明。
- `Depth 1 / 2` 可以改变渲染深度，但节点数量超过上限时必须截断。
- `Pin` 后 selection changed 不覆盖当前图。
- `Open detail` 能发出结构化事件。
- `Collapse` 后浮层不遮挡设计器主体。
- 自动化测试覆盖 DOM marker、状态转换、hover/click 事件、depth 切换、pin/collapse。
- Playwright 截图验证右下角九分之一尺寸、无文字重叠、graph 可见。
