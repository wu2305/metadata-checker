# 性能基线

M26 文档：记录 CLI 冷查询 vs stdio server 热查询的性能对比。

## 测试环境

- 项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- GraphDB 路径：`/tmp/m26_baseline.graphdb`
- 硬件：Apple Silicon (arm64)
- 版本：见对应 git commit

## 性能指标定义

| 指标 | 说明 |
|---|---|
| `graph_load_ms` | GraphDB 全量加载耗时（redb 读取 + 内存图构建） |
| `query_compute_ms` | 查询计算耗时（不含 graph 加载和序列化） |
| `serialize_ms` | JSON 序列化耗时 |
| `total_ms` | 整个请求总耗时 |
| `output_size_kb` | 响应 JSON 大小 |

## 基线 1：CLI 冷查询

命令：
```bash
time metadata-checker --project-dir /path/to/project \
  --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' \
  --budget compact --graph-db-path /tmp/m26_baseline.graphdb
```

典型结果（参考值，实际因硬件/数据量而异）：

| 指标 | 典型值 |
|---|---|
| real（总耗时） | ~2-5s |
| graph_load_ms | ~1500-4000ms |
| query_compute_ms | ~5-50ms |
| serialize_ms | ~0-5ms |
| output_size_kb | ~2-10KB |

**瓶颈**：冷查询 90%+ 时间花在 graphdb 全量加载（redb 反序列化 + petgraph 构建）。

## 基线 2：stdio server 首次查询

启动 server：
```bash
metadata-checker --serve-stdio --graph-db-path /tmp/m26_baseline.graphdb
```

stdin 请求：
```json
{"request_id": "r1", "command": "explain_condition", "target": "comp:app/销售.app/销售/合同协议.spg|input3", "budget": "compact", "human": false}
```

典型结果：

| 指标 | 典型值 |
|---|---|
| server 启动 real | ~2-5s（与 CLI 冷查询相当） |
| timing.graph_load_ms | 0（启动时已加载） |
| timing.query_compute_ms | ~5-50ms |
| timing.serialize_ms | ~0-5ms |
| timing.total_ms | ~5-55ms |

**特征**：server 启动时承担 graph 加载成本，首次查询只付查询计算成本。

## 基线 3：stdio server 第二次查询

stdin 请求：
```json
{"request_id": "r2", "command": "explain_condition", "target": "model:fact_qwSidebar", "budget": "compact", "human": false}
```

典型结果：

| 指标 | 典型值 |
|---|---|
| timing.graph_load_ms | 0 |
| timing.query_compute_ms | ~5-50ms |
| timing.serialize_ms | ~0-5ms |
| timing.total_ms | ~5-55ms |

**特征**：与首次查询性能一致，无额外 graph 加载开销。

## 性能对比总结

| 模式 | graph 加载 | 查询计算 | 总耗时 |
|---|---|---|---|
| CLI 冷查询 | 每次 ~2-5s | ~5-50ms | ~2-5s |
| stdio 首次查询 | 启动时 ~2-5s | ~5-50ms | ~5-55ms |
| stdio 第二次查询 | 0 | ~5-50ms | ~5-55ms |

**结论**：
- 同一项目连续 2 次查询：stdio 模式总耗时 ~2-5s + ~5-55ms = **~2-5s**；CLI 模式总耗时 **~4-10s**（每次冷启动）。
- 同一项目连续 N 次查询：stdio 模式优势随 N 增加而放大。
- 单次查询：CLI 更直接，无需管理进程生命周期。

## 采集方法

### CLI 冷查询
```bash
time ./target/release/metadata-checker \
  --project-dir /path/to/project \
  --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' \
  --budget compact --graph-db-path /tmp/m26_baseline.graphdb \
  > /dev/null
```

### stdio server 查询
```bash
# 终端 1：启动 server
./target/release/metadata-checker --serve-stdio --graph-db-path /tmp/m26_baseline.graphdb

# 终端 2：发送请求并计时
echo '{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact","human":false}' | \
  time ./target/release/metadata-checker --serve-stdio --graph-db-path /tmp/m26_baseline.graphdb
```

**注意**：stdio server 的 `time` 命令测量的是整个进程生命周期（含启动），不是单次请求。单次请求耗时应看响应中的 `timing.total_ms`。

## 真实项目验收命令

```bash
# 1. 构建 graphdb
./target/release/metadata-checker \
  --project-dir /Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi \
  --build-graph --graph-db-path /tmp/m26_baseline.graphdb

# 2. 启动 stdio server
./target/release/metadata-checker --serve-stdio --graph-db-path /tmp/m26_baseline.graphdb

# 3. stdin 输入以下三行：
{"request_id":"r1","command":"explain_condition","target":"comp:app/销售.app/销售/合同协议.spg|input3","budget":"compact"}
{"request_id":"r2","command":"explain_condition","target":"model:model22","budget":"compact"}
{"request_id":"r3","command":"status"}

# 4. 验证 r1.result.details.primary_path 包含 model22.phoneNumber / fact_qwSidebar.phoneNumber / action1 或 action4
# 5. 验证 r2.result.kind == "Explain"
# 6. 验证 r3.result.reload_count == 0, node_count > 0
# 7. 验证所有响应 timing.graph_load_ms == 0
```

## 维护

每次重大版本更新后重新采集基线，更新本文档中的"典型值"。
