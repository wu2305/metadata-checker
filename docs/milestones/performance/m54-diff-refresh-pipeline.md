# M54–M56：差量刷新一条龙

> 状态：**done**（已合入 `main` / `b4ec419`；后续产品化与 tick 成本见 [M57](m57-diff-refresh-closeout.md)）  
> Spec：[2026-07-12-diff-refresh-pipeline-design.md](../../specs/2026-07-12-diff-refresh-pipeline-design.md)  
> Plan：[2026-07-12-diff-refresh-pipeline-plan.md](../../plans/2026-07-12-diff-refresh-pipeline-plan.md)

## 目标

LongLived runtime：`tick → META_FILES(since watermark) → 文件差量 → 候选图/read model → graph+watermark 原子提交 → cache-preserving swap → selective re-warm`，多轮查询保持 hot。

| ID | 焦点 |
|----|------|
| M54 | 三层通路（fixture META_FILES）+ **多 dirty 文件批量删除/应用** + dirty files 规模曲线 |
| M55 | 真源活跃/删除事件 + PageDependencyIndex 覆盖 + 退避 + **失效/等价语义** |
| M56 | topology-dirty **真增量 persist**（v1 delta、v2 shadow stale/current、单 txn、fallback 报告） |

## 与产品路径 patch 的边界

以下由独立 patch 交付（不阻塞本线编排），**不**在本 journal 重复实现：

- Spec / Plan：[2026-07-13-perf-product-path-init-warm-patch-design.md](../../specs/2026-07-13-perf-product-path-init-warm-patch-design.md) / [plan](../../plans/2026-07-13-perf-product-path-init-warm-patch-plan.md)
- stdio server 默认 `RuntimeMode::LongLived`
- `[profile.bench] inherits = "release-fast"`
- DenseGraphSnapshot 单次遍历 / edge payload 去重
- budget-aware path JSON 物化（`PathMaterializeKeep`）；prerequisites 因 compact 输出依赖全量 `total_count`/`truncated`，本轮不在 cache 层截断
- 页面 `.spg` 元数据 process-local cache
- LongLived startup / warm N 页 Criterion + RSS 观测

本线只消费上述能力（尤其 LongLived + invalidate/rewarm）。

## 明确不做

- B1.2 dataflow facts（profile 门控另开）
- fragment 跨平台
- CI 性能 fail 阈值
- 用全量 session-refresh 冒充差量验收
- Dashboard/Report 格式（后推 **M59**）
- 继续压缩已 warm 的 query 到 81ms 以下

## 与其它里程碑

- **M53**：T3 地基已交付；本线消费 `invalidate_pages_for_dirty_nodes` / `warm_page_logic_batch`
- **M46**：复用本线 change-detect / orchestrator，不另起拉取栈
- **M59**：原 INDEX formats M56（rpt/dash）后推

## 实现进度

| 项 | 状态 |
|----|------|
| Spec / Plan / INDEX 登记 | done |
| M54 通路代码 | done（Tasks 1–8，见验收记录；已合入 main） |
| M55 正确性 | done（Tasks 9–12 + 真机验收通过；已合入 main） |
| M56 persist | done（Tasks 13–16，见验收记录；已合入 main） |
| 后续 | 见 [M57](m57-diff-refresh-closeout.md) |

## 计划核查记录（2026-07-17）

对 plan 与代码现状做了逐行核查（文件路径、类型、既有行为描述均行号级一致，无硬伤），执行时需注意以下四点：

1. **以下按「新建」估工作量，plan 措辞易误读为「扩展既有」**：
   - `PersistReport`（Task 15）：代码库中不存在该类型，`src/perf_report.rs` 只有 page-logic profile 报告。
   - `V2ShadowState::{Current, Stale}`（Task 14）：v2 meta 目前只有 `REDB_V2_SCHEMA_VERSION`（`src/graph_redb_v2.rs:18`）。
   - `META_TABLE`（Task 6）：`graph_redb.rs:34` 已定义但从未读写，checkpoint 写入是首次启用（旧库该表为空，migration 返回 `None` 的路径天然成立）。
2. **spec/plan 命名漂移，执行以 plan 为准**：spec 的 `IncrementalIndexer.prepare` 即 `ProjectIndexer::prepare`；spec 的 `GraphRuntime.swap(candidate)` 即 plan 的 `prepare_replacement`/`install_replacement`；FileMirrorSync 落在 plan 新建的 `src/diff_refresh/mirror.rs`（spec 写的 session 归属作废）。
3. **集成点需动现有签名/可见性**：`run_stdio_server` 目前只收 `(db_path, project_dir)`（`src/stdio_server.rs:163`）；`atomic_write_text` 是 `src/session/sync.rs:197` 私有函数需提升可见性；`remove_nodes_by_ids` 有 trait（`graph_store.rs:149`）与 GraphDB inherent（`graph_redb.rs:652`）两套同名签名，Task 5 需先定走哪套。
4. **Task 14 是三期最易低估点**：`GraphDB::open_inner`（`graph_redb.rs:144-148`）当前无条件优先 v2 hydrate，没有「跳过 stale v2」决策点；引入 `V2ShadowState` 必须同步改 open 的 hydrate 分支，plan 对此只有一句，实现时单列测试覆盖。

## 真源字段语义（M54 Task 2 固定）

typed ChangeSet 契约实现于 `src/diff_refresh/types.rs`，真源字段语义固定如下：

- **活跃事件**：字段 `ID/PARENT_DIR/NAME/TYPE/modifyTime/revision`，`event_id = active:<file_id>:<revision>`。
- **删除事件**：字段 `FILE_ID/PARENT_DIR/NAME/TYPE/deleteTime/uuid`，`event_id = deleted:<uuid>`。
- **时间**：统一为 Unix 毫秒（`updated_at_ms: u64`）。
- **双 cursor**：`MetaFilesWatermark { active, deleted }` 两路独立推进；`SourceCursor.boundary_event_ids` 记录最大毫秒内已见事件 ID，构造、序列化、反序列化均排序去重；同毫秒未见过的事件仍被接收（`cursor_accepts`）。

## M54 Task 5 批删规模曲线（2026-07-18）

`apply_incremental_changes` 合并 dirty previous IDs 与 deleted IDs 去重后每轮 apply 最多一次
`remove_nodes_by_ids`（GraphWriteStore trait 层；`apply_graph_updates`/`apply_deletions`
委托同一实现）。bench：合成 120 个最小 SPG 的临时项目，Criterion `sample_size=10`、
warm-up 1s、measurement 3s，`CARGO_TARGET_DIR=target/criterion/redb cargo bench --bench
redb_persistence_bench -- redb_batch_delete_curve`，衡量完整增量 `scan`（含 discover/diff/
parse/apply/persist 固定开销）。

| bench | time（median） | 区间 |
|-------|---------------|------|
| dirty_1 | 143.31 ms | [118.93, 167.09] |
| dirty_5 | 124.49 ms | [98.929, 152.19] |
| dirty_20 | 94.600 ms | [92.916, 95.997] |
| dirty_100 | 89.480 ms | [88.550, 90.397] |
| deleted_1 | 87.150 ms | [84.912, 89.215] |
| deleted_5 | 85.210 ms | [84.170, 85.954] |
| deleted_20 | 83.037 ms | [82.199, 84.492] |

结论：曲线平坦，耗时由 graphdb 打开/discover/persist 固定开销主导，不随 dirty/deleted
文件数线性放大——批删消除了 `文件数 × remove_nodes_by_ids` 退化。dirty_1/dirty_5 区间
偏宽（10 samples 仅约 20 iterations），只作趋势参考。

## 验收记录

### M54（Tasks 1–8，2026-07-18）

- **fixture 通路**：`FixtureMetaFilesChangeSource`（同毫秒双更新 + 删除 + 同 file_id 改名）→ `apply_changeset_to_mirror` → `ProjectIndexer::prepare` → 候选 read model → graph+checkpoint 单 txn 提交 → `install_replacement` → best-effort re-warm，端到端测试 `m54_diff_refresh_orchestrator_end_to_end_equivalence` 通过。
- **原子 checkpoint**：graph、file states、checkpoint（`META_TABLE`）同一 write transaction；提交前注入失败时 graph 与 checkpoint 均保持旧值，reopen 同见新图与新 checkpoint；checkpoint-only commit（无 dirty）也落 META_TABLE；旧库无 checkpoint 时 `load_diff_refresh_checkpoint` 返回 `None`。
- **warm-failure cold fallback**：注入单页 warm 失败后 checkpoint 已推进、该页无旧 cache、cold 查询正确、未受影响页 cache 仍命中；`warm_failures` 逐页取自 `BatchWarmReport.pages[*].success`，不凭外层 `Result::Ok`。
- **dirty/deleted 曲线**：见上文「M54 Task 5 批删规模曲线」；多 dirty/deleted 文件每轮 apply 最多一次 `remove_nodes_by_ids`。
- **差量真实性声明**：M54 验收未使用全量 `session-refresh` 或全 runtime reload 冒充三层差量；全部通路测试只拉变更文件（fetch 次数 == 非删除事件数）、只 dirty/失效相关节点与页面。
- **测试基线**：`cargo test --features cli-local m54` 27 个测试全绿（types 5 + fixture 4 + mirror 4 + batch_delete 5 + prepare 4 + orchestrator 5）；`cargo check --features cli-local` 干净；runtime / scanner / indexer / graph_store 等受影响面回归全绿。

### M54 遗留问题（输入 M55）

- **路径搜索对图构建顺序敏感**：候选图经 remove+re-add 后与 redb hydrate 图的边迭代顺序不同，path search 在 budget 内选出的路径集可能不同（观测到长度 5 vs 长度 2）。e2e 等价断言因此采用 `stable_semantic_view`（剔除 `primary_paths`/`related_context` 等路径搜索段后逐 key 比较，其余段严格相等）。**M55 失效/等价语义任务需决定**：路径搜索确定性化，或把该比较口径固化为等价定义。
- `GraphDB::open` 的 `ensure_redb_tables` 每次 open 提交建表 txn，「prepare 不写盘」按逻辑状态等价（file states 全等 + 节点/边数不变）断言；如需严格零写入 open，另开任务。

### M55（Tasks 9–12，2026-07-18）

- **真源字段表 / 录制 fixture / 错误码**：见上文「M55 Task 9 真源实现记录」；录制 fixture `tests/fixtures/diff_refresh/bi_active_and_deleted.json` 覆盖 active/deleted 同毫秒、parentDir+name 兜底、回收站原始 file_id 重复（去重键为唯一 event_id/uuid，不用 file_id）。
- **孤立单变更**：单个非空事件立即进入 mirror/index，不等待后续事件，无最小数量阈值（`m55_meta_files_poll_single_orphan_change_returned_immediately`）。
- **退避**：`BackoffSchedule` 1/2/4/8/16/32/60 秒封顶、成功 poll 清零，只对连续网络错误生效；数据错误（INVALID_*）直接失败上报不退避；真实 tick 循环未接（上层组装）。
- **双 cursor 正确性**：交错竞态测试证明 deleted cursor 不推进 active cursor；active 快照后发生的删除由下一次 deleted poll 捕获；bootstrap 固定 deleted→active 顺序，历史回收站只初始化 cursor 不重放。
- **依赖索引覆盖**：`PageDependencyIndex` 现覆盖 page logic 收集范围内的 Model/Field（含 DataFlow 输入/内部/输出结构边与 FieldAlias）；共享 Model 变化 → 多页失效（`assert_eq!` 集合断言）；索引降级时保守失效全部 warm 页并上报 `page_dep_index_coverage=partial`。
- **统一命令面**：CLI one-shot `--session-diff-refresh <ID>` 与 stdio `--runtime-session-id <ID>` 绑定 `DiffRefreshRuntimeContext`（单次登录、cookie jar 共享、凭据不落盘）；`diff_refresh` 注册 `mutates_runtime=true`，未绑定返回 `DIFF_REFRESH_CONTEXT_REQUIRED`；graph-path 不一致返回 `DIFF_REFRESH_GRAPH_PATH_MISMATCH`；secret-redaction 测试覆盖请求/响应/错误消息。
- **等价语义决定（M54 遗留问题收口）**：路径搜索确定性化**不在本期做**；e2e 等价定义为 `stable_semantic_view`（剔除路径搜索段后逐 key 严格相等），路径搜索对边迭代顺序的敏感性另开任务跟进。
- **已知保守行为**：fixture/真实项目中不同页面的同名本地模型（如都叫 `model1`）在图中合并为同一节点，会导致跨页保守（更大范围）失效——方向安全，不影响正确性。
- **测试基线**：`cargo test --features cli-local m55` 18 个全绿（meta_files 13 + page_dependency 5）、m54 27 个全绿、stdio 35 个、test_cli_runtime 9 个、`--lib` 114 个回归全绿；`cargo check --features cli-local` 干净。
- **真机 BI 验收**：需项目级 recycle-bin 可见性账号（404/403 分别记 `DELETE_CHANGE_SOURCE_UNAVAILABLE`/`FORBIDDEN`，不得记为真源验收通过）；本环境无账号，真机验收另计。

### M55 真机验收（2026-07-18，autocrm-test.xiaoshouyi.com / xiaoshouyi 项目）

环境：M40 沿用测试环境；账号 haocheng.wu（recycle-bin 可见性确认：getRecyclebinFiles 200，1952 条真实墓碑）。session `m55-smoke` 全量同步 1303 个可分析文件（discovered 2182、skipped 879 非 spg/tbl）、graph.redb 77077 节点。

**录制 fixture 核对（先行）**：真实 getFileDescendant 信封 `{project,module,files,users}`（非裸数组）、字段全小写 camelCase（`id/parentDir/name/type/modifyTime/revision`，非 spec 假设的大写）；回收站同小写，无缺失 uuid/deleteTime/id，deleteTime 为 Unix 毫秒，同一 file_id 最多 4 条墓碑。固化 `tests/fixtures/diff_refresh/bi_real_active_app.json`（15 条）与 `bi_real_recyclebin.json`（46 条全 22 类型 + 重复 id 组 + 文件夹墓碑），`tests/m55_bi_real_fixture_tests.rs` 5 个映射测试。

**真 bug（已修，commit 2a68cdb）**：非 spg/tbl 条目（docx/ts/json，真实清单中大量存在）修复前会生成 Unknown 事件并被 mirror 抓取污染，若缺 revision/modifyTime 还会以 `INVALID_ACTIVE_CHANGE_EVENT` 拖垮整次 poll。修复：source 侧 `is_analyzable` 在校验前过滤（type 优先、扩展名兜底），事件域对齐 manifest（spg/tbl）；**cursor boundary 不被过滤收窄**（覆盖全部非文件夹条目与全部墓碑），防止旧墓碑/docx 在后续 poll 重放。

**one-shot 验收轮次**（`--session-diff-refresh m55-smoke`，release）：

| 轮 | 操作 | change_count | 关键断言 |
|----|------|--------------|----------|
| 1 | 无变更（bootstrap） | 0 | 无漂移，checkpoint 持久化（P2 修复：空 bootstrap 也落 cursor） |
| 2 | createFile 新建 m55_accept.spg | 1 | 只 fetch 新文件（mirror 57ms）、双 cursor 初始化、invalidated = 该页 |
| 3 | modifyFile（value =1→=2） | 1 | poll 路径、event_id revision 0→1、mirror 字节验证 `=2`、commit 29ms（M56 delta） |
| 4 | rename → m55_accept_renamed.spg | 1 | mirror 只余新路径、旧路径删除、invalidated = 新页 |
| 5 | delete（进回收站） | 1 | deleted cursor 推进到新墓碑 uuid、active cursor 不动、manifest `deleted: true` |
| 6 | 无变更（poll） | 0 | checkpoint 原样保持、全程零写（各 stage 0ms），无事件丢失/重复 |

**观测记录**：
- one-shot 每轮 read_model 冷重建 ~292s（release，77k 节点）——这是 one-shot 固有成本；stdio LongLived 模式下由 cache-preserving swap 摊销，不属回归。
- bootstrap 空轮现在持久化 checkpoint-only commit（P2 修复 2026-07-22），下次走 poll 而非重新 bootstrap。
- 变更操作（create/modify/rename/delete）全部在 `/xiaoshouyi/test/` 沙盒目录完成，验收文件已删除（在回收站），未触碰其它项目（AAA「需求描述勿动」、sysdata 未动）。
- stdio 长驻模式未在真机跑（绑定通路已有 bound-context 测试覆盖）；等价语义以 mock e2e 的 `stable_semantic_view` 测试 + 真机每轮 invalidated_pages/mirror 字节/manifest 状态精确断言代替全量重建对比。

### M56（Tasks 13–16，2026-07-18）

- **基线（Task 13）**：scanner dirty 恒 remove+re-add → 恒 `topology_dirty` → edges/file_states/v2 shadow 全量重写（三类合计 >70ms），metadata-only 与 topology dirty 行为相同，v2 meta patch 对 scanner 路径不可达。见「M56 Task 13 topology-dirty 持久化基线」。
- **真增量 persist（Task 14）**：`IndexCommit.delta: Option<IndexDelta>`（dirty_edges/removed_edge_keys/changed_file_states/removed_file_paths），scanner 在 apply 前后收集 incident edge keys/edges，persist 只消费不扫全图；delta 路径单 write txn 只删/写受影响 keys + checkpoint，topology dirty 只置 `V2ShadowState::Stale` 不写 v2 blobs；显式全量路径（初始构建）写 Current v2；`open_inner` 跳过 Stale v2 走 v1 hydrate，旧库无 shadow 记录视为 Current（向后兼容）。
- **验收门达成**：小规模 topology dirty delta commit ≈ 24ms vs 全量 105ms（约 4.3×），bytes_written 降到 KB 级；reopen 等价（v1 hydrate 与完整 rebuild 集合相等）；见「M56 Task 15 增量 persist 规模曲线」。
- **PersistReport**：新建于 `src/graph_store.rs`（dirty_nodes/dirty_edges/changed_file_states/bytes_written/commit_ms/full_rewrite/v2_shadow_state），由 `GraphDB::persist_commit` 返回；`IndexStateStore::persist_index` 签名不变。
- **测试基线（Task 16 终验）**：`cargo test --features cli-local m56` 8 个全绿（incremental_persist 5 + persist_report 3）、m54 27 个、m55 18 个全绿；`cargo check --features cli-local` 干净。spec/plan/journal 禁词扫描（TBD/TODO/阈值饿死/warm 失败不推进/既有 v2 incremental）无命中。
- **遗留小项**：orchestrator/stdio adapter 上报 PersistReport 成本未接（后续小任务）；路径搜索确定性化另开任务（见 M55 验收）。

### 冷脸验收修复（2026-07-18 第二轮 review）

独立验收后发现并已修：

- **P0 删除节点未参与失效（commit bd57421）**：orchestrator 原只把 `dirty_node_ids` 传给 `prepare_replacement`，纯删除 ChangeSet 时失效集为空，被删页/依赖已删 .tbl 的页 warm cache 可继续命中。修复：`dirty ∪ deleted` 一并传入（旧∪新索引并集语义下，旧索引能命中被删节点原所属页）；并修复次生问题——re-warm 目标按候选图过滤，否则被删页/改名旧路径页失效后会被 re-warm 立刻复活。e2e 钉死：删除页 cache 摘除、共享 .tbl 删除多页失效、改名旧路径摘除（`tests/m55_delete_invalidation_tests.rs`）。
- **P1 v2 永久 Stale（commit 78a845b）**：真图测量（77077 节点，release）：v1 hydrate 1604ms、v2 hydrate 1408ms、全量重建 v2 8575ms——每轮重建约 44 轮才回本。采用阈值策略：stale 期间累计受影响节点 keys ≥ 1024 时在 persist 同一 write txn 内重建 v2 Current 并清零计数；否则保持 Stale + 计数入 v2 meta。失败保持 Stale 安全降级。注意：Task 15 曲线 1000/10000 档现在含阈值重建成本，数字不再与原表可比（预期演进）。

### 冷脸验收 follow-up 修复（2026-07-22）

以下 follow-up 项已全部修复，测试覆盖于 `tests/m54_m56_followup_tests.rs`（7 个测试全绿）：

| 项 | 级别 | 修复 |
|----|------|------|
| 每 tick 全量重建 Dense/Facts/PageDep | P1 | **deferred**：另开里程碑做派生索引增量更新；LongLived warm 路径不付此成本 |
| `remove_nodes_by_ids` 内部仍 O(V+E) 重建 | P1 | **已修**：改为 petgraph `remove_node` in-place 删除 + swap_remove 索引修复，O(k×avg_degree) |
| graph lock 只覆盖 open，mutate→persist 无锁 | P1 | **已修**：`persist_internal` 全程持 `GraphDbLockGuard`，覆盖预读→写入→提交 |
| mirror 失败时内存 manifest 已改、磁盘滞后 | P2 | **已修**：`apply_changeset_to_mirror` 在入口快照 manifest，任一事件失败回滚到快照 |
| 空 bootstrap 不落 checkpoint | P2 | **已修**：首次 bootstrap 空结果时持久化 checkpoint-only commit，下次走 poll |
| poll vs bootstrap 游标语义文档偏宽泛 | P2 | **已修**：见下文语义声明 |
| v2 hydrate 失败静默 fallback v1 | P2 | **已修**：`open_inner` 捕获 v2 hydrate 错误并存入 `v2_hydrate_warning`，新增 `v2_hydrate_warning()` 访问器 |
| `full_fallback` 命名：delta=None 空 persist 标 true | P2 | **已修**：改名 `full_rewrite`，空提交标 false |
| 阈值 ==1024 等值边界未测 | P2 | **已修**：新增 `followup_v2_threshold_exact_1024_triggers_rebuild` 和 `followup_v2_threshold_below_1024_stays_stale` 测试 |
| `invalidate_pages_for_dirty_nodes` dirty-only footgun | P2 | **已声明**：函数文档注释标明「只传 dirty 会遗漏删除节点失效」，推荐使用 `prepare_replacement` 的 `dirty ∪ deleted` 路径 |
| rename 后 cold on-demand 语义未声明 | P2 | **已声明**：rename 新路径页走 cold on-demand 而非 re-warm，属既定语义，orchestrator 注释标明 |
| 预读→提交间竞态窗口 | P2 | **已修**：persist 全程持锁后窗口消除；残余多进程并发写由锁覆盖 |

### 语义声明（2026-07-22）

- **poll 游标语义**：`poll` 只推进 analyzable 事件（spg/tbl）；cursor boundary 覆盖全部非文件夹条目与全部墓碑，防止旧墓碑/docx 在后续 poll 重放。非 analyzable 事件（docx/ts/json）由 `is_analyzable` 在校验前过滤（commit 2a68cdb 定型）。
- **rename 语义**：rename 后旧路径页 warm cache 摘除（invalidation 覆盖旧节点），新路径页走 cold on-demand 而非 re-warm——因为新路径在旧 warm cache 中不存在。re-warm 目标按候选图过滤，被删页/改名旧路径页不会被重建 cache。
- **persist lock 语义**：`persist_internal` 在入口获取 `GraphDbLockGuard`，覆盖预读（v2 shadow state）、写入（redb write txn）、提交全过程。`GraphDbLockGuard::drop` 释放锁文件。并发 CLI/--build-graph 在 persist 期间被阻塞。

### 冷脸验收 follow-up（原始记录，不挡合入）

| 项 | 级别 | 处置 |
|----|------|------|
| 每 tick 全量重建 Dense/Facts/PageDep（read_model 冷重建 release ~292s/77k 节点，主导 tick 成本） | P1 | 另开里程碑做派生索引增量更新；当前 one-shot 固有问题，LongLived warm 路径不付此成本 |
| `remove_nodes_by_ids` 内部仍 O(V+E) 重建（批删只合并了调用次数） | P1 | follow-up：redb 层真增量 node/edge 删除 |
| graph lock 只覆盖 open 阶段，mutate→persist 无锁；delta 写下并发 CLI/--build-graph 可能拼出不一致边集 | P1 | follow-up：锁覆盖 persist 或单写者约束文档化 |
| mirror 失败时内存 manifest 已改、磁盘滞后 | P2 | follow-up |
| 空 bootstrap 不落 checkpoint，下次重新全量扫（真机观测 ~2s/轮） | P2 | follow-up：空 bootstrap 也持久化 cursor |
| poll vs bootstrap 游标语义文档表述偏宽泛（poll 只推进 analyzable 事件） | P2 | 已由 2a68cdb 过滤设计定型，文档随下次契约更新 |
| v2 hydrate 失败静默 fallback v1，无诊断 | P2 | follow-up：加 diagnostic 上报 |
| `full_fallback` 命名：delta=None 的空 persist 也会标 true | P2 | follow-up：语义收窄或改名 |
| 测试缺口：mirror 成功/prepare 或 persist 失败的恢复 e2e；多进程并发写 + delta | P2 | 随对应 follow-up 补 |

**独立复验（2026-07-18，验收者只读）**：结论**可验收**，无 P0/P1 阻塞项。P0 修复逐项核验通过（旧索引时效、re-warm 过滤、测试钉死强度、无遗漏调用点）；v2 阈值策略逐项核验通过（计数路径自洽、同事务原子性、预读移位正确、无新 dead code）。复验新增 P2 观察项：

- 阈值恰好 ==1024 的等值边界用例未直接测（`>=` 两侧已覆盖）。
- `invalidate_pages_for_dirty_nodes`（runtime.rs:475）契约仍是 dirty-only，目前无生产调用者；未来调用方只传 dirty 会复现同类漏失效（footgun，后续命名/文档强化）。
- rename 后新路径页走 cold on-demand 而非 re-warm，属既定语义但未显式声明。
- 预读→提交间理论竞态窗口后果保守（多 Stale 一轮或多余重建），且多进程并发写本在锁语义 follow-up 范围。

## M56 Task 15 增量 persist 规模曲线（2026-07-18）

`PersistReport`（`src/graph_store.rs`）：`dirty_nodes/dirty_edges/changed_file_states/
bytes_written/commit_ms/full_rewrite/v2_shadow_state`，由 `GraphDB::persist_commit` 返回。
`V2ShadowState` 上移至 graph_store（ungated），graph_redb_v2 re-export。

### Criterion 曲线（链式 12000 节点 / 11999 边图；测量区只含 `persist_commit`；`CARGO_TARGET_DIR=target/criterion/redb`，sample_size=10、warm-up 1s、measurement 3s）

| bench | time（median） | 区间 |
|-------|---------------|------|
| redb_persist_curve_delta_10 | 24.672 ms | [24.237, 25.076] |
| redb_persist_curve_delta_100 | 23.238 ms | [22.404, 24.133] |
| redb_persist_curve_delta_1000 | 34.437 ms | [32.662, 36.607] |
| redb_persist_curve_delta_10000 | 74.152 ms | [68.454, 79.546] |
| redb_persist_curve_full_10（对照：全量路径） | 105.03 ms | [103.29, 106.83] |

### Report 快照（同图单发取数，`m56_persist_report_curve_snapshot` 测试 `--nocapture`）

| dirty nodes | dirty edges | bytes_written | commit_ms | full_rewrite | v2 |
|-------------|-------------|---------------|-----------|---------------|-----|
| 10 | 20 | 2,012 | 223 | false | Stale |
| 100 | 200 | 20,734 | 104 | false | Stale |
| 1,000 | 2,000 | 214,236 | 382 | false | Stale |
| 10,000 | 20,000 | 2,212,238 | 1,004 | false | Stale |

reopen hydrate 代价（12000 节点，debug 单发）：Stale 走 v1 = 750 ms，Current 走 v2 = 432 ms。

### 结论

- 小规模 topology dirty（10–100 节点）delta commit ≈ 24 ms，对照全量路径 105 ms，
  **约 4.3×**；bytes_written 从全量重写（edges+states+v2 blobs 全表）降到 KB 级。
- commit_ms 下限 ~23 ms 来自 persist 每次 `Database::create` 打开多 MB redb 文件的固定成本
  （snapshot 单发 223 ms 是冷开开销，Criterion 中位数已摊热）。
- dirty 规模到全图量级（10000/12000）时 delta（74 ms）仍低于全量（105 ms），
  差距主要来自跳过 v2 shadow 全量 blobs 写。
- 与 Task 13 基线（240 edges / 120 states 项目全量三类写 >70 ms）一致：全量重写与 dirty
  规模无关，delta 与受影响 keys 数挂钩。
- 注：本表不报告 transaction 次数（每路径恒为单 write txn），按 plan 要求记录的是
  commit 时间、写入 bytes、fallback 与 reopen hydrate 代价。

## M56 Task 13 topology-dirty 持久化基线（2026-07-18）

### 代码事实（行号级，已逐处核查）

- **v1 nodes 增量**：persist 只写 `removed_nodes`/`dirty_nodes`（`graph_redb.rs:753-770`）。
- **v1 edges 全量重写**：`edges_table.retain(|_, _| false)` 后全量重插（`graph_redb.rs:772-788`）。
- **file_states 全量重写**：`states_table.retain(|_, _| false)` 后全量重插（`graph_redb.rs:790-798`）。
- **v2 layout 选择**（`build_v2_layout_for_persist`，`graph_redb.rs:707-740`）：`can_try_incremental =
  !topology_dirty && removed_nodes.is_empty() && 0 < dirty_nodes <= 32` 且 dirty 全为既有 v2
  节点时走 `patch_v2_layout_node_meta`，否则 `build_v2_layout` 全量重建。
- **topology_dirty 置位**：`remove_nodes_by_ids` 恒置位（`graph_redb.rs:659`）、新节点 upsert
  （`:602`）、新边 add（`:648`）；既有节点 upsert 不置位（`:579-591`）。
- **关键结论**：scanner 的 dirty 应用路径（Task 5 起为 `apply_incremental_changes` 的
  remove+re-add）**恒置 topology_dirty**，因此 metadata-only 与 topology dirty 在持久化行为上
  完全相同——v2 恒 full layout、v1 edges/file_states 恒全量重写；`patch_v2_layout_node_meta`
  只对「不 remove 的既有节点 upsert」可达，scanner 路径永远走不到。

### 一次性观测（临时 eprintln，已还原未提交；合成 120 SPG / 240 edges / 120 file states）

| 场景 | v2 layout | nodes | edges 全量重写 | states 全量重写 | v2 shadow 写 |
|------|-----------|-------|---------------|----------------|--------------|
| metadata-only 1 SPG（scanner） | full_rebuild（removed=1, dirty=3） | 0.25 ms | 29.27 ms | 13.77 ms | 30.66 ms |
| topology 1 SPG | full_rebuild（removed=1, dirty=3） | 0.24 ms | 28.90 ms | 13.72 ms | 31.51 ms |
| topology 20 SPG | full_rebuild（removed=20, dirty=60） | 2.06 ms | 28.97 ms | 13.79 ms | 32.16 ms |
| 既有节点 upsert（探针） | **meta_patch**（dirty=1） | 0.13 ms | 29.12 ms | 13.98 ms | 31.64 ms |

即使 meta_patch 也只省 layout 构建（patch 自身还要读旧 layout 逐节点补 meta，实测 33.83 ms
比 full build 4.5 ms 更贵），edges/file_states/v2 shadow 写一概不免。

### 基线 bench（完整增量 scan；合成 120 SPG；sample_size=10、warm-up 1s、measurement 3s、`CARGO_TARGET_DIR=target/criterion/redb`）

| bench | time（median） | 区间 |
|-------|---------------|------|
| redb_topology_baseline_meta_only_1 | 77.492 ms | [76.563, 78.691] |
| redb_topology_baseline_topology_1 | 80.154 ms | [78.924, 81.638] |
| redb_topology_baseline_topology_20 | 79.986 ms | [79.298, 80.805] |

三组耗时在噪声内相等：commit 由全量 durable rewrite 主导，与 dirty 规模/类型无关。

### M56 验收门（固定）

1. 小规模 topology dirty（单 SPG 级别）**不允许全量 durable rewrite**——Task 14 后
   edges/file_states/v2 shadow 写入必须与受影响 key 数挂钩，而不是全表。
2. **不得**以「现有 v2 meta patch 已足够」跳过 Task 14：patch 当前对 scanner 路径不可达，
   且即便可达也不减少 edges/file_states/v2 shadow 的全量写。
3. Task 14/15 的对比基准即本表（topology_1 ≈ 80 ms、topology_20 ≈ 80 ms，
   其中 edges/states/shadow 三类全量写合计 >70 ms）。

## M55 Task 9 真源实现记录（2026-07-18）

- `BiMetaFilesChangeSource`（`src/diff_refresh/bi_meta_files_source.rs`）：活跃清单复用
  `GET /api/meta/services/getFileDescendant/{project_ref}`（含 `list_metafiles` fallback），
  删除清单新增 `POST /api/meta/file/getRecyclebinFiles`（body 固定
  `{"projectName": project_ref, "recur": true}`，见 `src/session/reqwest_provider.rs`）。
- 字段表同上文「真源字段语义」：active `id/path|parentDir+name/type/modifyTime/revision` →
  `active:<id>:<revision>`；deleted `FILE_ID/PARENT_DIR/NAME/TYPE/deleteTime/uuid` →
  `deleted:<uuid>`；时间统一 Unix 毫秒。
- 错误码：active 字段缺失/非法 → `INVALID_ACTIVE_CHANGE_EVENT`；删除缺失 uuid/deleteTime →
  `INVALID_DELETE_EVENT_ID`（任一错误使整次 poll 失败、双 cursor 不推进）；bootstrap 字段错误 →
  `DIFF_REFRESH_BOOTSTRAP_FAILED`；recyclebin 404/403 →
  `DELETE_CHANGE_SOURCE_UNAVAILABLE` / `DELETE_CHANGE_SOURCE_FORBIDDEN`。
- 传输注入：`BiMetaFilesTransport` trait；测试用脚本化 test double + 本机回环 HTTP
  （不打外网）；退避计算器 `BackoffSchedule`（1/2/4/8/16/32/60 封顶，成功 poll 清零）
  放 source 侧，tick 循环由上层组装。
- 录制 fixture：`tests/fixtures/diff_refresh/bi_active_and_deleted.json`
  （active/deleted 同毫秒、parentDir+name 兜底、回收站原始 file_id 重复）。
- 测试：`tests/m55_meta_files_source_tests.rs` 13 个全绿（含孤立单变更、交错竞态、
  bootstrap 交错与 404/403/200 回环 HTTP）；`cargo test --features cli-local m54`
  27 个与 session 50 个回归全绿；`cargo check --features cli-local` 干净。
