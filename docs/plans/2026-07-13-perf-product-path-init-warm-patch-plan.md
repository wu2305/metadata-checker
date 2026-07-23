# 产品路径 / Init / Warm Patch 实施计划

> **For agentic workers:** 按任务顺序改；每任务跑影响面测试。重叠的差量刷新项见 [2026-07-12-diff-refresh-pipeline-plan.md](2026-07-12-diff-refresh-pipeline-plan.md) Task 5b/12b。

**Goal:** 落地 [patch design](../specs/2026-07-13-perf-product-path-init-warm-patch-design.md)：stdio LongLived、bench profile、Dense/Facts/Warm/元数据缓存、LongLived E2E + RSS。

**Architecture:** 产品路径强制 LongLived 读模型；Dense CSR 共享 edge payload；Facts 存 typed；warm materialization 按 budget 跳过无用 JSON；页面元数据 process-local cache。

**Tech Stack:** Rust 2024, Criterion, 既有 GraphRuntime / page_logic。

---

## 文件结构

| 文件 | 职责 |
|------|------|
| `src/stdio_server.rs` / `src/runtime.rs` | LongLived + status 字段 |
| `Cargo.toml` | `profile.bench` → `release-fast` |
| `src/dense_graph.rs` | 单次遍历 + `edge_idx` |
| `src/explain/condition_facts/conditions.rs` | `ConditionFact` + precollect |
| `src/query/page_logic/materialized_availability.rs` | typed index |
| `src/query/page_logic/path_summary.rs` | `PathMaterializeKeep` |
| `src/query/page_logic/prerequisites.rs` | budget 截断 |
| `src/query/page_logic/metadata.rs` | mtime cache |
| `benches/runtime_bench.rs` + `benches/common/rss.rs` | E2E + RSS |
| `docs/milestones/performance/performance-baseline.md` | 场景登记 |

---

### Task 1: stdio LongLived + status（已基本落地，补测）

- Modify: `src/stdio_server.rs`, `src/runtime.rs`
- Test: `tests/stdio_server_tests.rs`

- [x] load 使用 `RuntimeMode::LongLived`
- [x] `RuntimeStatus::{runtime_mode,read_model_ready}`
- [ ] 跑 `cargo test --features cli-local stdio_server_tests -- --nocapture` 相关用例

### Task 2: bench profile + LongLived E2E（已基本落地）

- [x] `[profile.bench] inherits = "release-fast"`
- [x] `runtime_long_lived_startup` / `warm_pages_{1,10,50}`
- [ ] 增加 RSS 采样钩子

### Task 3: DenseGraphSnapshot（已落地）

- [x] 单次出边遍历、`edge_idx`、去掉重复 payload

### Task 4: typed ConditionFact

- [ ] 定义 `ConditionFact` + `to_json`
- [ ] `precollect` / `MaterializedAvailabilityFactsIndex` 改 typed
- [ ] `cargo test --features cli-local m53_materialized_availability`

### Task 5: budget-aware warm

- [x] path JSON `PathMaterializeKeep`
- [x] prerequisites：因 compact 输出 `truncated_array(..., 5)` + summary 全量计数，**不**在 cache 层截断（保持字节等价）

### Task 6: 页面元数据 cache（已落地 ponytail）

- [x] process-local mtime/size cache

### Task 7: 文档

- [x] design approved
- [x] baseline / m54 journal 交叉链接本 spec
- [x] docs README 挂 patch 指针（非新 M id）
