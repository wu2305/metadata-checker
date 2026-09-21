# 性能基线

M26-M30 文档：记录 CLI 冷查询、stdio server 热查询、统一 timing 与容量治理基线。

## 2026-09-21 语义修复的基线影响

Baseline impact: yes。`codex/m59-semantic-integrity` 保留全部引用出现与不同 metadata 的边事实，
影响表达式解析、追溯、建图、持久化以及图查询 bench 的输入规模和内存成本。
CNB 工作区 `cnb-c9g-1k30leog8` 执行编译与正确性验证，不是 full-criterion CI 测量。
本批没有真实语料性能重测，不能沿用下方历史耗时宣称性能不退化；M59 迁移真实语料
验收必须从源重建图并重新统计边数、内存及耗时。`cargo check --benches` 仅证明可编译。

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
| `runtime_load_graphdb` | graphdb 冷加载为 GraphRuntime（OneShot） |
| `runtime_long_lived_startup` | LongLived 端到端启动（含读模型） |
| `runtime_load_dense_snapshot_build` | LongLived load 阶段 dense snapshot 单步构建 |
| `runtime_load_availability_facts_build` | LongLived load 阶段 availability facts 单步构建 |
| `runtime_load_page_dependency_index_build` | LongLived load 阶段 page dependency index 单步构建 |
| `runtime_long_lived_warm_pages_1` | LongLived 按需 warm 1 页 |
| `runtime_long_lived_warm_pages_10` | LongLived 按需 warm 10 页 |
| `runtime_long_lived_warm_pages_50` | LongLived 按需 warm 50 页 |
| `runtime_status_warm` | warm status 成本 |
| `runtime_check_reload_unchanged` | 未变化 check_reload fast path |
| `runtime_reload_graph` | 强制 reload 成本 |
| `runtime_check_reload_changed` | graphdb 改变后的 check_reload slow path |
| `runtime_load_locked_graphdb` | 外部 graphdb.lock 持锁时的加载失败路径 |
| `runtime_load_corrupt_graphdb` | 损坏 graphdb header 的诊断路径 |
| `runtime_query_page_with_check_reload` | 查询前先执行 check_reload 的 page 查询 |
| `runtime_explain_condition_with_check_reload` | 查询前先执行 check_reload 的 explain_condition |

> `runtime_selective_rewarm_pages_*` 与 dirty-file 规模曲线归属 **M54–M56** 差量刷新线，不在本 patch 登记。  
> LongLived startup / warm N 场景前后会经 `benches/common/rss.rs` 打印 `[rss] … KB`（人工观测；不设 CI fail）。

### Query matrix Criterion bench

2026-09-08 契约修复影响说明：page-logic 恢复跨阶段邻接复用，profile counter 移除
1毫秒下限，core profile runner 的 unchanged 检查移到再次打开 graphdb 之前。
旧 counter 与新样本不直接横比；本轮没有重测真实项目 Criterion/Bencher 数值，不重置趋势。
PR39 后续补充：compact warm cache 的三组 prerequisites 各保留最多5条 JSON，另存完整
计数和模型引用；normal/full 保留全量。构建仍先完整收集与排序，不宣称峰值内存有界。
源码、回归与历史失败见 [集中修复记录](../../governance/performance-contract-fixes-2026-09-08.md)。

#### 毫秒计时的零值与兼容别名

- cold 的 `availability_index_build_ms`、投影耗时及其他真实计时使用毫秒整数，低于1毫秒
  可记录为0；0不代表没有执行，也不应重新抬高到1。
- warm 命中时 index build 记录为0，须结合 `availability_read_model_used=1` 判断路径。
  是否消除构建工作由实际模型读取测试验证，不能单凭计时为0断言。
- `availability_context_build_ms` 是 `availability_index_build_ms` 的兼容别名；两者
  相等，不能相加，也不能当作两个独立采样点。
- 字段缺失表示未记录，不能由消费方补成0参与统计；报告汇总只使用实际存在的样本。
  需要验证明确零值时使用 counter map 的 `get`，避开缺键也返回0的便捷读取函数。

#### 采集新口径报告

在 CNB workspace 的 `/workspace` 选定实际存在且已获授权的语料目录与页面，设置
`METADATA_CHECKER_REAL_PROJECT_DIR` 和 `METADATA_CHECKER_PROFILE_PAGE_ID`（含 `page:` 前缀）。
以下命令为新口径采样入口，未在本 PR 中作为真实语料基线验收执行：

```sh
(
  set -eu
  : "${METADATA_CHECKER_REAL_PROJECT_DIR:?必须指定已授权语料目录}"
  : "${METADATA_CHECKER_PROFILE_PAGE_ID:?必须指定语料中实际存在的页面}"
  mkdir -p target/profile-review
  METADATA_CHECKER_PROFILE_RUN_DIR="$(mktemp -d target/profile-review/run.XXXXXX)"
  git rev-parse HEAD > "$METADATA_CHECKER_PROFILE_RUN_DIR/code-commit.txt"
  rustc -Vv > "$METADATA_CHECKER_PROFILE_RUN_DIR/toolchain.txt"
  cargo run --profile release-fast --features cli-local --bin m51_profile_report -- \
    --project-dir "$METADATA_CHECKER_REAL_PROJECT_DIR" --sample-count 5 \
    --scenario "selected_compact|$METADATA_CHECKER_PROFILE_PAGE_ID|compact" \
    --scenario "selected_normal|$METADATA_CHECKER_PROFILE_PAGE_ID|normal" \
    --scenario "selected_full|$METADATA_CHECKER_PROFILE_PAGE_ID|full" \
    --output "$METADATA_CHECKER_PROFILE_RUN_DIR/page-logic-profile.json"
  cargo run --profile release-fast --features cli-local --bin m51_core_profile_report -- \
    --project-dir "$METADATA_CHECKER_REAL_PROJECT_DIR" --sample-count 5 \
    --output "$METADATA_CHECKER_PROFILE_RUN_DIR/core-profile.json"
)
```

每次使用新目录，失败即停止，不能让上一轮报告冒充新结果。另记录语料固定提交、页面、
CPU/内存配额与运行环境；只有同口径、同语料、同环境样本才可横比。上述分阶段报告不替代
真实项目 Criterion 分布或 Bencher 趋势重测，旧的1毫秒下限样本保留原标识。

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
| `main` push | `rust-ci` + browser offscreen smoke + fixture criterion + 5 路并行 full criterion（query / rebuild / rebuild-crud / redb / boundary） | merge 后权威记录 |
| 非 `main` push | `rust-ci`（stage `if` 跳过 `main`） | 分支轻量验证 |
| PR | `rust-ci` + browser offscreen smoke | 快速反馈 |

说明：

- 真实项目完整 Criterion bench 仍在 `main.push` 运行，拆为 `full-criterion-query-ci`、`full-criterion-rebuild-ci`、`full-criterion-rebuild-crud-ci`、`full-criterion-redb-ci`、`full-criterion-boundary-ci` 五路并行；每路使用独立 `warmup-*` 目录并在 stage 前清理对应 `target/criterion-ci/*` 子目录，避免并行 pipeline 共享增量产物。
- `tools/run-bencher-criterion-ci.mjs` 统一 `--profile release-fast` 跑 Criterion；`2026-07-02` 起 Bencher 趋势与此前 `opt-level=z` 的 bench 样本不可直接横比。
- 快 gate（`rust-ci` 4 核、`browser-offscreen-bench-ci` / `rust-ci-branch-push` 2 核、`fixture-bench-ci` 4 核）降配 CPU；full criterion 保持 8 核（`redb` / `boundary` 为 6 核）。
- `rust-ci` 合并原 `rust-only-ci` 与 `rust-coverage-ci`：单次 `cargo llvm-cov test` 兼做测试与覆盖率；`target/debug` symlink 兼容集成测试硬编码路径。
- `browser-wasm-env-probe` 已移除；WASM release 构建由 `browser-offscreen-bench-ci` 覆盖。
- `fixture-bench-ci` 与 `browser-offscreen-bench-ci` 配置 `ifModify`，仅源码/fixture 变更时编译；full criterion 与 `rust-ci` 始终运行。
- Rust 覆盖率使用 `cargo-llvm-cov` 生成 `lcov.info`，再交给 CNB `testing:coverage` 上传；当前只要求 coverage 文件存在，不设置全量或增量覆盖率阈值。
- `make perf-real` 这类 Node runner 仍默认手动或后续定时/web_trigger 触发，不进普通 CI。
- 当前 CI 不设性能阈值；变慢不 fail，仅报告 JSONL / summary / `browser-offscreen-ci-perf-index.json`。
- Bencher.dev 为可选上报层：`browser-offscreen-bench-ci`、`fixture-bench-ci` 与五路 full criterion pipeline 通过 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/bencher.yml` 注入 `BENCHER_API_KEY`、`BENCHER_PROJECT`、`BENCHER_TESTBED` 与 `BENCHER_UPLOAD_REQUIRED` 后，CI 会上传 browser offscreen BMF、fixture Criterion 与 full Criterion 结果；未配置时跳过。
- PR 事件下的 browser offscreen pipeline 不导入 `bencher.yml`，只生成本地报告；`main.push` 使用带 Bencher import 的同构 pipeline 上报趋势。
- 当前 Bencher 仅用于趋势留存，不启用 threshold / alert fail。
- 详细 browser offscreen 契约见 [m50-browser-offscreen-bench-plan.md](m50-browser-offscreen-bench-plan.md)。
- browser WASM 相关 CI 使用 `.cnb/images/browser-wasm-ci.Dockerfile` 作为 CNB `docker.build` 缓存镜像，避免每次构建重复 `apt-get`、`rustup` 和 `cargo install wasm-bindgen-cli`；镜像层含 `cargo fetch` 预热 registry（`versionBy` 绑定 `Cargo.lock`）。
- 同一缓存镜像预装 `bencher` CLI，避免每次流水线临时下载上报工具。
- browser WASM pipeline 使用独立 `CARGO_TARGET_DIR=target/cnb/...`，避免不同 target 与 bench 产物共享默认 `target`；`rust-ci` 通过 `target/debug` symlink 兼容直接执行 CLI 二进制的集成测试。
- `rust-ci` 与 bench pipeline 共用 `browser-wasm-ci` 缓存镜像（registry volume 预热依赖）。
- 真实项目 fixture 已上传到 CNB 私有仓库 `wu2305/metadata-checker-real-fixtures`，项目路径为 `xiaoshouyi`，当前固定 `REAL_PROJECT_FIXTURE_REF=c3c0528fdd28e2600e0b0235040fb349b3c2d446`。后续刷新 fixture 时必须更新该 SHA，避免同一主分支提交因为外部 fixture 漂移产生不可复现的性能样本。
- 五路 full criterion pipeline 通过 CNB 密钥仓库文件 `wu2305/metadata-checker-keys/real-fixture.yml` 注入 `REAL_PROJECT_FIXTURE_DEPLOY_TOKEN` 等真实项目 fixture 变量。

## M50 验收收口记录

验收时间：2026-06-20

验收结论：M50 phase-1 可验收。当前验收范围是“性能测量基础设施、真实项目权威 CI 样本、Bencher 趋势上报、覆盖率检测”完成；不把性能阈值 fail、CNB 报告留存预览、浏览器真实环境 perf 或具体查询算法优化纳入本阶段验收。

权威 CI 记录：

| 项目 | 值 |
|---|---|
| CNB build SN | `cnb-5r3-1jrhpbnkg` |
| CNB build URL | `https://cnb.cool/wu2305/metadata-checker/-/build/logs/cnb-5r3-1jrhpbnkg` |
| event / branch | `push` / `main` |
| commit | `99632ccd2b5eed4cd4f8d2fb86943e3c17572324` |
| commit title | `fix: clean up benchmark graph lock` |
| pipeline result | 7 / 7 success |
| total duration | 3,961,443 ms |

流水线结果：

| pipeline | 结果 | 耗时 | 验收含义 |
|---|---|---:|---|
| `rust-only-ci` | success | 53,459 ms | fmt、bench 编译、单元测试、browser-wasm check 通过 |
| `rust-coverage-ci` | success | 72,542 ms | `cargo llvm-cov` 生成并上传 `lcov.info` |
| `browser-wasm-env-probe` | success | 4,341 ms | 真实 `browser-wasm` 目标与 Node wasm-bindgen glue 可构建 |
| `browser-offscreen-bench-ci` | success | 39,510 ms | 不拉浏览器的真实 WASM offscreen replay bench 通过并上报 Bencher |
| `fixture-bench-ci` | success | 324,008 ms | fixture `parse_bench` / `query_micro_bench` 通过并上报 Bencher |
| `full-criterion-bench-ci` | success | 3,960,432 ms | 真实项目完整 Criterion bench 通过并上报 Bencher |
| `rust-only-ci-branch-push` | success | 7,115 ms | main 下按 stage `if` 跳过，确认没有重复跑分支轻量检查 |

`full-criterion-bench-ci` 分项：

| stage | 结果 | 耗时 |
|---|---|---:|
| `checkout real project fixture` | success | 1,748 ms |
| `real project preflight` | success | 185 ms |
| `query matrix bench` | success | 759,208 ms |
| `runtime lifecycle bench` | success | 445,076 ms |
| `rebuild mutation bench` | success | 1,437,203 ms |
| `redb persistence bench` | success | 512,401 ms |
| `stdio boundary bench` | success | 297,121 ms |
| `telemetry overhead bench` | success | 368,276 ms |
| `session sync bench` | success | 135,131 ms |

Bencher 上报证据：

| 类别 | report |
|---|---|
| browser offscreen p50 | `https://bencher.dev/perf/haocheng-wu-s-project/reports/82cff7ae-9c41-4136-b80c-d2e89e9c9338` |
| fixture Criterion query micro | `https://bencher.dev/perf/haocheng-wu-s-project/reports/15bb5953-a8d3-4a5c-836c-8f0bc68a98b8` |
| full Criterion query matrix | `https://bencher.dev/perf/haocheng-wu-s-project/reports/21f850eb-c99f-444a-991d-c50b2146311a` |

验收边界：

- 当前 `main.push` 是 M50 权威性能样本来源；PR 只保留快速验证，不跑真实项目完整 Criterion。
- 当前不设置 Bencher threshold，不使用 `--error-on-alert`。性能变慢先进入趋势观察，不阻塞 CI。
- Coverage 当前只要求可生成、可上传、可读；不设置最低覆盖率门禁。
- `main.push` 仍是 M50 权威性能样本来源；full criterion 四路并行后预期 wall time 约 24–35 分钟（`cnb-p28` 实测 48 min，mutation 串行瓶颈；拆分 rebuild ‖ redb 后待下一次 main push 验证）。
- Browser real perf 因浏览器环境干扰因素多，本阶段继续推后；M50 只验收不拉浏览器的真实 WASM offscreen 链路。
- 后续打开性能阈值前，至少需要积累多次 `main` 样本，并按核心指标分组设置阈值；不从单次样本直接拍阈值。

---

## M50+ CI 基线更新：cnb-078（`release-fast`）

验收时间：2026-07-05

背景：`f1e7285` 修复 Criterion 并行 cache 污染、rebuild bench 顺序与 `serialize_response_with_timing` 收敛后，九路 CI 首次全绿；Criterion 统一 `--profile release-fast`（见 `tools/run-bencher-criterion-ci.mjs`）。**此后 Bencher 趋势与 `opt-level=z` 时代样本不可横比。**

权威 CI 记录：

| 项目 | 值 |
|---|---|
| CNB build SN | `cnb-078-1jsmeegk1` |
| event / branch | `push` / `main` |
| commit | `f1e7285` |
| commit title | `fix: stdio serialize timing convergence and rebuild bench symmetry` |
| pipeline result | 9 / 9 success |
| 并行 wall time（约） | ~24 min |

流水线结果：

| pipeline | 结果 | 耗时 (ms) | 验收含义 |
|---|---|---:|---|
| `rust-ci` | success | 84,000 | fmt、llvm-cov 测试、browser-wasm check |
| `browser-offscreen-bench-ci` | success | （见 CNB 日志） | WASM offscreen replay + Bencher |
| `fixture-bench-ci` | success | 595,000 | fixture Criterion + Bencher |
| `full-criterion-query-ci` | success | 1,423,000 | query matrix Criterion（`release-fast`） |
| `full-criterion-rebuild-ci` | success | 808,000 | rebuild mutation Criterion |
| `full-criterion-rebuild-crud-ci` | success | 975,000 | rebuild CRUD Criterion |
| `full-criterion-redb-ci` | success | 685,000 | redb persistence Criterion |
| `full-criterion-boundary-ci` | success | 1,087,000 | stdio boundary + telemetry 等 |
| `rust-ci-branch-push` | success | （见 CNB 日志） | main 下轻量分支检查跳过确认 |

五路 full criterion 说明：

- 每路独立 `warmup-*` 与 `target/criterion-ci/<lane>/`，stage 前 `rm -rf` 对应子目录，避免并行污染。
- 编译 profile：`release-fast`（`[profile.bench] inherits = "release-fast"`，与 CI `--profile release-fast` 一致，避免 Criterion 默认走 `opt-level=z`）。
- 与 `cnb-5r3` 记录相比：pipeline 数 7→9（rust 合并 + criterion 五路拆分）；总 wall 由 ~66 min 降至 ~24 min（并行 + cache 隔离）。

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

---

## M57 Phase 2：真实项目 read-model dirty curve

验收时间：2026-07-26

采集命令：

```bash
cargo build --release --features cli-local
cargo test --release --features cli-local \
  --test m57_real_project_benchmark_tests -- --ignored --nocapture
```

固定输入：

- 项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- GraphDB：`/private/tmp/m57-xiaoshouyi.graphdb`
- 模式：release；LongLived runtime
- mutation：候选图对前 1/10/100 个节点追加 name suffix；node set 与拓扑保持不变，不写回 graphdb

图规模与冷 full 基线：

| 指标 | 实测值 |
|---|---:|
| node_count | 78,127 |
| edge_count | 150,164 |
| graph_load_ms | 1,688 |
| full_read_model_ms | 249,906 |
| release binary | 3,781,936 bytes（约 3.6M） |

增量曲线：

| dirty_count | dense_ms | facts_ms | page_dep_ms | read_model_ms | wall_ms | 相对 full |
|---:|---:|---:|---:|---:|---:|---:|
| 1 | 729 | 15 | 909 | 1,653 | 1,655 | 151.2x |
| 10 | 704 | 15 | 1,628 | 2,347 | 2,348 | 106.5x |
| 100 | 674 | 16 | 2,362 | 3,052 | 3,053 | 81.9x |

结论：稳定 node set 的真实 dirty replacement 没有回退 full；最大 dirty=100 仍比 cold full read-model 快约 81.9x，满足 M57 Phase 2 性能门。该曲线是 read-model replacement 成本，不包含下一阶段 LongLived 批量 persist；Phase 3 验收记录见下节。

## M57 Phase 3：LongLived 内存优先持久化验收（非性能）

验收时间：2026-07-26（复核）

验收目标：

- `persisted` 与 `pending_dirty_total` 是否与策略一致；
- one-shot、stdio、tick 的 `DiffRefreshReport` 字段口径统一；
- 环境变量策略、重试/回退与崩溃恢复语义可复现；
- 本阶段不新增可比“性能加速”样本，避免与 Phase 2 读模型替换曲线混用。

验收命令与结果（已执行）：

```bash
cargo test --features cli-local --test m54_diff_refresh_orchestrator_tests
cargo test --features cli-local --test m57_ai_contract_tests
cargo test --features cli-local --test m57_refresh_report_scope_tests
cargo test --features cli-local --test m57_tick_loop_tests
cargo test --features cli-local --test stdio_server_tests -- --exact test_stdio_diff_refresh_with_bound_context --test-threads=1
```

测试结果：

- `m54_diff_refresh_orchestrator_tests`：12 passed；
- `m57_ai_contract_tests`：1 passed；
- `m57_refresh_report_scope_tests`：2 passed；
- `m57_tick_loop_tests`：4 passed；
- `test_stdio_diff_refresh_with_bound_context`：1 passed（`--exact`）；

策略与边界：

- 默认 `LongLivedPersistPolicy`：
  - `METADATA_CHECKER_PERSIST_DIRTY_THRESHOLD=100`；
  - `METADATA_CHECKER_PERSIST_MAX_ROUNDS=10`；
- 一般策略：`dirty ∪ deleted > threshold` 或 `pending_rounds >= max_rounds` 时 durable persist；
- one-shot 默认 `set_one_shot_mode(true)`，每轮同步 persist；
- LongLived 默认 deferred；`install` 后立即可读，`persist` 可能滞后；
- 空事件轮次只推进 `pending_rounds`；`persist_report` 可能为 null；
- 持久化失败不回滚已安装 runtime；下一轮可重试 pending commit；
- 崩溃恢复仅靠 durable checkpoint，未写盘的 pending 轮次可能丢失，需由下一次 `poll` 重补齐。

---

## M58.3 PR2 真实项目性能基线（形态感知递归后，M50 runner 实测）

采集时间：2026-08-25

背景：PR2（F1 形态感知递归 + F2 祖先链 + ComponentProperty 图契约）的性能基线义务
（plan `2026-08-24-m58-3-command-surface-gap-fixes-plan.md` PR2 验收项；附录 A 推论组件
候选数 26,007 → 34,317，+32%）。按 M50 标准 runner 实测采集，不用推论反推预期值。

采集环境：

| 项目 | 值 |
|---|---|
| commit | `e27266b`（`codex/m58-slm-eval-foundation`） |
| 环境 | CNB workspace（`cnb-l6o-1k0s03fst`，Linux 容器），`release-fast` profile |
| 语料 | xiaoshouyi pin 基准 `6920ac51`（501 个 `.spg` / 828 个 `.tbl`，158MB；与 cnb `wu2305/succbi_project_container` 的 `xiaoshouyi-corpus` 分支 HEAD 一致，由本地 tar 推送进 workspace） |
| runner | `node tools/m50-real-project-perf.mjs --project-dir <语料> --bin <release-fast> --skip-build-bin --rebuild-graph`，每场景 5 轮 |

图规模与生命周期：

| 指标 | PR2 后实测 | M26 旧基线（仅数量级参考） |
|---|---:|---:|
| 节点数 | 89,094 | 78,114 |
| 边数 | 199,575 | 150,029 |
| graphdb 体积 | 538,972,160 B（约 514 MiB） | 约 257 MiB |
| 全量建图 wall | 45.9 s | — |
| stdio 启动加载（spawn→`Graph loaded`） | 1,008.1 s / 1,011.8 s（2 轮实测） | — |
| stdio 进程总 wall（图加载 + 60 请求） | 1,151.9 s | — |

口径注意：

- 节点数是全部图节点（组件/字段/模型/页面/action 等）；组件候选口径（34,317）见 spec
  附录 A「重跑确认」，两者不可直接相减比较。
- M26 旧基线为 macOS M3 debug 构建且语料非同一 pin，只作数量级参考，不作回归判据。
- **2026-09-06 复测（M59，workspace `cnb-j1o-1k1qvdkp3`，`release` profile，语料 pin
  `c3c0528f`）**：节点 89,178（+84）、边 200,028（+453）、graphdb
  538,972,160 B（与本表**逐字节相同**）、全量建图 wall 64.67 s、峰值 RSS
  1,693.0 MiB。字节数相同是 redb 页分配粒度的产物，**不构成「图未变」的证据**，
  不要当回归判据用。建图 wall 的 +41% 未归因（redb 写入路径将随 M59 退役）。
  完整读数与方法见
  [M59 真实语料实测](../../ai-eval-runs/2026-09-06-m59-real-corpus-measurements.md)。
- stdio 总 wall 中查询合计约 97 s（各场景 P95 × 5 轮上界），其余约 1,050 s 主要为
  515M graphdb 的加载与启动。加载耗时已独立实测闭环（2026-08-25，CNB workspace
  `cnb-7jg-1k0sh4l6r` 同口径重建 graphdb 后）：从进程 spawn 到 stderr
  `[stdio-server] Graph loaded` 标记的 wall time，2 轮分别 1,008.1 s / 1,011.8 s
  （page-cache 暖度差异 < 1%），与推算值吻合；两轮后 `status` 均正常返回、节点/边数
  与建图口径一致。stdio 模式下 60 条响应的 `graph_load_ms` 全为 0 是口径所致——
  启动期加载不归入任何请求，故加载耗时只能由 spawn→标记的 wall time 独立测量
  （测量脚本 `tools/graph-load-measure.py`，M50 runner 本身不拆加载耗时）。

查询 P50/P95（warm runtime，5 轮，全部 5/5 ok）：

| 类别 | 场景 | p50_ms | p95_ms | output_size_max_bytes |
|---|---|---:|---:|---:|
| condition | `explain_condition_input3_writer` | 198 | 242 | 15,919 |
| condition | `explain_condition_text41_display` | 248 | 270 | 9,700 |
| condition | `explain_condition_text41_value_source` | 248 | 251 | 12,357 |
| condition | `explain_condition_model11_availability` | 183 | 248 | 23,707 |
| page_logic | `query_page_logic_contract` | 9,077 | 9,826 | 156,464 |
| page_logic | `query_page_logic_member_registered` | 7,749 | 7,965 | 114,929 |
| context | `context_input3_depth2` | 10 | 11 | 31,765 |
| model | `query_model_fact_qwSidebar` | 10 | 10 | 12,461 |
| lookup | `find_page_member_registered` | 185 | 209 | 4,797 |
| lookup | `find_model_auto_customer_rel` | 185 | 194 | 3,125 |
| lookup | `find_component_text41` | 205 | 209 | 14,618 |
| lifecycle | `status_warm_runtime` | 0 | 0 | 1,273 |

加载期诊断（`status.load_diagnostics`）：

| code | count | 说明 |
|---|---:|---|
| `SCANNER_UNRECOGNIZED_CONTAINER_KEY` | 312 | 与附录 A 重跑脚本的 scanner 口径预测（`moreFields` 279 + `params` 33 = 312）精确吻合，互为交叉验证 |
| `SCANNER_DUPLICATE_COMPONENT_ID` | 1,105 | 同页重复组件 id，PR1 起持久化透出 |

两条计数曾被标记为「可能已陈旧」。2026-09-06 在 pin `c3c0528f` 上复测，
**312 / 1,105 一字未变**，该待办关闭。

Baseline impact 判读：本次是 PR2 后首个 runner 真实项目样本，无前序同口径样本可横比；
page_logic 两场景 P95 约 8–10 s 为当前最重能力，后续 PR（PR4b 页面局部子图迁移）应以本表
为基线观察漂移。artifact 在 CNB workspace（临时环境），数字已转写本节。
