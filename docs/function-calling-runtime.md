# Function Calling Runtime 接入指南

M26 文档：如何把 metadata-checker 的 stdio server 接入 AI Function Calling / tool use 流程。

## 目标

让 AI 在同一项目内连续执行多个查询时，通过长驻 stdio server 复用已加载的 GraphDB，消除每次 CLI 冷启动的 graph 加载开销。

## 非目标

- 不替代单次 CLI 查询。
- 不替代人工 REPL 交互。
- 不新增查询语义。

## 启动 stdio server

```bash
# 方式 1：指定 graphdb 路径
metadata-checker --serve-stdio --graph-db-path /tmp/project.graphdb

# 方式 2：指定项目目录（自动使用 project-dir/.metadata-checker.graphdb）
metadata-checker --serve-stdio --project-dir /path/to/project
```

stderr 输出启动日志，stdout 保持静默等待 JSONL 请求。

## 发送 JSONL 请求

stdin 每行一个 JSON 对象：

```json
{"request_id": "r1", "command": "explain_condition", "target": "comp:app/销售.app/销售/合同协议.spg|input3", "budget": "compact", "human": false}
```

字段说明：

| 字段 | 类型 | 必需 | 说明 |
|---|---|---|---|
| `request_id` | string | 是 | 请求标识，响应原样返回 |
| `command` | string | 是 | `explain_condition` / `status` / `reload` |
| `target` | string | explain_condition 必需 | 查询目标 |
| `budget` | string | 否 | `compact` / `normal` / `full`，默认 `normal` |
| `human` | bool | 否 | 是否生成 human_summary，默认 `false` |
| `check_reload` | bool | 否 | 查询前检测 graphdb 变更，默认 `false` |

## 读取 JSONL 响应

stdout 每行一个 JSON 对象：

成功响应：
```json
{"request_id": "r1", "ok": true, "result": {...}, "diagnostics": [], "timing": {"graph_load_ms": 0, "query_compute_ms": 5, "serialize_ms": 0, "total_ms": 5}}
```

错误响应：
```json
{"request_id": "r1", "ok": false, "error": "Unknown command: magic", "diagnostics": [], "timing": null}
```

## Function Calling 包装示例（伪代码）

```python
import subprocess
import json

class MetadataCheckerRuntime:
    def __init__(self, graph_db_path: str):
        self.proc = subprocess.Popen(
            ["metadata-checker", "--serve-stdio", "--graph-db-path", graph_db_path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self._counter = 0

    def _next_id(self) -> str:
        self._counter += 1
        return f"req-{self._counter}"

    def explain_condition(self, target: str, budget: str = "compact", check_reload: bool = False) -> dict:
        req = {
            "request_id": self._next_id(),
            "command": "explain_condition",
            "target": target,
            "budget": budget,
            "human": False,
            "check_reload": check_reload,
        }
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        return json.loads(line)

    def status(self) -> dict:
        req = {"request_id": self._next_id(), "command": "status"}
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        return json.loads(line)

    def reload(self) -> dict:
        req = {"request_id": self._next_id(), "command": "reload"}
        self.proc.stdin.write(json.dumps(req) + "\n")
        self.proc.stdin.flush()
        line = self.proc.stdout.readline()
        return json.loads(line)

    def close(self):
        self.proc.stdin.close()
        self.proc.wait()
```

## 使用模式

### 模式 1：单次查询（CLI）

```bash
metadata-checker --project-dir /path/to/project --explain-condition 'comp:app/销售.app/销售/合同协议.spg|input3' --budget compact
```

适用：临时问题、一次性分析、不需要连续追问。

### 模式 2：连续追问（stdio server）

```python
runtime = MetadataCheckerRuntime("/tmp/project.graphdb")

# 第一次：解释 input3
r1 = runtime.explain_condition("comp:app/销售.app/销售/合同协议.spg|input3")
# 第二次：解释关联模型
r2 = runtime.explain_condition("model:model22")
# 第三次：上下文查询（若 stdio 已支持）
# r3 = runtime.context("comp:app/销售.app/销售/合同协议.spg|input3", depth=2)

runtime.close()
```

适用：同一项目内多个相关问题、AI 连续追问场景。

### 模式 3：graphdb 变更后刷新

```python
# 项目文件可能已更新
status = runtime.status()
if status["result"]["reload_count"] == 0:
    # 从未 reload 过，检查是否需要
    pass

# 手动 reload
reload_result = runtime.reload()
if reload_result["ok"]:
    print("Graph reloaded")
else:
    print(f"Reload failed: {reload_result['error']}")
```

## 错误处理

| 场景 | 响应 | 处理建议 |
|---|---|---|
| 未知 command | `ok=false`, `error="Unknown command: ..."` | 检查 command 字段 |
| 缺少 target | `ok=false`, `error="Missing target for explain_condition"` | 补充 target 字段 |
| JSON 解析失败 | `ok=false`, `error="JSON parse error: ..."` | 检查请求格式 |
| reload 失败 | `ok=false`, `diagnostics` 含 `GRAPH_RELOAD_FAILED` | 保留旧 graph，检查 graphdb 文件完整性 |
| 查询异常 | `ok=false`, `error="Query failed: ..."` | 检查 target 是否存在 |

## stdout / stderr 边界

- **stdout**：只包含 JSONL 响应行，每行一个完整 JSON。
- **stderr**：启动日志、运行日志、graphdb 扫描日志、错误日志。
- AI/脚本消费方：**只读取 stdout**，忽略 stderr。

## 性能特征

- 启动时加载 graphdb（一次）。
- 首次查询 `timing.graph_load_ms == 0`（graph 已在内存中）。
- 连续查询无 graph 加载开销。
- 详细性能基线见 `docs/performance-baseline.md`。
