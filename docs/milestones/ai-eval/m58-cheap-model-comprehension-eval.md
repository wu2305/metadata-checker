# M58：廉价模型理解力评测

> 状态：**active**（spec/plan approved；真实 LLM baseline 通过 CNB API trigger 执行）
> Spec：[2026-07-17-cheap-model-comprehension-eval-design.md](../../specs/2026-07-17-cheap-model-comprehension-eval-design.md)  
> Plan：[2026-07-27-m58-cnb-ai-chat-runner-plan.md](../../plans/2026-07-27-m58-cnb-ai-chat-runner-plan.md)

## 目标

把「空上下文廉价模型只凭 SKILL.md + CLI 输出正确回答真实业务问题」从一次性人工验收变成可持续量化的自动化闭环：回答判分器 + 端到端 runner + 量化报告 + 分层执行契约。M58 是「支撑小端侧 LLM 理解元数据逻辑」方向的**验收门**，与 M54–M56（正确性基座）并行启动。

## 路线定位（2026-07-17 确认）

```
地基：M54 / M55 / M56 差量刷新（正确性基座）
    ＋ M58 评测闭环（验收门，并行）
收敛：M28 / M29 / M30 stdio/FC 契约收敛
高楼：MCP adapter、token 级输出预算、M59 rpt/dash、语义层体系化
```

## 与既有评测体系的关系

| 已有 | 本里程碑新增 |
|------|-------------|
| `ai_eval_cases.json` 结构化 case（M9-D/E/F、M15、M22） | case `tier` 字段与分层执行契约 |
| `tests/ai_eval_tests.rs` CLI 输出机器判分（CI 阻塞） | 保持现状，为 `fixture_structural` 层 |
| `docs/ai-eval-runs/` 三次人工运行记录 | runner 自动生成 RunReport，基线 run 归档 |
| `docs/reference/ai-eval.md` 判分规则与失败分类 | AnswerJudge 把规则落成确定性可单测函数 |

## 明确不做

- 修改 SKILL.md / 输出 schema / answer_contract（runner 发现的问题只记录）
- LLM 层设 CI fail 阈值（初期只观测趋势）
- 真实项目大 case 进自动层
- 多模型排行榜

## 实现进度

| 项 | 状态 |
|----|------|
| Spec（CNB AI Chat 版本）登记 | done |
| Spec 批准 | done（2026-07-27） |
| Plan | done（2026-07-27，runner 限定 CI tester） |
| tier loader + fixture 分层 | done |
| AnswerJudge + 单测 | done |
| ModelAdapter + 命令白名单执行 | done |
| RunReport schema + 归档 | done |
| 基线 run（≥3 fixture_llm case） | done（CNB `deepseek-v4-flash`，3/3） |

## 验收记录

本地 fake runner 已覆盖 3 个 `fixture_llm` case，命令 loop、白名单拒绝、history 回传、AnswerJudge、RunReport、SSE 和 token 脱敏测试通过。真实 CNB baseline 已由 `api_trigger_m58_llm` 在 feature branch 执行：provider=`cnb-ai-chat`、model=`deepseek-v4-flash`、build=`cnb-54f-1juijih5i`、pass rate=`1.0`、failure classes 为空；三个 case 均只发出一个 `compact` 命令，未发生 budget upgrade，页面/按钮/字段命令分别通过最小计划校验。报告只保留结构化摘要和脱敏 command trace，不归档 prompt、模型原文或 token。runtime contract preflight 的缺省回退与显式 override 已通过同一 live stage 验证。

提示词优化记录：早期 live run 暴露页面路径、按钮 target、裸 field 命令和 final JSON 收尾问题；随后固定 `page/comp/field` target 路由、compact-first、final literal 标签和单行 JSON 键集合，最终在 commit `b1221dd` 的 CNB run 中达到 3/3。该结果是当前 fixture/模型组合的基线，不等同于多模型或真实项目泛化结论。
