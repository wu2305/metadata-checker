# 性能基线

M26-M30 文档：记录 CLI 冷查询、stdio server 热查询、统一 timing 与容量治理基线。

## 测试环境

- 项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- GraphDB 路径：`/tmp/m23_runtime_test.graphdb`（历史命名沿用，内容为本轮 M26 验收图）
- GraphDB 大小：269 MB（`ls -lh` 显示约 257 MiB）
- 节点数：78,114 | 边数：150,029
- 硬件：Apple Silicon M3 (arm64), macOS
- 采集版本：`036efcd`（包含 M30 stdio timing/capacity 修改）
- 编译：`cargo build`（debug 模式）

## 性能指标定义

| 指标 | 说明 |
|---|---|
| `graph_load_ms` | GraphDB 全量加载耗时（redb 读取 + 内存图构建） |
| `query_compute_ms` | 查询计算耗时（不含 graph 加载和序列化） |
| `serialize_ms` | JSON 序列化耗时 |
| `total_ms` | 整个请求总耗时 |
| `output_size_bytes` | stdio 响应 `timing.output_size_bytes`，最终 stdout JSON 行字节数 |
| `output_size_kb` | 响应 JSON 大小，人工阅读时可由 `output_size_bytes / 1024` 换算 |
| `wall_time_s` | 进程 wall-clock 时间（含启动/加载） |

---

## 基线 1：CLI 冷查询

命令：
```bash
time ./target/debug/metadata-checker \
  --project-dir /path/to/project \
  --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' \
  --budget compact \
  --graph-db-path /tmp/m23_runtime_test.graphdb
```

实测结果（2026-05-15）：

| 指标 | 实测值 |
|---|---|
| wall_time_s | 6.59s |
| output_size_kb | 121KB |

**瓶颈**：冷查询 99%+ 时间花在 graphdb 全量加载（redb 反序列化 + petgraph 构建）。

---

## 基线 2：stdio server 启动 + 首次查询 + 第二次查询

命令（同一进程内连续三行请求）：
```bash
cat << 'EOF' | time ./target/debug/metadata-checker --serve-stdio --graph-db-path /tmp/m23_runtime_test.graphdb
{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}
{"request_id":"r2","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}
{"request_id":"r3","command":"status"}
EOF
```

实测结果（2026-05-15）：

| 请求 | graph_load_ms | query_compute_ms | serialize_ms | total_ms | output_size_bytes |
|---|---|---|---|---|---|
| r1 (explain_condition) | 0 | 3 | 2 | 5 | 约 76,000 |
| r2 (explain_condition) | 0 | 3 | 2 | 5 | 约 76,000 |
| r3 (status) | 0 | 0 | 0 | 0 | 约 300 |

进程 wall_time_s：7.35s（含 server 启动时的 graph 加载）。

---

## 性能对比总结

| 模式 | graph 加载 | 单次请求耗时 | 输出大小 |
|---|---|---|---|
| CLI 冷查询 | 每次 ~6.6s | ~6.6s | 121KB |
| stdio 启动加载 | 启动时 ~6.6s | — | — |
| stdio 首次查询 | 0 | 5ms | 74KB |
| stdio 第二次查询 | 0 | 5ms | 74KB |

**结论**：
- 同一项目连续 2 次查询：stdio 模式总耗时 ≈ 6.6s（启动加载）+ 5ms + 5ms = **~6.6s**；CLI 模式总耗时 = **~13.2s**（每次冷启动）。
- stdio 请求本身（不含启动加载）比 CLI 冷查询快 **~1300 倍**（5ms vs 6600ms）。
- 单次查询：CLI 更直接，无需管理进程生命周期。

---

## 基线 3：M30 stdio 多命令容量基线

命令（同一进程内连续四行请求）：
```bash
printf '%s\n' \
'{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}' \
'{"request_id":"r2","command":"context","target":"comp:app/销售.app/销售/合同协议.spg|input3","depth":2,"budget":"normal"}' \
'{"request_id":"r3","command":"query_model","target":"model:fact_qwSidebar","budget":"compact"}' \
'{"request_id":"r4","command":"query_page_logic","target":"page:app/销售.app/销售/合同协议.spg","budget":"compact"}' \
| ./target/debug/metadata-checker \
  --serve-stdio \
  --graph-db-path /tmp/m23_runtime_test.graphdb \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi
```

实测结果（2026-05-15）：

| 请求 | kind | graph_load_ms | query_compute_ms | serialize_ms | total_ms | output_size_bytes | 诊断 |
|---|---|---|---|---|---|---|---|
| r1 explain_condition input3 | Explain | 0 | 3 | 16 | 19 | 84,625 | 无 |
| r2 context input3 depth=2 | Context | 0 | 2 | 3 | 5 | 30,083 | `OUTPUT_TRUNCATED` |
| r3 query_model fact_qwSidebar | ModelQuery | 0 | 0 | 0 | 0 | 11,602 | `OUTPUT_TRUNCATED` |
| r4 query_page_logic 合同协议.spg | PageLogic | 0 | 2,998 | 31 | 3,029 | 142,542 | `PRIMARY_PATHS_TRUNCATED`, `EVIDENCE_SAMPLED`, `OUTPUT_TRUNCATED` |

验收含义：
- 第二个及后续查询的 `graph_load_ms == 0`，说明 stdio 热查询没有重复全量加载 GraphDB。
- `output_size_bytes` 是最终 stdout JSON 行字节数，可直接作为 function calling 容量治理指标。
- `query_page_logic` 是当前真实项目的大输出热点：计算耗时约 3 秒，且 compact 预算仍触发多类截断诊断，后续性能优化应优先拆分查询计算与输出采样策略。

---

## 采集方法（正确方法）

### M50 标准真实项目 runner

M50 起，真实项目性能基线优先通过标准 runner 采集：

```bash
make perf-real
make perf-real-mutation
make perf-real-runtime
make perf-real-query
```

该命令会调用 `tools/m50-real-project-perf.mjs`，使用真实项目
`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`，
构建或复用 `/tmp/metadata-checker-m50-real-project.graphdb`，并把 JSONL 与 Markdown
artifact 写入 `/tmp/metadata-checker-m50-perf`。

记录口径：
- `stdio_process_wall_ms` 表示 stdio 进程总耗时，包含 graphdb 加载。
- 每个场景的 `timing.total_ms` / `query_compute_ms` 表示 warm runtime 能力开销。
- P50 / P95 / max 由同一次 runner 的 JSONL 聚合得到。
- JSONL 同时包含 `record_type=lifecycle` 与 `record_type=capability`。前者记录 build binary、build graph、stdio process 的生命周期成本；后者记录单个产品能力的 warm runtime 成本。

标准场景覆盖：

| 类别 | 场景 |
|---|---|
| condition | `input3` writer、`text41` display、`text41` value-source、page-scoped `model11` availability |
| page_logic | `合同协议.spg`、`会员已注册.spg` |
| model / context | `fact_qwSidebar` model query、`input3` depth=2 context |
| lookup | `会员已注册` page lookup、`fact_autoCustomerAutoRel` model lookup、`text41` component lookup |
| lifecycle | warm runtime `status`、JSONL lifecycle records 中的 build binary / build graph / stdio process |

真实项目 CRUD / 增量测试必须使用 sandbox 目录，不能直接修改
`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`。CRUD 场景的目标是单独量化 create / update / delete 后的增量建图成本，以及 mutation 后查询结果是否仍稳定；它不应混入默认只读 runner。

### CLI cold start / trace boundary runner

CLI 冷启动与 trace 边界使用 `hyperfine` JSON，不再依赖手工 `time`：

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

固定场景：

| scenario | 含义 |
|---|---|
| `cli_cold_explain_condition_compact` | 冷进程执行 `--explain-condition`（compact / writer） |
| `cli_cold_query_page_logic_full` | 冷进程执行 `--query-page-logic`（full budget） |
| `cli_trace_json_stdout_clean` | `--trace json` 时 stdout 保持机器协议 JSON，stderr 不混入 payload |

输出：

- 每个 timing 场景一份 `*.hyperfine.json`
- 汇总 `cli-cold-start-*.jsonl` 与 Markdown artifact
- trace 边界记录写入 JSONL `record_type=boundary`

依赖：

- `hyperfine`（例如 `brew install hyperfine`）
- 二进制需带 `telemetry` feature（runner 默认 `cargo build --profile release-fast --features telemetry`）

仅验证 trace 边界、跳过 hyperfine：

```bash
node tools/cli-cold-start-perf.mjs --skip-hyperfine
```

### Fixture Criterion bench

fixture 解析与合成图 query 微基准，不依赖真实项目，可进入 CI：

```bash
make perf-fixture
```

或分别运行：

```bash
CARGO_TARGET_DIR=target/criterion/parse cargo bench --bench parse_bench
CARGO_TARGET_DIR=target/criterion/query-micro cargo bench --bench query_micro_bench
```

Criterion bench 按 target 使用独立 `CARGO_TARGET_DIR=target/criterion/<bench>`，避免 release `panic=abort` 产物、bench unwind 产物以及 `rlib`/`cdylib` 同名输出共享默认 `target/release`。

`parse_bench` 覆盖 SPG/TBL 解析、依赖图构建、优先级分析、表达式引用提取。

`query_micro_bench` 覆盖合成图上的 `query_page_logic`、`query_page`、`explain`，fanout 为 64 / 128 / 256；并补充深层树与高密模型场景：

| 场景 | 记录目标 |
|---|---|
| `query_page_logic_fanout_{64,128,256}` | 宽扇出合成图 page logic |
| `query_page_fanout_{64,128,256}` | 宽扇出合成图 page 关系查询 |
| `explain_component_fanout_256` | 宽扇出合成图 explain |
| `query_page_logic_deep_tree_fanout_1024_depth_8` | 深层树合成图 page logic（1024 leaf / depth 8） |
| `context_dense_model_depth3_fanout_512` | 高密度模型上下文闭包（depth 3 / fanout 512） |
| `query_dataflow_pathology_outputs_64_full` | 多 output fields 的 DataFlow full 输出膨胀 |
| `query_dataflow_pathology_joins_6_filters_8_compact` | 多 Join/Union/Filter 叠加 DataFlow 展开 |

### Stdio 协议边界 Criterion bench

warm runtime 进程内复用 `dispatch_stdio_line`，测量协议错误路径与大输出成本：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/stdio cargo bench --bench stdio_boundary_bench
```

或：

```bash
make perf-stdio
```

| 场景 | 记录目标 |
|---|---|
| `stdio_invalid_json_line` | 非法 JSON 行错误路径 |
| `stdio_unknown_command` | unknown command 错误路径 |
| `stdio_query_page_logic_full_large` | full budget 大页面逻辑输出 |
| `stdio_error_then_status_continue` | 错误后继续处理 status |

### OpenTelemetry overhead Criterion bench

对比 warm stdio 在 `trace off` / `trace json` /（可选）`trace otlp` 下的往返成本。二进制路径优先 `target/release-fast/metadata-checker`（与 `make perf-telemetry` 一致）。

```bash
make perf-telemetry
```

| 场景 | 记录目标 |
|---|---|
| `telemetry_stdio_status_trace_off` | trace off 下 status 往返 |
| `telemetry_stdio_query_trace_off` | trace off 下 explain_condition 往返 |
| `telemetry_stdio_query_trace_json` | trace json 下 explain_condition 往返 |
| `telemetry_stdio_query_trace_otlp` | trace otlp 下 explain_condition 往返（需 `METADATA_CHECKER_OTLP_ENDPOINT`） |

### remote/session sync Criterion bench

使用 `InMemoryRemoteSessionProvider` + recorded fixture，覆盖 `remote fetch -> mirror -> graph -> query`：

```bash
make perf-session
```

| 场景 | 记录目标 |
|---|---|
| `session_remote_sync_build_graph` | remote sync + graph build 端到端 |
| `session_remote_sync_then_query_page` | sync 后 warm query_page |

### Rebuild Criterion bench

真实项目 rebuild / mutation 成本使用 Rust Criterion bench，不使用单元测试统计耗时：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/rebuild cargo bench --bench rebuild_bench
```

也可以使用：

```bash
make perf-real-mutation
```

该 bench 会复制真实项目中的 `.spg` / `.tbl` 到 `/tmp` sandbox，并覆盖这些场景：

| 场景 | 记录目标 |
|---|---|
| `rebuild_cold_build_empty_graphdb` | 空 graphdb 完整构建成本 |
| `rebuild_noop_existing_graphdb` | 无变化重扫成本，包含 discover + read/hash + skip persistence |
| `rebuild_dirty_single_spg` | 单个真实页面元数据 dirty 后的 parse/apply/persist 成本 |
| `rebuild_dirty_single_tbl` | 单个真实表元数据 dirty 后的 parse/apply/persist 成本 |
| `rebuild_deleted_single_file` | 文件删除后的 node/edge 删除与持久化成本 |
| `rebuild_added_single_file` | 文件重新出现后的增量新增成本 |
| `crud_update_component_visibility_then_query` | 修改 `visibleCondition` 后 scan -> check_reload -> explain_condition（断言结果含新表达式 + 图中组件仍在） |
| `crud_add_action_write_then_query_model` | 追加 `updateData` 动作后 scan -> check_reload -> explain 新 action（断言图中 action 节点 + updateData 语义） |
| `crud_delete_page_then_find_page` | 删除页面后 scan -> check_reload -> find_page（断言结果与图中 page 节点均移除） |

### Runtime lifecycle Criterion bench

运行期生命周期成本使用：

```bash
CARGO_TARGET_DIR=target/criterion/runtime cargo bench --bench runtime_bench
```

覆盖：

| 场景 | 记录目标 |
|---|---|
| `runtime_load_graphdb` | graphdb 冷加载为 GraphRuntime |
| `runtime_status_warm` | warm status 成本 |
| `runtime_check_reload_unchanged` | 未变化 check_reload fast path |
| `runtime_reload_graph` | 强制 reload 成本 |
| `runtime_check_reload_changed` | graphdb 改变后的 check_reload slow path |
| `runtime_load_locked_graphdb` | 外部 graphdb.lock 持锁时的加载失败路径 |
| `runtime_load_corrupt_graphdb` | 损坏 graphdb header 的诊断路径 |
| `runtime_query_page_with_check_reload` | 查询前先执行 check_reload 的 page 查询 |
| `runtime_explain_condition_with_check_reload` | 查询前先执行 check_reload 的 explain_condition |

### Query matrix Criterion bench

非 page-logic 的运行期能力使用：

```bash
CARGO_TARGET_DIR=target/criterion/query-matrix cargo bench --bench query_matrix_bench
```

覆盖：

| 场景 | 记录目标 |
|---|---|
| `query_page_contract_compact` | 页面关系查询 |
| `query_page_contract_normal` | 页面关系查询（normal budget） |
| `query_page_member_registered_compact` | 第二个页面关系查询 |
| `query_cross_contract_member_compact` | 跨页面关系查询 |
| `query_dataflow_binding_car_compact` | DataFlow 展开 |
| `query_model_fact_qw_sidebar_compact` | 单模型关系查询 |
| `query_model_fact_qw_sidebar_normal` | 单模型关系查询（normal budget） |
| `explain_text41_compact` | 普通 explain |
| `explain_condition_*_compact` | 多 intent 条件解释 |
| `explain_condition_missing_target_compact` | 缺失目标的条件解释错误路径 |
| `explain_condition_input3_writer_normal` | writer 意图条件解释（normal budget） |
| `advise_query_input3_writer_compact` | 查询建议生成 |
| `context_input3_depth2_normal` | depth=2 上下文闭包 |
| `context_input3_depth3_full` | depth=3 上下文闭包（full budget） |
| `query_page_logic_contract_full` | full budget 页面逻辑成本 |
| `query_page_logic_contract_normal` | normal budget 页面逻辑成本 |
| `query_page_logic_member_registered_compact` | 第二个页面逻辑查询 |
| `explain_condition_text41_full` | full budget 条件解释成本 |
| `find_page_member_registered_compact` | 页面 lookup |
| `find_page_wide_keyword_compact` | 宽关键词页面 lookup（「销售」） |
| `find_model_auto_customer_rel_compact` | 模型 lookup |
| `find_component_text41_compact` | 组件 lookup |
| `query_page_contract_compact_check_reload` | 查询前 check_reload 的 page 关系查询 |
| `explain_condition_input3_writer_compact_check_reload` | 查询前 check_reload 的 writer 条件解释 |

### redb 持久化 Criterion bench

redb 打开、锁竞争、损坏检测与增量持久化成本使用：

```bash
METADATA_CHECKER_REAL_PROJECT_DIR=/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  CARGO_TARGET_DIR=target/criterion/redb cargo bench --bench redb_persistence_bench
```

也可以使用：

```bash
make perf-redb
```

覆盖：

| 场景 | 记录目标 |
|---|---|
| `redb_cold_open_readonly` | 冷打开已有 graphdb 只读 |
| `redb_load_file_states` | 加载 graphdb file_states 元数据 |
| `redb_lock_contention_open` | 外部 graphdb.lock 持锁时的打开失败路径 |
| `redb_corrupt_check_graphdb` | 损坏 graphdb header 的检查/诊断 |
| `redb_scan_discover_diff` | discover + diff（不含 parse/apply/persist） |
| `redb_incremental_parse_apply` | 脏文件 parse + apply_graph_updates |
| `redb_incremental_persist_commit` | persist_index 提交（setup 完成 parse/apply） |
| `redb_incremental_persist_dirty_spg` | 单个真实 `.spg` dirty 后的全量 scan 聚合成本 |

### CLI 冷查询
```bash
time ./target/debug/metadata-checker \
  --project-dir /path/to/project \
  --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' \
  --budget compact \
  --graph-db-path /tmp/project.graphdb \
  > /tmp/cli_output.json
```

### stdio server 连续查询（同一进程内）
```bash
# 方式 1：管道启动（测量整个进程）
cat << 'EOF' | time ./target/debug/metadata-checker --serve-stdio --graph-db-path /tmp/project.graphdb
{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}
{"request_id":"r2","command":"explain_condition","target":"model:model22","budget":"compact"}
EOF

# 方式 2：交互式（真实长驻场景，测量响应中的 timing.total_ms / output_size_bytes）
./target/debug/metadata-checker --serve-stdio --graph-db-path /tmp/project.graphdb
# 然后在另一个终端通过管道发送请求并读取响应 timing
```

**重要**：`echo ... | time ./metadata-checker --serve-stdio` 测量的是整个进程生命周期（含启动加载），不是"已长驻 server 的第二次请求"。真正的"第二次请求耗时"应读取响应中的 `timing.total_ms`，输出容量读取 `timing.output_size_bytes`。

---

## 真实项目验收记录

验收时间：2026-05-15
验收目标：`comp:app/销售.app/销售/合同协议.spg|input3`
验收命令：
```bash
cat << 'EOF' | ./target/debug/metadata-checker --serve-stdio --graph-db-path /tmp/m23_runtime_test.graphdb
{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}
{"request_id":"r2","command":"explain_condition","target":"field:fact_qwSidebar.phoneNumber","budget":"compact"}
EOF
```

验收结果：
- `r1.ok == true`
- `r1.timing.graph_load_ms == 0`
- `r1.result.kind == "Explain"`
- `r1.result.details.primary_path` 非空
- `r2.ok == true`
- `r2.result.details.primary_path` 或 `r2.result.details.related_context` 能看到 `fact_qwSidebar.phoneNumber` 关联链路

**主链路验证**：
```
input3
 -> field:model22.phoneNumber
 -> field:fact_qwSidebar.phoneNumber
 <- action:潜客信息跟进.spg|button1|action1
 <- action:潜客信息跟进.spg|button1|action4
```

**能力边界验证**：
- M27 起 stdio 已支持 `explain_condition` / `explain` / `context` / `query_model` / `query_page_logic` / `status` / `reload`。
- M30 已记录 `explain_condition input3`、`context input3 depth=2`、`query_model fact_qwSidebar`、`query_page_logic 合同协议.spg` 的 `timing.total_ms` 与 `timing.output_size_bytes`，见“基线 3”。

**抗漂移验证**：
- `timing` 字段只出现在根级，不在 `summary` / `details` / `evidence` 中。
- M33 起 AI 读取策略：`summary.primary_reason` > `details.answer_facts`；路径审计再用 `--budget normal` 读取 `details.primary_path` / `details.rejected_paths`。
- `timing` 仅用于性能判断，不作为业务证据。

---

## M33 Compact 输出体积基线

验收时间：2026-05-18

目标：显式 intent 下，compact 输出不再展开整页/全图大数组，优先交付 `summary + answer_facts`。

采集命令：
```bash
./target/debug/metadata-checker \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  --graph-db-path /tmp/m20_test_xiaoshouyi.graphdb \
  --explain-condition 'comp:app/售后.app/绑定车辆/会员已注册.spg|text41' \
  --intent display \
  --budget compact | wc -c

./target/debug/metadata-checker \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  --graph-db-path /tmp/m20_test_xiaoshouyi.graphdb \
  --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' \
  --intent writer \
  --budget compact | wc -c
```

实测值：

| 场景 | 输出字节数 | 验收口径 |
|---|---:|---|
| `text41 --intent display --budget compact` | 9852 | 只展开 `display_facts`，隐藏 `primary_path` / `value_source_context` / rejected 明细 |
| `input3 --intent writer --budget compact` | 15492 | 通过 `writer_facts.paths` 保留 `model22.phoneNumber -> fact_qwSidebar.phoneNumber -> action1/action4` |

后续若 `text41 display compact` 超过 12KB，或重新出现 active `value_source_facts` / 展开的 `primary_path`，应视为 M33 注意力漂移回退。

---

## 维护

每次重大版本更新后重新采集基线，更新本文档中的"实测值"。

---

## CNB CI 性能分层（M50+）

远程 CI 按分支与事件拆分，避免所有 push 都跑重 benchmark：

| 触发 | 内容 | 角色 |
|---|---|---|
| `main` push | rust-only + rust coverage + wasm probe + browser offscreen smoke + fixture criterion + full criterion | merge 后权威记录 |
| 非 `main` push | rust-only | 分支轻量验证 |
| PR | rust-only + rust coverage + wasm probe + browser offscreen smoke | 快速反馈 |

说明：

- 真实项目完整 Criterion bench 在 `main.push` 运行，用于 merge 后权威趋势记录；CI 会复用 `METADATA_CHECKER_REAL_PROJECT_DIR`，或用只读部署令牌 clone `REAL_PROJECT_FIXTURE_REPO_*` 指定的 fixture 仓库后导出该目录。
- Rust 覆盖率使用 `cargo-llvm-cov` 生成 `lcov.info`，再交给 CNB `testing:coverage` 上传；当前只要求 coverage 文件存在，不设置全量或增量覆盖率阈值。
- `make perf-real` 这类 Node runner 仍默认手动或后续定时/web_trigger 触发，不进普通 CI。
- 当前 CI 不设性能阈值；变慢不 fail，仅报告 JSONL / summary / `browser-offscreen-ci-perf-index.json`。
- Bencher.dev 为可选上报层：`browser-offscreen-bench-ci`、`fixture-bench-ci` 与 `full-criterion-bench-ci` 通过 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/bencher.yml` 注入 `BENCHER_API_KEY`、`BENCHER_PROJECT`、`BENCHER_TESTBED` 与 `BENCHER_UPLOAD_REQUIRED` 后，CI 会上传 browser offscreen BMF、fixture Criterion 与 full Criterion 结果；未配置时跳过。
- PR 事件下的 browser offscreen pipeline 不导入 `bencher.yml`，只生成本地报告；`main.push` 使用带 Bencher import 的同构 pipeline 上报趋势。
- 当前 Bencher 仅用于趋势留存，不启用 threshold / alert fail。
- 详细 browser offscreen 契约见 `docs/m50-browser-offscreen-bench-plan.md`。
- browser WASM 相关 CI 使用 `.cnb/images/browser-wasm-ci.Dockerfile` 作为 CNB `docker.build` 缓存镜像，避免每次构建重复 `apt-get`、`rustup` 和 `cargo install wasm-bindgen-cli`。
- 同一缓存镜像预装 `bencher` CLI，避免每次流水线临时下载上报工具。
- browser WASM pipeline 使用独立 `CARGO_TARGET_DIR=target/cnb/...`，避免不同 target 与 bench 产物共享默认 `target`；rust-only CI 保留默认 `target/debug` 以兼容直接执行 CLI 二进制的集成测试。
- 真实项目 fixture 已上传到 CNB 私有仓库 `wu2305/metadata-checker-real-fixtures`，项目路径为 `xiaoshouyi`，当前固定 `REAL_PROJECT_FIXTURE_REF=c3c0528fdd28e2600e0b0235040fb349b3c2d446`。后续刷新 fixture 时必须更新该 SHA，避免同一主分支提交因为外部 fixture 漂移产生不可复现的性能样本。
- `full-criterion-bench-ci` 通过 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/real-fixture.yml` 注入 `REAL_PROJECT_FIXTURE_DEPLOY_TOKEN` 等真实项目 fixture 变量。

---

## M34 Compact 输出体积基线

验收时间：2026-05-18

目标：DataFlow projection 接入 compact 输出，不输出完整内部拓扑，只交付 answer_facts 中的短事实。

采集命令：

- `--explain-condition 'comp:app/售后.app/绑定车辆/会员已注册.spg|text41' --intent value-source --budget compact`
- `--explain-condition 'comp:app/售后.app/绑定车辆/会员已注册.spg|text41' --intent display --budget compact`
- `--explain-condition 'model:app/售后.app/绑定车辆/会员已注册.spg|model11' --intent availability --budget compact`

实测值：

| 场景 | 输出字节数 | 验收口径 |
|---|---:|---|
| text41 --intent value-source --budget compact | 10647 | 包含 dataflow_table、physical_source_fields、via、original_node/original_field，不展开完整 DataFlow 拓扑 |
| text41 --intent display --budget compact | 9852 | 保留 M33 display 结论，compact 不额外展开 DataFlow availability |
| page-scoped model11 --intent availability --budget compact | 30047 | 通过 page-scoped target 展开 `$DATA:/加工表/小程序/绑车.tbl` 的 DataFlow availability 短事实，不混入其他页面同名 `model11` gates |

后续若 text41 value-source compact 超过 15KB，或 compact 中回退到输出完整 value_source_context / DataFlow 节点树，应视为 M34 注意力漂移回退。page-scoped model11 availability compact 若重新混入其他页面同名 `model11` gates，也应视为 M34 page-scoped 回退。
