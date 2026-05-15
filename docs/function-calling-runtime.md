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
# 方式 1：外置 graphdb 路径 + 项目目录
metadata-checker --serve-stdio --graph-db-path /tmp/project.graphdb --project-dir /path/to/project

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
| `command` | string | 是 | `explain_condition` / `explain` / `query_model` / `query_page_logic` / `context` / `status` / `reload` |
| `target` | string | 查询类命令必需 | 查询目标 |
| `budget` | string | 否 | `compact` / `normal` / `full`，默认 `normal`；非法值返回 `INVALID_BUDGET` |
| `human` | bool | 否 | 是否生成 human_summary，默认 `false`（目前仅 `explain_condition` 支持） |
| `depth` | usize | 否 | `context` 命令专用，默认 `1`；非法值返回 `INVALID_DEPTH` |
| `check_reload` | bool | 否 | 查询前检测 graphdb 变更，默认 `false` |

## 读取 JSONL 响应

stdout 每行一个 JSON 对象：

成功响应：
```json
{"request_id": "r1", "ok": true, "result": {...}, "diagnostics": [], "timing": {"graph_load_ms": 0, "query_compute_ms": 5, "serialize_ms": 0, "total_ms": 5}}
```

错误响应：
```json
{"request_id": "r1", "ok": false, "error": {"code": "UNKNOWN_COMMAND", "message": "Unknown command: magic"}, "diagnostics": [], "timing": {"graph_load_ms": 0, "query_compute_ms": 0, "serialize_ms": 0, "total_ms": 0}}
```

错误响应固定使用 `error.code` / `error.message`，function calling wrapper 不应解析错误字符串。

## Function Calling 包装示例（伪代码）

### 工具拆分契约（M29）

Function calling 层应暴露少量任务型工具，而不是让 AI 直接拼任意 stdio request。工具名、底层 command 和使用边界如下：

| Tool | stdio command | 必需参数 | 默认 budget | 适用问题 | 主要读取位置 |
|---|---|---|---|---|---|
| `metadata_explain_condition` | `explain_condition` | `target` | `compact` | 为什么不显示、为什么没数据、值从哪来、组件/页面可用性 | `result.summary`、`result.summary.key_primary_paths`、`result.details.primary_path`、`result.evidence` |
| `metadata_explain` | `explain` | `target` | `compact` | 这个对象是什么、它读写/跳转/影响什么 | `result.summary`、`result.details`、`result.evidence` |
| `metadata_context` | `context` | `target`, `depth` | `normal` | 周围关系是什么、需要扩展核查上下游 | `result.summary`、`result.details.upstream`、`result.details.downstream` |
| `metadata_query_model` | `query_model` | `target` | `compact` | 模型读写全貌、谁读/谁写、跨页 writer | `result.summary`、`result.details.readers`、`result.details.writers`、`result.evidence` |
| `metadata_query_page_logic` | `query_page_logic` | `target` | `compact` | 页面整体逻辑、入口、写入、跳转、可见性规则 | `result.summary`、`result.details.page_inputs`、`result.details.write_targets`、`result.details.action_flows` |
| `metadata_runtime_status` | `status` | 无 | 不适用 | 只检查 runtime/graph 状态 | `result`；不得作为业务证据 |
| `metadata_runtime_reload` | `reload` | 无 | 不适用 | graphdb 更新后手动刷新 | `ok`、`error.code`；不得作为业务证据 |

工具入参边界：

| Tool | 参数 |
|---|---|
| `metadata_explain_condition` | `target: string`, `budget?: compact|normal|full`, `human?: false`, `check_reload?: boolean` |
| `metadata_explain` | `target: string`, `budget?: compact|normal|full` |
| `metadata_context` | `target: string`, `depth?: number`, `budget?: compact|normal|full` |
| `metadata_query_model` | `target: string`, `budget?: compact|normal|full` |
| `metadata_query_page_logic` | `target: string`, `budget?: compact|normal|full`, `check_reload?: boolean` |
| `metadata_runtime_status` | 无业务参数 |
| `metadata_runtime_reload` | 无业务参数 |

选择规则：

- “为什么不显示 / 为什么没数据 / 值从哪来” → `metadata_explain_condition`。
- “这个对象是什么” → `metadata_explain`。
- “周围关系是什么 / 需要补看上下游” → `metadata_context`。
- “模型读写全貌 / 谁写了这个表” → `metadata_query_model`。
- “页面整体逻辑 / 入口 / 写入 / 跳转 / 可见性” → `metadata_query_page_logic`。
- `metadata_runtime_status` 和 `metadata_runtime_reload` 只能用于运行时健康与刷新判断，不能作为业务结论证据。

Anti-drift 约束：

- `timing` 只能用于性能判断，不能解释业务数据来源、显示条件或写入链路。
- 业务回答优先读取 `result.summary`。
- 证据核查读取 `result.details.primary_path` / `result.summary.key_primary_paths` / `result.evidence`。
- `related_context` 默认只是相关上下文，不是必要条件；只有输出明确标为阻塞条件或主路径证据时才可作为必要条件表述。
- 不默认使用 `budget=full`；先 `compact`，证据不足再升到 `normal`，只有审计长数组时才用 `full`。

```python
import subprocess
import json

class MetadataCheckerRuntime:
    def __init__(self, graph_db_path: str, project_dir: str | None = None):
        args = ["metadata-checker", "--serve-stdio", "--graph-db-path", graph_db_path]
        if project_dir is not None:
            args.extend(["--project-dir", project_dir])
        self.proc = subprocess.Popen(
            args,
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
        return self._request({
            "request_id": self._next_id(),
            "command": "explain_condition",
            "target": target,
            "budget": budget,
            "human": False,
            "check_reload": check_reload,
        })

    def explain(self, target: str, budget: str = "compact") -> dict:
        return self._request({
            "request_id": self._next_id(),
            "command": "explain",
            "target": target,
            "budget": budget,
            "human": False,
        })

    def query_model(self, target: str, budget: str = "compact") -> dict:
        return self._request({
            "request_id": self._next_id(),
            "command": "query_model",
            "target": target,
            "budget": budget,
            "human": False,
        })

    def query_page_logic(self, target: str, budget: str = "compact", check_reload: bool = False) -> dict:
        return self._request({
            "request_id": self._next_id(),
            "command": "query_page_logic",
            "target": target,
            "budget": budget,
            "human": False,
            "check_reload": check_reload,
        })

    def context(self, target: str, depth: int = 1, budget: str = "normal") -> dict:
        return self._request({
            "request_id": self._next_id(),
            "command": "context",
            "target": target,
            "depth": depth,
            "budget": budget,
            "human": False,
        })

    def status(self) -> dict:
        return self._request({"request_id": self._next_id(), "command": "status"})

    def reload(self) -> dict:
        return self._request({"request_id": self._next_id(), "command": "reload"})

    def _request(self, req: dict) -> dict:
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
runtime = MetadataCheckerRuntime("/tmp/project.graphdb", "/path/to/project")

# 第一次：解释 input3 主链路，对应 metadata_explain_condition
r1 = runtime.explain_condition("comp:app/销售.app/销售/合同协议.spg|input3")
# 第二次：解释局部模型，对应 metadata_explain_condition
r2 = runtime.explain_condition("model:model22")
# 第三次：核查物理表字段，对应 metadata_explain_condition
r3 = runtime.explain_condition("field:fact_qwSidebar.phoneNumber")
# 第四次：查询物理表读写，对应 metadata_query_model
r4 = runtime.query_model("model:fact_qwSidebar")
# 第五次：查询页面逻辑，对应 metadata_query_page_logic
r5 = runtime.query_page_logic("page:app/销售.app/销售/合同协议.spg")
# 第六次：按需看上下文，对应 metadata_context
r6 = runtime.context("comp:app/销售.app/销售/合同协议.spg|input3", depth=2)

runtime.close()
```

适用：同一项目内多个相关条件解释问题、AI 连续追问字段来源/显示条件/数据为空原因的场景。

M27 起 stdio server 支持全部核心查询命令：
`explain_condition`、`explain`、`query_model`、`query_page_logic`、`context`、`status`、`reload`。
所有项目级查询均可通过 stdio 执行，享受热 graph 复用。

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
    print(f"Reload failed: {reload_result['error']['code']} {reload_result['error']['message']}")
```

## 错误处理

| 场景 | `error.code` | 处理建议 |
|---|---|---|
| JSON 解析失败 | `INVALID_JSON` | 检查 JSONL 请求格式 |
| 未知 command | `UNKNOWN_COMMAND` | 检查 command 字段 |
| 缺少 target | `MISSING_TARGET` | 补充 target 字段 |
| 非法 target | `INVALID_TARGET` | 修正 target ID 格式或改用 `--context` 核查候选 ID |
| 非法 budget | `INVALID_BUDGET` | 只使用 `compact` / `normal` / `full` |
| 非法 depth | `INVALID_DEPTH` | `context.depth` 必须是非负整数 |
| reload 失败 | `GRAPH_RELOAD_FAILED` | 旧 graph 继续保留，检查 graphdb 文件完整性 |
| 查询异常 | `QUERY_FAILED` | 检查 target 是否存在，必要时重建 graphdb |

## stdout / stderr 边界

- **stdout**：只包含 JSONL 响应行，每行一个完整 JSON。
- **stderr**：启动日志、运行日志、graphdb 扫描日志、错误日志。
- AI/脚本消费方：**只读取 stdout**，忽略 stderr。

## 性能特征

- 启动时加载 graphdb（一次）。
- 首次查询 `timing.graph_load_ms == 0`（graph 已在内存中）。
- 连续查询无 graph 加载开销。
- 详细性能基线见 `docs/performance-baseline.md`。
