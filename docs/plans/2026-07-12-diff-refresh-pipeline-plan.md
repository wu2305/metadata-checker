# 差量刷新一条龙实施计划（M54 / M55 / M56）

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 LongLived 三层差量刷新：`META_FILES`/fixture → 文件 mirror 差量 → 候选图/read model → graph+watermark 原子提交 → cache-preserving runtime swap → selective re-warm，并完成真正增量的 redb 写路径。

**Architecture:** `MetaFilesChangeSource` 产出使用毫秒时间戳的 `ChangeSet`；`FileMirrorSync` 按稳定 `file_id` 处理更新、删除和改名；`ProjectIndexer` 先准备候选图与完整 dirty/deleted IDs。候选 read model 构建成功后，graph/file states/watermark 在同一 redb transaction 提交，runtime 随后只做内存 swap；re-warm 是 best-effort 性能动作。详见 [spec](../specs/2026-07-12-diff-refresh-pipeline-design.md)。

**Tech Stack:** Rust 2024、既有 session/remote provider、scanner/indexer、GraphRuntime、redb、PerfProfile。

## Global Constraints

- Rust/WASM core、CLI、stdio、图查询与解析保持 Rust 实现。
- 不新增 npm 依赖或 Rust crate；复用现有 redb、serde 与 session provider。
- 不使用 `#[allow(dead_code)]`，可恢复错误使用 `anyhow::Result` 与 `.with_context()`。
- 新增模块级、类型级、函数级注释使用中文；文档注释放在 `#[derive(...)]` 之前。
- 每个非空 `ChangeSet` 必须处理，不使用会饿死孤立变更的最小数量阈值。
- `main` 只接受 PR merge；M54、M55、M56 按 Phase 分 PR，并在每个可验收任务后提交。

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `src/diff_refresh/mod.rs` | 模块导出 |
| `src/diff_refresh/types.rs` | active/deleted 双 cursor、变更事件与报告 |
| `src/diff_refresh/source.rs` | `MetaFilesChangeSource` trait |
| `src/diff_refresh/fixture_source.rs` | M54 fixture 实现 |
| `src/diff_refresh/mirror.rs` | 调用 session sync 处理更新、删除、改名 |
| `src/diff_refresh/orchestrator.rs` | 候选准备、提交、runtime swap、best-effort warm |
| `src/scanner/indexer.rs` | 候选图准备与完整 dirty/deleted IDs |
| `src/graph_store.rs` | `DiffRefreshCheckpoint` 与 `IndexCommit` 提交契约 |
| `src/graph_redb.rs` | graph/file states/watermark 同事务；M56 v1 delta |
| `src/graph_redb_v2.rs` | M56 shadow current/stale 状态与显式全量重建 |
| `src/runtime.rs` | replacement read model 与 cache-preserving swap |
| `src/query/page_logic/page_diff.rs` | M55 依赖索引覆盖扩展 |
| `src/session/reqwest_provider.rs` | M55 活跃/删除清单查询 |
| `src/tool_contract.rs` / `src/stdio_server.rs` | `diff_refresh` 统一命令契约 |
| `src/cli.rs` / `src/main.rs` | CLI adapter |
| `tests/m54_diff_refresh_*.rs` | M54 通路、提交与失败语义 |
| `tests/m55_*.rs` | 真源事件归并与索引覆盖 |
| `tests/m56_*.rs` | 真增量 persist、reopen 等价与报告 |
| `tests/fixtures/diff_refresh/` | 活跃、删除、改名 fixture |
| `docs/milestones/performance/m54-diff-refresh-pipeline.md` | M54–M56 journal |

---

## Phase M54 — 通路

### Task 1: INDEX / journal 登记

**Files:**
- Modify: `docs/milestones/INDEX.md`
- Modify: `docs/milestones/performance/m54-diff-refresh-pipeline.md`
- Modify: `docs/README.md`

- [ ] **Step 1:** 确认 INDEX 含 M54/M55/M56（performance）与 M59（formats deferred）。
- [ ] **Step 2:** 实现开始时把 journal 状态从 `planned` 改为 `active`，M54 合并后只把 M54 改为 `done`。
- [ ] **Step 3:** 运行 `rg -n "M54|M55|M56|M59" docs/README.md docs/milestones/INDEX.md docs/milestones/performance/m54-diff-refresh-pipeline.md`，确认四个编号均有唯一含义。

### Task 2: 固定真源字段语义并实现 typed ChangeSet

**Files:**
- Create: `src/diff_refresh/mod.rs`
- Create: `src/diff_refresh/types.rs`
- Create: `src/diff_refresh/source.rs`
- Modify: `src/lib.rs`
- Modify: `docs/milestones/performance/m54-diff-refresh-pipeline.md`
- Test: `tests/m54_diff_refresh_types_tests.rs`

**Interfaces:**
- Produces: `SourceCursor { updated_at_ms: u64, boundary_event_ids: Vec<String> }`
- Produces: `MetaFilesWatermark { active: SourceCursor, deleted: SourceCursor }`
- Produces: `ChangedRemoteFile { event_id, file_id, source_path, previous_source_path, content_type, updated_at_ms, deleted }`
- Produces: `MetaFilesChangeSource::poll(&MetaFilesWatermark) -> Result<ChangeSet>`
- Produces: `MetaFilesChangeSource::bootstrap(&SessionManifest) -> Result<ChangeSet>`，供无 checkpoint 的既有 session 安全初始化。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn m54_cursor_accepts_unseen_same_millisecond_event() {
    let cursor = SourceCursor {
        updated_at_ms: 10,
        boundary_event_ids: vec!["active:a:1".into()],
    };
    assert_eq!(cursor_accepts(&cursor, 10, "active:b:1"), true);
    assert_eq!(cursor_accepts(&cursor, 10, "active:a:1"), false);
}

#[test]
fn m54_change_count_matches_changed_length() {
    let changes = fixture_change_set();
    assert_eq!(changes.change_count, changes.changed.len());
}
```

- [ ] **Step 2:** 运行 `cargo test m54_cursor_accepts --features cli-local`，确认因模块未定义而失败。
- [ ] **Step 3:** 实现上述类型与 `cursor_accepts`；`boundary_event_ids` 在构造和序列化前排序去重，`ChangeSet.change_count = changed.len()`。
- [ ] **Step 4:** 在 journal 字段表记录：活跃事件使用 `ID/PARENT_DIR/NAME/TYPE/modifyTime/revision` 与 `event_id=active:<file_id>:<revision>`；删除事件使用 `FILE_ID/PARENT_DIR/NAME/TYPE/deleteTime/uuid` 与 `event_id=deleted:<uuid>`；时间统一为 Unix 毫秒。
- [ ] **Step 5:** 运行 `cargo test m54_diff_refresh_types --features cli-local`，期望 PASS。
- [ ] **Step 6:** 提交 `feat: define typed diff refresh change contract`。

### Task 3: Fixture ChangeSource 覆盖更新、删除和改名

**Files:**
- Create: `src/diff_refresh/fixture_source.rs`
- Create: `tests/fixtures/diff_refresh/meta_files_delta.json`
- Test: `tests/m54_diff_refresh_fixture_tests.rs`

**Interfaces:**
- Consumes: Task 2 的 `MetaFilesChangeSource`。
- Produces: active/deleted 各自推进 cursor、合并后按 `(updated_at_ms, event_id)` 严格升序且无重复的 `ChangeSet`。

- [ ] **Step 1:** fixture 写入同毫秒两个更新、一个删除和一个同 `file_id` 改名事件。
- [ ] **Step 2:** 写测试断言 `poll` 只返回严格大于 `since` 的事件、顺序稳定、`change_count == changed.len()`。
- [ ] **Step 3:** 实现 JSON 反序列化、过滤、排序与去重；不得用字符串比较时间；去重键使用唯一 `event_id`，不能用可重复的 recycle-bin `file_id`。
- [ ] **Step 4:** 写 fixture bootstrap 测试：manifest 中一个未变文件、一个 revision 变化文件、一个远端已缺失文件；结果只含后两者，并分别把 active/deleted cursor 初始化到各自快照边界。（「未变」指对**已成功入图**的文件：M59-2 B 起 `unchanged` 还要求 `!needs_index_retry()`，`hash` 与 `indexed_hash` 失配的文件即使 revision 未变也重投，见 `src/session/manifest.rs`。）
- [ ] **Step 5:** 运行 `cargo test m54_diff_refresh_fixture --features cli-local`，期望 PASS。
- [ ] **Step 6:** 提交 `test: add diff refresh source fixtures`。

### Task 4: FileMirrorSync 差量写、删除与改名

**Files:**
- Create: `src/diff_refresh/mirror.rs`
- Modify: `src/session/sync.rs`
- Modify: `src/session/remote_sync.rs`
- Test: `tests/m54_diff_refresh_mirror_tests.rs`

**Interfaces:**
- Consumes: `ChangeSet`、`SessionManifest.files`、既有 remote content provider。
- Produces: `MirrorApplyReport { written, deleted, renamed, unchanged }`。

- [ ] **Step 1:** 写测试：仅 page_a 更新时 page_b 字节不变，fetch 次数等于非删除事件数。
- [ ] **Step 2:** 写测试：删除事件移除 mirror 文件并把 manifest 记录标为 deleted。
- [ ] **Step 3:** 写测试：同 `file_id` 从旧路径移动到新路径后，新路径内容存在、旧路径不存在、manifest 只保留新路径。
- [ ] **Step 4:** 复用 `sync.rs` 的 per-file atomic write 实现 `apply_changeset_to_mirror`；先写新路径，再删除同 `file_id` 的旧路径，防止失败时丢失唯一副本。
- [ ] **Step 5:** 运行 `cargo test m54_diff_refresh_mirror --features cli-local`，期望 PASS。
- [ ] **Step 6:** 提交 `feat: apply remote file deltas to session mirror`。

### Task 5: 多 dirty/deleted 文件只做一次图批删

**Files:**
- Modify: `src/scanner/indexer.rs`
- Test: `tests/m54_incremental_batch_delete_tests.rs`
- Bench: `benches/redb_persistence_bench.rs`

**Interfaces:**
- Consumes: `ParsedGraphUpdate.previous_node_ids` 与 `DeletedFile`。
- Produces: 每轮 apply 最多一次 `remove_nodes_by_ids` 调用。

- [ ] **Step 1:** 用 test double 写失败测试，断言 20 个 dirty 文件只调用一次 `remove_nodes_by_ids`。
- [ ] **Step 2:** 写失败测试，断言 20 个 deleted 文件也只调用一次 `remove_nodes_by_ids`。
- [ ] **Step 3:** 合并 dirty previous IDs 与 deleted IDs，去重后一次删除，再应用所有新增节点并更新 file states。
- [ ] **Step 4:** 运行 `cargo test m54_incremental_batch_delete --features cli-local`，期望 PASS。
- [ ] **Step 5:** 采集 dirty files `1/5/20/100` 与 deleted files `1/5/20` 曲线并写入 journal。
- [ ] **Step 6:** 提交 `perf: batch node deletes across changed files`。

### Task 6: 候选图/read model 与 graph+watermark 原子提交

**Files:**
- Modify: `src/graph_store.rs`
- Modify: `src/scanner/indexer.rs`
- Modify: `src/graph_redb.rs`
- Modify: `src/memory_graph_store.rs`
- Modify: `src/runtime.rs`
- Test: `tests/m54_diff_refresh_prepare_tests.rs`

**Interfaces:**
- Produces: `DiffRefreshCheckpoint { active: SourceCursor, deleted: SourceCursor }`。
- Produces: `PreparedIndexUpdate { graph, commit, dirty_node_ids, deleted_node_ids }`；prepare 不写盘。
- Produces: `GraphRuntime::prepare_replacement(&self, candidate: &GraphDB, dirty_ids: &[String]) -> Result<PreparedRuntimeReadModel>`。
- Produces: `GraphRuntime::install_replacement(&mut self, candidate: GraphDB, read_model: PreparedRuntimeReadModel)`；该方法只 move 已准备对象，不返回 `Result`。

- [ ] **Step 1:** 写测试断言 `ProjectIndexer::prepare` 返回完整 dirty/deleted IDs，但 graphdb 文件指纹不变。
- [ ] **Step 2:** 扩展 `IndexCommit`，加入 `checkpoint: Option<DiffRefreshCheckpoint>`；所有非 diff-refresh 调用显式传 `None`。
- [ ] **Step 3:** 实现 `ProjectIndexer::prepare(project_dir, db_path)`：打开候选 `GraphDB`、完成 diff/parse/apply、构造 commit，不调用 `persist_index`。
- [ ] **Step 4:** 写 runtime 测试：replacement 重新构建 dense graph、availability facts、PageDependencyIndex；旧、新索引计算出的 affected pages 取并集；只复制未受影响页的 warm cache。
- [ ] **Step 5:** 修改 redb persist，使 graph、file states、`META_TABLE` 中 checkpoint 在同一 write transaction 提交；增加 `load_diff_refresh_checkpoint`。
- [ ] **Step 6:** 写 migration 测试：旧 graph 无 checkpoint 时 `load_diff_refresh_checkpoint` 返回 `None`；checkpoint-only commit 即使 graph 无 dirty 也必须写入 `META_TABLE`。
- [ ] **Step 7:** 写故障测试：提交前注入失败时 graph 和 checkpoint 均保持旧值；成功提交后 reopen 同时看到新图和新 checkpoint。
- [ ] **Step 8:** 运行 `cargo test m54_diff_refresh_prepare --features cli-local` 与 `cargo test runtime --features cli-local`，期望 PASS。
- [ ] **Step 9:** 提交 `feat: prepare atomic graph and runtime refresh`。

### Task 7: DiffRefreshOrchestrator 与 best-effort re-warm

**Files:**
- Create: `src/diff_refresh/orchestrator.rs`
- Modify: `src/runtime.rs`
- Test: `tests/m54_diff_refresh_orchestrator_tests.rs`

**Interfaces:**
- Consumes: Tasks 2–6。
- Produces: `DiffRefreshReport { change_count, invalidated_pages, warm_failures, checkpoint, last_poll_at, timing }`。

- [ ] **Step 1:** 写端到端失败测试：page_a 变化后结果与同 mirror 冷 load+warm canonical JSON 相等，page_b cache 内容保持相同。
- [ ] **Step 2:** 写测试：空 `ChangeSet` 不写 graph/checkpoint，只更新 report 中 `last_poll_at`。
- [ ] **Step 3:** 写首次调用测试：无 checkpoint 时调用 `source.bootstrap(manifest)`，只拉 manifest 与 active snapshot 不一致的文件，删除远端已缺失文件，并原子提交配对 checkpoint。
- [ ] **Step 4:** 写测试：mirror 或 prepare 失败时 checkpoint 不推进，旧 runtime 查询仍成功。
- [ ] **Step 5:** 写测试：注入 page_a warm 失败后 checkpoint 已推进、page_a 无旧 cache、page_a cold 查询正确、page_b cache 仍命中。
- [ ] **Step 6:** 严格按以下顺序实现：load checkpoint；缺失则 bootstrap，否则 poll；随后 mirror → prepare candidate graph → prepare replacement read model/cache → persist graph+checkpoint → install replacement → best-effort batch warm。
- [ ] **Step 7:** 检查 `BatchWarmReport.pages[*].success` 生成 `warm_failures`；不得仅凭外层 `Result::Ok` 判定全部 warm 成功。
- [ ] **Step 8:** 记录 `poll_or_bootstrap/mirror/prepare/read_model/commit/swap/rewarm` stages。
- [ ] **Step 9:** 运行 `cargo test m54_diff_refresh_orchestrator --features cli-local`，期望 PASS。
- [ ] **Step 10:** 提交 `feat: orchestrate atomic diff refresh and rewarm`。

### Task 8: M54 收口

- [ ] **Step 1:** journal 记录 fixture 通路、原子 checkpoint、warm-failure cold fallback、dirty/deleted 曲线。
- [ ] **Step 2:** 明确记录未用全量 session refresh 或 runtime reload 冒充三层差量。
- [ ] **Step 3:** 运行 `cargo check --features cli-local` 与上述 M54 目标测试。
- [ ] **Step 4:** M54 实现 PR 内保持 `active`；合并后单独文档 PR 改 `done`。

---

## Phase M55 — 真源与依赖正确性

### Task 9: BI 活跃/删除事件 ChangeSource

**Files:**
- Create: `src/diff_refresh/bi_meta_files_source.rs`
- Modify: `src/session/reqwest_provider.rs`
- Test: `tests/m55_meta_files_source_tests.rs`
- Fixture: `tests/fixtures/diff_refresh/bi_active_and_deleted.json`
- Modify: `docs/milestones/performance/m54-diff-refresh-pipeline.md`

- [ ] **Step 1:** 活跃清单复用现有 `GET /api/meta/services/getFileDescendant/{project_ref}` 与 `list_metafiles` fallback。响应是完整数组、无分页；映射 `id`、`path`（或 `parentDir + name`）、`type`、`modifyTime: u64`、`revision`，生成 `event_id=active:<id>:<revision>`。
- [ ] **Step 2:** 删除清单新增 `POST /api/meta/file/getRecyclebinFiles`，JSON body 固定为 `{"projectName": project_ref, "recur": true}`。响应是完整 `DeletedMetaFileInfo[]`、无分页；映射原始 `id` 为 `file_id`、`deleteTime: u64` 为时间、`uuid` 生成 `event_id=deleted:<uuid>`。
- [ ] **Step 3:** active 响应只用 `checkpoint.active` 过滤并生成 `next.active`，deleted 响应只用 `checkpoint.deleted` 过滤并生成 `next.deleted`；每路接收时间更大的事件和同毫秒未出现在 `boundary_event_ids` 的事件。某路没有新事件时该路 cursor 原样保留；有新事件时 cursor 时间取该路最大事件时间，`boundary_event_ids` 保存该最大毫秒内全部已见 IDs，并在最大时间等于旧 cursor 时间时与旧边界 IDs 取并集。最后再合并两路事件排序。缺失/非法时间或 active revision 返回 `INVALID_ACTIVE_CHANGE_EVENT`，删除事件缺失 uuid 返回 `INVALID_DELETE_EVENT_ID`；任一错误使整次 poll 失败且两个 cursor 均不推进。
- [ ] **Step 4:** 写录制 fixture 测试，覆盖 active/deleted 同毫秒、recycle-bin 原始 file ID 重复、空数组、非法时间和缺失 revision/uuid；断言事件无丢失且顺序稳定。
- [ ] **Step 5:** 写孤立变更测试：只有一个非空事件时立即进入 mirror/index，不等待后续事件。
- [ ] **Step 6:** tick 侧只对连续网络错误做上限 60 秒的指数退避（1/2/4/8/16/32/60 秒）；成功 poll 立即清零失败计数。不得按 `change_count` 跳过非空集合。
- [ ] **Step 7:** M55 真源验收账号必须具有项目级 recycle-bin 可见性。HTTP 404/403 分别返回 `DELETE_CHANGE_SOURCE_UNAVAILABLE` / `DELETE_CHANGE_SOURCE_FORBIDDEN`，且不得把该环境记录为 M55 真源验收通过。
- [ ] **Step 8:** 写交错竞态测试：active 第一次快照返回后注入新的 active 事件，再让 deleted 快照返回更晚事件；提交后第二次 poll 必须仍返回该 active 事件，证明 deleted cursor 不会推进 active cursor。
- [ ] **Step 9:** 实现真实 bootstrap，固定请求顺序为 deleted 完整快照 → active 完整快照。active 按 `file_id` 对照 manifest 的 revision/mtime/path，只产出新增或不一致文件（M59-2 B 起额外产出 `!needs_index_retry()` 的文件——镜像已获取但图未成功索引，见 `src/session/manifest.rs`）；manifest 中第二个 active 快照已缺失的文件产出删除；历史 recycle-bin 仅用于初始化 deleted cursor，不逐条重放。快照不完整或字段错误返回 `DIFF_REFRESH_BOOTSTRAP_FAILED`。
- [ ] **Step 10:** 写 bootstrap 交错测试：deleted 快照返回后删除 manifest 中一个文件，再返回不含该文件的 active 快照；bootstrap 必须产出删除。另测 active 快照返回后发生删除时，本轮可不含该事件，但下一次 deleted poll 必须返回它。
- [ ] **Step 11:** 运行 `cargo test m55_meta_files --features cli-local`，期望 PASS。
- [ ] **Step 12:** 提交 `feat: poll BI metadata update and deletion events`。

### Task 10: 统一命令面

**Files:**
- Modify: `src/tool_contract.rs`
- Modify: `src/stdio_server.rs`
- Modify: `src/cli.rs`
- Modify: `src/main.rs`
- Modify: `src/session/mod.rs`
- Modify: `tests/stdio_server_tests.rs`
- Modify: `tests/regression_tests.rs`

- [ ] **Step 1:** CLI 增加 one-shot `--session-diff-refresh <ID>`；它复用现有 `--session-dir`、`--remote-username`、`--remote-password`。读取 `SessionManifest` 得到 `remote_server/project_ref/graph_db_path`，不得要求用户重复传这三个字段。
- [ ] **Step 2:** stdio 启动增加 `--runtime-session-id <ID>`。`main.rs` 在进入 `--serve-stdio` 前读取 manifest、校验 manifest `graph_db_path` 与 runtime 路径一致、构造 Task 9 的 `BiMetaFilesChangeSource`，并通过其 `ReqwestRemoteSessionProvider` 执行现有 `login(username, password, "sys")`；登录后的 cookie jar、change source 与 `SessionManager` 组成进程内 `DiffRefreshRuntimeContext`，不写入 manifest。
- [ ] **Step 3:** 在 `ToolCommand` / `ToolRegistry` 注册无 target 参数的 `diff_refresh`，标记 `mutates_runtime=true`。handler 只使用启动时绑定的 `DiffRefreshRuntimeContext`；未绑定时返回 `DIFF_REFRESH_CONTEXT_REQUIRED`。fixture 只由测试直接构造 context，不作为公开参数。
- [ ] **Step 4:** adapter 响应包含 `ok/change_count/invalidated_pages/warm_failures/checkpoint/timing`；请求、响应、Debug 与 diagnostic 均不得包含 username、password、cookie 或 access token。
- [ ] **Step 5:** 更新 exhaustive registry request fixture；增加 CLI one-shot、stdio bound context、graph-path mismatch、missing auth、missing context 和 secret-redaction 测试。
- [ ] **Step 6:** 运行 `cargo test stdio --features cli-local` 和 `cargo test test_cli_runtime --features cli-local`，期望 PASS。
- [ ] **Step 7:** 提交 `feat: expose authenticated diff refresh runtime command`。

### Task 11: PageDependencyIndex 覆盖 Model/Field/DataFlow

**Files:**
- Modify: `src/query/page_logic/page_diff.rs`
- Modify: `src/query/page_logic/graph_collect.rs`
- Test: `tests/m55_page_dependency_index_tests.rs`

- [ ] **Step 1:** 写共享 Model 测试，使用 `assert_eq!` 比较 affected page 集合与 `{page_a,page_b}`。
- [ ] **Step 2:** 分别写 Field 与 DataFlow 输出字段测试；无关页不得出现在集合中。
- [ ] **Step 3:** 扩展 build，注册 page logic 收集范围内的 Model、Field、DataFlow 反向关系。
- [ ] **Step 4:** 保留 Task 6 的旧/新索引 affected-pages 并集策略；覆盖无法证明时保守扩大 invalidation 并输出 `page_dep_index_coverage=partial`。
- [ ] **Step 5:** 更新 orchestrator 等价测试，覆盖共享 Model 变化导致多页失效。
- [ ] **Step 6:** 运行 `cargo test m55_page_dependency --features cli-local`，期望 PASS。
- [ ] **Step 7:** 提交 `feat: complete page dependency invalidation index`。

### Task 12: M55 收口

- [ ] **Step 1:** journal 写入活跃/删除字段表、录制 fixture 来源、孤立单变更、退避与索引覆盖结果。
- [ ] **Step 2:** 运行 M55 目标测试和 `cargo check --features cli-local`。
- [ ] **Step 3:** M55 合并后把 INDEX 中 M55 改为 `done`。

---

## Phase M56 — 真增量 Persist

### Task 13: 固定 topology-dirty 基线与验收门

**Files:**
- Modify: `docs/milestones/performance/m54-diff-refresh-pipeline.md`
- Bench: `benches/redb_persistence_bench.rs`

- [ ] **Step 1:** 采集 metadata-only dirty 与 topology dirty（单 SPG、20 SPG）两组 `redb_commit`；分别记录 v2 meta patch/full layout、v1 edge rewrite、file-state rewrite。
- [ ] **Step 2:** 在 journal 明确当前事实：metadata-only 可走 v2 meta patch；scanner remove/re-add 设置 `topology_dirty`，当前会全量重建 v2，并重写全部 v1 edges/file states。
- [ ] **Step 3:** M56 验收门固定为：小规模 topology dirty 不允许全量 durable rewrite；不能用“现有 v2 patch 已足够”跳过 Task 14。

### Task 14: v1 edge/file-state delta 与 v2 shadow 失效

**Files:**
- Modify: `src/graph_store.rs`
- Modify: `src/graph_redb.rs`
- Modify: `src/graph_redb_v2.rs`
- Modify: `src/scanner/indexer.rs`
- Test: `tests/m56_incremental_persist_tests.rs`

**Interfaces:**
- Extends: `IndexCommit` 携带 `dirty_edges`、`removed_edge_keys`、`changed_file_states`、`removed_file_paths`。
- Produces: `V2ShadowState::{Current, Stale}` 存入 v2 meta。

- [ ] **Step 1:** 写失败测试：单 SPG topology change 后只更新受影响 node/edge/file-state keys，未触碰无关 keys。
- [ ] **Step 2:** 写 reopen 等价测试：增量提交把 v2 shadow 标记 `Stale`，`GraphDB::open` 跳过 stale v2、从增量更新后的 v1 hydrate，结果与完整 rebuild 相等。
- [ ] **Step 3:** scanner 在 remove/re-add 时收集被删节点的 incident edge keys 与新增 edge keys；不得在 persist 阶段扫描并重写全图 edges。
- [ ] **Step 4:** redb 在一个 write transaction 中删除/写入 node、edge、file-state delta，并提交 checkpoint；topology dirty 时只把 v2 标记 stale，不写全量 v2 blobs。
- [ ] **Step 5:** 保留显式 full rebuild/compaction 路径生成 `Current` v2 shadow；普通 diff refresh 不触发该路径。
- [ ] **Step 6:** 运行 `cargo test m56_incremental_persist --features cli-local`，期望 PASS。
- [ ] **Step 7:** 提交 `perf: persist topology changes as graph deltas`。

### Task 15: PersistReport 与规模曲线

**Files:**
- Modify: `src/graph_store.rs`
- Modify: `src/graph_redb.rs`
- Modify: `src/perf_report.rs`
- Test: `tests/m56_persist_report_tests.rs`
- Bench: `benches/redb_persistence_bench.rs`

- [ ] **Step 1:** `PersistReport` 增加 `dirty_nodes/dirty_edges/changed_file_states/bytes_written/commit_ms/full_fallback/v2_shadow_state`。
- [ ] **Step 2:** 写测试断言小 topology dirty 的 `full_fallback == false`、`v2_shadow_state == Stale`，显式 rebuild 的 `full_fallback == true`、`v2_shadow_state == Current`。
- [ ] **Step 3:** 采集 dirty nodes `10/100/1000/10000` 曲线，并与 Task 13 基线对比。
- [ ] **Step 4:** 运行 `cargo test m56_persist_report --features cli-local` 与对应 Criterion bench。
- [ ] **Step 5:** journal 记录 commit 时间、写入 bytes、fallback 与 reopen hydrate 代价，禁止只报告 transaction 次数。
- [ ] **Step 6:** 提交 `perf: report incremental graph persist costs`。

### Task 16: 全线文档收口

**Files:**
- Modify: `docs/milestones/INDEX.md`
- Modify: `docs/milestones/performance/m54-diff-refresh-pipeline.md`
- Modify: `docs/README.md`
- Modify: `docs/archive/roadmap/m46-local-cli-remote-metadata-index-and.md`

- [ ] **Step 1:** M54–M56 合并后分别改为 `done`；M59 保持 formats planned。
- [ ] **Step 2:** M46 文档明确复用 `MetaFilesChangeSource`、mirror 与 orchestrator，不实现第二套 change detection。
- [ ] **Step 3:** 运行 `rg -n "[T]BD|[T]ODO|session manifest [o]r|change_count < [t]hreshold|warm 失败.*不[推]进|既有 v2 [i]ncremental" docs/specs/2026-07-12-diff-refresh-pipeline-design.md docs/plans/2026-07-12-diff-refresh-pipeline-plan.md docs/milestones/performance/m54-diff-refresh-pipeline.md`，期望无输出。
- [ ] **Step 4:** 运行所有 M54–M56 目标测试与 `cargo check --features cli-local`，把命令和结果写入 journal。

---

## 任务 ↔ Spec 覆盖

| Spec 要求 | Task |
|-----------|------|
| typed ChangeSet / 真源字段前置 | 2–3 |
| 文件更新、删除、改名差量 mirror | 4 |
| 多 dirty/deleted 批量删除 | 5 |
| 候选图/read model 与原子 graph+watermark | 6 |
| cache-preserving swap 与 best-effort rewarm | 7 |
| 真 META_FILES + recycle-bin/tombstone + 独立 cursors + 退避 | 9 |
| 统一命令契约 | 10 |
| 非空孤立变更不饿死 | 9 |
| Model/Field/DataFlow 依赖索引 | 11 |
| topology-dirty 基线 | 13 |
| 真增量 node/edge/file-state persist | 14 |
| v2 shadow stale/current 与 reopen 等价 | 14–15 |
| PersistReport 与规模曲线 | 15 |
| formats → M59 / M46 复用 | 1, 16 |

## 执行说明

- 按 Phase 开 M54、M55、M56 三个 PR；`main` 只接受 PR merge。
- M56 可在 M54 候选 dirty 路径合并后与 M55 并行，但 Task 13 的 topology-dirty 基线必须先完成。
- 默认按任务运行目标测试；跨 graph store/runtime 提交边界的 Tasks 6、7、14 完成时追加 `cargo check --features cli-local`。
