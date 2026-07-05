# M42 Real BI Smoke & 性能记录 Runbook

适用范围：对 `M42.8`（真实 BI smoke）和 `M42.9`（性能预算与真实样例）形成可复用执行方法。除非本文件有明确定版记录，不要在里程碑里断言真实环境通过。

## 1. 先决条件

- M41 真实远程会话已能拉取 `analyzer` 项目，`metadata-checker` 在该项目可正常查询。
- 本地已按 [m40-real-bi-environment-runbook.md](m40-real-bi-environment-runbook.md) 完成登录态与页面级 `custom.js` 注入核验。
- `custom.js` 当前建议路径：
  - `/analyzer/public/hooks/custom.js`（推荐 smoke 用例）
  - `/{projectName}/public/hooks/custom.js`
- `metadata-checker-custom.js` 在 BI 设计器加载成功。
- M42 真实入口不是单文件假设。`custom.js` 会动态加载同目录 sidecar：
  - `/analyzer/public/hooks/metadata-checker-browser-entry.mjs`
  - `/analyzer/public/hooks/plugin-core/metadata-checker-plugin.mjs`
  - `/analyzer/public/hooks/integration/metadata-checker-controller.mjs`
  - `/analyzer/public/hooks/platform-glue/superpage-designer-glue.mjs`
  - `/analyzer/public/hooks/renderer/echarts-runtime-resolver.mjs`
  - `/analyzer/public/hooks/renderer/graph-panel-host.mjs`
  - `/analyzer/public/hooks/renderer/graph-panel-renderer.mjs`
  - `/analyzer/public/hooks/renderer/graph-layout.mjs`
  - `/analyzer/public/hooks/renderer/graph-dom.mjs`
  - `/analyzer/public/hooks/metadata-checker-sw.js`
  - 若测试真实 WASM：`/analyzer/public/hooks/metadata_checker_bg.wasm`
  - 若测试真实 WASM：`/analyzer/public/hooks/metadata_checker.js`（wasm-bindgen 生成的 JS glue，需暴露全局 `wasm_bindgen`）

真实 WASM artifact 生成方式：

```bash
cargo build --release --no-default-features --features browser-wasm --target wasm32-unknown-unknown
wasm-bindgen --target no-modules --out-dir /tmp/metadata-checker-wasm --out-name metadata_checker target/wasm32-unknown-unknown/release/metadata_checker.wasm
```

上传 `/tmp/metadata-checker-wasm/metadata_checker.js` 和 `/tmp/metadata-checker-wasm/metadata_checker_bg.wasm`。`--target no-modules` 是为了让 Service Worker 通过 `importScripts()` 加载后获得全局 `wasm_bindgen`。
`wasm-bindgen` CLI 版本需与 `Cargo.lock` 中的 `wasm-bindgen` crate 版本一致；当前为 `0.2.122`。

## 2. 核验 custom.js / onInitDesigner

1. 打开真实设计器页，例如 `/analyzer/app/M40HookSmoke.app?:edit=true`。
2. 在开发者工具 Console 执行以下脚本：

```js
function marker(v) {
  return document.querySelector(`[data-metadata-checker-${v}]`)?.getAttribute(`data-metadata-checker-${v}`) ?? null;
}

function panelState() {
  const panel = document.querySelector("[data-metadata-checker-graph-panel]");
  return {
    graphPanel: marker("graph-panel"),
    graphNodes: panel?.getAttribute("data-metadata-checker-graph-nodes") ?? null,
    graphEdges: panel?.getAttribute("data-metadata-checker-graph-edges") ?? null,
    graphFocus: panel?.getAttribute("data-metadata-checker-graph-focus") ?? null,
    graphTruncated: panel?.getAttribute("data-metadata-checker-graph-truncated") ?? null,
    graphDepth: panel?.getAttribute("data-metadata-checker-graph-depth") ?? null,
    graphRenderer: panel?.getAttribute("data-metadata-checker-graph-renderer") ?? marker("renderer"),
    analysisStatus: panel?.getAttribute("data-metadata-checker-analysis-status") ?? null,
    hostStatus: marker("analysis-status"),
  };
}

console.table({
  onInitDesigner: marker("on-init-designer"),
  sw: marker("sw"),
  runtime: marker("runtime"),
  wasm: marker("wasm"),
  fallbackUsed: marker("fallback-used"),
  fallbackCode: marker("fallback-code"),
  graphPanel: marker("graph-panel"),
  graphFactory: marker("graph-factory"),
  factories: marker("factories"),
  factoryEntry: marker("factory-entry"),
  realBundle: marker("real-bundle"),
  graphRenderer: marker("graph-renderer"),
  graphEchartsResolver: marker("graph-echarts-resolver"),
});
console.log("初始 panel 状态：", panelState());
```

3. 对照 `metadata-checker-custom.js` 标记定义确认下列最低要求满足：

- `data-metadata-checker-on-init-designer=called`
- `data-metadata-checker-sw=registered|active|failed|unsupported`
- `data-metadata-checker-runtime=service-worker|page-fallback`
- `data-metadata-checker-wasm` 不为空
- `data-metadata-checker-real-bundle=loaded`
- `data-metadata-checker-factories=installed|preinstalled`
- `data-metadata-checker-analysis-status=ready`（至少进入过一次）
- `data-metadata-checker-graph-panel=mounted`（若面板可见）
- 若使用页面级 `custom.js` 注入，确认实际 provider 请求到 `/api/meta/services/getFileContent/...`（说明走 page-RC provider）；否则记录为无法判定。

## 3. M42.8 真实 BI Smoke（可执行流程）

### 3.1 标准触发序列

按顺序操作：

1. 打开页面，确认自定义脚本和 onInitDesigner 已成功加载。
2. 选中组件 A（有关系）。
3. 选中组件 B（无关系）。
4. 快速连续选中 C/D/E（用于验证 debounce 与 stale 丢弃）。
5. 对可展开组节点执行一次展开动作。
6. 切换到 fallback 场景（若可控）：让 runtime launcher 不可用或暂停 SW，观察 fallback。

每次操作后读取一次 `panelState()`，并记录下列字段：

- `analysis-status`
- `graph-panel`
- `graph-renderer`
- `graph-nodes`
- `graph-edges`
- `graph-truncated`
- `graph-depth`
- `graph-focus`

### 3.2 缓存与事件可观测（可选但推荐）

默认状态下 host event 不一定可见。推荐在 smoke 的调试版 `custom.js` 中临时加一段探针：

```js
const origPluginFactory = window.__metadata_checker_plugin_factory;
if (typeof origPluginFactory === "function") {
  window.__metadata_checker_host_events = [];
  window.__metadata_checker_plugin_factory = (options) => {
    const host = options?.host;
    if (host && typeof host.emit === "function") {
      const rawEmit = host.emit.bind(host);
      host.emit = (eventName, payload) => {
        window.__metadata_checker_host_events.push({
          eventName,
          payload,
          ts: performance.now(),
        });
        return rawEmit(eventName, payload);
      };
    }
    return origPluginFactory(options);
  };
}
```

通过 `window.__metadata_checker_host_events` 查证：

- `analysis_cache_hit`
- `analysis_cache_miss`
- `analysis_stale_discarded`
- `analysis_started`
- `analysis_completed`

### 3.3 失败/诊断可观测

- 若图为空但无报错，检查 `graph-truncated`/`graph-truncated-reason`。
- 若 `graph-panel` 处于 `error` 或无法渲染，检查下列 marker：
  - `factories`
  - `factory-entry`
  - `factory-diagnostic`
  - `graph-factory-diagnostic`
  - `graph-panel`（`error`/`unavailable`）
  - `last-render-error-code`
  - `last-render-error-message`

注意：page fallback 只允许返回 diagnostic graph，不能用 mock analysis 冒充真实图关系。若未部署真实 WASM 或 SW 无法调用 WASM export，预期是稳定 diagnostic，而不是 `Fallback Analysis`。
真实 SW runtime 必须通过 wasm-bindgen JS glue 初始化 `metadata_checker_bg.wasm`，不能直接把裸 `.wasm` 的 `instance.exports` 当成 `initRuntime` / `analyzeSuperpageSelection` 等 JS API 使用。

## 4. renderer 与 fallback 复核

在每次成功渲染后要求记录：

- `data-metadata-checker-graph-renderer = echarts | html | svg`
- 当不应可用 ECharts 时，允许回退到 `html`，但要保留关系图可读。
- 当 marker 指示 `graph-echarts-resolver=failed` 或 `graph-factory-diagnostic=GRAPH_ECHARTS_RESOLVER_FAILED`，不允许因此把整条链路判为失败；须出现 fallback 渲染。

## 5. 性能基线记录模板（真实 BI）

每条记录按一次 selection 执行一次。

| run_id | timestamp | page | selection | runtime-kind | runtime-fallback | sw | provider-kind | renderer | cache-hit | cache-miss | nodes | edges | truncated | depth | analyze_started_ms | analyze_done_ms | analyze_ms | render_ms | panel_status_before | panel_status_after | stale_seen | diag_code | diag_msg | notes |
|---|---|---|---|---|---|---|---|---|---:|---:|---|---:|---:|---:|---:|---:|---|---|---|---|---|---|
|  |  |  |  | service-worker/page-fallback | true/false | registered/unsupported/failed | page-rc/unknown | echarts/html/svg | true/false | true/false |  |  |  |  |  |  |  |  | ready/running/error |  | true/false |  |  |  |

填写说明：

- `analyze_started_ms` / `analyze_done_ms`：使用 `analysis_started` 与 `analysis_completed` host 事件时间戳，或在插桩脚本里记录 `selection` 前后的 `performance.now()`。
- `render_ms`：首次 `graph` marker 字段变化到位到 `graph-panel` 指示 `ready` 所用毫秒。
- `cache-hit` / `cache-miss`：来自 host 事件。
- `runtime-kind`：从 `data-metadata-checker-runtime` 读取。
- `provider-kind`：`page-rc` / `unknown`（是否出现 `/api/meta/services/getFileContent/...`）
- `diag_code`：若有失败，优先填 host event 或 `last-render-error-code`。

## 6. 跑样例集

- 简单页面：`/analyzer/app/M40HookSmoke.app?:edit=true`
- 中复杂页面：`xiaoshouyi` 中包含条件较多的 SuperPage。
- 高复杂页面：`xiaoshouyi` 中包含 DataFlow `.tbl` 后链路的页面。
- 负例页面：无明显关系的页面。

> 所有运行记录需保存在本文件或附录文件中，作为里程碑 `M42.8` / `M42.9` 的验收证据。
