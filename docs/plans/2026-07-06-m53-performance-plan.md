# M53 性能优化实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 落地 M53 B1（materialized availability facts）→ B2（dense-native path finder）→ A（redb v2 hydrate + incremental persist），C 线仅 profile 门控。

**Architecture:** Long-lived runtime 在 load 时构建 `MaterializedAvailabilityFactsIndex` 与 `DenseGraphSnapshot`；page logic 优先消费物化索引与 CSR；GraphDB open 优先 v2 hydrate。详见 [spec](../specs/2026-07-06-m53-performance-design.md)。

**Tech Stack:** Rust 2024, redb, petgraph, existing `PerfProfile` / `m52_*` 测试模式。

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `src/query/page_logic/materialized_availability.rs` | 全图 conditions 预收集索引 |
| `src/explain/condition_facts/conditions.rs` | `precollect_all_node_conditions` + `ConditionCollectorCache::from_prefilled` |
| `src/query/page_logic.rs` | batch / warm / inner 接入 optional index |
| `src/runtime.rs` | `RuntimeReadModel` 持有 index |
| `src/query/page_logic/path_summary.rs` | B2 dense-native finder |
| `src/graph_redb.rs` | A hydrate + incremental persist |
| `tests/m53_materialized_availability_tests.rs` | B1 等价与 counter |
| `tests/m53_redb_v2_hydrate_tests.rs` | A hydrate 默认路径 |

---

### Task 1: Materialized availability facts 索引

**Files:**
- Create: `src/query/page_logic/materialized_availability.rs`
- Modify: `src/explain/condition_facts/conditions.rs`
- Modify: `src/query/page_logic.rs`
- Modify: `src/runtime.rs`
- Modify: `src/query.rs` (pub use)
- Test: `tests/m53_materialized_availability_tests.rs`

- [ ] **Step 1: 写失败测试**

```rust
// tests/m53_materialized_availability_tests.rs
#[test]
fn m53_materialized_index_matches_baseline_output() -> anyhow::Result<()> {
    let graph = fixture_graph("materialized-equiv")?;
    let page_id = "page:app/actions_test.spg";
    let index = MaterializedAvailabilityFactsIndex::build(&graph)?;
    let (baseline, _) = build_query_page_logic_output_profiled(&graph, page_id, None, "normal")?;
    let (indexed, profile) = build_query_page_logic_output_profiled_with_materialized_availability(
        &graph, Some(&index), page_id, None, "normal",
    )?;
    assert_eq!(canonicalize(baseline), canonicalize(indexed));
    assert!(profile.counter("availability_materialized_hits") > 0);
}
```

- [ ] **Step 2: 运行测试确认 FAIL**

Run: `cargo test m53_materialized_index_matches_baseline_output --features cli-local`
Expected: FAIL — symbol / API 未定义

- [ ] **Step 3: 实现索引与接入**

- `precollect_all_node_conditions(graph) -> HashMap<String, Vec<Value>>`
- `MaterializedAvailabilityFactsIndex::build`
- `ConditionCollectorCache::from_prefilled(Arc<HashMap<...>>)`
- `build_key_model_availability_batch(..., materialized: Option<&MaterializedAvailabilityFactsIndex>)`
- `RuntimeReadModel.availability_facts: Arc<MaterializedAvailabilityFactsIndex>`
- 在 `DenseGraphSnapshot::from_graph` 同级构建 index

- [ ] **Step 4: 测试 PASS**

Run: `cargo test m53_materialized --features cli-local`

- [ ] **Step 5: 更新 m53 journal B1 小节**

---

### Task 2: Long-lived runtime 默认传递 materialized index

**Files:**
- Modify: `src/runtime.rs`（query dispatch 传 index）
- Modify: `tests/m52_page_logic_profile_tests.rs` 或 `tests/m53_*`（warm + index 组合）

- [ ] **Step 1: 测试 warm 路径在 index 存在时 `availability_index_build_ms` 更低**
- [ ] **Step 2: `GraphRuntime::execute_query` / dispatch 传入 `read_model.availability_facts`**
- [ ] **Step 3: `cargo test m52_page_logic m53_materialized --features cli-local`**

---

### Task 3: Dense-native path finder (B2)

**Files:**
- Modify: `src/query/page_logic/path_summary.rs`
- Modify: `src/dense_graph.rs`（如需邻接查询 helper）
- Test: `tests/m53_dense_path_finder_tests.rs`

- [ ] **Step 1: 等价测试 — dense vs 非 dense path_summary 输出一致**
- [ ] **Step 2: `BoundedCausalPathFinder` 增加 `DenseGraphSnapshot` 邻接读取分支**
- [ ] **Step 3: profile counter `path_dense_adjacency_hits`**
- [ ] **Step 4: `cargo test m53_dense --features cli-local`**

---

### Task 4: redb v2 hydrate 默认路径 (A1)

**Files:**
- Modify: `src/graph_redb.rs` — `open_inner`
- Test: `tests/m53_redb_v2_hydrate_tests.rs`

- [ ] **Step 1: 测试 persist 后 `GraphDB::open` 与 v1 hydrate 图等价**
- [ ] **Step 2: `open_inner` 先 `read_v2_layout` + `hydrate_graph_from_v2`，失败 fallback v1**
- [ ] **Step 3: 扩展 `m52_redb_v2_shadow_tests` fingerprint / miss 场景**
- [ ] **Step 4: `cargo test m52_redb_v2 m53_redb_v2 --features cli-local`**

---

### Task 5: redb v2 incremental persist (A2)

**Files:**
- Modify: `src/graph_redb.rs` — `persist`
- Modify: `src/graph_redb_v2.rs` — incremental adjacency patch helpers

- [ ] **Step 1: 测试 dirty 单节点更新后 shadow compare 仍等价**
- [ ] **Step 2: v2 邻接增量 patch；v1 edges 仍全量 rewrite（保持兼容）**
- [ ] **Step 3: scanner 增量 fixture 回归**

---

### Task 6: 收口

**Files:**
- Modify: `docs/milestones/performance/m53-performance-continuation.md`
- Modify: `docs/milestones/INDEX.md`（spec/plan 链接）

- [ ] **Step 1: journal 填 B1/B2/A before/after**
- [ ] **Step 2: INDEX 更新 spec/plan 列**
- [ ] **Step 3: `cargo test --features cli-local` 影响面测试**

---

### Task 7 (可选 / profile 门控): C 线 spike

仅当 Task 6 后 core profile 显示 `redb_commit` &gt; 2s 或 fragment I/O 可测：

- [ ] redb commit batch 原型 + 计时
- [ ] native fragment round-trip（若 journal §4 仍无代码）

**不阻塞 M53 done。**

---

## Self-Review

| Spec 要求 | Task |
|-----------|------|
| B1 materialized facts | 1, 2 |
| B2 dense path | 3 |
| A hydrate | 4 |
| A incremental | 5 |
| 等价测试 + journal | 6 |
| C 暂缓 | 7 optional |
| 不做 fail threshold | — |
