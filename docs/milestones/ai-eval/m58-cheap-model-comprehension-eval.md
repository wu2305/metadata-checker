# M58：廉价模型理解力评测

> 状态：**active**（spec/plan approved；M58.1 多 trial/任务维度已落地，CNB live baseline 已完成，待独立 review 收口）
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
| 历史基线 run（3 fixture_llm case，单 trial） | done（CNB `deepseek-v4-flash`，3/3；仅作 smoke） |
| M58.1 多 trial runner、稳定性指标与任务维度 | done（本地 fake，13 active fixture_llm case） |
| M58.1 CNB 3-trial live baseline | done（最新 SN `cnb-ubg-1juus86tj`；结果为观察基线，不设模型通过门槛） |

## 验收记录

本地 fake runner 已覆盖 13 个 active `fixture_llm` case，覆盖 page/action/lineage/dataflow/navigation/diagnostic/context/condition 等任务族；每个 case 都有 `evaluation_dimensions`，并通过独立 trial、命令 loop、白名单拒绝、history 回传、AnswerJudge、RunReport、SSE 和 token 脱敏测试。历史真实 CNB smoke 曾由 `api_trigger_m58_llm` 执行：provider=`cnb-ai-chat`、model=`deepseek-v4-flash`、build=`cnb-54f-1juijih5i`、单 trial pass rate=`1.0`、failure classes 为空；它只证明当时三个 case 的一次端到端路径可用，不证明 Skill 的稳定理解或任务泛化。新的 live runner 默认每个 case 执行 3 个独立 trial，报告同时输出 `trial_pass_rate`、`case_stable_pass_rate` 和 `cases_with_flaky_trials`；只保留结构化摘要和脱敏 command trace，不归档 prompt、模型原文或 token。runtime contract preflight 的缺省回退与显式 override 已通过同一 live stage 验证。

首次 CNB 3-trial baseline（2026-07-28，SN `cnb-u1g-1juj6fivd`）完成 13 个 case、39 个 trial；其中发现评测器对 fixture plan 省略 `budget` 与模型显式 `compact` 的等价语义未统一。修复该 evaluator contract 后，最新 CNB 3-trial baseline（SN `cnb-qi8-1jujfcuvp`）重新完成 13 个 case、39 个 trial，pipeline、release build、SSE runner 和脱敏报告 stage 均成功：`trial_pass_rate=0.1538`、`case_stable_pass_rate=0.0`、`cases_with_flaky_trials=3`；失败分类为 `wrong_command=21`、`missed_fact=9`、`needs_human_review=5`、`hallucination=3`、`ignored_diagnostic=3`、`protocol_error=2`。模型理解结果仍然较低，且没有 case 达到三次全通过；该结果将作为后续修改 Skill 路由、命令计划和模型选择的对照基线，不能解释为 CNB API 或 metadata-checker runner 故障，也不能解释为模型已通过。

在独立 review 修复三项 contract 缺口后，最终 CNB 3-trial baseline（2026-07-28，SN `cnb-voo-1jujgnsbf`）再次完成 13 个 case、39 个 trial，且 exact case-count guard、SSE Content-Type/line validation、runner 和脱敏报告 stage 均成功：`trial_pass_rate=0.051282`、`case_stable_pass_rate=0.0`、`cases_with_flaky_trials=2`；失败分类为 `wrong_command=21`、`missed_fact=10`、`needs_human_review=7`、`hallucination=4`、`ignored_diagnostic=3`、`protocol_error=3`。分数变化属于模型多 trial 随机性和 contract 收紧后的观测变化；没有 case 达到三次全通过，结果仍只作为后续 Skill 路由、命令计划和模型选择的对照基线，不能解释为模型已通过。

提示词优化记录：早期 live run 暴露页面路径、按钮 target、裸 field 命令和 final JSON 收尾问题；随后固定 `page/comp/field` target 路由、compact-first、final literal 标签和单行 JSON 键集合。commit `b1221dd` 的 CNB run 曾在当时的 3 个 smoke case 上达到 3/3；该数字只覆盖那一版 3-case 集合，不适用于其后的 13-case baseline。

### 判分器修复后的基线（2026-08-01，SN `cnb-ubg-1juus86tj`）

在修复三项判分器缺陷——`must_include` 改为同义组、证据检查改为「是否存在被接受的命令」而非复述英文 section 名、`wrong_command` 拆分为 `no_command`/`command_rejected`/`wrong_command`——并把 `key_primary_paths` 从 10 条收敛到 3 条（与同级 `top_*` 一致，compact 输出 131,190 → 102,158 字节）后，CNB 3-trial baseline 完成 13 个 case、39 个 trial：`trial_pass_rate=0.2308`、`case_stable_pass_rate=0.2308`、`cases_with_flaky_trials=0`；失败分类为 `wrong_command=23`、`missed_fact=7`，`hallucination`、`ignored_diagnostic`、`needs_human_review`、`protocol_error` 全部归零。

两项结构性变化值得单独记录：

- `case_stable_pass_rate` 与 `trial_pass_rate` 完全相等且 `cases_with_flaky_trials=0`，即每个 case 都是 3/3 通过或 3/3 失败，评测结果首次完全确定。此前所有 baseline 都存在 2–3 个 flaky case。
- 失败分类恰好划分全部 trial：`9 通过 + 7 missed_fact + 23 wrong_command = 39`。由于命令被拒绝会在执行 CLI 前终止 trial，这个划分把分数拆成两段独立指标：**路由成功率 16/39 = 41%**，**路由成功后的理解通过率 9/16 = 56%**。

结论：`trial_pass_rate=0.2308` 不是理解能力指标。真正的瓶颈在命令路由，即 `--query-*` 动词表面对模型不是自解释的；这属于工具接口缺陷，不是模型能力不足。此外 `deepseek-v4-flash` 在此期间发生过模型能力更新，跨 baseline 的绝对分数不可直接比较，但上述分段归因不受影响。

该 run 中 23 次拒绝全部由 runner 的 `policy.validate` 失败路径产出，当时该路径仍硬编码 `wrong_command`，使 `command_rejected` 实际不可达；随后已修正，并新增 `command_routing_confusion`（按 `<task_family> -> <模型选择的命令>` 聚合被拒绝命令），下一次 run 起可直接读出具体的误路由动词对。
