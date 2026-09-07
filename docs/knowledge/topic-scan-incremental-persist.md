# 主题二：扫描 — 增量索引 — 持久化生命周期

> 状态分层：**当前实现** + **已知缺陷**（缺陷是已确认未修的事实，不是计划）
> 分析 SHA：`31d020417d1ecbea440b0c7e92dffa2bd4b42d4b`（PR33 head；squash-merge 进 `main` 后内容一致）
> 行号核验：本文所有 `src/xxx.rs:NNN` 均按上述 SHA 逐条核对；PR33 在 `graph_redb.rs` 新增 2 行注释后，
> `open_inner_v1` 由 288 → 290、`hydrate_diagnostics` 由 383 → 385、`persist_commit` 由 977 → 979，
> `acquire_graph_db_lock` 调用点由 1001 → 1003、`write_txn.commit()` 由 1200 → 1203
> 适用版本：`main` @ 上述 SHA；本主题是 M59-2 的重点改造区，A3 落地后 1.4 / 3.1 / 3.2 会变
> 维护责任：改 `scanner/` / `graph_redb.rs` / `graph_redb_v2.rs` / `graph_store.rs` 后必须更新本文件

## 0. 检索速查（自包含，放文首以保证落在首个分块内）

> **问：`--build-graph` 的入口和六个阶段是什么？**
> **答**：入口 `scanner::scan_project_with_report`（`src/scanner/mod.rs:34`）→
> `ProjectIndexer::scan_with_diagnostics`（`src/scanner/indexer.rs:607`）；六阶段全部在
> `src/scanner/indexer.rs`：`discover_files`(325) → `diff_file_states`(351) →
> `parse_dirty_files_with_failures`(417) → `apply_incremental_changes`(535，4+5 合并) →
> `persist_index`(572)。判脏用 **xxhash64 内容 hash**，不用 mtime。

## 1. 当前实现

### 1.1 入口

```
--build-graph / --session-refresh
  → scanner::scan_project_with_report      src/scanner/mod.rs:34
      → ProjectIndexer::scan_with_diagnostics   src/scanner/indexer.rs:607
      → GraphDB::open(db_path) + hydrate_diagnostics   src/graph_redb.rs:167 / :385
  → ScanReport { indexed, unchanged, dirty, deleted, node_count, edge_count, diagnostics }
```

`ScanReport` 的 `indexed/dirty/deleted/unchanged` 是**文件口径**（在 `scan_with_diagnostics`
里用 `plan` 覆盖 store 的返回值，`src/scanner/indexer.rs:733-736`），`node_count/edge_count`
来自 `GraphDB::graph`。

### 1.2 六个阶段（`src/scanner/indexer.rs`）

| 阶段 | 函数 | 行 | 做什么 |
|------|------|----|--------|
| 1 发现 | `discover_files` | 325 | 递归收集 `.spg` / `.tbl`，不读内容 |
| 2 差分 | `diff_file_states` | 351 | 按 **xxhash64 内容 hash** 比 `prev_states`；产出 `dirty` / `deleted` |
| 3 解析 | `parse_dirty_files_with_failures` | 417 | 解析脏文件，返回 `ParseFailure` |
| 4+5 应用 | `apply_incremental_changes` | 535 | 合并 `previous_node_ids` + deleted，一次批删后重建（写内存图） |
| 6 持久化 | `persist_index` | 572 | → `IndexStateStore::persist_index` → `GraphDB::persist_commit`（写 redb） |

（表格压缩为四列以保证六个阶段落在同一检索分块内；「读/写什么」并入「做什么」列。）

关键点：**阶段 3 的失败文件不进 `updates`**，因此既不删它的旧节点、也不写新 `FileState`
——旧图保留、文件保持脏、下轮重试（`src/scanner/indexer.rs:417` 上方注释）。
SPG 解析失败仍然整轮报错（响亮失败）；**只有 TBL** 走 `ParseFailure` 静默分支
（UTF-8 非法 / JSON 非法，`src/scanner/indexer.rs:463-490`）。

### 1.3 写入与事务

```
IndexCommit { file_states, dirty_nodes, deleted_nodes, checkpoint, delta, scanner_entries, scanner_deleted_paths }
   src/graph_store.rs:165
  → GraphDB::persist_commit      src/graph_redb.rs:979
  → persist_internal             src/graph_redb.rs:992
     ├─ acquire_graph_db_lock（全程持锁，覆盖预读→写入→提交）  src/graph_redb.rs:1003
     ├─ 空提交短路：!is_dirty && 无 checkpoint && 无 scanner 载荷 → 不写盘
     ├─ delta = None  → 全量路径：重建 v2 layout 并置 Current
     └─ delta = Some  → 增量路径：只改受影响 keys，v2 置 Stale
  → 单次 write_txn.commit()      src/graph_redb.rs:1203
```

- **锁**：`acquire_graph_lock`（`src/graph_redb.rs:92`）用 `<db>.graphdb.lock` 文件实现进程间串行
  （redb 单文件不支持多进程并发打开）。超时由 `--graph-lock-timeout-ms` 控制，默认 10000ms
  （`src/cli.rs:91`），失败码 `GRAPH_DB_LOCK_TIMEOUT`（`src/graph_store.rs:53`）。
- **`delta` 的选择**（`src/scanner/indexer.rs:883`）：`prev_states.is_empty()` 时 `delta = None`
  ——即**首次全量构建走 full rebuild，重建 v2 并置 `Current`**；后续增量提交走
  delta，通常将 v2 置为 `Stale`。当累计受影响节点数达到下述 1024 阈值时，
  同一增量事务会重建 v2 并回到 `Current`，因此不能断言所有增量提交结束时都是 `Stale`。
- **v2 shadow**：`V2ShadowState::{Current, Stale}`（`src/graph_store.rs:205`）。
  `Stale` 时 `GraphDB::open` 直接走 v1 hydrate（`src/graph_redb.rs:209`）；
  增量提交累计受影响节点数 ≥ `V2_STALE_REBUILD_THRESHOLD_NODES = 1024`（`src/graph_redb.rs:87`）
  时本轮同事务重建 v2 并回 `Current`（`src/graph_redb.rs:1169`）。

### 1.4 诊断（SCANNER_*）的生命周期

| code | 触发 |
|------|------|
| `SCANNER_UNRECOGNIZED_CONTAINER_KEY` | SPG 有未识别容器键 |
| `SCANNER_DUPLICATE_COMPONENT_ID` | 同文件重复组件 id |
| `SCANNER_FILE_PARSE_FAILED` | TBL 解析失败（`ParseFailure`） |
| `SCANNER_DIAGNOSTICS_LOAD_FAILED` | runtime 加载期读诊断失败 |
| `SCANNER_DIAGNOSTICS_REFRESH_FAILED` | diff-refresh 侧刷新失败 |

- per-file 计数序列化为 bytes，随 `IndexCommit.scanner_entries` **与图同事务落库**（`src/scanner/indexer.rs:134`）。
- 删除文件与「对账出的陈旧路径」经 `scanner_deleted_paths` 同事务移除（`src/scanner/indexer.rs:221`、`src/scanner/indexer.rs:250`）。
- **陈旧对账是当前已实现的修复机制，不是未修复缺陷**。
  `stale_scanner_diagnostic_paths`（`src/scanner/indexer.rs:221`）处理两个场景：
  孤儿 entry（文件消失但从未写过 `FileState`）、内容改回上次成功字节（hash 相同⇒不脏⇒不覆盖）。
  解码不出来的 entry **一律不碰**（保留损坏证据，不静默抹平）。
- 报告出口用全库口径：`persist` 后重新 `load` 合并全部 entry（`src/scanner/indexer.rs:730`）。
  no-op 构建也从库里 load，诊断不会因无变更而消失（`src/scanner/indexer.rs:744`）。
- runtime 加载期把全库合并结果并入 `load_diagnostics`（`src/runtime.rs:403` 附近），
  因此 `--status` 与查询响应能透出 `SCANNER_*`。

### 1.5 `prepare` 路径（diff-refresh 用）

`ProjectIndexer::prepare`（`src/scanner/indexer.rs:770`）与 `scan` 复用同一套阶段函数，
**但不调用 `persist_index`**——graphdb 文件在 prepare 前后不变。
它的 `commit.scanner_entries` / `scanner_deleted_paths` **被置空**（`src/scanner/indexer.rs:895-896`），
诊断载荷经 `PreparedIndexUpdate` 透传给 diff-refresh 编排器，由调用方合并 pending 后随
最终 commit 落库。**调用方若忘记挂载，诊断就丢了**（注释明确此责任）。

### 1.6 失败后怎样

| 场景 | 行为 |
|------|------|
| 文件 hash 未变 | 跳过解析，`unchanged` 计数 +1 |
| TBL 坏字节 / 非法 JSON | 记 `ParseFailure` → `SCANNER_FILE_PARSE_FAILED`，旧图保留，下轮重试 |
| SPG JSON 解析失败 | **整轮索引失败**（`?` 向上抛）——刻意保持响亮 |
| 拿不到图锁 | `GRAPH_DB_LOCK_TIMEOUT`，超时可配 |
| v2 hydrate 失败 | 回落 `open_inner_v1`（`src/graph_redb.rs:242`），记 `v2_hydrate_warning` 并入 `HydrateDiagnostics`；转诊断时复用 `GRAPH_DB_V2_LAYOUT_UNREADABLE` 码（`src/diagnostics.rs:254-262`，仅当 `v2_layout_unreadable == 0` 才另发一条）。**注意**：回落的是 v1 表，不等于 hydrate 结果完整——见 3.5 |
| 诊断 entry 解码失败 | 记 `SCANNER_DIAGNOSTICS_LOAD_FAILED`，**不阻塞加载** |
| 无变更但有陈旧诊断要清 | 仍进写入分支（`src/scanner/indexer.rs:634` 的条件含 `stale`） |

### 1.7 改动时跑哪些测试

```bash
cargo test --features cli-local --test project_indexer_tests
cargo test --features cli-local --test m59_b3_tbl_parse_failure_tests
cargo test --features cli-local --test m59_b5_full_vs_incremental_diff_tests
cargo test --features cli-local --test m58_3_pr1_refix_scanner_persistence_tests
cargo test --features cli-local --test m56_incremental_persist_tests
cargo test --features cli-local --test m56_v2_stale_rebuild_tests
cargo test --features cli-local --test scanner_tests --test graph_store_tests
```

## 2. 已批准计划（尚未实现）

- **M59-2（A3/A4）**：节点与边的 `origin_file`、共享目标归属、schema 版本键、
  拒绝不兼容库、强制重建。`docs/plans/2026-09-06-m59-grafeo-implementation-plan.md` 要求：
  删除/修改被引用文件、删除唯一引用者、重启、修复后，**全量与增量的节点/边/全部属性及重复次数完全相等**。
- **M59-3**：Grafeo store 与导入；保留 redb 为默认实现，不提前退役。
- **M59-5**：真实语料测量 + redb/v2 shadow 退役。

## 3. 已知缺陷（已确认，未修）

### 3.1 增量更新丢失跨文件边

`tests/m59_b5_full_vs_incremental_diff_tests.rs:340` 的
`cross_file_edge_is_lost_by_incremental_update` 钉住：A 内嵌 B 时，增量更新会丢掉跨文件边。
根因：删除某文件的 `previous_node_ids` 时，会连带删掉**其它文件指向这些节点的边**，
而这些边属于别的文件，不会被重建。

### 3.2 占位页节点在唯一引用者删除后泄漏

> **问：占位页节点在引用者删除后会被清理吗？答：不会，会永久泄漏。**
> 缺陷已确认未修，A3 的 `origin_file` 落地后才有清理依据。

`tests/m59_b5_full_vs_incremental_diff_tests.rs:441` 的
`placeholder_page_node_leaks_after_its_only_referrer_is_deleted` 钉住：
`src/scanner/spg.rs:1069-1076` 为内嵌目标 `add_node`，但**不做 `node_ids.insert`**——
对照同文件 `:584` / `:606` / `:757` 等处都做了 insert。
于是占位节点 `page:<ghost>` 不属于任何文件：A 被删后没有文件声明它，
`remove_nodes_by_ids` 永远碰不到，图中留下悬空页节点。
与 3.1 是**同一根因的两面**（节点归属未记账），A3 的 `origin_file` 必须同时覆盖节点与边。

### 3.3 B5 测试通过 ≠ 增量正确

`tests/m59_b5_full_vs_incremental_diff_tests.rs` 的两条缺陷测试是**如实钉住当前分歧**，
不是认可。文件头部注释写明：A3 落地后这两条会变红，届时把期望改成「两侧完全一致」。
**因此「B5 全绿」不能作为增量索引正确性的证据，也不能作为 Grafeo 迁移正确性的证据。**

### 3.4 Grafeo 尚未接入

当前默认后端仍是 **redb**（`src/graph_redb.rs`）加 v2 shadow（`src/graph_redb_v2.rs`）。
Grafeo 只在已批准设计与计划里，**没有任何实现代码**。
M59 journal 记录的真实语料测量（pin `c3c0528`、501 SPG / 828 TBL、旧后端
89,178 节点 / 200,028 边、驻留约 3.72 GiB）是**迁移前测量**，**不是 Grafeo 验收**。
该测量文件位于 `docs/ai-eval-runs/2026-09-06-m59-real-corpus-measurements.md`，
**已被排除出知识库索引**（评测产物）。

### 3.5 v1 hydrate 也可能得到**部分缺失**的图

`GraphDB::open_inner_v1`（`src/graph_redb.rs:290`）逐行解码 v1 节点/边表时，
对三类坏数据**计数并跳过**，不报错、不中断：

| 情况 | 处理 | 诊断码 |
|------|------|--------|
| 节点 value 反序列化失败 | 跳过该节点，`node_decode_failed += 1` | `GRAPH_DB_NODE_DECODE_FAILED` |
| 边 value 反序列化失败 | 跳过该边，`edge_decode_failed += 1` | `GRAPH_DB_EDGE_DECODE_FAILED` |
| 边端点节点不存在 | 跳过该边，`dangling_edge += 1` | `GRAPH_DB_EDGE_DANGLING_ENDPOINT` |

三者任一非零 → `HydrateDiagnostics::has_partial_loss()` 为真（`src/diagnostics.rs:182`）→
再汇总一条 `GRAPH_DB_PARTIAL_HYDRATE`（`src/diagnostics.rs:195` 起）。
这三类码的 `impact` 都是 `partial`（`src/diagnostics.rs:64-66`），即**当前图只覆盖部分数据，
查询结论可能是错的**；对照 `GRAPH_DB_V2_LAYOUT_UNREADABLE` 的 `impact` 是 `none`
（`src/diagnostics.rs:67`）——丢 v2 shadow 不影响正确性，丢 v1 行会。

因此「v1 是权威布局、v2 只是 shadow」成立，但「fallback v1 得到的图一定完整」**不成立**。
旧的仓库注释与文档曾写「fallback v1（始终正确）」，其本意只是「v2 损坏时应当回落到 v1
布局」，不是保证 hydrate 结果无缺失（`src/graph_redb.rs:239-241` 注释已随之修正措辞）。
调用方应读 `GraphDB::hydrate_diagnostics()`（`src/graph_redb.rs:385`）并上报，
不要假设 `GraphDB::open` 成功即图完整。

## 4. 历史状态

- M54 建 diff-refresh 通路（`prepare` + `install_replacement` 分离）。
- M56 落 delta 持久化（`commit batch`）：增量提交只改受影响 keys。
- M58.3 PR1 refix（F2）把 per-file scanner 诊断从「随报告消失」改为「随 commit 同事务落库」，
  并改为全库口径合并。
- M59-B3 修掉 TBL 静默成功：坏 TBL 不再「删旧图 + 写空图 + 照常记 hash」。

## 5. 边界与限制

- 本文只覆盖**本地目录扫描 + redb 持久化**。远程 BI 会话同步（`src/session/`）、
  diff-refresh 编排（`src/diff_refresh/`）、browser/WASM 持久化（`src/persistence/indexeddb.rs`）
  **均不在范围**。
- `docs/ai-eval-runs/2026-09-05-grafeo-spike-measurements.md` 与
  `2026-09-06-m59-real-corpus-measurements.md` 属评测产物，被排除出知识库索引。
