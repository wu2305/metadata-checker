# M45：Remote Metadata Auto Fetch and Background Analysis

| milestone | M45 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M45：Remote Metadata Auto Fetch and Background Analysis

目标：让 CLI 与浏览器插件都能建立远程 session，并让浏览器插件在设计器中自动获取、缓存、分析当前页面及用户可见元数据。浏览器端采用“简单 indexing 进度 + 当前页面优先”的产品策略：优先保证当前页面和当前选择可用，后台 indexing 以可见进度条展示，不追求完全无感知的强抢占调度。

边界要求：

- 不修改远程 `.spg` / `.tbl`。
- 不把 token、cookie、password 写入 IndexedDB、graphdb、日志、diagnostics、DOM 文本或 AI output。
- JS 只做插件接入、消息桥、Service Worker 编排、缓存 provider 和 UI 状态展示；解析、建图、依赖追踪、分析仍由 Rust/WASM core 完成。
- `RemoteMetadataProvider` 不负责登录，只接收已认证 session/client 后读取 raw metadata。
- 当前不要求为了浏览器体验引入完整强状态机。Rust orchestrator 可作为可复用基础能力保留，但 M45 的验收路径以简单 indexing、显式进度、当前页面优先为准。

登录链路：

- CLI：
  - 使用账号、密码、用户目录调用 `/api/auth/signin`。
  - 使用 `reqwest` cookie jar 保存 session，后续元数据请求复用同一个 client。
  - 默认使用内存 cookie jar；持久化 session file 只作为显式配置。
- Browser Extension：
  - content script 在 isolated world 中调用 `/api/auth/getAccessToken`，并使用 `credentials: include` 复用当前浏览器登录态。
  - 一次性 token 不允许经过 page-visible `postMessage`、DOM、diagnostics、console 或 panel 文本。
  - content script 将一次性 token 通过 `chrome.runtime.sendMessage` 转发给 extension service worker。
  - extension service worker 调用 `/api/me/whoami?access_token=...` bootstrap 插件侧 session。
  - 成功后 service worker 用该 session 拉取远程元数据；如果真实环境证明 SW 不能可靠建立 session，返回稳定 diagnostic 后再补 page-context fallback。

远程元数据清单：

- 使用 `/api/me/getPermissionInfo` 获取当前用户可见 project/module 信息。
- 使用 `/api/meta/services/getFileChildren/{project}` 获取项目下一级模块。
- 使用 `/api/meta/services/getFileDescendant/{project}/{module}` 获取模块内递归文件清单。
- 只把 `.spg` / `.tbl` 放入分析队列，其它文件只记录为可见但不可分析。

任务清单：

- [x] M45.1：Auth Session Contract
  - Rust 增加 `AccessTokenProvider`、`SessionBootstrapper`、`AuthenticatedSession` contract。
  - CLI path 继续使用 `/api/auth/signin` 登录并复用 reqwest cookie jar。
  - Native provider 增加 `/api/me/whoami?access_token=...` bootstrap 能力。
  - 稳定错误码：
    - `ACCESS_TOKEN_UNAVAILABLE`
    - `SESSION_BOOTSTRAP_FAILED`
    - `SESSION_BOOTSTRAP_ANONYMOUS`
    - `SESSION_COOKIE_NOT_ESTABLISHED`

- [x] M45.2：Remote Metadata Index
  - Extension SW 增加 visible metadata index controller。
  - 通过 `getPermissionInfo -> getFileChildren -> getFileDescendant` 建立可见元数据清单。
  - 队列只分析 `.spg` / `.tbl`，其它类型记录但不进入分析。

- [x] M45.3：Foreground Auto Analysis
  - content script 直接获取一次性 token，page script / page bridge 不返回 token 明文。
  - content script 在 bridge ready 后获取 token 并触发 SW bootstrap。
  - selection 变化后更新 panel，并把当前 selection 作为前台高优先级任务通知 SW 后触发一次限量处理。
  - foreground analysis cache key 必须包含 selection 信息，避免被后台页面级 artifact 覆盖。

- [x] M45.4：SW Background Worker Queue
  - SW 维护后台扫描队列。
  - 优先级：当前页面 -> 当前页面依赖 -> 同 app -> 同 project/module -> 其它可见项目。
  - 首轮最小限流语义：采用单并发处理（`concurrency=1`）；
  - 首轮最小速率参数：每批次处理 `batchLimit`（可配置，默认先保守），任务间隔 `minIntervalMs`（可配置）。
  - 支持可控暂停与恢复（`pause`/`resume`）；暂停期间保留队列与任务状态，恢复后继续处理不丢任务。
  - 限速与暂停语义以可配置参数为主，后续再收敛为更高并发。
  - 后台任务只做渐进预取和运行时可用时的分析触发，不阻塞 panel、selection、当前页面分析。
  - 如果当前选择关联元数据尚未完成 indexing，panel 显示“正在索引 / 可重试 / 当前页面优先处理”，不要求做到完全无感知等待与抢占。

- [x] M45.5：IndexedDB Cache
  - Browser 侧 cache provider 预留 IndexedDB adapter。
  - cache key 包含 server origin、project、source_path、file_id、revision/hash。
  - token/cookie/password 永不落盘。
  - Node 测试使用内存 fake IndexedDB 证明敏感信息不会进入缓存。

- [x] M45.6：Panel and Diagnostics
  - panel 展示当前分析状态、后台扫描进度、缓存命中、最近错误。
  - 新增结构化事件：
    - `session_bootstrapped`
    - `visible_metadata_indexed`
    - `metadata_prefetched`
    - `background_analysis_progress`
    - `background_analysis_completed`
  - 所有输出必须脱敏。

- [x] M45.7：Simplified Indexing Progress + Current Page First（代码能力已落地，真实 BI 证据仍按验收标准补齐）
  - 保留已落地的 Rust orchestrator 作为后续可选基础能力，但 M45 不强制把浏览器端全部后台调度切到 Rust 状态机。
  - 浏览器端验收改为更简单的产品路径：
    - 打开设计器后优先获取并分析当前 `.spg`；
    - 当前 selection 变化时优先处理当前页面，不等待全量可见元数据 indexing 完成；
    - 后台 indexing 显式展示进度：已发现文件数、已处理数、失败数、当前处理文件、缓存命中数；
    - 当前 selection 所需元数据未就绪时，panel 给出稳定状态：`indexing_current_page`、`waiting_for_metadata`、`retry_available`；
    - 用户可手动触发“优先处理当前页面/重试当前选择”；
    - 不要求实现完全无感知的 artifact waiter、强抢占、复杂多并发调度。
  - JS 侧允许保留轻量队列与进度统计，但不得承载解析、图查询或业务推理；复杂队列算法不作为 M45 验收要求。
  - Rust/WASM core 继续负责真实解析、建图、selection 分析；JS 只负责远程 fetch、缓存、触发分析和展示进度。
  - 2026-05-28 已补齐 Chromium popup 的 indexing/progress/current/cache 展示，并新增 `Process background` 与 `Retry current selection` 手动验收入口。
  - `Retry current selection` 由 content script 读取当前 bridge selection 后转发给 extension service worker 的 foreground queue，不经过 page-visible token、raw metadata 或业务解析逻辑。


验收标准：

- Rust 测试覆盖账号密码登录、whoami bootstrap、anonymous/401/403/token 失效、敏感信息脱敏。
- Browser JS 测试覆盖 page script 获取 token、content script 转发、SW whoami bootstrap、visible metadata index、cache 脱敏、`selection -> runtime adapter` 的边界调用（不做 raw 解析/业务推理）。
- 真实环境闭环（待执行）：
  - 状态：本里程碑的真实 BI 端到端验收未闭合，**不得把测试通过当成真实环境闭环完成**。
  - 验收步骤（需补充人工与自动化证据）：
    1. 使用目标环境：`https://autocrm-test.xiaoshouyi.com`，目标应用：`/xiaoshouyi/app/价审.app`，并保留原始页面交互上下文；
    2. 执行真实 token 获取流程，不上传、记录账号密码或 token；
    3. 验证 SW whoami bootstrap 成功；
    4. 验证可见元数据目录的拉取与队列入队；
    5. 验证一条 SPG/TBL 的 WASM analysis 成功返回；
    6. 验证后台扫描任务可持续运行并产出进度；
    7. 验证 panel 可展示分析状态、后台扫描进度与错误；
    8. 不修改任何 `.spg` / `.tbl` 文件；
    9. 对 panel 展开态、关键分析状态、后台扫描状态留存截图（含时间戳或运行上下文）作为验收证据。
  - 2026-05-29 真实环境阶段性记录：
    - 已在 `https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/价审.app?:edit=true&:file=销售订单价格审批-信息补充.spg` 验证 extension content script、page script、runtime adapter、BI custom bridge、selection bridge 均加载成功。
    - 已验证 content script 获取一次性 token 后 SW session bootstrap 返回 `session=ready`，DOM marker 未暴露 token 明文。
    - 已验证真实 selection 变化会更新 panel：`Status: analyzing`、`Indexing: waiting_for_metadata`、`Retry available: yes`。
    - 发现 MV3 extension service worker 可能在页面停留期间被回收，导致 DOM 上仍保留旧 `session=ready` marker，但 selection 消息进入新 SW 实例后返回 `waiting_for_metadata`。已补 content script 自动 rebootstrap 并重放当前 selection 的修复，待 reload unpacked extension 后复测。
    - reload 后复测显示自动 rebootstrap 已推进当前 selection 到后台队列，panel 从 `waiting_for_metadata` 变为 `Background progress: 1/1`、`Background failed: 1`。当前仍需定位单文件分析失败原因；已补 panel 首条 diagnostic code/message 展示，待再次 reload 后读取真实失败码。
    - 后续验收不使用假 BI 服务替代真实环境；继续以真实 `autocrm-test.xiaoshouyi.com` 页面、真实登录态、真实 metadata API 和真实设计器对象为准。
    - 已确认 Chromium MV3 extension 包需要 `wasm-bindgen --target web` 产物，因为 background module service worker 通过动态 `import()` 加载 `metadata_checker.js`；`--target no-modules` 只保留给 BI hook / `importScripts()` 场景。
  - 2026-05-31 真实环境闭环记录：
    - 已将 MV3 WASM runtime 从 Service Worker 迁移到 offscreen document；Service Worker 只做 bootstrap、session、远程 API 和 runtime message 编排。
    - 已在真实 `xiaoshouyi` 页面验证当前 `.spg` raw metadata 拉取、WASM load/build/analyze、foreground artifact ready、后台 failed=0。
    - 已补 `browser/tools/m45-real-bi-collect-evidence.mjs`，可通过 CDP 自动采集 DOM marker、panel 文本、extension/offscreen target、脱敏 console 和截图路径。
    - 冷脸验收关注测试完整性后，已补 offscreen runtime 本体 smoke、SW/offscreen bridge failure matrix、package 产物校验、完整 WASM selection/options contract 和大体量 synthetic raw metadata 样本。
