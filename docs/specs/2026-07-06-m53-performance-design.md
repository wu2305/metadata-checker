# M53 性能优化设计

> 状态：approved（用户确认 2026-07-06）  
> 范围：M53 Phase B（query 热路径）+ Phase A（redb v2 hydrate/incremental）；不含 C 线默认交付。

## 背景

M52 已验证：page-local batch、warm availability、dense snapshot、redb v2 shadow。M53 把验证过的方向切入默认路径，并收口观测与测试。

真实项目热点（M52 末快照）：

| 阶段 | full/normal 量级 |
|------|------------------|
| `key_model_availability` | ~2.7–2.8s |
| `path_summary` | ~500ms |
| `prerequisites` | &lt;125ms |

## 目标

1. **B1**：build/load-time materialized availability facts，消除 query-time 重复 condition / dataflow 遍历。
2. **B2**：dense-native `BoundedCausalPathFinder`，直接消费 CSR，降低 `path_candidate_search_ms`。
3. **A**：`GraphDB::open_inner` 优先 v2 hydrate；persist 支持 incremental adjacency 更新。
4. **收口**：等价测试、profile counter、journal 数据表；不设 CI fail threshold。

## 非目标（M53 不做）

- per-query 临时 fact index（M52 已反证）
- DuckDB / Parquet
- browser real perf
- 公开 JSON 语义变更
- CI 性能门禁（M55）

## C 线（infra persist）— 暂缓

以下项 **不纳入 M53 默认范围**；仅当 B+A 完成后 core profile 仍显示 `redb_commit` / fragment I/O 为瓶颈时再开 spike：

- redb commit batching
- page-logic fragment 跨平台持久化（journal §4 描述为方向，代码未默认接入）
- runtime fingerprint sidecar

用户判断：C 线收益不确定，M53 以 **profile 门控** 决定是否追加任务。

## 架构

### B1 — Materialized availability facts

```
Graph load (LongLived)
  └─ RuntimeReadModel
       ├─ DenseGraphSnapshot
       └─ MaterializedAvailabilityFactsIndex  ← 一次扫描全图预收集 conditions
              └─ page logic / warm 复用 ConditionCollectorCache::from_prefilled
```

- 索引在 `RuntimeReadModel` 构建时生成，与 dense snapshot 同级。
- query-time 仍做 page_path 过滤与 `expand_total_row_count_gates`（页面相关），但不重复 `collect_condition_objects_for_node` 图遍历。
- 输出必须与无索引路径 **字节级等价**（现有 m52 shadow compare 风格）。

### B2 — Dense-native path finder

- `path_summary` 在 `dense_snapshot` 存在时走 CSR 邻接，避免 `GraphReadStore` owned clone adapter。
- 保留无 dense 时的现有路径（OneShot CLI）。

### A — redb v2

- `open_inner`：读 v2 → `hydrate_graph_from_v2`；miss/fingerprint 失败 fallback v1。
- `persist`：dirty node 增量更新 v2 adjacency；edges 表仍写 v1 直至 A2 验收完成（shadow compare 持续）。

## 验收

| 项 | 标准 |
|----|------|
| B1 | fixture + 等价测试；`availability_index_build_ms` 在带 index 的 long-lived 路径下降 |
| B2 | path 输出等价；`path_candidate_search_ms` 或 wall 下降 |
| A | `m52_redb_v2_shadow_tests` 扩展 + open hydrate 默认路径 |
| 文档 | journal 填 before/after 表；INDEX 补 spec/plan 链接 |

## 依赖

- Phase C 文档治理已完成
- M52 shadow layout：`src/graph_redb_v2.rs`
