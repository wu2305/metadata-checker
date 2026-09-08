# M53 性能优化落地记录

> 状态：**done**（B1 / B2 / A 已合并 #9–#10；warm-cache 可扩展性 T1–T3、init/load 分层观测已在 main。2026-09-08 按 `3d595d3` 逐项复核源码与原测试断言保留）。B1.2 dataflow facts、`redb_commit` / commit batch 仍 profile 门控，非阻塞。fragment 跨平台已移出 M53（无跨 session 复用需求时不做）。

## 从 M52 带入的已验证优化

### 1. Cost model report

已把 page logic profile 从单纯耗时列表扩展为可解释成本模型：

- `purpose`：热点服务的产品语义。
- `why_expensive`：复杂项目中成本被放大的原因。
- `cost_drivers`：stage / counter 量化来源。
- `space_time_candidates`：空间换时间候选。
- `next_action`：下一步动作。

M53 继续复用这套报告，不另建 profiling 类型。

### 2. Page-local availability batch

`query_page_logic.key_model_availability` 已从逐个 model 重复构造，推进到单次 page logic 内共享 graph read cache / condition collector cache。

真实项目 `sample-count=1` 已验证：

| 场景 | 优化前 | 优化后 | 变化 |
|---|---:|---:|---:|
| `full.key_model_availability` | 5227ms | 3819ms | -26.9% |
| `normal.key_model_availability` | 5040ms | 3508ms | -30.4% |
| `compact.key_model_availability` | 164ms | 124ms | -24.4% |

M53 不再重复做 query-time 临时 fact index。M52 已证明它会变慢。

### 3. Dense snapshot / warm read-model

已验证把 availability / prerequisites / path summary 前移到 long-lived runtime warm 阶段，可以显著降低 query-time。

真实项目 `合同协议.spg` compact 链路（**史料**，来自从未合并进 `main` 的分支 `codex/m52-performance-optimization` 提交 `15804ea`；该实现未进入当前代码，**下表 105ms 不适用于修复前的 `main`**）：

| 指标 | one-shot | long-lived | long-lived + warm |
|---|---:|---:|---:|
| query dispatch | 916ms | 898ms | 105ms |
| total N=10 | 9758ms | 12292ms | 5195ms |
| total N=50 | 46398ms | 48212ms | 9395ms |

修复前 `main`（仅 warm `key_model_availability`，2026-07-07 多轮采样 p50）：

| 指标 | p50 | min–max |
|---|---:|---|
| `warmed_query_dispatch` | **639ms** | 595–716 |
| `availability_warm` | 189ms | 170–204 |

任务 C instrumentation（`release-fast`，`sample-count=1`，2026-07-07）确认 B1/B2 在真实 LongLived runtime 路径已生效（`target/m51-profile/m53-c-instrumentation.json`）：

| counter | long-lived query（warm 前） | warmed query |
|---|---:|---:|
| `availability_materialized_hits` | 2 | 2 |
| `availability_read_model_used` | 0 | 1 |
| `path_dense_graph_used` | 1 | 0（path cache hit 跳过 query-time 重建） |
| `path_dense_adjacency_hits` | 17774 | 0 |

任务 A 扩展 warm 覆盖 prerequisites + path_summary 后（3 轮 `sample-count=1`，`target/m51-profile/m53-a-extended-warm-*.json`）：

| 指标 | 修复前 p50 | 任务 A p50 | min–max |
|---|---:|---:|---|
| `warmed_query_dispatch` | 639ms | **81ms** | 79–82 |
| `availability_warm` | 189ms | **639ms** | 633–640 |

M53 默认接受 init/load 变重，只要 query-time 和多次查询总成本下降。

### 4. Page logic fragment contract（历史原型，未合并）

`codex/m52-performance-optimization` 分支曾实现 `cli-local` native 文件后端 spike，
未合入 main；以下记录原型能力，不代表当前代码提供文件后端：

- 持久化 `availability` / `prerequisites` / `paths` / `warm_stages` / `meta`。
- fragment hit 后恢复为现有 `PageLogicAvailabilityCache`。
- miss / schema mismatch / fingerprint mismatch fallback 当前 Rust 路径。

原型提出以 page-logic fragment bundle 作为稳定边界，native 文件、browser IndexedDB、memory
作为候选后端。fragment 跨平台持久化已移出 M53，待有跨 session 复用需求再评估。

### 5. Reload unchanged 快路径（历史原型，未合并）

归档分支曾尝试避开 redb 文件头采样，并仅以创建时间、大小、采样 hash 判定重载，
将 `mtime_changed` 留作诊断。该策略和 `ReloadCheckProfile` 没有合入当前 main。

以下是原型记录的历史测量，本次未重跑，不代表当前 main 性能：

| 指标 | 优化前 | 优化后 |
|---|---:|---:|
| `runtime_check_reload_unchanged` | 676ms | 0ms |
| `runtime_check_reload_reloaded` | 误触发 | 0 |

按 `main=db158cb8dc609a0050c214aed788dde459044361` 核对：
`src/runtime.rs:238` 读取整个文件、仅对前 4096 字节求 hash；`:1361` 的
`is_graph_changed` 仍将 mtime、size 或该 hash 任一变化作为重载条件。
`src/perf_report.rs:991` 保留 `runtime_check_reload_unchanged` 阶段名称，
但没有检查结果一定为 `Unchanged`，也没有原型的八个细分计时/变化 counter。
因此不能从阶段名称推断已走 unchanged 快路径。

后续若优化这条路径，须先验证 redb 打开引起的变化、同大小内容更新、文件替换和读取失败，
证明不会漏检更新后再决定检测策略；不直接恢复旧采样方案。
完整去向见 [性能归档复核](../../governance/branch-retention-performance-2026-09-08.md)。

## M53 继续推进的性能问题

### B1 materialized availability facts（已完成）

- 新增 `MaterializedAvailabilityFactsIndex`：load-time 预收集全图 condition 对象。
- `RuntimeReadModel` 在 LongLived load 时与 `DenseGraphSnapshot` 一并构建。
- page logic / warm 路径通过 `ConditionCollectorCache::from_prefilled` 复用，profile counter：`availability_materialized_hits`。
- 测试：`tests/m53_materialized_availability_tests.rs`（输出等价）。

### B2 dense-native path finder（已完成）

- `DensePathTraversal` 直接遍历 CSR 邻接，候选搜索 bypass `PathGraphCache`。
- profile counter：`path_dense_adjacency_hits`。
- 测试：`tests/m53_dense_path_finder_tests.rs`（dense vs baseline 等价）。

### A redb v2 hydrate + incremental persist（已完成）

- `GraphDB::open_inner` 优先 `read_v2_layout` + `hydrate_graph_from_v2`，失败 fallback v1。
- `persist` 在写事务前读取 v2；≤32 dirty node 且无 removed 时 `patch_v2_layout_node_meta` 增量更新。
- 测试：`tests/m53_redb_v2_hydrate_tests.rs`（v2 优先打开、v1 fallback、增量 patch）。

## M53 验收（合并后 profile，`xiaoshouyi`，`sample-count=1`）

报告：`target/m51-profile/m53-core-profile.json`、`m53-page-logic-profile.json`（`release-fast`，2026-07-07）。

### one-shot page logic（合同页 / 会员页）

| 场景 | M52 dense-final | M53 | 变化 |
|---|---:|---:|---:|
| `full.key_model_availability` | 4747ms | 3822ms | -19.5% |
| `normal.key_model_availability` | 4732ms | 3805ms | -19.6% |
| `compact.key_model_availability` | 136ms | 201ms | +47.8% |

one-shot 路径不经过 LongLived read model，故 `availability_materialized_hits` / `path_dense_adjacency_hits` 为 0；等价性由 `tests/m53_*` 覆盖。

### LongLived runtime（合同页 compact warm）

| 指标 | M52 path-prereq-warm（史料） | 修复前 M53 main | 任务 A 后 | 备注 |
|---|---:|---:|---:|---|
| `runtime_long_lived_read_model_build_ms` | 2743 | 5837 | ~3153 | load 时预建 materialized facts + dense |
| `redb_open` | 555 | 866 | ~815 | v2 hydrate 默认路径 |
| `runtime_long_lived_warmed_query_dispatch` | 105 | 639（p50） | **81（p50）** | 扩展 warm 后恢复预期量级 |
| `runtime_long_lived_warmed_total_ms_n50` | 9395 | 36400（p50） | **8292** | 单次采样；warm 成本前移 |
| `runtime_long_lived_availability_warm_ms` | 833 | 189 | **639** | 现覆盖 availability + prerequisites + paths |

M53 核心交付是 **语义等价 + 默认 v2 hydrate + load-time 物化索引 + 完整 page logic warm**；`redb_commit` 仍 ~10.8s，C 线 commit batch 保持 profile 门控。

### 多轮采样（5× core + 3× page logic，2026-07-07 晚，修复前 main）

| 指标 | p50 | min–max | M52 path-warm（史料，单次） |
|---|---:|---|---:|
| `warmed_query_dispatch` | **639ms** | 595–716 | 105 |
| `warmed_total N=50` | **36400ms** | 33779–40284 | 9395 |
| `read_model_build` | 3736ms | 3376–3860 | 2743 |
| `redb_open` | 815ms | 768–831 | 555 |
| `availability_warm` | 189ms | 170–204 | 833 |

修复前 main 仅 warm availability，path_summary（~435ms）与 prerequisites（~59ms）仍在 query-time 执行，故 warmed dispatch 仅比 one-shot 快 ~20%。任务 A 后 warmed dispatch p50 **~81ms**（3 轮 79–82ms）。

compact one-shot（3 轮 p50）：wall **694ms**（M52 537）、`path_summary` **435ms**（M52 312）、`key_model_availability` **166ms**（M52 136）。path 仍是 compact 回退主因；首轮 201ms avail 属正常方差。

PR：[#9](https://cnb.cool/wu2305/metadata-checker/-/pulls/9)（CI/docs）、[#10](https://cnb.cool/wu2305/metadata-checker/-/pulls/10)（B1/B2/A）。

## M53 收尾交付（已合入，历史记录）

### warm-cache 可扩展性（T1–T3）

规格：`docs/specs/2026-07-07-m53-warm-cache-scalability-design.md`。

| 项 | 内容 |
|---|---|
| T1 | `Arc::get_mut` 消除 warm 时 O(P²) map clone；新增 `warm_page_logic_batch` |
| T2 | warm 物化按 budget 裁剪 path 缓存（compact `candidate_paths` 6706→0）；输出字节级等价 |
| T3 | `PageDependencyIndex` + `invalidate_pages_for_dirty_nodes`（page-diff 地基；不含触发业务） |

### init/load 分层观测

规格：`docs/specs/2026-07-09-m53-init-load-observability-design.md`。

- `read_model_build_ms` 拆为 dense / availability facts / page dependency index 三段计时。
- `PageDependencyIndex::build` 失败降级为空索引（与 dense/facts 容错一致）。
- Criterion：`runtime_load_*_build` 三场景进 Bencher trend（无 fail threshold）。

xiaoshouyi 真实项目 `release-fast` Criterion 单步采样（2026-07-09，p50）：

| 阶段 | p50 | 占 read model 三步之和 |
|---|---:|---:|
| `runtime_load_dense_snapshot_build` | 2968ms | 57% |
| `runtime_load_availability_facts_build` | 1801ms | 35% |
| `runtime_load_page_dependency_index_build` | 442ms | 8% |
| 合计 | 5211ms | 100% |

## M53 剩余 / 后续

1. ~~redb v2 hydrate / incremental persist~~（已完成 §A）。
2. ~~path_summary dense-native~~（已完成 §B2）。
3. ~~prerequisites / path_summary warm cache~~（已完成，任务 A；与下述 B1.2 不同）。
4. ~~warm-cache 可扩展性 T1–T3~~（已完成，见上节）。
5. ~~init/load 分层观测补全~~（已完成 §init-load-observability）。
6. **B1.2**：`key_model_availability` 更深物化（**dataflow facts** 批量索引，非 prerequisites/paths warm）——需真实项目 profile 验证收益；可挂后续里程碑。
7. **`redb_commit` / commit batch**（C 线，profile 门控，非阻塞）。
8. **runtime page diff 触发**（阈值 / 远程改动量 / 自动 re-warm）——T3 仅地基，业务编排不在 M53。

## M53 不重复做

- 不重跑 per-query fact index 方向。
- 不把 DuckDB / Parquet 当作 M53 前置条件。
- 不设置性能 fail threshold。
- 不做 browser real perf。
- 不改公开 JSON 语义。
- 不做 fragment 跨平台持久化（已移出 M53；旧 spike 在未合并分支，待有跨 session 复用需求再评估）。
