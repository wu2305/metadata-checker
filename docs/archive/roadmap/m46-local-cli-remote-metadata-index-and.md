# M46：Local CLI Remote Metadata Index and Analysis

| milestone | M46 |
| status | planned |
| depends | M45, M57 |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M46：Local CLI Remote Metadata Index and Analysis

目标：把 M45 在真实浏览器中验证过的远程元数据获取与 raw text 分析链路，沉淀成本地 CLI 可复用能力。用户可以在本地命令行用真实远程服务账号/session 拉取指定 project/module/file 的 `.spg/.tbl`，写入受控 session mirror，构建或增量更新 graphdb，并立即执行 query/analyze。M46 优先复用 M41 native session/provider 能力，不重新发明第二套远程协议。

## 与 M54–M57 的边界（2026-07-23 调整）

**M46 只做 CLI 产品化包装**，不实现第二套 change detection / mirror / prepare-persist。

| 能力 | 归属 |
|------|------|
| `MetaFilesChangeSource` / `BiMetaFilesChangeSource` | M54–M56（已合入） |
| session mirror 差量应用、`DiffRefreshOrchestrator`、`refresh_once` | M54–M56 |
| tick 循环 + `BackoffSchedule`、PersistReport 对外观测 | **M57 Phase 1** |
| SKILL / `--help` / README 差量刷新叙事同步 | **M57 Phase 1** |
| RefreshScope：**自动探索推断 + 输出申明**（可显式覆盖） | **M57 Phase 1** |
| 凭证失效稳定码 + 重登/换绑 session 原语（不落盘密码） | **M57 Phase 1** |
| Dense/Facts/PageDep 增量更新 | **M57 Phase 2** |
| 聚合 CLI 入口、发现/筛选矩阵 UX、analyze 后处理、进度展示打磨、脱敏与真实验收 | **M46** |

INDEX：`depends = M45, M57`。M57 Phase 1 完成后即可开 M46（不必等 Phase 2/3）。  
详见 [M57 spec](../../specs/2026-07-23-m57-diff-refresh-closeout-design.md)。

边界要求：

- Rust/CLI 是主实现；不得把远程下载、index、graph build 的核心逻辑放到 JS。
- 不修改远程 `.spg` / `.tbl`。
- token、cookie、password 不进入 stdout/stderr、graphdb、session manifest、cache、diagnostics 或测试 fixture。
- remote provider 只返回 raw metadata text 和文件元信息，不解析 `.spg/.tbl`、不建图、不做业务推理。
- 本地 session mirror 可以保存 raw metadata 文件，但必须和凭证存储隔离；默认不持久化凭证。
- 继续复用已有 scanner、graph store、query/analyze 与 **M54–M57 diff refresh 栈**，不为 remote-index 另建一套 graph snapshot。

任务清单：

- [ ] M46.1：CLI 命令契约与参数收敛
  - 设计并落地一个清晰入口（例如 `remote-index`，或组合既有 `--session-diff-refresh` / `--serve-stdio --runtime-session-id` + analyze）。
  - 参数至少覆盖：`--base-url`、`--project`、`--module` / `--source-path` / `--file-id`、`--graph-db-path`、`--session-dir`、`--json`、`--dry-run`。
  - 认证只允许 env、交互或一次性参数；输出必须脱敏；`--help` / README / 错误示例同步。
  - **禁止**在 CLI 层再实现一套 META_FILES poll / mirror 写入。

- [ ] M46.2：远程发现与筛选策略（UX 层）
  - 复用既有 whoami / permission / children / descendant / content API。
  - 支持 project/module/source_path/file_id 四种发现粒度；默认只下载 `.spg` / `.tbl`。
  - current page / explicit file first 的**产品默认值与提示**；底层范围过滤调用 **M57 RefreshScope**，不在 M46 再实现过滤内核。
  - 发现结果喂给既有 session / diff refresh 入口，不新建拉取协议。

- [ ] M46.3：Session 与 diff refresh 接线（不再自研增量同步）
  - 创建/绑定 session、对齐 `project_ref` / `graph_db_path` / mirror 根。
  - 调用 M57 的 one-shot / tick / RefreshScope / 重登换绑面。
  - `--clean-remote-mirror` 或同等显式清理；默认不破坏已有 session。
  - 远程删除/不可见：消费 diff refresh 的 stable diagnostic，不静默污染旧图。
  - 认证交互/参数组装可做 UX 打磨，但错误码与「不落盘密码」语义以 M57 为准。

- [ ] M46.4：Graph / index（复用，不重写）
  - 不另写 scanner 主链路；变更文件走 orchestrator → `ProjectIndexer::prepare` / persist。
  - 支持显式 `--graph-db-path` 与 graphdb lock 诊断。
  - 单文件失败不终止整批时，优先暴露 orchestrator/scanner 已有计数，再补 CLI 聚合。

- [ ] M46.5：Analyze / Query 后处理
  - refresh/index 后可选执行 page summary / component analyze / model·table·dataflow query。
  - JSON envelope 稳定：`status`、`summary`、`diagnostics`、`timing`、`graph_db_path`、`session_id`；并透传 M57 的 PersistReport / DiffRefreshReport 关键字段（若可得）。
  - human 默认低噪声，不打印 raw metadata。

- [ ] M46.6：进度、诊断与安全审计
  - human/JSON 进度：discovered/downloaded/indexed/skipped/failed（与 diff refresh 计数对齐）。
  - 错误码覆盖 unauthorized/forbidden/not_found/invalid_response/network/content_empty/graph_locked，以及 diff refresh 既有码。
  - 敏感信息脱敏测试覆盖 URL、headers、body、manifest、graphdb metadata、日志。

- [ ] M46.7：测试与真实环境验收
  - 单测/集成：mock remote →（复用）diff refresh → query；重点测 CLI 包装与脱敏，不复制 M54–M57 核心用例。
  - 真实环境：`autocrm-test.xiaoshouyi.com` 已知项目；记录命令、脱敏输出、graphdb 路径、计数与 query 样例。

验收标准：

- 影响面 Rust 测试覆盖 CLI 包装、脱敏、与 orchestrator 的接线；不要求重跑全部 M54–M57 套件作为 M46 门禁。
- 一条命令可从真实远程 project 完成：session 绑定 → diff refresh（或显式 sync）→ 至少一个 query/analyze。
- 真实验收记录不含 token/cookie/password/raw metadata。
- remote CLI 与 browser extension 共用同一 remote/session 语义，不出现两套矛盾的 source_path/file_id/revision 规则。
- **无第二套** META_FILES / mirror / graph delta 实现（代码检索验收）。
