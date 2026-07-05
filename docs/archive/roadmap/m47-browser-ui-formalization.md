# M47：Browser UI Formalization

| milestone | M47 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M47：Browser UI Formalization

目标：一次性完成浏览器端正式 UI 的两个产品面：设计器内嵌 Local Graph popup 与浏览器插件弹出设置页。内嵌 popup 服务当前组件关系，插件弹出页服务全局设置、同步、诊断与运行状态。两者共享设计语言，但职责不能混用。

设计器内嵌 Local Graph popup 详见 [m47-embedded-local-graph-popup-design.md](../../milestones/browser/m47-embedded-local-graph-popup-design.md)。

边界要求：

- JS 仍只做 glue/provider/runtime launcher/renderer/test harness，不解析 `.spg/.tbl`，不建图，不做业务推理。
- 内嵌 popup 不提供 Refresh、手动 Analyze、后台索引控制、session/runtime/offscreen/IndexedDB 健康面板。
- 插件弹出页更接近设置页，承载 Refresh/Sync、session 状态、runtime/offscreen/IndexedDB 状态、后台索引入口、诊断和设置。
- 内嵌 popup 展示当前组件局部关系：WASM/Rust 输出 2-hop，视觉默认突出 1-hop，2-hop 半透明虚化，不提供 Depth 切换。
- 点击邻居节点只显示邻居详情，不反向切换设计器选中组件。
- 边类型可以通过颜色区分；字段级 detail 只消费 WASM/Rust 输出的 edge evidence，JS 不补推理。
- 图主体优先使用 ECharts graph 或 canvas；DOM 只负责容器、详情、控制和可访问性文本。
- 不读取或展示 token/cookie/password/raw metadata/完整 component JSON。
- 可见 UI 真实验收必须截图，不允许只凭 marker 判定可见性。

任务清单：

- [ ] M47.1：Local Graph 数据 contract
  - Rust/WASM 支持按 `active_component_id` 输出当前组件 2-hop VisualGraph。
  - VisualGraph 标记 `focus_node`、`depth=2`、`visible_hop=1`、node hop、edge type 和脱敏 metadata。
  - edge evidence 若可得，输出具体字段、来源属性、目标属性和规则摘要；若不可得，返回稳定降级字段。
  - selection payload 继续保持轻量，不包含 raw text 或完整 component JSON。

- [ ] M47.2：内嵌 popup host 与尺寸策略
  - 固定右下角，尺寸目标不超过九分之一页面。
  - 参考低代码平台右侧属性栏 `230px-350px` 宽度，默认 `min(350px, 33vw)`。
  - 真实环境测试右侧属性栏展开/收起时是否遮挡关键控件；必要时锚定到属性栏左侧。
  - 提供 `mounted/collapsed/hidden`、analysis status、focus component、node/edge count、graph depth/visible hop marker。

- [ ] M47.3：ECharts/canvas Local Graph 渲染
  - 图主体使用 ECharts graph 或 canvas 渲染。
  - 中心节点为当前组件；1-hop 正常权重，2-hop 半透明虚化。
  - hover/click 节点高亮相关边和邻居，非相关节点降透明。
  - 点击节点显示详情，不切换设计器选中组件。
  - 点击边显示关系说明；有 evidence 显示字段级 detail，无 evidence 显示边类型和方向。

- [ ] M47.4：插件弹出页设置化
  - popup 从调试控制台改为设置/状态入口。
  - 保留全局能力：Refresh/Sync、session 状态、runtime/offscreen/IndexedDB 状态、后台索引入口、缓存/诊断、Open full panel、设置项。
  - 不展示当前组件 Local Graph，不放手动 Analyze 当前组件按钮。
  - 所有操作有 loading/disabled/error/success 状态和 stable diagnostic。

- [ ] M47.5：视觉设计与安全 UX
  - 内嵌 popup 与插件弹出页共享 token：spacing、font size、border、status color、focus ring。
  - 使用开发工具风格，紧凑、可扫描；不做 landing page、营销式 hero、装饰性大图。
  - 所有 UI 文本、tooltip、复制内容都通过敏感信息扫描。
  - 错误状态有 `aria-live` 或结构化状态，不只靠颜色。

- [ ] M47.6：自动化测试
  - Rust/WASM 测试覆盖 2-hop LocalGraph contract、edge type、edge evidence 降级。
  - Node/browser smoke 覆盖 host marker、状态转换、1-hop/2-hop 视觉权重、node/edge click、pin/collapse。
  - 插件 popup 测试覆盖设置页状态渲染、message contract、button command、diagnostic 脱敏。
  - 可见 UI 变更补 Playwright/Chrome 截图。

- [ ] M47.7：真实 BI 复测
  - 在真实 `autocrm-test.xiaoshouyi.com` 设计器中验证 selection changed 自动更新。
  - 验证 1-hop 高亮、2-hop 半透明虚化、节点详情、边详情。
  - 验证右侧属性栏展开/收起时浮层不遮挡关键控件。
  - 验证插件弹出页的 Refresh/Sync、session/runtime 状态、后台索引入口、诊断复制。
  - 保存截图和脱敏 evidence JSON，不用 marker 单点成功替代 UI 可见性验收。

验收标准：

- 设计器内嵌 popup 能在真实 BI 中自动跟随当前组件显示 2-hop LocalGraph，并默认突出 1-hop。
- 插件弹出页完成设置页归位，不再混入当前组件关系图。
- 自动化测试覆盖 contract、渲染、交互、脱敏、布局边界。
- 真实 BI 截图证明内嵌 popup 与插件弹出页均可见可用。
- 浏览器 JS 仍保持接入层边界，不承载 Rust core 的解析、建图、查询或业务推理。
