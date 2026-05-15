# 性能基线

M26 文档：记录 CLI 冷查询 vs stdio server 热查询的性能对比。

## 测试环境

- 项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- GraphDB 路径：`/tmp/m23_runtime_test.graphdb`（历史命名沿用，内容为本轮 M26 验收图）
- GraphDB 大小：269 MB
- 节点数：78,114 | 边数：150,029
- 硬件：Apple Silicon M3 (arm64), macOS
- 采集版本：`master 6fdf681`（包含 M23-M26 runtime/stdio/reload/skill 文档修正）
- 编译：`cargo build`（debug 模式）

## 性能指标定义

| 指标 | 说明 |
|---|---|
| `graph_load_ms` | GraphDB 全量加载耗时（redb 读取 + 内存图构建） |
| `query_compute_ms` | 查询计算耗时（不含 graph 加载和序列化） |
| `serialize_ms` | JSON 序列化耗时 |
| `total_ms` | 整个请求总耗时 |
| `output_size_kb` | 响应 JSON 大小 |
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

| 请求 | graph_load_ms | query_compute_ms | serialize_ms | total_ms | output_size_kb |
|---|---|---|---|---|---|
| r1 (explain_condition) | 0 | 3 | 2 | 5 | 74KB |
| r2 (explain_condition) | 0 | 3 | 2 | 5 | 74KB |
| r3 (status) | — | — | — | — | 0.3KB |

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

# 方式 2：交互式（真实长驻场景，测量响应中的 timing.total_ms）
./target/debug/metadata-checker --serve-stdio --graph-db-path /tmp/project.graphdb
# 然后在另一个终端通过管道发送请求并读取响应 timing
```

**重要**：`echo ... | time ./metadata-checker --serve-stdio` 测量的是整个进程生命周期（含启动加载），不是"已长驻 server 的第二次请求"。真正的"第二次请求耗时"应读取响应中的 `timing.total_ms`。

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
- 本基线只验证当前 stdio 已支持的 `explain_condition` / `status` / `reload`。
- `--context`、`--query-model`、`--query-page-logic` 当前不属于 stdio command，仍通过 CLI 验收。
- 因此 M26 不把 unsupported command 的 function-calling 包装视为已完成能力。

**抗漂移验证**：
- `timing` 字段只出现在根级，不在 `summary` / `details` / `evidence` 中。
- AI 读取策略：`summary.primary_reason` > `details.blocking_conditions` > `details.primary_path` > `details.related_context`。
- `timing` 仅用于性能判断，不作为业务证据。

---

## 维护

每次重大版本更新后重新采集基线，更新本文档中的"实测值"。
