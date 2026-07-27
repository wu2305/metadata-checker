# 廉价模型理解力评测设计（M58）

> 状态：approved（2026-07-27；采用 CNB AI Chat API，runner 实现在 CI tester 内）
> 范围：把「空上下文廉价模型只凭 SKILL.md + CLI 输出能否正确回答真实业务问题」从一次性人工验收，变成可持续量化的自动化评测闭环。
> 编号：ai-eval 线 **M58**；depends M22（条件类评测与协议收敛已 done）。

本次修订将 LLM runner 的首选后端从通用 OpenAI-compatible endpoint 调整为 CNB AI Chat API。依据 [CNB OpenAPI 文档](https://docs.cnb.cool/zh/develops/openapi.html) 与当前 [Swagger 契约](https://api.cnb.cool/swagger.json)：接口为 `POST /{repo}/-/ai/chat/completions`，请求包含 `messages`、`model`、`stream`，认证使用仅允许在 CNB 流水线内调用的 `CNB_TOKEN`。当前契约未公开 `tools` / `tool_calls` 字段，因此 M58 使用可验证的 JSON 命令协议，不假设未声明的原生 tool calling 能力。

## 2026-07-28 执行修订

首次真实 CNB 探测确认当前服务不接受 `stream:false`，返回 `400 Non-stream chat request is currently not supported`；`stream:true` 返回 SSE `data:` 分片并以 `data: [DONE]` 结束。因此 `CnbChatAdapter` 固定发送 `stream:true`，只拼接 `choices[].delta.content`，不把 provider-specific SSE 逻辑带入 `metadata-checker` 主工具。

本阶段增加小模型稳健性目标：bootstrap 必须先给出最小决策流程、严格 JSON 形态、查询预算和证据边界；模型输出的每一轮仍由 `CommandPolicy` 精确校验，提示词优化不得放宽命令白名单或答案判分。每次提示词/runner 逻辑调整都通过同一组 fake fixture、真实 CNB smoke 和 `RunReport` 指标比较，不能只凭单次主观回答判断改进。

## 背景

M9-D/E/F 与 M15/M16/M22 已交付：

- 结构化评测集 `tests/fixtures/corpus/ai_eval/ai_eval_cases.json`：`minimal_command_plan`（结构化命令计划）、`answer_assertions`（must_include / must_not_include / diagnostic_disclaimer_required / evidence_reference_required）、`expected_output_assertions`（机器判分 CLI 输出）、`risk_tags` / `difficulty` / `case_status`
- `tests/ai_eval_tests.rs`：对 CLI 输出的**确定性**机器判分（无 LLM 参与），已在 CI
- 判分规则与失败分类：`docs/reference/ai-eval.md`（missed_fact / hallucination / wrong_command / ignored_diagnostic / over_read_details）
- 大型真实项目探究集 `xiaoshouyi_large_real_cases.json`：人工/Agent 评测，不进 CI
- 三次人工运行记录：`docs/ai-eval-runs/`（gpt-5.4-mini，最新 2026-05-08）

尚未闭环：

- **没有端到端自动化 runner**：让真实廉价模型在空上下文下走完整「读 SKILL.md → 选命令 → 读输出 → 回答」回路，并对其**回答**自动判分。现有机器判分只校验 CLI 输出本身，不校验模型的理解与表达。
- **没有趋势追踪**：三次人工记录是孤立的快照，无法回答「某次改动后理解力变好了还是变坏了」。
- **没有分层执行契约**：哪些 case 进 CI、哪些定期观测、哪些纯人工，目前靠口头约定。

这意味着「支撑小端侧 LLM 理解元数据逻辑」这一目标当前**不可测**：任何优化都无法量化验证。M58 是该目标方向的验收门，与 M54–M56（正确性基座）并行启动。

## 目标

1. **回答判分器**：给定模型回答文本 + case 的 `answer_assertions`，确定性输出 pass/fail 与失败分类候选；无需 LLM 参与，可单测。
2. **端到端 runner**：每个 case 在 CNB 流水线内拉起**全新空上下文**廉价模型会话，使用 CNB AI Chat API 发送仅包含评测协议、SKILL.md 和 question 的 bootstrap 消息；模型以 JSON 内容请求命令，runner 执行真实二进制并把 stdout 作为下一条 user 消息回传，直至模型给出最终回答；记录完整命令轨迹与回答。
3. **量化报告**：输出结构化 JSON 报告 + 人类可读摘要（pass 率、失败分类直方图、命令数/预算遵守率），按模型 ID + 日期归档到 `docs/ai-eval-runs/`，支持跨次对比。
4. **分层执行契约**：固定 tier 划分（见下），CI 只跑确定性层，LLM 层默认定期/手动观测，初期**不设 CI fail 阈值**。

## 非目标

- 修改 SKILL.md、输出 schema、answer_contract 等被测对象（runner 发现的问题只记录进报告，修复走各自里程碑）
- 大模型对比矩阵、多模型排行榜
- 把 LLM 层设为 CI 阻塞门槛（初期只观测趋势）
- 真实项目大 case（`xiaoshouyi_large_real_cases.json`）进任何自动层
- Dashboard / Report 格式（M59）
- 评测 runner 进入 release 二进制或 stdio/MCP 命令面
- 把 CNB Knowledge Base、NPC 对话或仓库上下文作为隐式评测输入

## 架构

```text
ai_eval_cases.json (tier=fixture_llm)
  → CaseLoader（按 tier / case_status / difficulty 过滤）
  → 每 case：
      fresh model session（CNB AI Chat API，CNB_TOKEN 仅在流水线内可见）
      → bootstrap user message（评测协议 + SKILL.md + question）
      ⇄ JSON command loop：assistant JSON → 白名单校验 → 执行真实 CLI 二进制 → user 回传 stdout
      → 最终回答文本
  → AnswerJudge（确定性判分：answer_assertions + 诊断降级检查）
  → RunReport { provider, model_id, started_at, cases[], pass_rate, failure_classes, command_trace_stats }
  → docs/ai-eval-runs/YYYY-MM-DD-<model>-run.md（摘要）+ JSON 报告
```

### 组件契约

- **CaseLoader**：复用现有 `ai_eval_cases.json` schema，新增可选字段 `tier`（`fixture_structural` / `fixture_llm` / `real_manual`）；缺省按现有规则归类（`requires_real_project: true` → `real_manual`，其余 → 两层皆可）。新增字段必须向后兼容，`tests/ai_eval_tests.rs` 现有断言不得因加字段而失败。
- **ModelAdapter**：首个真实后端为 `CnbChatAdapter`，在 CNB 流水线内向 `POST /{repo}/-/ai/chat/completions` 发送 `{ messages, model, stream: true }`，解析 SSE `data:` 分片并拼接 assistant delta content。`CNB_TOKEN`、仓库 slug、model id 从流水线环境变量读取；token 只放在 Authorization header，不进入消息、请求日志、响应日志或报告。`FakeModelAdapter` 用于本地单测和无凭证回归。
- **消息协议**：遵循当前 CNB Swagger 暴露的 `user` / `assistant` 消息与字符串 `content`。模型命令必须返回单个 JSON 对象，例如 `{"kind":"command","command_kind":"--query-page-logic","target":"page:app/actions_test.spg","args":[],"budget":"compact"}`；最终回答返回 `{"kind":"final","answer":"..."}`。解析失败、字段未知或违反命令预算时记录 runner protocol diagnostic，不把未执行的命令当作真实证据。
- **命令执行**：只允许 `minimal_command_plan` 派生的命令 kind、target 形态、参数和 budget；project-dir、graphdb 路径和二进制路径由 runner 固定，模型不能提供或覆盖这些路径，也不能注入 shell。直接调用 `target/release/metadata-checker`（或 cargo 构建产物），复用 M9 系列的临时目录 + graphdb 隔离与串行锁约定（见 `docs/reference/ai-eval.md` 串行执行/GraphDB 隔离两节）。
- **AnswerJudge**：输入回答文本 + `answer_assertions` + 该 case 收集到的 diagnostics；输出 `passed`、`failure_classes[]`（五类，可多个）、`judge_notes`。`evidence_reference_required` 与 `diagnostic_disclaimer_required` 的判定规则必须写成可单测的确定性函数；无法确定性判定的子项标 `needs_human_review`，**不得**为追求自动化率而放宽判分。
- **RunReport**：固定 schema（`schema_version` 开头），包含 `provider`、`model_id`、CNB build 标识（不含 token）、per-case 结果、聚合指标、命令轨迹统计（平均命令数、超 `max_command_count` 的 case 数、budget 升级次数）。JSON 报告是跨次对比的唯一数据源，人类可读 md 由它生成。

### CNB 流水线边界

- 真实 LLM tier 只能通过 CNB 手动或定时流水线执行；本地测试不依赖 CNB 网络、账号或 token。
- 流水线镜像需包含 release `metadata-checker`、fixture 项目、runner 和评测 Skill；每个 case 复制到独立临时目录并使用独立 graphdb，case 间串行执行。
- `CNB_TOKEN` 只由 runner 进程读取并用于 CNB AI Chat API；不得传给模型、命令白名单执行器或报告生成器。
- CNB AI Chat API 未公开 token usage 字段时，RunReport 不自行推断 token 数；成本/调用量如需记录，使用 CNB 的聚合审计数据，不把请求详情写入仓库报告。
- M58 不使用 CNB Knowledge Base 或 NPC 隐式上下文；否则无法证明模型只消费固定的 `SKILL.md + CLI 输出 + question`。

### 分层执行

| tier | 内容 | 是否含 LLM | 执行时机 | CI |
|------|------|-----------|----------|-----|
| `fixture_structural` | 现有 `tests/ai_eval_tests.rs`（CLI 输出机器判分） | 否 | 每次 CI | 阻塞（现状不变） |
| `fixture_llm` | fixture case 端到端模型回答判分 | 是 | 定期 / 手动 / 重大输出契约变更前 | 不阻塞，只归档报告 |
| `real_manual` | `xiaoshouyi_large_real_cases.json` 探究集 | 是 | 人工/Agent | 不进 CI（现状不变） |

## 指标

- per-case pass/fail 与失败分类直方图（五类 + needs_human_review）
- 命令纪律：命令数 ≤ `max_command_count` 的比例；`over_read_details` 发生率
- 预算纪律：compact 截断后是否正确升级 budget 的 case 占比
- 趋势：同一 model_id 相邻两次 run 的 pass 率差与新增/修复失败分类

## 验收

1. `CnbChatAdapter` 在 CNB 流水线内对 ≥3 个 `fixture_llm` case 端到端跑通（真实模型、空上下文、JSON 命令协议、命令白名单）。
2. AnswerJudge 对人工已判分的 ≥3 个历史回答（取自 `docs/ai-eval-runs/` 结论）判定一致；不一致处写明原因并标 `needs_human_review` 规则缺口。
3. RunReport JSON schema 固定并有 snapshot 测试；md 摘要可由 JSON 重新生成。
4. 密钥脱敏测试：构造含假 CNB_TOKEN 的请求、响应和 runner error，断言日志与报告无泄漏。
5. 命令协议安全测试：拒绝未知命令 kind、越权 target、shell 元字符、覆盖 project-dir/graphdb 的字段和超出 `max_command_count` 的请求。
6. journal 记录首次 CNB 基线 run（provider、model_id、CNB build 标识、pass 率、失败分类），作为后续趋势对比原点。

## 与其它里程碑

- **M54–M56**：并行线。diff refresh 不改 LLM 可见输出契约；M58 runner 可作为其回归保障（证明性能改动未破坏输出）。
- **M28–M30**：M58 报告中的 wrong_command / over_read_details 分布是契约收敛的输入数据。
- **M22**：评测集与判分规则的直接前代，本 spec 复用其 schema，不另起格式。
- **MCP**：runner 只打 CLI 二进制，不依赖未来 MCP 层。
