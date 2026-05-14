# Stdio Server 协议

M24 引入的 JSONL stdin/stdout 长驻查询服务。

## 启动

```bash
metadata-checker --serve-stdio --graph-db-path /path/to/graph.db
# 或
metadata-checker --serve-stdio --project-dir /path/to/project
```

启动时 stderr 输出日志，stdout 保持静默（只输出 JSONL 响应）。

## 请求格式

每行一个 JSON 对象，stdin 逐行读取：

```json
{
  "request_id": "req-1",
  "command": "explain_condition",
  "target": "comp:app/销售.app/销售/合同协议.spg|input3",
  "budget": "compact",
  "human": false
}
```

字段说明：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `request_id` | string | 是 | 请求标识，响应原样返回 |
| `command` | string | 是 | 命令名，当前只支持 `explain_condition` |
| `target` | string | 是 | 查询目标 |
| `budget` | string | 否 | `compact` / `normal` / `full`，默认 `normal` |
| `human` | bool | 否 | 是否生成 human_summary，默认 `false` |

## 响应格式

每行一个 JSON 对象，stdout 逐行输出：

成功响应：

```json
{
  "request_id": "req-1",
  "ok": true,
  "result": { ... },
  "diagnostics": [],
  "timing": {
    "graph_load_ms": 0,
    "query_compute_ms": 5,
    "serialize_ms": 0,
    "total_ms": 5
  }
}
```

错误响应：

```json
{
  "request_id": "",
  "ok": false,
  "error": "JSON parse error: ...",
  "diagnostics": [],
  "timing": null
}
```

字段说明：

| 字段 | 类型 | 说明 |
|---|---|---|
| `request_id` | string | 原始请求标识 |
| `ok` | bool | 是否成功 |
| `result` | object / null | 查询结果 |
| `error` | string / null | 错误描述 |
| `diagnostics` | string[] | 诊断信息 |
| `timing` | object / null | 阶段耗时 |

## stdout / stderr 边界

- **stdout**：只包含 JSONL 响应行，每行一个完整 JSON。
- **stderr**：启动日志、运行日志、graphdb 扫描日志。
- 消费方读取 stdout 的 JSONL，忽略 stderr。

## 错误处理

- 空行：跳过，不产生响应。
- 非法 JSON：返回 `ok=false`，`error` 包含解析错误。
- 未知 command：返回 `ok=false`，`error` 包含 "Unknown command"。
- 查询异常：返回 `ok=false`，`error` 包含异常信息。

## 热查询特性

同一个 stdio server 进程内连续执行多个查询时：

- 第一次查询包含 graph 加载。
- 第二次及以后查询 `timing.graph_load_ms == 0`。
- 通过 `request_id` 关联请求和响应。
