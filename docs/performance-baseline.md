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
- AI 读取策略：`summary.primary_reason` > `details.blocking_conditions` > `details.primary_path` > `details.related_context`。
- `timing` 仅用于性能判断，不作为业务证据。

---

## 维护

每次重大版本更新后重新采集基线，更新本文档中的"实测值"。
