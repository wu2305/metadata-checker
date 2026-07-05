# M51 性能堵点归因与优化方案

> 后续优化记录已进入 [M52 性能优化收口记录](m52-performance-optimization.md)。本文件保留 M51 的归因基线、阶段定义和第一批热点判断。

> 状态：M51 起始落地。M50 已完成 benchmark / CI / Bencher 上报；M51 聚焦确定堵点、形成优化方案，并只落低风险小修。

## 验收目标

M51 不以“大幅提速”作为验收目标，而以“每个核心慢场景都有证据、归因、方案、验收 benchmark”作为验收目标。

本阶段允许：

- 补 benchmark 内部阶段归因。
- 补 `tracing` / OpenTelemetry 阶段 span。
- 增加运行期内存索引、局部缓存或 GraphReadStore helper。
- 做语义不变的小修，例如复用邻接查询结果、减少重复 JSON 构建。

本阶段不做：

- redb 持久化 schema 改造。
- query 层绕过 `GraphReadStore` 直接依赖 redb。
- browser real perf。
- 以单次 Bencher 样本设置性能 fail threshold。

## M51 归因接口

### Bench 内归因

`query_page_logic` 已新增 profiling API：

```rust
metadata_checker::query::build_query_page_logic_output_profiled(
    graph,
    page_id,
    project_dir,
    budget,
)
```

返回：

- 原始 JSON 输出。
- `PerfProfile`，包含 `capability`、`wall_duration_ms`、`stages`、`counters`。

该 API 只供 benchmark / profiling runner 使用，不改变默认 CLI / stdio / browser 输出。

### Profile Report

M51 已提供 Rust 原生阶段报告 runner：

```bash
cargo run --bin m51_profile_report -- \
  --project-dir <PROJECT> \
  --output target/m51-profile/page-logic-profile.json \
  --sample-count 3
```

默认会跑第一批 P0 页面逻辑场景：

- `query_page_logic_contract_full`
- `query_page_logic_contract_normal`
- `query_page_logic_member_registered_compact`

也可以通过 `--scenario name|page_id|budget` 指定场景，参数可重复。

报告 JSON 结构包含：

- `samples[].profile.stages`：每次采样的完整阶段耗时。
- `samples[].profile.counters`：每次采样的复杂度变量。
- `stage_summary`：按阶段聚合的 `min_ms` / `max_ms` / `avg_ms`。
- `counter_summary`：按 counter 聚合的 `min` / `max` / `avg`。
- `output_bytes`：对应输出 JSON 大小，用于解释 full output 放大成本。

CNB `main.push` 的 `full-criterion-bench-ci` 会在 `query matrix bench` 后生成：

```text
target/criterion-ci/reports/m51-page-logic-profile.json
target/criterion-ci/reports/m51-core-profile.json
```

这些报告先用于阶段瓶颈定位与 CI 日志留存，不进入 Bencher 趋势指标，也不设置 fail threshold。

非 page-logic 核心慢场景可以单独运行：

```bash
cargo run --bin m51_core_profile_report -- \
  --project-dir <PROJECT> \
  --output target/m51-profile/core-profile.json \
  --sample-count 1
```

该报告覆盖：

- `rebuild`：`rebuild_cold_build`、`rebuild_noop`。
- `redb`：`redb_open`、`redb_load_file_states`、`redb_check_graphdb`。
- `runtime`：`runtime_load`、`runtime_status_query`、`runtime_check_reload_unchanged`。

当前固定阶段名：

| stage | 含义 |
|---|---|
| `target_lookup` | page node 查找或候选诊断 |
| `collect_page_nodes` | 收集页面下组件与 action |
| `load_page_metadata` | 从 `.spg` 原文件读取 page inputs、action meta、visibility rules |
| `entrypoint_scan` | 识别有触发动作的入口组件 |
| `edge_scan` | 扫描 component / action 出边，分类 data sources、writes、navigation |
| `action_flow_build` | 聚合 action reads/writes/navigation/params |
| `prerequisites` | 收集 display/data/action prerequisites |
| `path_summary` | 构造 primary / supporting / related / rejected paths |
| `diagnostics` | 构造风险诊断与 related context summary |
| `key_model_availability` | 对关键模型递归 explain availability |
| `output_build` | 构造 summary/details 主体 |
| `evidence_build` | 构造 evidence、compact evidence summary |
| `validation` | 输出契约 validate |

当前固定 counters：

| counter | 含义 |
|---|---|
| `child_components` | 页面内组件数 |
| `child_actions` | 页面内 action 数 |
| `edges_scanned` | 本次 page logic 中被扫描的邻接边数量 |
| `entrypoints` | 用户入口组件数 |
| `data_sources` | 读取目标数量 |
| `write_targets` | 写入目标数量 |
| `navigation` | 跳转/参数传递数量 |
| `action_flows` | action flow 数量 |
| `primary_paths` / `related_context` / `candidate_paths` / `supporting_paths` / `rejected_paths` | 路径分类数量 |
| `key_models` / `key_model_availability` | 关键模型及 availability 摘要数量 |
| `evidence` | 最终 evidence 数量 |
| `path_*_ms` / `path_*_anchors` / `path_*_candidates` | `path_summary` 内部归因与规模 |
| `prerequisites_*_ms` / `prerequisites_*_nodes` | prerequisites 内部归因与规模 |
| `key_model_availability_fast_path` | 通过轻量 availability 路径构造的 key model 摘要数量 |

### Trace 归因

`query_page_logic` 阶段会在 `telemetry` feature 下输出通用 span：

```text
metadata_checker.stage
```

关键属性：

| 属性 | 含义 |
|---|---|
| `metadata_checker.stage.name` | 例如 `query_page_logic.edge_scan` |
| `metadata_checker.timing.stage_ms` | 阶段耗时 |
| `metadata_checker.status` | `ok` |

示例：

```bash
cargo build --profile release-fast --features telemetry
./target/release-fast/metadata-checker \
  --trace json \
  --project-dir <PROJECT> \
  --graph-db-path <GRAPHDB> \
  --query-page-logic 'page:app/销售.app/销售/合同协议.spg' \
  --budget compact \
  >/tmp/m51-page-logic.json \
  2>/tmp/m51-page-logic-trace.jsonl
```

## 第一批热点判断

基于 M50 验收记录，M51 第一批热点按收益优先级排序：

| 优先级 | 场景 | M50 信号 | 初始假设 | M51 动作 |
|---|---|---|---|---|
| P0 | `query_page_logic_contract_full` / `normal` | 单场景约 11-13s | 可能集中在 path summary、key model availability、full details JSON 构建 | 用 profiled API 拆阶段，优先确认是否可通过局部缓存/索引降低重复遍历 |
| P0 | `query_page_logic_member_registered_compact` | 约 3.2s | 复杂页面即使 compact 也受 page logic 主流程影响 | 与合同协议页对比 counters，找复杂度解释变量 |
| P1 | `rebuild mutation bench` | full CI stage 约 1,437s | 冷构建、dirty parse/apply、persist commit 中至少一个阶段占主导 | 后续给 rebuild bench 增加同名阶段归因 |
| P1 | `redb_persistence_bench` | full CI stage 约 512s | open/file_states/diff/persist 成本需拆分 | 优先确认是否是 redb 打开或 persist commit |
| P1 | `runtime_check_reload_*` | 已有 benchmark 覆盖 | check_reload slow path 可能重复加载 graphdb | 先看 trace，再决定是否需要 fingerprint 快速路径 |
| P2 | browser offscreen | CI smoke 约 39s | 主要用于链路可用性，不先做 real perf | 只保留 WASM 阶段归因，不拉浏览器 |

## 已落低风险小修

`query_page_logic` 现在在单次调用内缓存节点邻接结果：

- 缓存只在本次 page logic 调用内存在。
- 不改变 `GraphReadStore` trait。
- 不改变输出结构。
- 避免 entrypoint scan、edge scan、action flow build 对同一 component/action 重复调用 `get_node_edges`。

验收测试：

```bash
cargo test --test m51_page_logic_profile_tests
```

该测试验证：

- profiled API 的 JSON 输出与原 `build_query_page_logic_output` 完全一致。
- profile 包含 `collect_page_nodes`、`edge_scan`、`output_build`。
- profile 暴露 `child_components` 与 `edges_scanned` counters。

### 2026-06-21 第一轮热点修复

真实项目 M51 profile 初始结论：

- `output_build` 原计时窗口包含了 `key_model_availability`，属于阶段嵌套导致的重复归因。
- 修正计时窗口后，合同页 `full/normal` 的首要热点为 `key_model_availability`，其次为 `path_summary`。
- 会员已注册页 `compact` 也由 `key_model_availability` 主导，但 compact 输出最终只展示 3 个 key model。

已落修复：

- `output_build` 计时起点移动到 `key_model_availability` 之后，使 stage 互斥。
- `compact` 预算下只展开可见的 3 个 `key_model_availability` entry，同时保留完整 `total_count`。
- `resolve_model_target_in_page` 优先从模型入边反查当前页面引用路径，避免每个模型都重复遍历整页子树；找不到时仍 fallback 到旧逻辑。
- `Availability` intent 不再构建 value-source 上下文，避免混入非 availability 语义的额外工作。

验证要点：

- `m51_compact_page_logic_only_expands_visible_key_model_availability`：锁定 compact 只展开 3 个 entry，仍保留总数。
- `m51_availability_intent_uses_incoming_model_edges_for_page_scope`：锁定 page-scoped model 解析不再触发整页子树扫描。
- 真实项目报告显示 `query_page_logic_member_registered_compact` 的 `key_model_availability` 从约 798ms 降至约 116-149ms；`full/normal` 仍需要后续 fast path 处理完整 88 个模型展开。

### 2026-06-21 第二轮 M51 收口

已新增 `build_explain_availability_fast_output`，供 page logic 内嵌 `key_model_availability` 使用：

- 只返回 page logic 消费的 `answer_facts.availability_facts`、`data_empty_gates`、`truncation_guard`。
- 不改变默认 explain-condition 输出。
- `m51_key_model_availability_uses_fast_path_for_expanded_models` 锁定 normal/full 展开的 entry 都走 fast path，且保留 availability facts。

真实项目复测结论：

- 该 fast path 建立了可测接口和 counter，但没有单独解决 P0。full/normal 仍被 88 个模型的 availability fact / dataflow 解析主导。
- M52 若继续压 `key_model_availability`，应优先做 page-local availability cache 或批量 dataflow resolution，而不是继续优化 output shaping。

`path_summary` 已拆出内部 counters：

- `path_anchor_extract_ms`
- `path_candidate_search_ms`
- `path_field_candidate_build_ms`
- `path_dedup_ms`
- `path_classification_ms`
- `path_json_build_ms`
- `path_side_context_ms`
- `path_sort_ms`

真实项目 `sample-count=1` 对比显示，去重 anchor 后：

| 场景 | `path_summary` before | after | 主因 |
|---|---:|---:|---|
| `query_page_logic_contract_full` | 2373ms | 1320ms | anchor extract / candidate search 下降 |
| `query_page_logic_contract_normal` | 2441ms | 1326ms | anchor extract / candidate search 下降 |
| `query_page_logic_member_registered_compact` | 667ms | 470ms | anchor extract / candidate search 下降 |

已落修复：

- `AnchorExtractor` 对 source / sink / bridge / excluded anchors 保序去重。
- excluded anchor 收集改为按唯一 model target 遍历，避免同一模型重复读取入边。
- `m51_profiled_page_logic_records_path_summary_breakdown` 和 `anchor_dedup_preserves_first_seen_order` 锁定该归因与去重行为。

`prerequisites` 已拆出内部 counters：

- `prerequisites_component_scan_ms`
- `prerequisites_action_scan_ms`
- `prerequisites_data_source_scan_ms`
- `prerequisites_sort_ms`
- `prerequisites_component_nodes`
- `prerequisites_action_nodes`
- `prerequisites_data_source_nodes`
- `prerequisites_unique_conditions`

真实项目 `sample-count=1` 结论：

- 合同页 `prerequisites` 约 89-106ms，几乎全部在 `prerequisites_data_source_scan_ms`。
- 会员已注册页 `prerequisites` 约 62ms，也由 data source target 扫描主导。
- 组件/action 扫描与 sort 成本很低，当前无需优先优化。

非 page-logic 核心慢场景已补 `m51_core_profile_report`：

- `rebuild` 覆盖 cold build 与 noop rebuild。
- `redb` 覆盖 open、load file states、check graphdb。
- `runtime` 覆盖 load、status query、unchanged check_reload。
- CNB `full-criterion-bench-ci` 生成 `target/criterion-ci/reports/m51-core-profile.json`，只做报告留存，不进 Bencher，不设置阈值。

当前 M51 验收判断：

- page logic 三个真实项目场景已有稳定 stage/counter report。
- compact availability 已完成实质优化。
- full/normal availability 已有 fast-path 接口和明确后续方向，但仍是 M52 级优化重点。
- `path_summary` 已从黑盒拆开，并落了低风险去重修复。
- `prerequisites` 已从黑盒拆开，确认 data source target 扫描是主因。
- rebuild / redb / runtime 已有基础 profile report，后续可在 M52 继续扩展 dirty mutation / persist commit 细分。

## 后续优化决策规则

每个 M51 优化候选必须先补证据，再落实现：

1. 用 Criterion / profiled API / trace 标出主导阶段。
2. 写出复杂度解释变量，例如组件数、action 数、edges scanned、primary paths、output bytes。
3. 确认优化不改变输出语义。
4. 指定唯一验收 benchmark。
5. 优化后记录同一 benchmark 的前后对比。

优先采用：

- page-local cache。
- runtime 内存索引。
- GraphReadStore helper。
- 减少中间 JSON clone。

暂缓采用：

- redb schema 改造。
- 独立图数据库替换。
- browser real perf。
- 大范围查询语义重写。
