# M29：Function Calling 工具层优化

| milestone | M29 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M29：Function Calling 工具层优化

### 目标

基于 M27-M28 的 stdio 能力，为 AI 提供更不易误用的工具描述、调用边界和读取策略。

### 非目标

- 不实现 MCP server。
- 不新增底层图查询能力。
- 不把所有 CLI 参数暴露给 AI。

### 工作清单

- [x] 设计工具拆分：
  - `metadata_explain_condition`
  - `metadata_explain`
  - `metadata_context`
  - `metadata_query_model`
  - `metadata_query_page_logic`
  - `metadata_runtime_status`
  - `metadata_runtime_reload`
- [x] 明确每个 tool 的使用边界：
  - “为什么不显示 / 为什么没数据 / 值从哪来” → `metadata_explain_condition`
  - “这个对象是什么” → `metadata_explain`
  - “周围关系是什么” → `metadata_context`
  - “模型读写全貌” → `metadata_query_model`
  - “页面整体逻辑” → `metadata_query_page_logic`
- [x] 更新 skill 决策树：
  - 单次问题可用 CLI。
  - 同一项目连续追问优先 stdio/function calling。
  - graphdb 变更后先 `status` / `reload`。
  - graphdb 不可用时才单文件 fallback。
- [x] 增加 anti-drift 约束：
  - `timing` 只能用于性能判断。
  - 业务回答优先读 `summary`。
  - 证据核查读 `details.primary_path` / `evidence`。
  - `related_context` 默认不是必要条件。
- [x] 增加 function calling 示例：
  - 连续解释 `input3`、`model:model22`、`field:fact_qwSidebar.phoneNumber`。
  - 查询 `model:fact_qwSidebar`。
  - 查询 `合同协议.spg` 页面逻辑。

### 验收目标

- AI 能根据问题类型选择正确 tool。
- tool 描述不引导 AI 读取 timing/status 作为业务证据。
- skill 文档与 stdio 已支持命令完全一致。
- 真实项目样例能覆盖 input3 主链路、fact_qwSidebar writer、页面逻辑三类问题。
