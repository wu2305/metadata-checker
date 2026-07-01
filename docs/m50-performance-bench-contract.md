# M50 性能基线、Criterion Benchmark 与 OpenTelemetry 观测契约

> 状态：M50 起始契约。本文先规定怎么 bench、怎么记录、怎么上报，不调整查询算法。

## 目标

M50 的目标是让每个产品能力都有可重复的执行成本记录，并能把本地 benchmark 与远程观测平台串起来。性能结论必须以能力级耗时为主，fanout、节点数、边数、扩展路径数只作为解释变量。

核心分工：

- Criterion / runner 是性能测量真值，用于 P50 / P95 / max、回归阈值和优化验收。
- OpenTelemetry 是观测与上报通道，用于 traces、metrics、远程统计平台、真实调用趋势和告警。
- OpenTelemetry 指标可以帮助定位趋势，但不得替代 Criterion / runner 的 benchmark 结论。

## 观测分层

| 层级 | 目的 | 工具 |
|---|---|---|
| Black-box runner | 重复执行同一个产品能力，形成 P50 / P95 / max | `hyperfine`、stdio 脚本、browser CDP 脚本 |
| Micro benchmark | 验证内部算法或数据结构优化，作为本地性能真值 | Criterion.rs |
| Span breakdown | 记录一次能力调用内部阶段耗时 | Rust `tracing` span、browser `Performance.measure` |
| Metrics export | 记录能力耗时、调用次数、输出大小等观测指标 | OpenTelemetry Rust SDK metrics |
| Remote export | 接入远程统计平台，长期观察真实项目趋势 | OTLP HTTP/protobuf -> OpenTelemetry Collector |

## 产品能力集合

M50 第一批固定能力：

| capability | 入口 | 场景 |
|---|---|---|
| `query_page_logic` | CLI / stdio / browser | 大页面逻辑总览 |
| `explain_condition` | CLI / stdio | 组件显示、值来源、writer、availability |
| `query_model` | CLI / stdio | 单模型关系查询 |
| `query_dataflow` | CLI / stdio | DataFlow 内部展开 |
| `context` | CLI / stdio | 目标节点上下文闭包 |
| `build_graph` | CLI | 真实项目增量与冷构建 |
| `browser_analyze_selection` | browser extension | 真实 BI 选中组件后端到端分析 |

## Span 命名

Rust 侧插桩可以继续使用 `tracing` span 作为轻量标注入口；远程上报、metrics provider 和 OTLP exporter 必须通过标准 OpenTelemetry Rust SDK / OTLP crate 初始化。也就是说，`tracing` 只承担本地 span facade，不承担 benchmark 或指标聚合职责。

根 span 命名遵循：

```text
metadata_checker.capability.<capability>
```

阶段 span 命名遵循：

```text
metadata_checker.stage.<stage>
```

推荐阶段：

| stage | 含义 |
|---|---|
| `graph_load` | graphdb 加载 |
| `reload_check` | graphdb 指纹检查与可选 reload |
| `query_compute` | 查询主体计算 |
| `candidate_lookup` | 目标解析或候选查找 |
| `traversal` | 图遍历、路径搜索、evidence 收集 |
| `response_build` | answer contract / JSON value 构建 |
| `serialize` | 最终协议 envelope 序列化 |
| `remote_fetch` | browser / remote metadata 获取 |
| `wasm_runtime` | browser WASM 初始化和调用 |
| `render` | popup / visual graph 渲染 |

## Span 属性

所有根 span 必须尽量带上：

| 属性 | 示例 | 说明 |
|---|---|---|
| `metadata_checker.capability` | `query_page_logic` | 产品能力名 |
| `metadata_checker.mode` | `cli_cold` / `stdio_warm` / `browser_interactive` | 执行模式 |
| `metadata_checker.command` | `QueryPageLogic` | Rust tool command |
| `metadata_checker.target` | `page:xxx.spg` | 目标，必要时脱敏或截断 |
| `metadata_checker.budget` | `compact` | 输出预算 |
| `metadata_checker.intent` | `display` | 查询意图 |
| `metadata_checker.status` | `ok` / `error` | 执行结果 |
| `metadata_checker.graph.nodes` | `78114` | 图节点数 |
| `metadata_checker.graph.edges` | `150029` | 图边数 |
| `metadata_checker.output.bytes` | `142542` | 输出字节数 |
| `metadata_checker.timing.total_ms` | `3029` | 当前兼容字段中的总耗时 |

高 fanout 相关字段只作为解释变量：

| 属性 | 说明 |
|---|---|
| `metadata_checker.traversal.expanded_nodes` | 本次展开节点数 |
| `metadata_checker.traversal.expanded_edges` | 本次展开边数 |
| `metadata_checker.traversal.max_fanout` | 最大 fanout |
| `metadata_checker.traversal.candidate_count` | 候选节点数 |
| `metadata_checker.traversal.pruned_count` | 被剪枝数量 |

## 本地 JSON 记录

本地 runner artifact 使用 JSONL，每行记录一次能力调用。该 artifact 来自 runner 或 stdio replay，不来自 OpenTelemetry exporter：

```json
{
  "schema_version": 1,
  "commit": "391701f",
  "capability": "query_page_logic",
  "mode": "stdio_warm",
  "scenario": "real_large_page_compact",
  "status": "ok",
  "duration_ms": 3029,
  "p50_ms": null,
  "p95_ms": null,
  "max_ms": null,
  "timing": {
    "graph_load_ms": 0,
    "query_compute_ms": 2998,
    "serialize_ms": 31,
    "total_ms": 3029,
    "output_size_bytes": 142542
  },
  "dimensions": {
    "graph_nodes": 78114,
    "graph_edges": 150029,
    "budget": "compact",
    "target": "page:..."
  }
}
```

聚合报告再从 JSONL 生成 Markdown 或 JSON summary，不让业务代码承担统计聚合职责。

## OpenTelemetry Metrics

OTLP 模式至少记录这些指标：

| metric | 类型 | 说明 |
|---|---|---|
| `metadata_checker.capability.duration_ms` | histogram | 单次能力调用总耗时，仅用于观测趋势 |
| `metadata_checker.output.bytes` | histogram | 单次能力输出大小 |
| `metadata_checker.capability.calls` | counter | 能力调用次数 |

基础属性：

| 属性 | 说明 |
|---|---|
| `command` | Runtime tool command |
| `budget` | compact / normal / full |

后续可增加 `mode`、`scenario`、`status`、`project_hash`，但不得把 raw metadata、cookie、token、password 或完整组件 JSON 放入属性。

## CLI 契约

M50 允许显式打开 trace：

```bash
metadata-checker --trace json --project-dir <PROJECT> --query-page-logic <PAGE>
metadata-checker --trace otlp --otlp-endpoint http://localhost:4318/v1/traces --project-dir <PROJECT> --query-page-logic <PAGE>
metadata-checker --trace otlp --otlp-endpoint http://localhost:4318/v1/traces --otlp-metrics-endpoint http://localhost:4318/v1/metrics --project-dir <PROJECT> --query-page-logic <PAGE>
```

约束：

- 默认 `--trace off`，不影响普通 CLI 输出。
- `json` 模式只把 span/event 写到 stderr 或指定文件，stdout 仍保留机器协议输出。
- `otlp` 模式通过 OTLP HTTP/protobuf 输出 traces 和 metrics。
- `--otlp-metrics-endpoint` 未指定时，可从 `/v1/traces` endpoint 推导 `/v1/metrics`。
- 没有启用 `telemetry-otlp` feature 时，`--trace otlp` 必须给出稳定错误。
- stdio 模式不能向 stdout 写观测日志，只能写 stderr 或远程 exporter。

## Criterion 契约

Criterion 是内部性能测量的默认工具。M50 起至少保留以下可运行 bench：

```bash
CARGO_TARGET_DIR=target/criterion/parse cargo bench --bench parse_bench
CARGO_TARGET_DIR=target/criterion/query-micro cargo bench --bench query_micro_bench
CARGO_TARGET_DIR=target/criterion/rebuild cargo bench --bench rebuild_bench
CARGO_TARGET_DIR=target/criterion/runtime cargo bench --bench runtime_bench
CARGO_TARGET_DIR=target/criterion/query-matrix cargo bench --bench query_matrix_bench
CARGO_TARGET_DIR=target/criterion/redb cargo bench --bench redb_persistence_bench
```

或使用 Makefile：

```bash
make perf-fixture          # parse_bench + query_micro_bench（fixture，可进 CI）
make perf-real-mutation    # rebuild_bench
make perf-real-runtime     # runtime_bench
make perf-real-query       # query_matrix_bench
make perf-cli-cold         # hyperfine CLI cold start + trace boundary
make perf-redb             # redb_persistence_bench
make perf-stdio            # stdio_boundary_bench
make perf-telemetry        # telemetry_overhead_bench (requires --features telemetry)
make perf-session          # session_sync_bench
```

要求：

- bench fixture 使用稳定小图或 ignored/manual 真实项目，不依赖远程服务。
- bench 函数必须命名出能力和场景，例如 `query_page_logic_fanout_256`、`rebuild_cold_build_empty_graphdb`。
- rebuild / mutation bench 必须使用 Rust Criterion bench target，不使用单元测试或 JS runner 伪装。
- rebuild / mutation bench 必须在 `/tmp` sandbox 中复制真实项目元数据，不得修改真实项目目录。
- Criterion bench 建议每个 bench target 使用独立 `CARGO_TARGET_DIR=target/criterion/<bench>`，避免 release `panic=abort` 产物、bench unwind 产物以及 `rlib`/`cdylib` 同名输出共享默认 `target/release`。
- Criterion 输出用于回归判断；OpenTelemetry 指标只用于解释趋势。
- 优化提交必须同时说明 Criterion 结果和真实项目 runner 结果，不能只给远程观测截图。

## Runner 契约

M50 runner 最少需要支持：

| runner | 输出 | 用途 |
|---|---|---|
| `hyperfine` | JSON / Markdown | CLI cold start 和 build graph |
| stdio replay | JSONL | warm runtime 能力调用 |
| browser CDP acceptance | JSON | 真实 BI selection 分析 |
| Criterion | HTML / JSON | 内部函数热点与回归判断 |

真实项目 benchmark 默认手动执行，不进入普通 CI。fixture benchmark 可进入 CI，但阈值必须按 M50 首次基线设置，不得拍脑袋。

### CNB CI 分层（2026-06）

| 事件 | 进入 CI 的 bench |
|---|---|
| `main` push | `fixture-bench-ci`（Criterion parse/query-micro smoke）、`browser-offscreen-bench-ci`（WASM offscreen smoke）、四路并行 full criterion（query / rebuild / redb / boundary） |
| PR | `browser-offscreen-bench-ci` only（无 fixture-bench-ci） |
| 非 `main` push | 无性能 bench，仅 `rust-ci` |

`main` push 是 merge 后的权威记录来源；不在 `pull_request.merged` 重复执行。当前不设性能阈值 fail。

CI 启动成本通过 CNB 原生能力优化：

- browser WASM 相关 pipeline 使用 `.cnb/images/browser-wasm-ci.Dockerfile` 的 `docker.build` 缓存镜像，预装 Node、Rust、`wasm32-unknown-unknown`、匹配 `Cargo.lock` 的 `wasm-bindgen-cli` 与 `cargo-llvm-cov`。
- 同一缓存镜像预装 `bencher` CLI 与 `cargo-llvm-cov`；`rust-ci` 与 bench pipeline 共用该镜像。
- `docker.build.versionBy` 包含 Dockerfile 与 `Cargo.lock`，工具链或依赖变化才重建镜像。
- `rust-ci` 单次 `cargo llvm-cov test` 替代原 `cargo test` + `cargo llvm-cov test` 双跑；`target/debug` symlink 兼容集成测试。
- full criterion 拆为四路并行 pipeline（query / rebuild / redb / boundary），每路含 `criterion warmup` stage；per-bench 仍用独立 `CARGO_TARGET_DIR` 避免 `panic=abort` 与 bench unwind 冲突。
- browser WASM pipeline 使用独立 `CARGO_TARGET_DIR=target/cnb/...` 与 volume，避免默认 `target` 交叉污染。

Bencher.dev 上报策略：

- Bencher 配置由 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/bencher.yml` 注入到 `browser-offscreen-bench-ci`、`fixture-bench-ci` 与四路 full criterion pipeline。
- 仅当 CI 环境中配置 `BENCHER_API_KEY` 与 `BENCHER_PROJECT` 时上报；未配置时跳过，不影响现有 CI。
- `browser-offscreen-bench-ci` 将 `browser-offscreen-summary.json` 转成 Bencher Metric Format JSON，再通过 `bencher run --adapter json --file ...` 上报。
- `fixture-bench-ci` 与四路 full criterion pipeline 通过 `tools/run-bencher-criterion-ci.mjs` 捕获 Criterion 输出，再通过 `bencher run --adapter rust_criterion --file ...` 上报。
- `full-criterion-query-ci`：`query_matrix_bench`、`runtime_bench`、M51 profile reports。
- `full-criterion-rebuild-ci`：`rebuild_bench`（core rebuild）。
- `full-criterion-rebuild-crud-ci`：`rebuild_crud_bench`（CRUD 闭环）。
- `full-criterion-redb-ci`：`redb_persistence_bench`（含 v2 shadow persist 回归观测）。
- `full-criterion-boundary-ci`：`stdio_boundary_bench`、`telemetry_overhead_bench`、`session_sync_bench`。
- 除 `session_sync_bench` 外，完整真实项目 bench 需要真实项目目录。每路 full criterion pipeline 会先复用已有 `METADATA_CHECKER_REAL_PROJECT_DIR`；若未配置，则通过只读部署令牌 clone fixture 仓库，并导出 `METADATA_CHECKER_REAL_PROJECT_DIR` 给后续 stages。该目录缺失时主分支性能流水线应失败，避免把 skip 当作成功样本。
- `BENCHER_TESTBED` 默认 `cnb-amd64`；`BENCHER_UPLOAD_REQUIRED=true` 时，上报失败才阻塞 CI。
- 当前不设置 Bencher threshold，也不使用 `--error-on-alert`。阈值需要等 `main` 分支积累稳定样本后再启用。

真实项目 fixture 已上传到独立私有仓库 `wu2305/metadata-checker-real-fixtures`：

- fixture project path: `xiaoshouyi`
- fixture commit: `c3c0528fdd28e2600e0b0235040fb349b3c2d446`
- metadata files: 501 `.spg` + 828 `.tbl`

真实项目 fixture 的 CNB 密钥仓库文件固定为 `wu2305/metadata-checker-keys/real-fixture.yml`，并由
`.cnb.yml` 的四路 full criterion pipeline 通过 `imports` 引用。文件内容声明：

```yaml
allow_slugs:
  - wu2305/metadata-checker
allow_events:
  - push
allow_branches:
  - main

REAL_PROJECT_FIXTURE_REPO_SLUG: wu2305/metadata-checker-real-fixtures
REAL_PROJECT_FIXTURE_DEPLOY_TOKEN: <readonly-deploy-token>
REAL_PROJECT_FIXTURE_REF: c3c0528fdd28e2600e0b0235040fb349b3c2d446
REAL_PROJECT_FIXTURE_PROJECT_PATH: xiaoshouyi
```

也可以直接提供完整 URL：

```yaml
REAL_PROJECT_FIXTURE_REPO_URL: https://cnb.cool/wu2305/metadata-checker-real-fixtures.git
```

## 真实项目标准测试

真实项目标准测试入口：

```bash
make perf-real
make perf-real-mutation
make perf-real-runtime
make perf-real-query
make perf-cli-cold
```

等价脚本：

```bash
node tools/m50-real-project-perf.mjs \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  --graph-db-path /tmp/metadata-checker-m50-real-project.graphdb \
  --output-dir /tmp/metadata-checker-m50-perf \
  --iterations 5
```

默认行为：

- 使用 `release-fast` profile 构建二进制。
- graphdb 不存在时先构建；已有 graphdb 默认复用，`--rebuild-graph` 可强制重建。
- 启动一次 stdio runtime，重复执行固定 M50 场景，读取响应中的 `timing` 字段。
- 输出 JSONL 与 Markdown artifact 到 `/tmp/metadata-checker-m50-perf`。

真实项目 rebuild / mutation bench 使用 Rust Criterion：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/path/to/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/rebuild cargo bench --bench rebuild_bench
```

固定 rebuild 场景：

| scenario | bench | 含义 |
|---|---|---|
| `rebuild_cold_build_empty_graphdb` | `rebuild_bench` | 空 graphdb 上完整构建真实项目图 |
| `rebuild_noop_existing_graphdb` | `rebuild_bench` | 无文件变化时重扫、hash、跳过持久化 |
| `rebuild_dirty_single_tbl` | `rebuild_bench` | 单个真实 `.tbl` 内容变化后的增量 rebuild |
| `rebuild_deleted_single_file` | `rebuild_bench` | 删除一个真实元数据文件后的增量 rebuild |
| `rebuild_added_single_file` | `rebuild_bench` | 恢复/新增一个真实元数据文件后的增量 rebuild |
| `crud_update_component_visibility_then_query` | `rebuild_crud_bench` | 结构化修改组件 `visibleCondition` 后的 scan -> check_reload -> explain_condition |
| `crud_add_action_write_then_query_model` | `rebuild_crud_bench` | 追加 `updateData` 写模型动作后的 scan -> check_reload -> explain |
| `crud_delete_page_then_find_page` | `rebuild_crud_bench` | 删除真实页面后的 scan -> check_reload -> find_page |

`rebuild_dirty_single_spg` 由 `redb_persistence_bench` 的 `redb_incremental_persist_dirty_spg` 覆盖，避免与 rebuild 重复采样。

真实项目 runtime lifecycle bench：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/path/to/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/runtime cargo bench --bench runtime_bench
```

固定 runtime 场景：

| scenario | 含义 |
|---|---|
| `runtime_load_graphdb` | 已有真实 graphdb 加载为 GraphRuntime 的成本 |
| `runtime_status_warm` | warm runtime status 查询成本 |
| `runtime_check_reload_unchanged` | graphdb 未变化时 check_reload fast path |
| `runtime_reload_graph` | 强制 reload graphdb 成本 |
| `runtime_check_reload_changed` | graphdb 变化后 check_reload slow path |
| `runtime_load_locked_graphdb` | 外部 graphdb.lock 持锁时的加载失败路径 |
| `runtime_load_corrupt_graphdb` | 损坏 graphdb header 的诊断路径 |
| `runtime_query_page_with_check_reload` | 查询前先执行 check_reload 的 page 查询 |
| `runtime_explain_condition_with_check_reload` | 查询前先执行 check_reload 的 explain_condition |

真实项目查询矩阵 bench：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/path/to/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/query-matrix cargo bench --bench query_matrix_bench
```

固定 query matrix 场景：

| scenario | 含义 |
|---|---|
| `query_page_contract_compact` | 页面关系查询 |
| `query_page_contract_normal` | 页面关系查询（normal budget） |
| `query_page_member_registered_compact` | 第二个页面关系查询 |
| `query_cross_contract_member_compact` | 跨页面关系查询 |
| `query_dataflow_binding_car_compact` | 真实 DataFlow 展开 |
| `query_model_fact_qw_sidebar_compact` | 单模型关系查询 |
| `query_model_fact_qw_sidebar_normal` | 单模型关系查询（normal budget） |
| `explain_text41_compact` | 普通 explain |
| `explain_condition_input3_writer_compact` | writer 意图条件解释 |
| `explain_condition_input3_writer_normal` | writer 意图条件解释（normal budget） |
| `explain_condition_missing_target_compact` | 缺失目标的条件解释错误路径 |
| `explain_condition_text41_display_compact` | display 意图条件解释 |
| `explain_condition_text41_value_source_compact` | value-source 意图条件解释 |
| `explain_condition_model11_availability_compact` | availability 意图条件解释 |
| `advise_query_input3_writer_compact` | 查询建议生成 |
| `context_input3_depth2_normal` | depth=2 上下文闭包 |
| `context_input3_depth3_full` | depth=3 上下文闭包（full budget） |
| `query_page_logic_contract_full` | full budget 页面逻辑输出 |
| `query_page_logic_contract_normal` | normal budget 页面逻辑输出 |
| `query_page_logic_member_registered_compact` | 第二个页面逻辑查询 |
| `explain_condition_text41_full` | full budget 条件解释输出 |
| `find_page_member_registered_compact` | 页面 lookup |
| `find_page_wide_keyword_compact` | 宽关键词页面 lookup（「销售」） |
| `find_model_auto_customer_rel_compact` | 模型 lookup |
| `find_component_text41_compact` | 组件 lookup |
| `query_page_contract_compact_check_reload` | 查询前 check_reload 的 page 关系查询 |
| `explain_condition_input3_writer_compact_check_reload` | 查询前 check_reload 的 writer 条件解释 |

fixture 微基准 bench：

```bash
CARGO_TARGET_DIR=target/criterion/parse cargo bench --bench parse_bench
CARGO_TARGET_DIR=target/criterion/query-micro cargo bench --bench query_micro_bench
```

固定 fixture 场景：

| scenario | 含义 |
|---|---|
| `parse_spg_large_page` | 大页面 SPG 解析 |
| `parse_spg_actions_test` | 动作页 SPG 解析 |
| `parse_tbl_dataflow_table` | DataFlow TBL 解析 |
| `dependency_graph_large_page` | 依赖图构建 |
| `priority_analysis_large_page` | 计算优先级分析 |
| `parse_expr_refs_complex_expressions` | 复杂表达式引用提取 |
| `query_page_logic_fanout_{64,128,256}` | 合成图 page logic fanout |
| `query_page_fanout_{64,128,256}` | 合成图 page 关系查询 |
| `explain_component_fanout_256` | 合成图 explain |
| `query_page_logic_deep_tree_fanout_1024_depth_8` | 深层树合成图 page logic（1024 leaf / depth 8） |
| `context_dense_model_depth3_fanout_512` | 高密度模型上下文闭包（depth 3 / fanout 512） |
| `query_dataflow_pathology_outputs_{64}_full` | 多 output fields 的 DataFlow full 输出膨胀 |
| `query_dataflow_pathology_joins_{6}_filters_{8}_compact` | 多 Join/Union/Filter 叠加 DataFlow 展开 |

redb 持久化 bench：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/path/to/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/redb cargo bench --bench redb_persistence_bench
```

或使用：

```bash
make perf-redb
```

固定 redb 场景：

| scenario | 含义 |
|---|---|
| `redb_cold_open_readonly` | 冷打开已有 graphdb 只读 |
| `redb_lock_contention_open` | 外部 graphdb.lock 持锁时的打开失败路径 |
| `redb_corrupt_check_graphdb` | 损坏 graphdb header 的检查/诊断 |
| `redb_incremental_persist_dirty_spg` | 单个真实 `.spg` dirty 后的增量持久化 |
| `redb_load_file_states` | 加载 graphdb file_states 元数据 |
| `redb_scan_discover_diff` | discover + diff（不含 parse/apply/persist） |
| `redb_incremental_parse_apply` | 脏文件 parse + apply_graph_updates |
| `redb_incremental_persist_commit` | persist_index 提交（setup 完成 parse/apply） |

stdio 协议边界 bench：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/path/to/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/stdio cargo bench --bench stdio_boundary_bench
```

固定 stdio 场景：

| scenario | 含义 |
|---|---|
| `stdio_invalid_json_line` | 非法 JSON 行错误路径 |
| `stdio_unknown_command` | unknown command 错误路径 |
| `stdio_query_page_logic_full_large` | full budget 大页面逻辑输出 |
| `stdio_error_then_status_continue` | 错误后继续处理 status |

OpenTelemetry overhead bench：

```bash
make perf-telemetry
```

对比 `trace off` / `trace json` /（可选）`trace otlp` 下 warm stdio 往返成本。OTLP 需设置 `METADATA_CHECKER_OTLP_ENDPOINT`。

remote/session sync bench：

```bash
make perf-session
```

使用 `InMemoryRemoteSessionProvider` + recorded fixture，覆盖 `remote fetch -> mirror -> graph -> query`。

CLI cold start / trace boundary runner：

```bash
make perf-cli-cold
```

等价脚本：

```bash
node tools/cli-cold-start-perf.mjs \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  --graph-db-path /tmp/metadata-checker-m50-real-project.graphdb \
  --output-dir /tmp/metadata-checker-cli-cold-perf
```

固定 CLI cold 场景：

| scenario | 含义 |
|---|---|
| `cli_cold_explain_condition_compact` | 冷进程 `--explain-condition`（compact / writer） |
| `cli_cold_query_page_logic_full` | 冷进程 `--query-page-logic`（full budget） |
| `cli_trace_json_stdout_clean` | `--trace json` 时 stdout 保持机器协议 JSON |

固定场景：

| category | scenario | command | target | budget / intent |
|---|---|---|---|---|
| condition | `explain_condition_input3_writer` | `explain_condition` | `comp:app/销售.app/销售/合同协议.spg\|input3` | `compact` / `writer` |
| context | `context_input3_depth2` | `context` | `comp:app/销售.app/销售/合同协议.spg\|input3` | `normal` / depth 2 |
| model | `query_model_fact_qwSidebar` | `query_model` | `model:fact_qwSidebar` | `compact` |
| page_logic | `query_page_logic_contract` | `query_page_logic` | `page:app/销售.app/销售/合同协议.spg` | `compact` |
| condition | `explain_condition_text41_display` | `explain_condition` | `comp:app/售后.app/绑定车辆/会员已注册.spg\|text41` | `compact` / `display` |
| condition | `explain_condition_text41_value_source` | `explain_condition` | `comp:app/售后.app/绑定车辆/会员已注册.spg\|text41` | `compact` / `value-source` |
| condition | `explain_condition_model11_availability` | `explain_condition` | `model:app/售后.app/绑定车辆/会员已注册.spg\|model11` | `compact` / `availability` |
| page_logic | `query_page_logic_member_registered` | `query_page_logic` | `page:app/售后.app/绑定车辆/会员已注册.spg` | `compact` |
| lookup | `find_page_member_registered` | `find_page` | `会员已注册` | `compact` |
| lookup | `find_model_auto_customer_rel` | `find_model` | `fact_autoCustomerAutoRel` | `compact` |
| lookup | `find_component_text41` | `find_component` | `text41` | `compact` |
| lifecycle | `status_warm_runtime` | `status` | 无 | 无 |

记录口径：

- `build_binary_wall_ms`：release-fast 二进制构建耗时。
- `build_graph_wall_ms`：真实项目 graphdb 构建耗时。
- `stdio_process_wall_ms`：stdio runtime 进程总耗时，包含 graphdb 加载。
- JSONL 中 `record_type=lifecycle` 记录 build binary、build graph、stdio process 这类运行期生命周期成本。
- JSONL 中 `record_type=capability` 记录固定 stdio 场景的单次能力成本。
- 每条 JSONL 的 `timing.total_ms` / `query_compute_ms` / `output_size_bytes`：能力本身的 warm runtime 成本。
- Markdown 汇总中的 P50 / P95 / max 来自同一次 runner 的 JSONL。

CRUD / 增量场景不得直接修改真实项目目录。后续若要测数据创建、修改、删除的索引成本，应在 `/tmp` 复制真实项目子集或完整项目，独立构建 sandbox graphdb，再记录 create / update / delete / rebuild / query-after-mutation 的 wall time 与 warm query 结果。

## 验收标准

- 每个 M50 能力至少有一个固定命令或脚本能重复执行。
- 每次能力调用有根 span，且能看到 `query_compute` / `serialize` 等阶段信息。
- 默认构建不启用远程上报依赖。
- OTLP 模式能连接本地 OpenTelemetry Collector 或 Jaeger OTLP receiver。
- `docs/performance-baseline.md` 记录基线时必须写明 commit、命令、项目路径、graphdb 路径、runner、样本数和 P50 / P95 / max。

## 参考

- OpenTelemetry Rust：https://opentelemetry.io/docs/languages/rust/
- `tracing-opentelemetry`：https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/
- `opentelemetry-otlp`：https://docs.rs/opentelemetry-otlp/latest/opentelemetry_otlp/
- `hyperfine`：https://github.com/sharkdp/hyperfine
- Criterion.rs：https://bheisler.github.io/criterion.rs/book/getting_started.html
