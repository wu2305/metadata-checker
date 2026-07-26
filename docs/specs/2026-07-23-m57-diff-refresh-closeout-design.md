# M57：Diff refresh 产品化与 tick 成本收口

> 状态：**approved**（2026-07-26；AI contract foundation 先行落地）  
> 范围：M54–M56 合入后的性能 + 产品化收口；同时为 **M46** 提供稳定编排/观测面，为 **M58** 降低输出噪声（不阻塞 M58）。  
> 编号：performance 线 **M57**；depends **M56**（已合入）。

## 背景

M54–M56 已交付：fixture/真源 META_FILES → mirror → 原子图/runtime 刷新 → 增量 persist，真机 one-shot 验收通过。明确遗留：

1. one-shot / 冷 rebuild 每 tick **全量**重建 Dense / AvailabilityFacts / PageDependencyIndex（真机 ~292s / 77k 节点）
2. `BackoffSchedule` 有了，**真实 tick 循环**未接（上层组装）
3. `PersistReport` 有了，**未接到** CLI/stdio 观测面
4. stdio LongLived **真机**未跑（仅 bound-context 单测）
5. 路径搜索对边迭代顺序敏感；e2e 用 `stable_semantic_view` 绕过；**路径确定性不做**（路径随业务变化失效，除非固定业务视角）
6. M46 需消费 M57 交付面；INDEX depends 已改为 `M45, M57`
7. Agent/文档面未跟上：SKILL / `--help` / README 几乎不覆盖差量刷新命令
8. 刷新默认全项目：缺 **页级/路径级刷新范围**；凭证过期与重登无一等公民流

9. 后续 Small Language Model 测评缺少稳定的 diff-refresh 机器结果契约：当前编排器已经生成 `PersistReport`，但报告、one-shot 和 stdio 没有统一透传；登录 401/403 还丢失服务端诊断。M57 先补这层基础，M58 再实现空上下文 runner、AnswerJudge 和模型适配器。

## 目标

1. **产品壳**：tick 循环 + Backoff；PersistReport 进入 CLI/stdio；stdio LongLived 真机冒烟记录。
2. **SKILL / 帮助同步**：仓库内 skill（若有）、CLI `--help`、README 写清 one-shot / 长驻 / `diff_refresh` / 错误码 / 推荐路径；并给出已安装 skill 路径的同步说明（不强制改用户机器上的私有副本为 CI 门禁）。
3. **页级刷新范围**：RefreshScope **自动探索推断 + 输出申明**（可显式覆盖；可回落 project）；仍走同一 META_FILES → mirror → orchestrator 栈。
4. **凭证流**：401/登录失效时的稳定错误码与 **重登/换绑 session** 路径（env / 交互 / 一次性参数）；默认仍不落盘密码；多 session 切换语义写清。
5. **Tick 成本**：Dirty∪deleted 驱动的 Dense/Facts/PageDep **增量更新**，规模曲线验收。
6. **LongLived 内存优先**：`install` 后查询立即可见；persist **阈值批量 + 轮次兜底**（one-shot 每轮同步）；mirror→persist 失败恢复 e2e。
7. **M46 handoff**：M46 只包装 CLI UX；消费本里程碑的编排/范围/凭证面。

## 非目标

- M28–M30 契约大改、MCP adapter
- M59 Dashboard/Report（rpt/dash）
- B1.2 dataflow facts 全量（仍 profile 门控另开）
- 继续压已 warm query &lt; 81ms
- 在 M57 内实现 M46.1–M46.7 全部 CLI 产品化（发现矩阵、analyze 组合命令、完整进度 UX 仍属 **M46**）
- 路径搜索确定性化（跨 tick / 跨项目可复现路径）；e2e 继续用 `stable_semantic_view` 剔除路径段
- 第二套 change detection / mirror / graph build
- 系统钥匙串/OS keychain 深度集成、浏览器活刷新、多项目并行 tick 守护进程（可后续另开）

## 三阶段

| Phase | 焦点 | 验收信号 |
|-------|------|----------|
| 1 | 观测与产品壳 + **SKILL/帮助** + **刷新范围** + **凭证流** + M46 可包装面 | PersistReport 对外；tick+Backoff 可跑；stdio 真机冒烟有记录；SKILL/`--help`/README 覆盖差量刷新；范围过滤有单测；401→重登/换绑路径有稳定码与脱敏测；M46 边界文档一致 |
| 2 | 派生索引增量 | one-shot/冷 tick 主导成本相对 Phase 1 基线明显下降；dirty 规模曲线；等价口径达标 |
| 3 | LongLived 内存优先 + 失败恢复 | 阈值/轮次批量 persist；`install` 后立即可查；watermark 内存推进；mirror→persist 失败 reconcile e2e |

## 进度约定

- **journal 随 commits 滚动更新**
- INDEX：开工即 `active`；**三阶段全收完**再改 `done`
- 多 PR 须另补 `approved` plan（本设计批准后）

## 与 M46 的边界（本设计固定）

```text
M57 交付（Rust core / stdio / one-shot / 文档）
  MetaFilesChangeSource + mirror + DiffRefreshOrchestrator
  + tick 循环 / Backoff
  + PersistReport 结构化输出
  + RefreshScope（project|module|source_path|…）
  + 凭证失效诊断 + 重登/换绑 session API/CLI 开关
  + SKILL / --help / README 同步
  + （Phase 2+）增量 read model
  + （Phase 3）LongLived 内存优先 persist 策略

M46 只做
  聚合 CLI 入口、发现/筛选矩阵 UX、analyze 后处理组合、进度展示打磨
  调用上述面；不实现第二套 META_FILES / mirror / prepare-persist / 范围过滤内核
```

## 与 M58 的关系

- **可并行**：M58 不依赖 Phase 2/3
- Phase 1 的 SKILL 同步直接利于 M58 runner（system 注入的协议文本）
- 路径确定性已移出 M57，**不阻塞** M58

## 架构要点

```text
Phase 1
  refresh_once / tick_loop(Backoff)
    → PersistReport + DiffRefreshReport → JSON envelope
  RefreshScope
    → 自动探索推断（可显式覆盖）→ 过滤 changelist / fetch 集合
    → 每次输出必须申明生效 scope（及回落原因）
    → cursor boundary 不被错误收窄
  Auth
    → 401/失效 → 稳定码；可选 rebind/relogin 后继续 tick（凭据不落盘）
  Docs
    → SKILL + --help + README 同一套命令叙事

Phase 2
  prepare_replacement(dirty∪deleted)
    → 增量 Dense / Facts / PageDep
    → selective rewarm 不变

Phase 3 — LongLived 内存优先 persist
  one-shot: 保持 persist → install（每轮同步落盘）
  LongLived:
    poll → mirror → prepare → prepare_replacement
    → install_replacement              # 查询立即可见
    → if dirty∪deleted > threshold OR pending_rounds >= max_rounds:
         persist_index + checkpoint; pending_rounds = 0
       else:
         pending_rounds++; persist_deferred
    poll watermark: max(disk_checkpoint, pending_watermark)
  默认: persist_dirty_threshold=100, persist_max_rounds=10（CLI/env 可覆盖）
  persist 失败: 内存已推进，下轮重试 persist（不重复 install）
  崩溃: 从磁盘 checkpoint 恢复；未 persist 轮次丢失，META_FILES 重拉补齐
  失败恢复 e2e: mirror ok ∧ (prepare|persist) fail 的 reconcile
  （可选）修 ensure_redb_tables 假脏写盘；非 Phase 3 验收门
```

## 验收门（摘要）

1. Phase 1：PersistReport 对外可见；tick 网络错误退避、数据错误不退避；**至少一条** SKILL 与 `--help` 路径描述 one-shot 与长驻；RefreshScope **自动探索 + 输出申明**（可显式覆盖）有单测；凭证流脱敏测 + 重登/换绑不写密码进 manifest。
2. Phase 2：小规模 topology dirty 的 read_model rebuild 不得接近全图冷重建量级；须有曲线与 journal 数字。
3. Phase 3：LongLived `install` 后查询立即可见；阈值或轮次兜底触发 persist；`DiffRefreshReport` 含 `persisted` / `pending_dirty_total`；watermark 内存推进单测；失败 reconcile e2e。
4. M46：depends = `M45, M57`；清单与本边界表一致。

## 开放问题（批准前可决）

1. ~~路径确定性~~ → **已决（2026-07-24）**：**不做**
   - 路径随业务变化失效；除非固定业务视角，同项目内也无法保证跨 tick 可复现。
   - e2e 继续用 `stable_semantic_view` 剔除路径段；不纳入 M57 验收门。
2. ~~零写盘 open~~ → **已决（2026-07-24）**：**降级；主线改为 LongLived 内存优先**
   - 「零写盘」指 open 时 `ensure_redb_tables` 的假脏写盘，与远端变更是否落盘无关。
   - 远端变更本就在 tick 内实时维护；LongLived 改为 `install` 优先、persist 批量。
   - 严格零写盘 open 可选修，非 Phase 3 验收门；逻辑等价文档 + 单测即可。
3. ~~RefreshScope 默认值~~ → **已决（2026-07-23）**：**自动探索 + 申明**
   - 运行时根据 session / 当前页或显式 file·module 线索 **自动推断** RefreshScope（可回落到 project）。
   - 每次 refresh/tick 必须在 JSON/human 输出中 **申明**实际生效的 scope（及回落原因）；禁止静默全项目、也不强制调用方每次手写 scope。
   - 调用方仍可显式传入 scope **覆盖**自动推断。
4. ~~重登默认行为~~ → **已决（2026-07-24）**：**默认只返回稳定错误码，不自动重登**
   - tick/refresh 遇鉴权失败：返回稳定码（如 `SESSION_AUTH_REQUIRED` / `401`），由调用方决定是否重绑；不加默认静默 `--relogin-on-auth-failure`。
   - **服务端额外信息**：BI `/api/auth/signin` 在失败时通常会在 JSON body 提供 `message`（或 `error.message`），例如密码错误说明。M57 须在错误 envelope 中保留 **脱敏后的** 该字段（与稳定码并存），便于人工排障。
   - **现状缺口（实现时修）**：当前 `login()` 在 HTTP **401/403** 时只返回状态文案、**丢弃 body message**；仅在 HTTP 2xx 且 `ok:false` 时才读取并 `sanitize` message。Phase 1 凭证流应统一：无论 HTTP 状态，优先解析 body 的 `message`/`error.message`（脱敏），再附稳定码。
5. ~~LongLived 查询可见性~~ → **已决（2026-07-24）**：**install 后立即可查**
   - 内存为 LongLived 会话内权威查询面；persist 不挡热路径。
   - 崩溃可丢未 persist 的最近几轮，可接受。
6. ~~批量 persist 策略~~ → **已决（2026-07-24）**：**脏节点阈值 + 轮次兜底**
   - 主触发：`dirty_node_ids ∪ deleted_node_ids > persist_dirty_threshold`（默认 **100**）。
   - 兜底：连续 **10** 轮未落盘则强制 persist（`persist_max_rounds`）。
   - one-shot：每轮同步 persist，忽略阈值与轮次兜底。
   - poll watermark：内存维护 `pending_watermark`，取 `max(disk_checkpoint, pending_watermark)`。
   - 报告：`persisted: bool`、`pending_dirty_total` 写入 `DiffRefreshReport` / `PersistReport`。
