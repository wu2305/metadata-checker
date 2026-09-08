# M52 性能优化收口记录

> 状态：M52 起始落地。目标是“安全地加快”：真实项目 profile 先证明收益，再做语义等价优化；大改必须有旧路径 fallback 或输出等价测试。

> 历史记录说明（2026-09-08）：下文“当前”“下一步”和测量表保留 M52 当时语境，
> 不代表当前 main 状态。例如默认 v1 hydrate 的边界已在 M53 改为优先 v2、失败回退 v1。
> 归档分支中额外的 native fragment 与 reload 采样原型未合入；去向与测试缺口见
> [性能归档复核](../../governance/branch-retention-performance-2026-09-08.md)。

## 验收口径

M52 不设置 CI 性能 fail threshold。每个优化小步必须满足：

- 输出语义不变，至少覆盖目标单测。
- 真实项目 profile 有 before / after 数据。
- 优化只改变当前热点路径，不改 redb schema，不引入第三方图数据库。
- 不能达到 30% 目标时，必须记录原因和下一步。

## 已实施优化

### 0. Cost model report

M52 增加 `cost_model` 报告字段，把每个热点从“耗时数字”提升为可讨论的优化单元：

- `purpose`：该热点服务的产品语义。
- `why_expensive`：为什么复杂项目会放大这个成本。
- `cost_drivers`：从 stage / counter 汇总出来的量化原因。
- `space_time_candidates`：可评估的空间换时间方案。
- `next_action`：下一步决策。

这不是另起一套 profiling 类型，而是复用 `PerfProfile` / `PerfStage` 的报告层增强。CI 继续生成 JSON report，不设置性能 fail threshold。

真实项目 `sample-count=1` 验证快照：

| 场景 | hotspot | 主要成本驱动 | 空间换时间候选 |
|---|---|---|---|
| `query_page_logic_contract_full` | `key_model_availability` | `key_models=88`、`availability_condition_groups=395` | runtime/load-time availability fact index、page-scoped dataflow metadata cache |
| `query_page_logic_contract_full` | `path_summary` | `path_anchor_extract_ms=317`、`path_candidate_search_ms=589` | page-local path anchor index、bounded candidate adjacency cache |
| `query_page_logic_member_registered_compact` | `path_summary` | `path_anchor_extract_ms=163`、`path_candidate_search_ms=105` | page-local path anchor index、field reachability summary |

结论：

- `key_model_availability` 的核心目的不是“输出更多 JSON”，而是避免把 filter / empty gate / join / union / 输入缺失误判为数据可用。
- `path_summary` 的核心目的不是“遍历图”，而是把页面字段来源和影响面归类成可解释路径。
- 后续优化不应只看阶段名，应先判断是否能把这些语义提前物化成索引或摘要。

### 1. Page-local availability batch

`query_page_logic` 的 `key_model_availability` 从逐个 model 调完整 fast output，改为单次 page logic 内共享：

- node / edge graph read cache。
- condition collector cache。
- availability entry 统一输出 helper。
- 旧 `build_explain_availability_fast_output` 仍保留为 fallback。

新增 counters：

- `availability_context_build_ms`
- `availability_models_batched`
- `availability_fallback_models`
- `availability_dataflow_meta_cache_hits`
- `availability_condition_groups`

真实项目 `sample-count=1` 对比：

| 场景 | M52 graph-cache only | M52 condition batch | 变化 |
|---|---:|---:|---:|
| `query_page_logic_contract_full.key_model_availability` | 5227ms | 3819ms | -26.9% |
| `query_page_logic_contract_normal.key_model_availability` | 5040ms | 3508ms | -30.4% |
| `query_page_logic_member_registered_compact.key_model_availability` | 164ms | 124ms | -24.4% |

结论：

- normal 达到 30% 目标。
- full 接近但未达到 30%，主因是 88 个 key model 仍要生成 395 组 availability condition facts。
- 下一步若继续压该热点，应做更深的 dataflow availability facts 批量索引，而不是继续优化 JSON shaping。

#### 反证实验：per-query fact index

曾尝试在单次 page logic 内临时构造 availability fact index，希望用空间换时间减少逐模型 condition 查找。真实项目 `sample-count=1` 结果变慢：

| 场景 | batch baseline | per-query fact index | 变化 |
|---|---:|---:|---:|
| `query_page_logic_contract_full.key_model_availability` | 2815ms | 3693ms | +31.2% |
| `query_page_logic_contract_normal.key_model_availability` | 2740ms | 3703ms | +35.1% |
| `query_page_logic_member_registered_compact.key_model_availability` | 92ms | 178ms | +93.5% |

该实现未保留。结论是：索引方向仍然合理，但索引不能在每次 query 里扫描全量 condition facts 临时构建；要么在 graph build / runtime load 期物化，要么只缓存更窄的 dataflow metadata。

### 2. Page-local path graph cache

`path_summary` 在单次 page logic 内共享 graph reads，覆盖：

- anchor extract。
- bounded candidate search。
- field causal path 构造。
- cross-page side context。

新增 counters：

- `path_context_build_ms`
- `path_graph_cache_hits`

真实项目 `sample-count=1` 对比：

| 场景 | before `path_summary` | after `path_summary` | `path_graph_cache_hits` |
|---|---:|---:|---:|
| `query_page_logic_contract_full` | 611ms | 503ms | 8631 |
| `query_page_logic_contract_normal` | 750ms | 496ms | 8631 |
| `query_page_logic_member_registered_compact` | 233ms | 190ms | 4182 |

结论：

- cache 有稳定收益，且不改变路径分类规则。
- `path_candidate_search_ms` 仍是 path 内最大项；更进一步需要调整 `BoundedCausalPathFinder` 的 BFS 候选生成策略。

### 3. Core profile 细分

`m51_core_profile_report` 已扩展到 M52 细分阶段。

`rebuild`：

- `rebuild_load_file_states`
- `rebuild_cold_discover`
- `rebuild_cold_dirty_detection`
- `rebuild_cold_apply_deletions`
- `rebuild_cold_parse`
- `rebuild_cold_graph_apply`
- `rebuild_cold_persist_commit`
- `rebuild_noop_scan`
- `rebuild_noop_dirty_detection`

`redb`：

- `redb_open`
- `redb_load_file_states`
- `redb_check_graphdb`
- `redb_node_write`
- `redb_edge_write`
- `redb_commit`

`runtime`：

- `runtime_graphdb_load`
- `runtime_status_dispatch`
- `runtime_query_dispatch`
- `runtime_check_reload_unchanged`

结论：

- M52 当前只完成 core 慢场景归因细分，未做 redb schema 或 runtime 生命周期大改。
- 后续是否优化 redb / runtime，应以真实项目 core profile 为准。

真实项目 `sample-count=1` 快照：

| 场景 | 主阶段 | 耗时 |
|---|---|---:|
| `rebuild` | `rebuild_cold_graph_apply` | 1680ms |
| `rebuild` | `rebuild_cold_persist_commit` | 1603ms |
| `rebuild` | `rebuild_cold_parse` | 585ms |
| `redb` | `redb_commit` | 10625ms |
| `redb` | `redb_open` | 622ms |
| `runtime` | `runtime_graphdb_load` | 621ms |
| `runtime` | `runtime_check_reload_unchanged` | 38ms |

该快照说明：如果 M53 继续优化非 page-logic，优先级应是 redb commit / rebuild persist，其次是 runtime load。

### 4. Dense read model seam

已新增 `DenseGraphSnapshot`，把事实图转换为稠密 ID + CSR 风格只读快照。当前落地边界：

- `DenseGraphSnapshot` 实现 `GraphReadStore` 适配器，结构等价测试覆盖 node / edge / incoming / outgoing。
- `path_summary` 支持传入预构建 dense snapshot，但默认 query-time 不构建。
- `GraphRuntime` 支持显式 opt-in 构建 dense snapshot；默认 runtime load 不构建。
- `PageAvailabilityIndex` 已作为 availability 读模型 seam 接入，第一版包装现有 batch fast path，只稳定 build/projection/fallback counters，不改语义。

#### 反证实验：query-time dense snapshot

曾尝试在每次 `path_summary` 内构建全项目 dense snapshot。真实项目 `sample-count=1` 结果明显变慢：

| 场景 | baseline `path_summary` | query-time dense `path_summary` | `dense_snapshot_build_ms` |
|---|---:|---:|---:|
| `query_page_logic_contract_full` | 518ms | 2768ms | 1877ms |
| `query_page_logic_contract_normal` | 509ms | 2713ms | 1603ms |
| `query_page_logic_member_registered_compact` | 191ms | 1924ms | 1605ms |

该默认路径已撤回。结论是：CSR / 稠密 ID 方向成立，但不能在每次 query 现场构建全项目 snapshot；必须来自 runtime/load、build-time，或更小的 page-local 派生结构。

#### 反证实验：默认 runtime dense snapshot

曾尝试在 `GraphRuntime::load_with_project_dir` 默认构建 dense snapshot。真实项目 core profile 显示：

| runtime 项 | 默认 dense | 安全默认 |
|---|---:|---:|
| `runtime_graphdb_load` | 4672ms | 622ms |
| `runtime_dense_snapshot_build_ms` | 4029ms | 0ms |
| `runtime_query_dispatch` | 108ms | 63ms |

该默认路径已撤回，改为显式 opt-in。当前 dense snapshot 只是后续 dense-native path finder / availability index 的底座，不能仅靠 `GraphReadStore` 适配器获得收益。

#### 当前 profile contract

最新真实项目 `sample-count=1` page logic 快照：

| 场景 | wall | `key_model_availability` | `path_summary` | dense used | `availability_index_build_ms` | `availability_index_projection_ms` |
|---|---:|---:|---:|---:|---:|---:|
| `query_page_logic_contract_full` | 5847ms | 4747ms | 761ms | 0 | 4747ms | 1ms |
| `query_page_logic_contract_normal` | 6293ms | 4732ms | 1202ms | 0 | 4732ms | 1ms |
| `query_page_logic_member_registered_compact` | 537ms | 136ms | 312ms | 0 | 136ms | 1ms |

该快照说明：`PageAvailabilityIndex` 当前只是 seam，尚未减少 availability 构造成本；真正优化必须把内部 facts 从 batch output 包装替换为 build/load-time materialized facts。

#### 验证推进：init-heavy long-lived runtime

M52 后续验证口径调整为：允许长生命周期 runtime 在 init/load 阶段构建派生读模型，但必须和 one-shot 路径分开计量。

已新增 `RuntimeMode`：

- `RuntimeMode::OneShot`：默认 CLI / 单次查询路径，不构建 init-heavy read model。
- `RuntimeMode::LongLived`：浏览器插件、server/runtime、CI profile 这类长生命周期路径，在 load/reload 阶段构建 `RuntimeReadModel`。

`RuntimeReadModel` 当前包含：

- `dense_graph: Arc<DenseGraphSnapshot>`

core profile 现在同时记录：

- `runtime_one_shot_load_ms`
- `runtime_one_shot_query_dispatch_ms`
- `runtime_long_lived_load_ms`
- `runtime_long_lived_read_model_build_ms`
- `runtime_long_lived_query_dispatch_ms`
- `runtime_one_shot_total_ms_n1/n5/n10/n50`
- `runtime_long_lived_total_ms_n1/n5/n10/n50`

验收公式：

```text
one_shot_total(N) = one_shot_load + N * one_shot_query
long_lived_total(N) = long_lived_load + N * long_lived_query
```

因此后续不再用“load 是否变慢”单独否决 dense/read-model；只有当 `long_lived_total(N)` 在真实 query 次数下仍不划算时，才撤回默认长生命周期路径。

第一轮真实项目验证：

| 指标 | one-shot | long-lived read model |
|---|---:|---:|
| load | 539ms | 3665ms |
| read model build | 0ms | 3116ms |
| query dispatch | 57ms | 56ms |
| total N=1 | 596ms | 3721ms |
| total N=5 | 824ms | 3945ms |
| total N=10 | 1109ms | 4225ms |
| total N=50 | 3389ms | 6465ms |

该结果证明：仅预构建 `DenseGraphSnapshot` 并通过 `GraphReadStore` adapter 传入 query，不能摊销 init 成本。下一步必须让热点直接消费 read model：

- `PageAvailabilityIndex`：从 batch fast path 包装改成 materialized facts。
- `path_summary`：从 `GraphReadStore` adapter 改成 dense-native CSR traversal。

第二轮针对性验证：availability materialized warm chain

本轮只改一条链路：在 long-lived runtime 中显式 warm 指定页面的 `key_model_availability`，query-time 命中 `PageLogicAvailabilityCache` 后直接投影，跳过 availability index build。

这一轮专门落实前一轮结论：

> 把 availability 构造前移到 init/warm 阶段，query-time 能下降。但当前 warm 仍是验证实现，它通过完整 page logic 输出提取 cache；下一步应把 warm 内部改成真正的 materialized facts，而不是跑完整 page logic。

当前实现已经不再通过完整 page logic 输出反向提取 cache，而是只执行 availability 所需链路：

- target lookup
- page child components/actions 收集
- data/write model edge scan
- prerequisites scan
- `PageAvailabilityIndex` materialize

真实项目 `合同协议.spg` compact 链路结果：

| 指标 | one-shot | long-lived | long-lived + availability warm |
|---|---:|---:|---:|
| load | 456ms | 2586ms | 2586ms |
| read model build | 0ms | 2149ms | 2149ms |
| availability warm | 0ms | 0ms | 173ms |
| query dispatch | 697ms | 660ms | 560ms |
| total N=10 | 7426ms | 9186ms | 8359ms |
| total N=50 | 35306ms | 35586ms | 30759ms |

结论：

- warm 成本从上一版完整 page logic 提取 cache 的 684ms 降到 173ms。
- query-time 单次下降约 15.2%（660ms -> 560ms），证明 materialized facts 方向有效。
- N=50 时 warm 版本明显优于 one-shot（30759ms vs 35306ms）。
- 剩余 query-time 大头已经不是 availability，而是 `path_summary` 和 `prerequisites`。

### 5. redb v2 shadow layout

M52 收口项：完成 redb v2 图布局设计与 shadow build 验证，不切换默认 v1 读写路径。

布局组成：

| 组件 | redb 表 | 内容 |
|---|---|---|
| meta | `v2_meta` | schema version、node/edge/field path/file state 计数、content fingerprint |
| dense id 映射 | `v2_node_ids` | 排序后的 `node_id` 列表，下标即 dense id |
| node meta | `v2_node_meta` | 与 dense id 对齐的 `node_type/path/name/meta` |
| out adjacency | `v2_out_adj` | CSR `offsets + entries(adjacent_dense_id, edge_type, field_path_id, meta)` |
| in adjacency | `v2_in_adj` | 同上，入边视角 |
| field path 字典 | `v2_field_paths` | 去重后的 `field_path` 字符串表 |
| file states | `v2_file_states` | 与 v1 等价的 `FileState` 映射 |

实现边界：

- `GraphDB::persist` 在单次 write transaction 内写完 v1 后同步 shadow 写 v2；默认 `open_inner` 仍只 hydrate v1。
- `read_v2_layout` 在 schema / fingerprint 不匹配时返回 miss，不阻断 v1。
- `hydrate_graph_from_v2` + `shadow_compare_v1_v2` 提供等价校验，供 M53 hydrate 切入前复用。
- fingerprint 对 `file_states` 使用排序后的 path 迭代，避免 HashMap 顺序导致误判。

测试覆盖：`tests/m52_redb_v2_shadow_tests.rs`

- scan/persist 后 v2 shadow 与 v1 图等价。
- round-trip 写入 / 读取 / hydrate 后节点边一致。
- fingerprint 篡改时 v2 miss、v1 仍可打开。

## 当前真实项目快照

最后一次 page logic profile：

| 场景 | wall | `key_model_availability` | `path_summary` | `prerequisites` |
|---|---:|---:|---:|---:|
| `query_page_logic_contract_full` | 3545ms | 2815ms | 503ms | 73ms |
| `query_page_logic_contract_normal` | 3403ms | 2740ms | 496ms | 124ms |
| `query_page_logic_member_registered_compact` | 337ms | 92ms | 190ms | 56ms |

热点排序：

1. full / normal `key_model_availability`：仍是主导成本。
2. full / normal `path_summary`：candidate search 仍有优化空间。
3. compact `path_summary`：已低于 200ms，但仍是 compact 主项。
4. `prerequisites`：稳定低于 125ms，暂不优先。

## M53 候选

M53 接续基线见 [M53 性能优化接续记录](m53-performance-continuation.md)。

- ~~M52 结束前先完成 redb v2 graph layout 设计和 shadow build 验证~~（已完成，见上文 §5）。
- 把 `PageAvailabilityIndex` 内部从 batch fast path 包装替换为 build/load-time materialized facts，减少 88 个 key model 的重复 filter / input path / row semantic 构造。
- 增加 dense-native path finder，直接消费 CSR slices，避免 `GraphReadStore` owned clone 适配器抵消 CSR 收益。
- 在 `BoundedCausalPathFinder` 内增加候选节点剪枝或 anchor pair 预索引，降低 `path_candidate_search_ms`。
- 基于真实 core profile 决定是否做 redb commit batch 或 runtime fingerprint 快速路径。

## M52 下一步决策框架

每个候选优化先回答四个问题：

1. 这个热点的产品目的是什么，是否必须在查询时即时计算？
2. 当前主要成本来自 fanout、重复遍历、重复解析、IO commit，还是 JSON 构造？
3. 是否可以在 graph build、runtime load 或 page-local context 中用空间换时间？
4. 是否能提供旧路径 shadow compare 或输出等价测试？

只有四个问题都能回答清楚，才进入实现；否则先补 stage / counter，而不是继续猜测优化。

## 明确不做

- 不设置性能 fail threshold。
- 不做 browser real perf。
- 不切换默认 redb 读写 schema；M52 只允许 v2 layout 设计和 shadow build 验证。
- 不引入 DuckDB / 图数据库替换。
- 不改 query 层公开 JSON 语义。
- 不默认启用 query-time 或 runtime-load dense snapshot，除非真实 profile 证明收益。
