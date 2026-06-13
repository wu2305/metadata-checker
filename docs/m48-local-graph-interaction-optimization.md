# M48 Local Graph 点图交互优化计划

## Summary

M48 承接 M47 已验收的 Pixi Local Graph 主路径：`renderer=pixi`、2-hop VisualGraph、1-hop 突出、稠密图聚合为 `+N` 节点。M48 不再重新证明图能显示，而是把右下角 Local Graph 从“可见”推进到“可探索”：用户能在很小的浮层里快速判断当前组件的关键链路、点/边含义、隐藏关系规模，并用明确交互进入完整详情。

M48 默认不修改 Rust/WASM VisualGraph contract。JS 仍只做 renderer view model、交互状态、DOM marker 和测试，不解析 `.spg/.tbl`，不建图，不从 raw metadata 推断业务含义。

## Product Goals

- Hover 能快速看清当前点/边的邻接关系，不改变设计器选中组件。
- Click 能锁定点/边详情，用户移动鼠标后详情不丢。
- 用户能用滚轮或触控板在图面内缩放细节，在不扩大面板的前提下查看更多局部结构。
- 用户点击某条边或某个节点后，图面快速突出该目标；设计器 focus 节点仍保留为中心语义锚点。
- Aggregate 节点不是死计数：能解释隐藏了什么、为什么被聚合、下一步去哪里看完整列表。
- 小浮层保持辅助性质，不扩成 graph dashboard。
- 真实 BI 验收能证明交互真的可用，而不是只看 marker。

## Non-Goals

- 不做完整 graph 搜索、全局过滤器、路径追踪 UI。
- 不在小浮层里展开全部 aggregate hidden nodes。
- 不新增 Refresh / Analyze / Sync 控件。
- 不让点击邻居节点反向切换设计器当前选中组件。
- 不把图内 click/zoom/pan 状态写回设计器 selection，也不触发新的 metadata analysis。
- 不在 JS 中推断字段级 evidence；没有 evidence 时继续显示 `EDGE_EVIDENCE_UNAVAILABLE`。
- 不把 raw metadata、完整 component JSON、token/cookie/password 放入 DOM、截图或复制内容。

## Interaction Model

### Hover

- Hover node：
  - 当前 node、直接相连 edge、直接邻居提升亮度。
  - 非相关 node/edge 降透明，但 focus node 保持可见。
  - 底部摘要临时显示 node kind、depth、neighbor count、priority summary。
  - 鼠标离开后恢复到 locked detail 或 focus detail。

- Hover edge：
  - 当前 edge 提亮，from/to node 提亮。
  - 摘要显示 kind、priority、direction、summary、evidence_status。
  - 无 evidence 时明确显示 `EDGE_EVIDENCE_UNAVAILABLE`。

- Hover aggregate：
  - aggregate node 与 aggregate edge 提亮。
  - 摘要显示 hidden node count、hidden edge count、bucket、priority summary。
  - 不展开隐藏节点。

### Click

- Click node：
  - 锁定 node detail。
  - 图面保留该 node 的邻接高亮。
  - 视口在 120-180ms 内轻微平移/缩放到该 node 附近，让用户看清该点的邻域。
  - 设计器 focus node 不改变，仍以稳定 halo 或中心锚点显示。
  - 不触发设计器 selection 切换。

- Click edge：
  - 锁定 edge detail。
  - 图面保留 from/to 端点和该 edge 高亮。
  - 视口在 120-180ms 内快速聚焦到该 edge 的中点和两端节点。
  - 设计器 focus node 不改变；若该 edge 不连接 focus node，focus node 仍保持弱化但可见。
  - detail 中显示 Rust/WASM 给出的 evidence；没有 evidence 时显示稳定 diagnostic。

- Click aggregate：
  - 锁定 aggregate detail。
  - 视口轻微靠近 aggregate node，但不展开隐藏节点。
  - 显示 `Aggregate · +N hidden · bucket · hidden_edges · priority summary`。
  - `Open detail` 后续打开完整 panel，并带上 aggregate bucket / focus node context。

- Click background：
  - 清除 locked detail，回到 focus detail。
  - 视口恢复到以设计器 focus node 为中心的默认 framing。
  - 不影响 pinned/collapsed 状态。

### Zoom And Pan

- Wheel / trackpad scroll：
  - 只在 graph canvas 内生效。
  - 默认以鼠标位置为缩放锚点，缩放范围建议 `0.75x - 2.4x`。
  - 缩放后不改变 VisualGraph、不触发分析、不改变设计器 selection。
  - 当页面自身也可滚动时，只有 pointer 位于 graph canvas 内才拦截 wheel。

- Pan：
  - 按住拖动 graph canvas 可轻量平移视口。
  - 面板外拖动或点击不劫持设计器交互。
  - `Esc` 或 background double click 可恢复默认视口。

- Viewport state：
  - 记录 `scale`、`translateX`、`translateY`、`focusedGraphTarget`。
  - selection changed 后重置 viewport；pinned 状态下保留当前 viewport。
  - zoom/pan 只属于 renderer state，不进入 Rust/WASM contract。

### Keyboard And Controls

- `Esc`：清除 locked detail。
- `0`：恢复默认 zoom/framing。
- `+` / `-`：键盘缩放。
- `Enter` / `Space`：对当前 keyboard-focused node/edge 执行 click。
- `Tab`：可访问地遍历可交互目标，顺序为 focus node、高优先 edge、1-hop node、aggregate node。
- `Open detail`：打开完整 inspector，并携带当前 locked target。
- `Copy id`：复制 locked node id；如果 locked target 是 edge，则复制 edge id；如果是 aggregate，则复制 aggregate bucket summary。

## Visual Rules

- Hover 高亮优先用亮度、透明度和线宽，不依赖单一颜色。
- Click locked 状态要和 hover 区分：locked 用稳定描边或 halo，hover 用临时亮度。
- 默认 label 只显示 focus 和 aggregate `+N`；hover/click 后才显示目标短 label。
- Aggregate node 必须保持低饱和、虚线/描边感，不能伪装成真实节点。
- Edge detail 摘要不能撑开 panel；长文本省略，完整内容进入 `Open detail`。
- 图面默认保持 `36` 个真实节点、`90` 条真实边、最多 `4` 个 aggregate node；M48 可以验证略高密度档位，例如 `48` 个真实节点、`120` 条真实边，但必须通过截图证明不回到全量点云。
- Zoom in 时允许显示更多短 label，但不能新增真实节点；节点密度由 viewGraph 上限决定，不由缩放动态拉取。

## DOM Markers And Events

新增或稳定以下 marker：

- `data-metadata-checker-graph-hover-target="<node|edge|aggregate|none>:<id>"`
- `data-metadata-checker-graph-locked-target="<node|edge|aggregate|none>:<id>"`
- `data-metadata-checker-graph-detail-kind="focus|node|edge|aggregate|empty|error"`
- `data-metadata-checker-graph-highlight-node-count="<number>"`
- `data-metadata-checker-graph-highlight-edge-count="<number>"`
- `data-metadata-checker-graph-open-detail-context="<redacted-context>"`
- `data-metadata-checker-graph-viewport-scale="<number>"`
- `data-metadata-checker-graph-viewport-target="<node|edge|aggregate|focus|none>:<id>"`
- `data-metadata-checker-graph-density-profile="compact|balanced"`

结构化事件：

- `metadata-checker:local-graph-hover`
- `metadata-checker:local-graph-hover-end`
- `metadata-checker:local-graph-lock`
- `metadata-checker:local-graph-clear-lock`
- `metadata-checker:local-graph-copy`（导出当前可见子图文本）
- `metadata-checker:local-graph-viewport-change`

事件 payload 只能包含脱敏 id、kind、bucket、priority、depth、count、diagnostic code，不包含 raw metadata 或完整组件 JSON。

## 实施验收清单（截图 smoke）

M48 截图/视觉验收不改动 M47 已验收定义，作为补充清单如下：

- `node --test browser/test/m47-pixi-graph-screenshot.test.mjs` 必过，包含:
  - popup 尺寸、挂载 marker 与非空 `canvas` 占位（沿用 M47 base）。
  - compact 档位：`<=36` 可见节点、`<=90` 可见边；`fullNodeCount/fullEdgeCount` 大于可见数量（允许聚合，不是全量点云）。
  - balanced 档位：`<=48` 可见节点、`<=120` 可见边；`fullNodeCount/fullEdgeCount` 大于可见数量（balanced 不回掩盖 full graph）。
  - zoomed/locked edge：存在 `graph-viewport-scale >= 1.5` 与 `graph-locked-target="edge:..."`；`graph-detail-kind=edge`。
  - aggregate visible：存在 aggregate node 片段，并写明 `+N hidden`。
  - detail 文案裁剪：`textOverflow/overflow/whiteSpace` 在 smoke 标记中检查，不撑破 panel。
  - layout shape 检查：不为 `ring`（通过 smoke marker `data-metadata-checker-visual-shape`）。
- 若 `data-metadata-checker-graph-*` 标记尚未在 Worker A/B renderer 中落地，当前测试使用 renderer 输出/DOM fixture 的 smoke helper 标记覆盖，验收记录应明确依赖该落地项后再切到“真实 marker 验证”。

## Task Breakdown

### M48.1 Hover/Lock State Model

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/test/pixi-local-graph-renderer.test.mjs`

任务：

- 在 renderer state 中区分 `hoveredTarget` 与 `lockedTarget`。
- Hover 不覆盖 locked detail；hover end 恢复 locked/focus detail。
- Click node/edge/aggregate 锁定 target，并写 marker。
- Click target 后设置 `focusedGraphTarget`，但不改变 `designerFocusNodeId`。
- Background click / `Esc` 清除 locked target。
- 高亮上下文以 viewGraph 为准，不能把 hidden nodes 强行加入小浮层。

验收：

- Node 测试覆盖 hover -> hoverEnd -> click -> hover other -> clear lock。
- Node 测试覆盖 designer focus node 与 graph locked target 同时存在。
- Aggregate lock detail 稳定，且不触发设计器 selection 切换 callback。

### M48.1.1 Viewport Zoom And Target Framing

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/test/pixi-local-graph-renderer.test.mjs`

任务：

- 监听 graph canvas `wheel` 事件，按 pointer 位置缩放 viewport。
- 支持 drag pan，并限制极端 translate，避免图完全拖出画布。
- Click node/edge/aggregate 后计算目标 bounds，快速 framing 到目标附近。
- `0`、background double click 或 clear lock 后恢复默认 viewport。
- 输出 viewport scale/target marker 与结构化 viewport-change event。

验收：

- Fake DOM/Pixi 测试覆盖 wheel zoom、drag pan、reset viewport。
- Click edge 后 viewport target 为该 edge，from/to 节点保持高亮。
- Zoom/pan 不触发 analyze callback、不改变 focus node。

### M48.2 Pixi Visual Feedback

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/test/m47-pixi-graph-screenshot.test.mjs` 或新增 M48 screenshot smoke

任务：

- Node/edge hover 高亮邻接关系。
- Locked node/edge/aggregate 使用稳定描边或 halo。
- Edge hover/click 增加线宽，但不改变 layout。
- Click target 后 graph viewport 聚焦该 target，同时保留设计器 focus node 的弱化锚点。
- Aggregate 使用虚线/描边视觉或等价低饱和编码。
- 默认只显示 focus 和 aggregate label；hover/click 显示目标短 label。

验收：

- Fake Pixi runtime 测试能观测 alpha/line width/radius/label 状态变化。
- Screenshot smoke 覆盖 zoomed/locked edge 状态。
- 截图 smoke 证明 hover/locked 状态不会导致文本溢出或 panel 撑开。

### M48.3 Detail Summary And Footer Actions

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/renderer/graph-panel-host.mjs`
- `browser/renderer/graph-dom.mjs`
- `browser/test/graph-panel-host.test.mjs`

任务：

- 统一 node / edge / aggregate detail summary 格式；底栏 edge detail 形如 `Edge · {priority} · {from -> to} · {evidence}`。
- 底栏仅 **Pin · Copy · Collapse** 三个按钮；**不实现 Open detail**（完整配置在设计器属性栏阅读）。
- **Copy** 导出当前可见子图文本（`focus:`、`visible:`、`nodes:`、`edges:` 段落）；不做二次脱敏，但 CDP 仍拦截 token/cookie/password。
- detail 文本单行省略；`graph-open-detail-context` marker 仅保留给 a11y/键盘，不对应 UI 按钮。

验收：

- DOM marker 覆盖 detail kind、locked target、copy text length。
- Host 测试证明仅 3 个 footer 按钮，Copy 走 `getVisibleGraphText()`。

### M48.4 Accessibility And Keyboard

写入范围：

- `browser/renderer/pixi-local-graph-renderer.mjs`
- `browser/renderer/graph-panel-host.mjs`
- `browser/test/*`

任务：

- 隐藏交互索引继续作为 keyboard/accessibility target。
- `Tab` 顺序稳定：focus node -> high-priority edges -> 1-hop nodes -> aggregate nodes。
- `Enter`/`Space` 锁定 target。
- `Esc` 清除 locked target。
- detail host 使用合适的 live region 或 marker 表达状态变化。

验收：

- Node DOM 测试覆盖 keyboard 操作。
- 不要求 canvas 内真实焦点绘制，但必须有可测试的 DOM target。

### M48.5 Real BI Acceptance

写入范围：

- `browser/tools/m47-real-bi-cdp-acceptance.mjs` 可继续复用并扩展为 M48 checks，或新增 `m48-real-bi-cdp-acceptance.mjs`
- `browser/test/m47-real-bi-cdp-acceptance.test.mjs` 或新增 M48 acceptance test

任务：

- 验证 hover node/edge/aggregate 后 marker 和截图变化。
- 验证 click node/edge/aggregate 后 locked marker、detail kind、detail text。
- 验证 wheel zoom 后 viewport scale marker 变化，且设计器页面没有被误滚动。
- 验证 click edge/node 后 viewport target marker 正确，设计器 focus node marker 不变。
- 验证 `copy_graph_text` 类别：复制文本含 `nodes:` 与 `edges:` 段落。
- 验证底栏仅 pin/copy/toggle 三个按钮（`actionButtonCount === 3`）。
- 保留 M47 硬性要求：`renderer=pixi`、canvas 非空、`depth=2`、`visible_hop=1`、尺寸约束、selection changed probe。

验收：

- 真实 BI 输出 JSON + PNG。
- JSON 明确列出 interaction categories：`hover_node`、`hover_edge`、`hover_aggregate`、`lock_node`、`lock_edge`、`lock_aggregate`、`viewport_zoom`、`viewport_focus_edge`、`copy_graph_text`。

## Test Plan

- JS renderer：
  - `node --test browser/test/pixi-local-graph-renderer.test.mjs`
- Host/DOM：
  - `node --test browser/test/graph-panel-host.test.mjs browser/test/graph-panel-renderer.test.mjs`
- Screenshot：
  - `node --test browser/test/m47-pixi-graph-screenshot.test.mjs`
- Acceptance script：
  - `node --test browser/test/m47-real-bi-cdp-acceptance.test.mjs`
- Real BI：
  - 重新打包 extension 后运行 Chrome for Testing + CDP 验收，保存 JSON 和 PNG。

## Completion Definition

M48 可验收必须同时满足：

- 用户能 hover/click node、edge、aggregate，并获得稳定可解释的视觉反馈和详情摘要。
- locked target 与 hover target 分离，交互状态不会互相覆盖。
- 用户能在 graph canvas 内缩放/平移，并能点击 node/edge 后快速聚焦该图内目标。
- 图内聚焦不改变设计器 focus node，不触发重新分析。
- 底栏 Pin toggle 可冻结自动分析；Copy 导出可见子图文本。
- 小浮层不展开隐藏节点、不新增 Refresh/Analyze、不变成 dashboard。
- 真实 BI 验收包含交互 JSON 和截图证据。
- JS 未越界解析或推理 metadata，Rust/WASM contract 未被 renderer-only 需求污染。
