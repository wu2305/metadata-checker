# M53 init/load 分层观测补全设计

> 状态：approved（用户确认 2026-07-09）
> 范围：把 LongLived runtime 加载阶段的合并计时拆成三段独立计时（dense snapshot / availability facts / page dependency index），接入 Criterion + Bencher trend 做长期回归可视化；顺带修复 `PageDependencyIndex::build` 的容错风格不一致问题。**不含** fragment 跨平台持久化（已确认放弃，移出 M53，待未来有真实跨 session 复用 warm cache 需求时再评估）、不改 JSONL runner（`tools/m50-real-project-perf.mjs` / `make perf-real`）、不设 CI fail 阈值/Bencher alert。

## 背景

M53 journal「剩余/后续」第 6 项：「init/load 分层观测补全（graph load / dense / read model / warm）」。当前 `src/runtime.rs` 中：

```rust
let read_model_build_ms = if build_read_model {
    read_model_started.elapsed().as_millis()
} else {
    0
};
let dense_snapshot_build_ms = read_model_build_ms;
```

`dense_snapshot_build_ms` 只是 `read_model_build_ms` 的别名，两者是同一个数字。而这段合并计时实际覆盖三步：

1. `DenseGraphSnapshot::from_graph(&graph)`
2. `MaterializedAvailabilityFactsIndex::build(&graph)`
3. `PageDependencyIndex::build(&graph)`（M53 T3 新增，见 `docs/specs/2026-07-07-m53-warm-cache-scalability-design.md`）

无法判断三步各自占比，这挡住了后续决策（例如 B1.2 dataflow facts 是否值得投入、PageDependencyIndex 的常驻加载成本是否可接受）。

顺带发现（上一轮 code review）：`PageDependencyIndex::build(&graph).map(Arc::new)?` 用硬 `?` 传播错误，与同一代码块里 `dense_graph`（`.ok()` 降级）、`availability_facts`（`match ... Err => diagnostics.push + fallback`）的容错风格不一致——图存储读取失败时，前两者会降级为「该能力不可用」但 LongLived runtime 仍能加载，唯独 `page_dependency_index` 会让整个 LongLived 加载失败。本次拆分计时正好会重新经过这块代码，一并修复。

## 目标

1. 把 `read_model_build_ms` 拆成三段独立计时，三段之和等于原来的合并值。
2. `PageDependencyIndex::build` 失败时降级为空索引 + diagnostics，不再让整个 LongLived 加载失败。
3. 三段计时接入 Criterion bench，通过现有 `tools/run-bencher-criterion-ci.mjs` 自动进入 Bencher trend，供长期人工观察是否变慢；不设 CI fail gate、不设 Bencher alert/threshold（维持 `docs/milestones/performance/performance-baseline.md` 现有约定）。

## 非目标

- fragment 跨平台持久化（native 文件 / IndexedDB / memory 后端统一契约）——旧 spike 在未合并分支 `codex/m52-performance-optimization`，与当前 `main` 代码结构差异较大且当前无明确跨 session 复用 warm cache 的需求，移出本次范围。
- 修改 `tools/m50-real-project-perf.mjs`（`make perf-real` JSONL/Markdown runner）的输出口径——该 runner 用于一次性真实项目叙事快照，与 Criterion/Bencher 的长期趋势目的不同，本次不动。
- 设置任何性能 CI fail 阈值或 Bencher alert。
- 改变任何默认查询输出的公开 JSON 契约。

## 架构

### 1. 运行时字段拆分（`src/runtime.rs`）

`GraphRuntime` 新增两个公开字段：

```rust
/// availability facts 物化索引构建耗时（毫秒）。
pub availability_facts_build_ms: u128,
/// page 反向依赖索引构建耗时（毫秒）。
pub page_dependency_index_build_ms: u128,
```

`load_with_project_dir_internal` 内部改为对三步分别计时（各自独立 `Instant::now()`），例如：

```rust
let dense_started = Instant::now();
let dense_graph = DenseGraphSnapshot::from_graph(&graph).ok().map(Arc::new);
let dense_snapshot_build_ms = dense_started.elapsed().as_millis();

let facts_started = Instant::now();
let availability_facts = match MaterializedAvailabilityFactsIndex::build(&graph) { ... };
let availability_facts_build_ms = facts_started.elapsed().as_millis();

let page_dep_started = Instant::now();
let page_dependency_index = match PageDependencyIndex::build(&graph) {
    Ok(index) => Arc::new(index),
    Err(error) => {
        diagnostics.push(format!("PageDependencyIndex build failed: {error}; using empty index"));
        Arc::new(PageDependencyIndex::empty())
    }
};
let page_dependency_index_build_ms = page_dep_started.elapsed().as_millis();

let read_model_build_ms = dense_snapshot_build_ms + availability_facts_build_ms + page_dependency_index_build_ms;
```

`dense_snapshot_build_ms` 不再是 `read_model_build_ms` 的别名，而是真实只覆盖第一步；`read_model_build_ms` 保留为三段之和（字段名、语义边界、现有测试断言 `> 0` / `== 0` 均不变）。

`OneShot` 模式（`build_read_model == false`）下三个新字段均为 `0`，与现有 `dense_snapshot_build_ms`/`read_model_build_ms` 行为一致。

### 2. `PageDependencyIndex::empty()`（`src/query/page_logic/page_diff.rs`）

新增：

```rust
/// 构造一个空的反向依赖索引（降级路径使用）。
pub fn empty() -> Self {
    Self { node_to_pages: HashMap::new() }
}
```

`affected_pages` 在空索引下自然返回空集合，`invalidate_pages_for_dirty_nodes` 表现为「无受影响页面」，不引入新的错误分支。

### 3. profile 报告（`src/perf_report.rs`）

新增两个 counter：`runtime_availability_facts_build_ms`、`runtime_page_dependency_index_build_ms`。保留 `runtime_dense_snapshot_build_ms`（现在是真实值）、`runtime_long_lived_read_model_build_ms`（三段之和，字段名不变）。

### 4. Criterion bench + Bencher trend（`benches/runtime_bench.rs`）

新增三个 benchmark，复用文件内已有的 `require_real_project_dir` + `create_indexed_workspace`/`GraphDB::open` 模式。三步都是只读、无副作用操作，图只需打开一次，循环体内只调用单步函数：

```rust
fn bench_runtime_load_dense_snapshot_build(c: &mut Criterion, source_project_dir: &Path) {
    let workspace = create_indexed_workspace("runtime-load-dense", source_project_dir)
        .expect("create workspace");
    let graph = GraphDB::open(&workspace.db_path).expect("open graphdb");
    c.bench_function("runtime_load_dense_snapshot_build", |bench| {
        bench.iter(|| black_box(DenseGraphSnapshot::from_graph(black_box(&graph)).ok()));
    });
}
```

同样模式新增 `runtime_load_availability_facts_build`（调用 `MaterializedAvailabilityFactsIndex::build`）、`runtime_load_page_dependency_index_build`（调用 `PageDependencyIndex::build`）。`MaterializedAvailabilityFactsIndex`、`PageDependencyIndex` 均已通过 `metadata_checker::query::{...}` 对外可见（T1-T3 阶段已 `pub use`），bench 直接引用即可，无需新增导出。三个函数注册进现有 `bench_runtime_scenarios`。

`tools/run-bencher-criterion-ci.mjs` 用 `rust_criterion` adapter 透传整份 Criterion stdout，新增的 benchmark ID 会自动出现在 Bencher trend，无需改 CI 脚本、Makefile 或流水线配置。

### 5. 文档

- `docs/milestones/performance/performance-baseline.md`「Runtime lifecycle Criterion bench」表新增三行场景说明。
- `docs/milestones/performance/m53-performance-continuation.md`：把「剩余/后续」第 6 项标记完成，补一次真实项目采样的三段耗时占比。
- `docs/milestones/INDEX.md`：M53 状态保持 `done`，不需要改动（本项是收尾观测补全，不影响里程碑整体状态）。

## 测试

1. **和为总数**：LongLived 成功路径下，新增断言 `dense_snapshot_build_ms + availability_facts_build_ms + page_dependency_index_build_ms == read_model_build_ms`。
2. **降级路径**：注入图读取失败场景（参考现有 `dense_graph`/`availability_facts` 降级测试的注入手法），验证 `PageDependencyIndex::build` 失败时：LongLived runtime 仍加载成功、diagnostics 含对应文案、`page_dependency_index_build_ms` 仍被正确记录（非跳过）、`invalidate_pages_for_dirty_nodes` 在空索引下返回空列表而非报错。
3. **OneShot 不变**：`OneShot` 模式下三个新字段均为 `0`（复用现有 `runtime.dense_snapshot_build_ms == 0` 断言模式扩展）。
4. **profile report**：`tests/m51_profile_report_tests.rs` 补充新 counter key 存在性断言（不校验具体数值，保持现有 style）。
5. **Criterion bench 手动验证**：`cargo bench --bench runtime_bench`（真实项目环境）确认三个新 benchmark 能正确注册并产出非零耗时；不要求写入 `cargo test`。

## 验收标准

| 项 | 标准 |
|----|------|
| 计时拆分 | 三段计时之和恒等于原 `read_model_build_ms`；`cargo test --features cli-local` 影响面测试全过 |
| 容错修复 | `PageDependencyIndex::build` 失败不再导致 LongLived runtime 加载失败；有对应降级测试覆盖 |
| Bencher 接入 | 三个新 Criterion benchmark 注册成功，`cargo bench --bench runtime_bench` 本地跑通；不改 CI/Makefile 配置 |
| 全局 | 不改变任何默认查询输出的公开 JSON 契约；`cargo check` 无新增警告；不设性能 fail 阈值 |

## 依赖

- M53 warm cache 可扩展性设计（`docs/specs/2026-07-07-m53-warm-cache-scalability-design.md`）已完成，`PageDependencyIndex` 已存在于 `src/query/page_logic/page_diff.rs`。
