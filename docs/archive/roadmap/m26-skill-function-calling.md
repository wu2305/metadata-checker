# M26：Skill / Function Calling 接入与性能验收 ✅

| milestone | M26 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M26：Skill / Function Calling 接入与性能验收 ✅

### 目标

把 M23-M25 的长驻查询能力接入 AI 使用路径，并用真实项目性能基线验证“第二次查询不再冷启动”。

M26 只解决 AI 使用协议和性能验收，不再大改 runtime 架构。

### 非目标

- 不新增新的查询语义。
- 不重写 pathfinder。
- 不做 Lazy GraphDB。
- 不做 MCP server，除非 M24 stdio 已稳定且另开后续里程碑。

### 工作清单

- 更新 `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md` 或仓库内同步文档，增加 stdio server 决策树。
- 决策规则：
  - 单次、低成本、临时查询可继续 CLI。
  - 同一项目连续多个 `explain_condition` 查询优先 stdio server。
  - `context`、`query-model`、`query-page-logic` 连续追问当前仍走 CLI；除非后续里程碑显式扩展 stdio command，不能在 skill/function calling 文档中宣称已支持。
  - graphdb 变更后先发 `status` / `reload`。
- 增加 function calling wrapper 示例文档，建议：
  - `docs/function-calling-runtime.md`
- 文档包含：
  - 如何启动 stdio server。
  - 如何发送 JSONL 请求。
  - 如何处理 `request_id`。
  - 如何处理错误响应。
  - 如何关闭进程。
- 建立性能基线文档，建议：
  - `docs/performance-baseline.md`
- 基线至少包含：
  - CLI 冷查询 `input3 --explain-condition` 耗时。
  - stdio server 首次查询耗时。
  - stdio server 第二次查询耗时。
  - graph load 阶段耗时。
  - query compute 阶段耗时。
  - 输出大小。
- 用真实项目 `xiaoshouyi` 验收：
  - `comp:app/销售.app/销售/合同协议.spg|input3`
  - `field:fact_qwSidebar.phoneNumber`（通过当前已接入的 `explain_condition` 验证）
  - `--context` / `--query-model` / `--query-page-logic` 作为 CLI 边界记录，不纳入当前 stdio 验收。
- 更新 ai-eval 或手工验收清单：
  - 记录 stdio 模式下输出仍满足 M22 主链路断言。
  - 记录模型不会因为新增 timing/status 字段发生注意力漂移。
- 可选运行 `samply`：
  - 对 stdio 第二次查询采样。
  - 确认热点不再是 `GraphDB::load_from_db`。

### 落地任务拆解

#### M26.1：Skill 决策树更新

- 更新 skill 文档时保持 summary-first 原则。
- 把 stdio server 放在“同一项目连续多问”路径，不替代所有 CLI。
- 明确 stdout JSONL 不能混入日志。
- 明确单次查询仍可用 CLI，避免模型为了简单问题启动长驻服务。

验收：

- skill 文档能指导 AI 在连续追问时复用服务。

#### M26.2：性能基线采集

- 记录命令：
  - CLI 冷查询。
  - stdio 启动。
  - stdio 第一次请求。
  - stdio 第二次请求。
- 每个命令记录：
  - real/user/sys 或 runtime timing。
  - 输出大小。
  - graphdb 路径。
  - git commit。
- 基线数字不要求绝对固定，但必须能证明第二次 stdio 查询绕过 graph load。

验收：

- `docs/performance-baseline.md` 有真实项目数据。

#### M26.3：AI 输出抗漂移检查

- 用 M22 的 input3 问题重新跑一轮工具输出。
- 确认新增 timing/status 不被模型当作业务证据。
- 如果输出新增字段会干扰 AI，调整 skill 读取策略：
  - 先读 `summary`
  - 再读 `details.primary_path`
  - timing 只用于性能判断

验收：

- 业务回答仍优先引用主链路，不引用 runtime timing 作为业务原因。

### 验收目标

- AI 连续 `explain_condition` 查询不再天然冷启动。
- stdio 第二次查询的 `graph_load_ms == 0`。
- 真实项目 input3 主链路不回退。
- skill 文档明确 CLI 与 stdio server 的选择边界。
- 性能基线文档记录可复现命令和结果。
