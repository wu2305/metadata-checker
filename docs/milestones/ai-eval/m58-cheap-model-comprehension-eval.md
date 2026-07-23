# M58：廉价模型理解力评测

> 状态：**planned**（spec draft 已提交评审，`approved` 后转 active）  
> Spec：[2026-07-17-cheap-model-comprehension-eval-design.md](../../specs/2026-07-17-cheap-model-comprehension-eval-design.md)  
> Plan：（spec 批准后补，多 PR 工作须有 approved plan）

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
| Spec（draft）登记 | done（本文档 PR） |
| Spec 批准 | pending |
| Plan | pending |
| AnswerJudge + 单测 | pending |
| ModelAdapter + 命令白名单执行 | pending |
| RunReport schema + 归档 | pending |
| 基线 run（≥3 fixture_llm case） | pending |

## 验收记录

（实现后填写：基线 run model_id / pass 率 / 失败分类；judge 与人工判分一致性；密钥脱敏测试）
