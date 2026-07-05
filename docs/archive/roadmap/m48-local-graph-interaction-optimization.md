# M48：Local Graph Interaction Optimization

| milestone | M48 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M48：Local Graph Interaction Optimization

目标：在 M47 已证明 Pixi Local Graph 真实可见、稠密图可聚合的基础上，优化点图交互。M48 不重新定义 VisualGraph contract，不把 JS 变成解析/推理层；它只负责 renderer view model、交互状态、DOM marker、详情摘要和真实环境交互验收。

详细计划见 [m48-local-graph-interaction-optimization.md](../../milestones/browser/m48-local-graph-interaction-optimization.md)。

边界要求：

- 保留 M47 的 Pixi 主路径、2-hop VisualGraph、1-hop 突出、`36` 真实节点 / `90` 真实边 / 最多 `4` 个 aggregate node 默认上限；M48 可验证 `48` / `120` 的 balanced density 档位，但不得回到全量点云。
- Hover 只做探索增强，不改变设计器 selection。
- Click 锁定 node/edge/aggregate detail，不反向切换设计器当前组件。
- Click node/edge 后可以快速聚焦图内目标，但设计器 focus node 仍是语义锚点，不改变 selection、不触发重新分析。
- Wheel / trackpad zoom 只作用于 graph canvas，不能误滚动设计器页面。
- Aggregate 节点只解释隐藏关系规模和 bucket，不在小浮层展开 hidden nodes。
- 底栏仅 **Pin · Copy · Collapse**；不实现 Open detail（完整配置在设计器属性栏阅读）。
- **Copy** 导出当前可见子图文本（含 focus、nodes/edges 列表）；CDP 验收仍拦截 token/cookie/password。

任务清单：

- [ ] M48.0：Screenshot smoke 与密度档位闭环
  - `node --test browser/test/m47-pixi-graph-screenshot.test.mjs` 全部通过。
  - compact 档位检查：`<=36` 可见节点、`<=90` 可见边，且未恢复到点云全量（可见 < 全量）。
  - balanced 档位检查：`<=48` 可见节点、`<=120` 可见边，且可见 < 全量。
  - smoke 覆盖 zoomed + locked edge、aggregate、detail 裁剪与非环形布局标记检查。
  - 若 renderer marker 仍以 fixture/smoke 辅助形式存在，需明确记录依赖（Worker A/B）并跟踪切换到真实标记后的 re-baseline。

- [ ] M48.1：Hover / Lock 状态模型
  - 区分 `hoveredTarget` 与 `lockedTarget`。
  - 区分 `designerFocusNodeId` 与 `focusedGraphTarget`，图内聚焦不改变设计器 focus。
  - hover end 后恢复 locked detail 或 focus detail。
  - background click / `Esc` 清除 locked target。
  - 输出 `graph-hover-target`、`graph-locked-target`、highlight count、viewport target marker。

- [ ] M48.2：Pixi 视觉反馈
  - hover node/edge/aggregate 高亮邻接关系。
  - locked target 使用稳定描边或 halo。
  - click node/edge/aggregate 后对目标做轻量 viewport framing。
  - 默认只显示 focus 和 aggregate `+N`；hover/click 后显示目标短 label。
  - 视觉反馈不改变 layout，不撑开 panel。

- [ ] M48.3：Zoom / Pan 与密度档位
  - graph canvas 监听 wheel，按 pointer 位置缩放，建议范围 `0.75x - 2.4x`。
  - 支持轻量 drag pan，并提供 `0` 或 background double click 恢复默认视口。
  - 缩放和平移只改 renderer viewport state，不触发 analyze，不改变 VisualGraph。
  - 验证 compact `36/90` 与 balanced `48/120` 两档截图，不允许全量点云。

- [ ] M48.4：详情摘要与底栏 Copy
  - 统一 node / edge / aggregate detail summary。
  - 底栏仅 Pin · Copy · Collapse；移除 Open detail 按钮。
  - Copy 通过 `getVisibleGraphText()` 导出可见子图文本。
  - 长文本单行省略；完整配置在设计器属性栏阅读。

- [ ] M48.5：键盘与可访问性
  - 隐藏交互索引作为 keyboard/accessibility target。
  - `Tab` 顺序稳定：focus node -> 高优先 edge -> 1-hop node -> aggregate node。
  - `Enter` / `Space` 锁定 target，`Esc` 清除 locked target，`0` 恢复默认 zoom/framing。
  - detail 状态变化可被测试和辅助技术感知。

- [ ] M48.6：真实 BI 交互验收
  - 验证 hover node/edge/aggregate 后 marker 和截图变化。
  - 验证 click node/edge/aggregate 后 locked marker、detail kind、detail text。
  - 验证 wheel zoom 后 viewport scale marker 变化，且设计器页面没有被误滚动。
  - 验证 click edge/node 后 viewport target marker 正确，设计器 focus node marker 不变。
  - 验证 `copy_graph_text` 交互类别与三按钮底栏（pin/copy/toggle）。
  - 验证 Pin 后 selection 分析跳过（`graph-panel-pinned` marker）。
  - 保留 M47 硬性项：`renderer=pixi`、canvas 非空、`depth=2`、`visible_hop=1`、selection changed probe、尺寸约束。

验收标准：

- 用户能在真实 BI 小浮层中 hover/click node、edge、aggregate，并获得稳定可解释的视觉反馈和详情摘要。
- locked target 与 hover target 分离，快速移动鼠标不会丢失已锁定详情。
- 用户能在 graph canvas 内缩放/平移，并能点击 node/edge 后快速聚焦图内目标。
- 图内聚焦不改变设计器 focus node，不触发重新分析。
- `Open detail` 和 `Copy id` 按当前 locked target 工作且脱敏。
- 小浮层不新增 Refresh/Analyze，不展开 hidden nodes，不变成 dashboard。
- 自动化测试和真实环境 JSON+PNG 证明交互完整性。
