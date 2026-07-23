# 差量刷新一条龙设计（M54 / M55 / M56）

> 状态：approved（用户确认 2026-07-12；2026-07-17 按代码评审修订提交/重试语义）
> 范围：长驻 runtime 上「tick → META_FILES → 文件差量 → 候选图/read model → graph+watermark 原子提交 → cache-preserving swap → selective re-warm」三层差量闭环。
> 编号：性能线占用 **M54 / M55 / M56**；原 INDEX 中 formats 的 Dashboard / Report（rpt/dash）**后推**为 **M59**（本 spec 不实现 formats）。

## 背景

场景：stdio / session **LongLived** runtime，多轮连续查询，图会因远程元数据变更而变。

M53 已交付：

- warm 覆盖 availability + prerequisites + path_summary（`warmed_query_dispatch` p50 ~81ms）
- `PageDependencyIndex` + `invalidate_pages_for_dirty_nodes`（T3 **地基**，无触发业务）
- init/load 三段计时（dense / facts / page-dep）

尚未闭环：

- 变更信号（远程 `META_FILES` + watermark + tick）
- 文件层差量拉取（禁止整库 refresh 冒充）
- 图层增量 apply/persist 与 dirty 集合回传 runtime
- warm 层选择性 re-warm 编排
- 依赖索引对共享 Model / Field / DataFlow 的覆盖（T3 当前主要挂 page / Contains / Triggers）

历史 M98 曾把「物化事实」称为 M54；本线 **不是** B1.2 dataflow facts，而是 **差量刷新管道**。B1.2 仍 profile 门控，不在本三期默认范围。

## 目标

1. **三层差量均成立**：文件 mirror、图存储、runtime warm。
2. **长驻保持 hot**：差量刷新后，未受影响页的 warm cache 仍命中；受影响页 re-warm 后多轮查询回到 ~百毫秒量级（以 xiaoshouyi 合同页 compact 为参照，不设 CI fail 阈值）。
3. **graph 与 watermark 同事务提交**：候选图与派生 read model 准备成功后，图变更和业务 watermark 在同一 redb transaction 提交；re-warm 是提交后的 best-effort 性能动作，不参与正确性事务。
4. **与 M46 复用**：change-detect / mirror-diff API 由本线定义；browser/CLI 产品化（M46）复用，不另起第二套拉取逻辑。

## 非目标（三期合计）

- B1.2 dataflow facts 更深物化（独立 profile 门控）
- fragment 跨平台持久化
- CI 性能 fail 阈值 / Bencher alert（趋势观测可沿用 M50）
- Dashboard / Report 格式（**M59**，后推）
- MCP、正式 browser UI（M47）、完整 M46 CLI 产品包装（可依赖本线 API，但不在本三期做完 M46 清单）
- 用整项目 `session-refresh` + 全 runtime reload 作为唯一「刷新」路径并通过验收

## 里程碑切片（垂直通路）

| ID | 标题 | 差量深度 | 验收重心 |
|----|------|----------|----------|
| **M54** | Diff refresh 通路 | 三层打通；`META_FILES` 可用 fixture | 改 1 文件 → 只拉该文件 → 只 dirty 相关 → 只 invalidate/re-warm 相关页 |
| **M55** | 正确性与真源 | 真 `META_FILES` + 删除事件 + 依赖索引补全 | 共享 Model 脏则多页失效；退避；孤立变更不饿死 |
| **M56** | Persist 写路径 | `redb_commit` / commit batch | 大 dirty 或频繁差量下写盘成本相对 M53 基线下降 |

M56（本线）与原 formats「M56 Dashboard/Report」：**后者改登记为 M59**。

## 架构

```text
tick / 显式命令
  → MetaFilesChangeSource.poll(watermark) → ChangeSet
  → FileMirrorSync.apply_diff(ChangeSet)     // 只拉变更 .spg/.tbl
  → IncrementalIndexer.prepare(mirror)       // 候选 GraphDB + dirty/deleted IDs，不提交
  → RuntimeReadModel.prepare(candidate)      // 新 dense/facts/page-dep，保留无关 warm cache
  → redb.commit(candidate + watermark')      // 同一 write transaction
  → GraphRuntime.swap(candidate)             // 仅 move，提交后无 fallible rebuild
  → warm_page_logic_batch(affected)          // best-effort；失败只影响性能
```

### 核心类型（契约）

```rust
/// 单个远程事件流的游标；boundary_event_ids 防止同毫秒迟到事件被跳过。
pub struct SourceCursor {
    pub updated_at_ms: u64,
    pub boundary_event_ids: Vec<String>, // 排序、去重后持久化
}

/// 两个 HTTP 快照独立推进，在 graph commit 中作为一个 checkpoint 原子持久化。
pub struct MetaFilesWatermark {
    pub active: SourceCursor,
    pub deleted: SourceCursor,
}

pub struct ChangedRemoteFile {
    pub event_id: String,
    pub file_id: String,
    pub source_path: String,
    pub previous_source_path: Option<String>, // 同 file_id 移动/改名时删除旧 mirror 路径
    pub content_type: MetadataContentType, // 仅 .spg/.tbl 进入分析
    pub updated_at_ms: u64,
    pub deleted: bool,
}

pub struct ChangeSet {
    pub changed: Vec<ChangedRemoteFile>,
    pub change_count: usize,       // 必须等于 changed.len()
    pub next_watermark: MetaFilesWatermark,
    pub diagnostics: Vec<String>,
}

pub trait MetaFilesChangeSource {
    fn poll(&self, since: &MetaFilesWatermark) -> anyhow::Result<ChangeSet>;
    fn bootstrap(&self, manifest: &SessionManifest) -> anyhow::Result<ChangeSet>;
}
```

- **Fixture 实现**（M54）：读测试 JSON / 内存表，不碰真网。
- **BI 实现**（M55）：更新/新增复用 `GET /api/meta/services/getFileDescendant/{project_ref}` 返回的 `modifyTime/revision`，删除复用 `POST /api/meta/file/getRecyclebinFiles` 返回的 `deleteTime/uuid`。两个现有 API 均返回完整数组、无分页；active/deleted 各自按自己的 `SourceCursor` 过滤和推进，再合并事件。provider **只返回清单与 raw text**，不解析、不建图。
- **事件 ID**：活跃事件用 `active:<file_id>:<revision>`；删除事件用 `deleted:<uuid>`，因为 recycle-bin 中原始 `file_id` 可重复。缺失/非法时间、活跃 revision 或删除 uuid 时整次 poll 返回稳定错误，不得跳过后推进 watermark。
- **同毫秒边界**：事件时间大于 cursor 时接收；等于 cursor 时仅接收不在 `boundary_event_ids` 中的事件。每路 next cursor 取该路最大时间，并保存该毫秒已见 event IDs。两次 HTTP 请求发生交错时，不得用另一条流的较新时间推进本流 cursor。
- **移动/改名**：以稳定 `file_id` 对照 session manifest 中旧 `source_path`，在写入新路径后删除旧 mirror 路径；不得留下新旧两份文件。

### 模块职责

| 单元 | 路径建议 | 职责 | 禁止 |
|------|----------|------|------|
| `MetaFilesChangeSource` | `src/diff_refresh/source.rs` | watermark → `ChangeSet` | 建图、warm |
| `FileMirrorSync` | 扩展 `src/session/remote_sync.rs` / `sync.rs` | 按 `ChangeSet` 差量写 mirror；删除事件 | 存 token/cookie |
| `DiffRefreshOrchestrator` | `src/diff_refresh/orchestrator.rs` | 编排候选图/read model、同事务提交与 runtime swap | 直接 HTTP |
| `PageDependencyIndex` | `src/query/page_logic/page_diff.rs` | M54 用现网；M55 补 Model/Field/DataFlow→pages | 触发策略 |

复用既有：

- `SessionSyncMode` partial / 按 file 列表同步（`remote_sync.rs`）
- `ProjectIndexer::diff_file_states` + apply；新增 prepare 入口返回候选图与完整 dirty/deleted IDs
- `GraphRuntime` 的 read model 构建逻辑；新增 cache-preserving swap，复用 `warm_page_logic_batch`
- redb `META_TABLE` 存业务 watermark；与 `IndexCommit` 共用一次 write transaction

### Watermark 与失败语义

1. **既有 session 首次 bootstrap**：graph meta 无 checkpoint 时，必须先读取 deleted 完整快照、再读取 active 完整快照。active 与 `SessionManifest.files` 按稳定 `file_id` 比较 revision/mtime/path，只把新增或不一致文件列为拉取；manifest 中仍为 active 但第二个 active 快照已缺失的文件列为删除。两个 cursor 分别初始化为各自快照最大时间及边界 IDs，并与必要的 mirror/graph 变更在同一 redb transaction 提交；不得把历史 recycle-bin 全量事件逐条重放。若删除发生在两次请求之间，后读的 active 快照会把文件判为缺失；若删除发生在 active 快照之后，下次 deleted poll 仍能读取该事件。
2. **空变更**（`change_count == 0`）：已 bootstrap 的 session 不碰 graph/warm，业务 watermark 保持不变；`last_poll_at` 仅作 runtime 观测，不能替代业务游标。
3. **mirror / prepare 失败**：不提交 graph/watermark；per-file 原子写已经落盘的 mirror 文件可保留，下次 poll 仍返回同一变更集合。
4. **提交边界**：候选 graph、file states 与 `next_watermark` 在同一 redb write transaction 提交。提交前先构建候选 dense graph、availability facts、PageDependencyIndex 和保留无关页 cache 的 replacement read model；提交后 runtime swap 只做内存 move。
5. **re-warm 失败**：watermark 已随 graph 提交；受影响页 cache 已失效，后续查询走正确的 cold path。返回逐页 diagnostic，但不得回滚或重复消费变更。
6. **不设最小变更阈值**：每个非空 `ChangeSet` 都处理，避免孤立变更永久饿死。需要降压时只在 tick 调度层做 debounce/coalesce，不改变业务 watermark 语义。

### 依赖索引（M55）

当前 T3 覆盖：Page 自身、Contains 子组件、Triggers 动作。

M55 必须补齐至少：

- 页面 key models / 绑定 Model、Field
- DataFlow 模型及其输出字段被页引用时的反向边

验收：脏一个共享 Model → `affected_pages` 含所有引用页；无关页不在集合中。

漏索引时宁可 **多 invalidate**（保守），不可漏 invalidate；并打 diagnostic `page_dep_index_coverage=partial` 若走降级启发。

## 分里程碑详细范围

### M54 — 通路

**含**

- `MetaFilesChangeSource` trait + fixture
- `ChangeSet` / typed watermark；watermark 进入 graph redb `META_TABLE`，与 graph/file states 同事务提交
- `FileMirrorSync`：仅下载 `ChangeSet.changed` 中的文件
- 编排：mirror → 候选图/read model → graph+watermark commit → cache-preserving runtime swap → best-effort `warm_page_logic_batch`
- 测试：单文件变更 fixture；未变更页 warm 仍命中
- **增量建图批删**：多 dirty 文件时合并全部待删节点，**一次** `remove_nodes_by_ids`，再批量应用新增/修改；禁止 `dirty_file_count × 全图 rebuild` 退化（见 `ProjectIndexer::apply_graph_updates` / `GraphDB::remove_nodes_by_ids`）
- **规模观测**：dirty files `1/5/20/100`、deleted files `1/5/20` 的增量 apply 曲线（Criterion 或 `make perf-*`）

**不含**

- 真 BI HTTP/query 接入（字段类型和删除来源已在契约中固定）
- tick 调度层 debounce/coalesce
- Model/Field/DataFlow 索引补全；M54 fixture 只验收页面自身变更，不能据此宣称共享依赖正确
- `redb_commit` 专项优化
- stdio LongLived / Dense 构建 / budget-aware warm / bench profile（产品路径与 init/warm patch，不属本线）

### M55 — 正确性

**含**

- 真源 `META_FILES` + recycle-bin/tombstone（或录制 HTTP fixture）与字段对齐文档
- authenticated CLI one-shot 与绑定 session/auth context 的 stdio `diff_refresh` 命令
- 连续失败 diagnostic 与 tick 侧退避；非空变更不得因批量阈值跳过
- `PageDependencyIndex` 覆盖补全 + 回归测试
- 失败不污染：提交前失败时旧 graph/runtime 仍可用；提交后 rewarm 失败时 cold 查询正确
- **失效语义**：未受影响页 warm cache 保持有效；dirty 页必须失效；共享 Model / Field / DataFlow 变化必须失效所有引用页
- **等价性**：增量更新结果与完整 rebuild 语义等价；selective rewarm 后 compact 与冷 load+warm 等价

**不含**

- M47 UI、M46 全部 CLI 清单
- persist batch 性能专项（属 M56）

### M56 — Persist 性能

**含**

- 基于 core profile 的 `redb_commit` / commit batch（或等价减少 commit 次数）优化
- 与差量 dirty 集结合：拓扑变化必须增量写 v1 edge/file-state，并把 v2 shadow 标为 stale；仅 metadata 变化可复用既有 v2 meta patch，显式 full rebuild 才重建完整 v2 shadow
- journal 前后对比表（相对 M53 ~10.8s 基线）
- **真增量写盘**：仅更新 dirty nodes、受影响 edges、变化的 file states；多文件共享一次 redb transaction；避免长期同时全量写 v1+v2
- **Persist 报告字段**：dirty node 数、dirty edge 数、写入字节数、transaction commit 耗时、是否触发全量 fallback
- **规模观测**：dirty nodes `10/100/1000/10000` 写路径曲线

**不含**

- 查询语义物化（B1.2）
- 改变公开 JSON 查询契约

## 与 M46 / M59 关系

| 里程碑 | 关系 |
|--------|------|
| **M46** | 依赖本线 `MetaFilesChangeSource` / orchestrator；做 remote-index CLI 产品化与发现策略包装 |
| **M59** | 原 formats Dashboard/Report；本线占用 M56 后 **后推**，INDEX 改登记 |

## 测试策略

| 期 | 测试 |
|----|------|
| M54 | fixture `ChangeSet` → mirror 文件数断言 → 候选图/read model → graph+watermark 原子提交 → runtime swap → warm hit/miss；另测提交后 warm 失败仍可 cold 查询 |
| M55 | 活跃/删除事件归并、改名清理、索引覆盖、孤立单变更立即处理、退避；可选 `#[ignore]` 真网 |
| M56 | topology dirty 下 `full_fallback=false`、v2 stale 后 reopen 走 v1 且等价；Criterion / `make perf-*` 影响面；`redb_commit` 前后表写入 journal |

等价性：受影响页在 diff refresh 后的 `query_page_logic` compact（或约定 budget）与「同 mirror 上冷 load + warm」结果 **语义等价**（canonicalize JSON）。

## 观测

- 复用 `PerfProfile` stages：建议 `diff_refresh_poll_ms` / `diff_refresh_mirror_ms` / `diff_refresh_index_ms` / `diff_refresh_invalidate_ms` / `diff_refresh_rewarm_ms`
- 不设 CI fail 阈值；可进 Bencher trend（可选，非门禁）

## 风险

| 风险 | 缓解 |
|------|------|
| T3 索引漏页 | M54 只验收 page 自身变更；M55 补齐共享 Model/Field/DataFlow 后再验收跨页失效 |
| runtime graph/read model 代际不一致 | 提交前构建 replacement，提交后仅 move；测试 changed 页结果与 cold load 等价且无关 cache 世代不变 |
| `META_FILES` 无删除位 | 调用 `/api/meta/file/getRecyclebinFiles` 合并 `deleteTime/uuid`；若账号无项目级删除可见性或部署不暴露该 API，则不得宣称 M55 真源验收通过 |
| 与 full session-refresh 双路径漂移 | Orchestrator 为 LongLived 唯一推荐刷新路径；full refresh 仅显式逃生 |
| M46 并行重复实现 | INDEX `depends`：M46 注明复用 M54+ API |
| M56 与 formats 编号混淆 | formats → M59；文档交叉链接 |

## 参考

- [m53-performance-continuation.md](../milestones/performance/m53-performance-continuation.md)
- [2026-07-07-m53-warm-cache-scalability-design.md](2026-07-07-m53-warm-cache-scalability-design.md)（T3）
- [m41-plan.md](../milestones/browser/m41-plan.md) / `src/session/remote_sync.rs`
- [m46 archive](../archive/roadmap/m46-local-cli-remote-metadata-index-and.md)
