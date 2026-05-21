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

### explain_condition

```json
{
  "request_id": "req-1",
  "command": "explain_condition",
  "target": "comp:app/销售.app/销售/合同协议.spg|input3",
  "budget": "compact",
  "human": false,
  "check_reload": false
}
```

### status

```json
{
  "request_id": "req-status",
  "command": "status"
}
```

### reload_graph

```json
{
  "request_id": "req-reload",
  "command": "reload_graph"
}
```

字段说明：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `request_id` | string | 是 | 请求标识，响应原样返回 |
| `command` | string | 是 | 命令名：`explain_condition` / `explain` / `query_model` / `query_page` / `query_cross` / `query_dataflow` / `query_page_logic` / `context` / `find_page` / `find_model` / `find_component` / `advise_query` / `status` / `reload_graph` / `check_reload` |
| `target` | string | explain_condition 必需 | 查询目标 |
| `budget` | string | 否 | `compact` / `normal` / `full`，默认 `normal` |
| `human` | bool | 否 | 是否生成 human_summary，默认 `false` |
| `check_reload` | bool | 否 | 查询前检查 graphdb 是否变更并自动 reload，默认 `false` |

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
    "serialize_ms": 1,
    "total_ms": 6,
    "output_size_bytes": 76000
  }
}
```

错误响应：

```json
{
  "request_id": "",
  "ok": false,
  "error": {"code": "INVALID_JSON", "message": "JSON parse error: ..."},
  "diagnostics": [],
  "timing": {
    "graph_load_ms": 0,
    "query_compute_ms": 0,
    "serialize_ms": 0,
    "total_ms": 0,
    "output_size_bytes": 180
  }
}
```

字段说明：

| 字段 | 类型 | 说明 |
|---|---|---|
| `request_id` | string | 原始请求标识 |
| `ok` | bool | 是否成功 |
| `result` | object / null | 查询结果 |
| `error` | object / null | 结构化错误，包含 `code` 和 `message` |
| `diagnostics` | string[] | 诊断信息 |
| `timing` | object | 阶段耗时与 `output_size_bytes`，其中 `output_size_bytes` 是最终 stdout JSON 行字节数 |

## stdout / stderr 边界

- **stdout**：只包含 JSONL 响应行，每行一个完整 JSON。
- **stderr**：启动日志、运行日志、graphdb 扫描日志。
- 消费方读取 stdout 的 JSONL，忽略 stderr。

## 错误处理

- 空行：跳过，不产生响应。
- 非法 JSON：返回 `ok=false`，`error` 包含解析错误。
- 未知 command：返回 `ok=false`，`error` 包含 "Unknown command"。
- 查询异常：返回 `ok=false`，`error` 包含异常信息。
- 缺少 target：对 `explain_condition` 返回 `ok=false`，`error` 包含 "Missing target"。

## 热查询特性

同一个 stdio server 进程内连续执行多个查询时：

- 第一次查询包含 graph 加载。
- 第二次及以后查询 `timing.graph_load_ms == 0`。
- 通过 `request_id` 关联请求和响应。
