# M42：Browser Designer Graph Relationship Rendering

| milestone | M42 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M42：Browser Designer Graph Relationship Rendering

目标：废弃当前 MCP Adapter 主线，把浏览器中的图关系可视化和组件关系阅读能力前移到 BI SuperPage 设计器。M42 的价值是让用户在真实 BI Designer 中选中任意组件后，直接看到该组件的关联关系、条件、数据来源、写入来源和下一跳探索入口。当前不继续投入偏文本向的 MCP tool/resource 协议，也不实现上传/粘贴 `.spg/.tbl` 的 standalone reader。

背景纠偏：

- 当前不做 MCP。Zed MCP 只作为可选远期方向保留，不能占用 M42 主线。
- M42 不实现新的图查询语义；它消费已有 runtime / browser API / VisualGraph 中间模型。
- M42 不把关系渲染写进 Plugin Core。Plugin Core 仍只维护插件状态和事件。
- M42 不把 BI 特化对象直接传给 renderer。Designer Glue / Integration Controller 负责转换 selection 与 runtime result。
- M42 不渲染 raw `.spg/.tbl` 内容，不把完整 component JSON 放进 DOM。
- M42 首要目标是浏览器里“用上”：可见、可验证、可复现，而不是追求完整正式 UI。
- 真实 BI 接入只走 `custom.js` / `onInitDesigner` launcher。已放弃 extension spike：当前 BI extension 体系没有全局 Designer 插件扩展点证据，只能扩展组件、组件设计态、组件命令、组件右键菜单和交互动作。
- `dataVisualization` 可以注册 SuperPage 组件，但不作为 M42 的入口。M42 不要求用户拖入辅助组件，不污染业务元数据。
- BI 内已有 ECharts runtime。M42 不打包自己的 ECharts，不新增 npm 图表依赖；Browser renderer 应优先解析 BI AMD 模块，失败后降级 HTML/SVG renderer。
- 原 M43 不再作为独立里程碑，也不并入 M42。standalone browser reader 暂不规划，除非后续出现明确的人类离线阅读需求。

状态更新（截至 2026-05-25）：

- `M42.1` ~ `M42.6` 在仓库代码和本地测试链路上已完成落地。
- 2026-05-26 冷脸验收指出真实 smoke 前必须补齐：真实 `custom.js` sidecar factory 加载、WASM/Runtime `visual_graph` contract、真实 renderer+host DOM 绑定。上述阻塞已转入 M42.8 前置验收项。
- `M42.8`、`M42.9` 为真实 BI 验收与实测指标补全点，当前不代表已在真实环境通跑。

任务清单：

- [x] M42.1：确认并收敛 VisualGraph contract
  - 复核 M40.7 的 `VisualGraph` / Mermaid / ECharts option 能力是否已经可复用。
  - 若 M40.7 尚未完全落地，M42 只补 browser 必需的最小 contract：
    - `nodes`
    - `edges`
    - `focus_node`
    - `diagnostics`
    - `truncated`
    - `source_summary`
  - 明确 renderer 输入优先来自 `analyzeSuperpageSelection` 的 analysis envelope / query result。
  - 不新增第二套 browser-only graph schema；browser JS 只消费 Rust 输出的稳定结构。
  - 为浏览器渐进探索补足字段：
    - `depth`
    - `direction`
    - `collapsed`
    - `expand_token` 或可重新查询的 `target`
    - `edge_type`
    - `importance`
    - `truncated_reason`
  - 测试：
    - ready result -> `VisualGraph`
    - error result -> diagnostic graph
    - empty result -> empty graph
    - large result -> truncated graph
    - 1 跳、2-3 跳、折叠组节点均可表达
    - sensitive 字段不进入 graph label / tooltip
  - 证据：`browser/renderer/graph-layout.mjs`、`browser/test/graph-panel-renderer.test.mjs`。

- [x] M42.2：浏览器 graph renderer 模块
  - 建议位置：
    - `browser/renderer/graph-panel-renderer.mjs`
    - `browser/renderer/graph-layout.mjs`
    - `browser/renderer/graph-dom.mjs`
    - `browser/test/graph-panel-renderer.test.mjs`
  - renderer 输入：
    - `VisualGraph`
    - `analysis envelope`（仅作为兼容入口，内部先转换成 `VisualGraph`）
  - renderer 输出：
    - DOM 面板内容
    - DOM marker
    - 可选 Mermaid 文本
    - 可选 ECharts option JSON dump（不引入 ECharts runtime）
  - 要求：
    - 不 import Plugin Core。
    - 不 import BI designer glue。
    - 不 fetch metadata。
    - 不调用 runtime。
    - 不绑定 ECharts runtime；可接收外部注入的 ECharts 实例。
    - ECharts 不可用时必须降级到轻量 SVG/HTML graph。
    - 所有节点 label 做长度限制和脱敏。
    - 关系边必须表达方向：reads / writes / condition / action / alias / dataflow。
    - 默认清晰展示 1 跳关系。
    - 2-3 跳关系以弱化样式预览，不抢占当前 focus。
    - 超过预算的远端关系折叠成组节点，允许点击后继续探索。
    - JS renderer 只能渲染 Rust 输出的关系，不能自行推断业务关系。
  - 测试：
    - 节点、边、诊断、截断状态都能渲染。
    - 1 跳节点可读，2-3 跳节点有 fade/weak 样式，折叠组节点可见。
    - 点击可展开节点时发出结构化 `expand_requested` 事件，不直接编造关系。
    - label 中中文、空格、冒号、引号、换行不破坏 DOM。
    - token/cookie/password/cipherPassport 不出现在 DOM textContent。
  - 证据：`browser/renderer/graph-panel-renderer.mjs`、`browser/renderer/graph-dom.mjs`、`browser/test/graph-panel-renderer.test.mjs`。

- [x] M42.3：BI ECharts runtime resolver
  - 建议位置：
    - `browser/renderer/echarts-runtime-resolver.mjs`
    - `browser/test/echarts-runtime-resolver.test.mjs`
  - 解析顺序：
    - AMD `require(["commons/echarts/echarts-ext"])`，读取 `getEcharts()`。
    - AMD `require(["echarts"])`。
    - `window.echarts`。
    - 返回 `null` 并触发 fallback renderer。
  - 要求：
    - 不把 ECharts 打包进仓库。
    - 不新增 npm 依赖。
    - 不假设 ECharts 是全局变量。
    - 兼容 BI 当前 ECharts 4.x graph option。
    - 加载失败返回稳定 diagnostic，不阻塞 panel 展示。
  - 测试：
    - AMD `commons/echarts/echarts-ext` 正常解析。
    - AMD `echarts` fallback 正常解析。
    - 全局 `window.echarts` fallback 正常解析。
    - 全部失败时返回 `null`，renderer 使用 HTML/SVG fallback。
    - resolver 不访问 metadata、不创建 DOM、不调用 runtime。
  - 证据：`browser/renderer/echarts-runtime-resolver.mjs`、`browser/test/echarts-runtime-resolver.test.mjs`。

- [x] M42.4：Integration Controller 接入 graph renderer
  - 修改范围：
    - `browser/integration/metadata-checker-controller.mjs`
    - 对应 integration 测试
  - controller 在 `analyze` 成功后调用 graph renderer。
  - controller 在 `analyze` 失败后渲染 diagnostic graph，而不是只显示文本错误。
  - controller 负责把 runtime result 转换/传递给 graph renderer。
  - Plugin Core 仍只发事件和保存 lastResult，不直接知道 graph renderer。
  - controller 负责连接 `expand_requested`：
    - 优先使用 runtime 支持的 target/depth 查询。
    - runtime 不支持时显示 stable diagnostic。
    - 不在 JS 中走图推理。
  - 测试：
    - selection -> provider -> runtime load/build/analyze -> graph render。
    - runtime error -> diagnostic graph render。
    - metadata fetch error -> diagnostic graph render。
    - duplicate init 不重复创建 panel / listener / renderer instance。
    - stale response 不覆盖新 selection 的 graph。
    - expand request 能触发 runtime 查询或 diagnostic。
  - 证据：`browser/integration/metadata-checker-controller.mjs`、`browser/test/plugin-integration-smoke.test.mjs`。

- [x] M42.5：设计器浮动容器 / 面板最小实现
  - 建议位置：
    - `browser/renderer/graph-panel-host.mjs`
    - 或放在现有 integration host 中作为 browser-only consumer。
  - UI 目标：
    - 在 SuperPage 设计器中显示当前选中组件的关系图。
    - 面板可收起/展开。
    - 面板不阻挡设计器主要操作。
    - 面板显示当前 focus target。
    - 面板显示空状态、错误状态、运行中状态。
    - 默认展示关系图，不展示 raw JSON。
    - 提供“展开下一跳”的轻量交互入口。
  - DOM marker：
    - `data-metadata-checker-graph-panel="mounted|hidden|error"`
    - `data-metadata-checker-graph-nodes="<count>"`
    - `data-metadata-checker-graph-edges="<count>"`
    - `data-metadata-checker-graph-focus="<target>"`
    - `data-metadata-checker-analysis-status="idle|running|ready|error"`
    - `data-metadata-checker-renderer="echarts|html|svg"`
    - `data-metadata-checker-graph-truncated="true|false"`
    - `data-metadata-checker-graph-depth="<depth>"`
  - 要求：
    - 不修改 BI viewlet。
    - 不依赖 iframe 假设。
    - 不把 raw metadata 写入 DOM。
    - 自动化验收以 DOM marker 为准，console 只辅助人工观察。
  - 证据：`browser/renderer/graph-panel-host.mjs`。

- [x] M42.6：Selection trigger / debounce / cache
  - 目标：真实设计器里频繁点击组件时，关系图足够快且不会乱序。
  - 要求：
    - selection 改变后 debounce 分析。
    - 同一 `source_path + component_id + options` 命中内存缓存。
    - 新 selection 到来后旧请求结果不得覆盖新图。
    - 面板先显示 loading/previous stale 状态，再切换 ready/error。
    - provider/runtime 失败时返回 diagnostic graph。
  - 测试：
    - 快速连续 selection 只渲染最后一次结果。
    - cache hit 不重复调用 runtime analyze。
    - cache miss 正常调用 provider/runtime。
    - stale response 被丢弃并记录 marker 或 event。
  - 证据：`browser/integration/metadata-checker-controller.mjs`、`browser/test/plugin-integration-smoke.test.mjs`。

- [ ] M42.7：渐进多跳探索与注意力控制（与真实 smoke 联动）
  - 目标：让复杂多跳关系可读，不一次性把大图塞给模型或用户。
  - 展示策略：
    - 1 跳：默认清晰显示。
    - 2-3 跳：默认弱化显示，作为可探索预览。
    - 3 跳外：折叠成组节点，点击后按 target/depth 继续查询。
  - 可执行验收（按步骤执行）：
    - 使用同一 selection，比较 `data-metadata-checker-graph-truncated` 与 `data-metadata-checker-graph-depth`，确认 2-3 跳样式和 `collapsed` 组节点存在。
    - 点击可展开节点后只允许出现 `expand_requested` 事件；不允许新建无来源的边。
    - 点击展开后若 runtime 返回 unsupported，必须展示 diagnostic graph。
    - 将 `graph` 输出中的 `collapsed` / `expandable` 标记归入 `groups` 或 `node.metadata`，不在 renderer 侧推理新增关系。
  - 输出策略：
    - 默认只展示 `summary`、`primary_edges`、`diagnostics`、`next_queries`。
    - raw evidence 只在展开详情时短展示。
    - 大图必须按节点/边预算截断，并明确 `truncated_reason`。
  - 交付条件：
    - 本地测试继续保持：large graph 截断、expand 事件、diagnostic 展示。
    - real BI 运行时记录一例可展示 `collapsed group` + `expand_requested` + 失败降级。
  - 参考：`docs/m42-real-bi-smoke-and-performance-runbook.md`

- [ ] M42.8：真实 BI 环境 smoke
  - 前置：
    - M41 真实远程 session 已能拉取 `analyzer` 项目。
    - M40 custom.js / Service Worker / runtime launcher 接入流程可复用。
    - 真实上传物必须包含 `metadata-checker-browser-entry.mjs`、其 ESM sidecar、`metadata_checker.js` wasm-bindgen glue 和 `metadata_checker_bg.wasm`；不能只上传裸 `custom.js` 后依赖手工 mock `window.__metadata_checker_*_factory`。
    - browser runtime `analyzeSuperpageSelection` 必须返回 Rust/WASM 产生的 `visual_graph` item；page fallback 只能返回 diagnostic graph，不能返回 mock analysis。
  - 现场可执行项（按顺序）：
    - 在真实设计器中加载含 `custom.js` 的页面后，先完成 `custom.js` 注入与 `onInitDesigner` marker 校验。
    - 进行至少 5 次 selection，包含：
      - 普通组件 selection。
      - 无关联组件 selection。
      - 快速连点 3 次 selection（验证 debounce 与 stale 丢弃）。
    - 在每次 selection 后读取节点/边 marker、renderer 类型、runtime 标记。
    - 至少一次触发 `expand_requested`，确认成功回调或 `GRAPH_EXPAND_UNSUPPORTED`。
    - 至少一次模拟/触发 SW/runtime fallback（如移除或禁用 SW 启动），确认 fallback marker 与图 fallback。
  - 验收证据：
    - 完成后将命令/截图/DOM marker 记录填入 `docs/m42-real-bi-smoke-and-performance-runbook.md`。
    - 仅在记录完整证据后将任务标为已通过。
  - 参考：`docs/m42-real-bi-smoke-and-performance-runbook.md`

- [ ] M42.9：性能预算与真实文件样例
  - 目标：确保真实 Designer 点击组件后的图关系反馈足够快，不因为大页面、大图、多跳关系卡住 UI。
  - 容量策略：
    - 5MB `.spg` 不通过 selection payload。
    - 大文件加载继续走 M41 remote/session/runtime 异步路径。
    - 输出分 summary/detail。
    - 默认节点/边预算可配置。
    - 每次渲染需要记录 analyze/render/cache timing。
  - 真实样例：
    - `xiaoshouyi` 中一个复杂 SuperPage。
    - 一个包含 DataFlow 的 `.tbl` 背后页面。
    - 一个条件较多的页面。
    - 一个无明显关系的负例页面。
  - 测试：
    - 大页面 selection 不冻结 UI。
    - graph truncation 可见。
    - summary-first 输出不超过默认预算。
    - sensitive 字段不进入 DOM。
    - renderer 不因 ECharts 不存在而失败。
  - 交付项：
    - 在真实场景补齐 3 条性能记录：
      - 简单页面。
      - 中等复杂页面。
      - DataFlow/高复杂页面。
    - 每条记录需要包含：runtime 渲染标记、节点/边数、truncated、analyze timing、render timing、cache hit/miss。
    - 记录模板见 `docs/m42-real-bi-smoke-and-performance-runbook.md`。

验收标准：

- 浏览器里能看到当前选中组件的图关系，不需要 MCP。
- Graph renderer 是 browser consumer，不污染 Rust core、Plugin Core 或 Designer Glue。
- Runtime result / VisualGraph 是唯一关系图输入 contract，不新增 JS-only 业务推理。
- DOM marker 能支持自动化验收。
- 真实 BI smoke 完成后至少跑通一次 selection -> analyze -> graph render。
- ECharts 可用时使用 BI 现有 runtime；ECharts 不可用时 fallback renderer 仍可用。
- 默认 1 跳清晰、2-3 跳弱化、远端折叠，并支持点击探索。
- 快速切换 selection 不出现旧结果覆盖新结果。
- 失败时返回 diagnostic graph，不静默失败。
- sensitive 字段不进入 DOM、diagnostics、tooltip、Mermaid/ECharts 输出。

仍需在实现阶段确认的细节：

- 当前 VisualGraph 是否已经足够表达 `depth` / `collapsed` / `expand_token`。若不足，只补 Rust 输出 contract，不让 JS 自行补业务语义。
- BI 真实页面中 AMD `commons/echarts/echarts-ext` 是否在 `custom.js` 执行时可直接 require。若不可直接 require，resolver 必须异步等待或降级 fallback。
- 浮动面板挂载位置需要在真实 Designer 中选择最小侵入点。原则是不改 BI viewlet，不遮挡画布和属性面板。
- 点击展开下一跳时，runtime 是否已有 target/depth 查询入口。若没有，M42 只能显示 stable diagnostic，新增查询 API 另拆任务。
- 性能预算阈值需要真实环境基线后再定硬指标。M42 先要求记录 analyze/render/cache timing 和节点/边规模。
- ECharts graph option 与 BI 当前 ECharts 4.x 的细节兼容性需要 node test + 真实 BI smoke 双重覆盖。
