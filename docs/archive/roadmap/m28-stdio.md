# M28：Stdio 请求/响应契约收敛

| milestone | M28 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M28：Stdio 请求/响应契约收敛

### 目标

统一 stdio 的 request / response / error schema，让 function calling wrapper 可以稳定消费，不靠字符串猜测或每个 command 特判。

### 非目标

- 不新增查询命令。
- 不改变 CLI human 输出。
- 不重写已有 JSON 结果中的业务字段。

### 工作清单

- [x] 定义统一 request schema：
  - `request_id`
  - `command`
  - `target`
  - `budget`
  - `depth`
  - `human`
  - `check_reload`
- [x] 定义统一 success response：
  - `request_id`
  - `ok: true`
  - `result`
  - `diagnostics`
  - `timing`
- [x] 定义统一 error response：
  - `request_id`
  - `ok: false`
  - `error.code`
  - `error.message`
  - `diagnostics`
  - `timing`
- [x] 统一错误码：
  - `INVALID_JSON`
  - `UNKNOWN_COMMAND`
  - `MISSING_TARGET`
  - `INVALID_TARGET`
  - `INVALID_BUDGET`
  - `INVALID_DEPTH`
  - `GRAPH_RELOAD_FAILED`
  - `QUERY_FAILED`
- [x] 更新文档：
  - `docs/function-calling-runtime.md`
  - `docs/schema.md`
  - `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`
- [x] 增加负例测试：
  - malformed JSONL
  - unknown command
  - missing target
  - invalid depth
  - invalid budget
  - reload failed 后旧 graph 仍可用

### 验收目标

- 所有 stdio command 使用同一响应 envelope。
- stdout 只输出 JSONL，stderr 只输出日志。
- function calling wrapper 不需要按 command 猜测错误格式。
- schema 文档和测试断言一致。
