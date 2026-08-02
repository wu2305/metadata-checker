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

### 等价路由与发现步骤（RunReport 1.3.0，2026-08-02）

上一节把瓶颈定位到路由后，检查 4 个提问最模糊的 case 发现：`minimal_command_plan` 此前只承认一条规范写法，模型任何等价写法都算 `command_rejected`。这在本项目里是错误的判分口径——**终端用户没有能力精确提问，而 `.spg` 文件大到无法进入模型上下文**，所以模型必须在没有任何工具输出的情况下盲选第一条命令。提问模糊是被测条件本身，不是 fixture 缺陷；把问句改精确等于把 benchmark 改简单。

因此 plan 放宽为两种手段，用途严格区分：

- `alternatives`：与 primary 拿到**同一批事实**的等价写法，直接算路由成功，但在报告里保留 `alternate` 标记。
- 追加 plan step：first guess 拿不到全部事实时的补救/发现步骤。模型仍要自己意识到需要补查，只是不再被一次拒绝直接判死。

红线：**拿不到事实的动词不得写进 `alternatives`**，死路必须继续记为 `command_rejected`，否则等于把路由失败洗成理解失败。四个 case 的每条候选写法都用真实 CLI 跑过、逐条核对事实可达性后才写入。

渐进披露有实证支撑：组件级 `--explain-condition comp:app/actions_test.spg|button2` 的输出里 `condition_id` 就是 `cond:app/actions_test.spg|button2#action1#conditionExp`，即粗查会自己暴露内部 action id 与条件字段名，动作级命令因此作为第二步而非唯一入口。

RunReport 升到 `1.3.0`，新增 `command_route_usage`（按 `<task_family> -> <命令> (primary|alternate)` 聚合**被接受**的命令）。它与 `command_routing_confusion` 互斥互补：后者记被拒绝的误路由，前者记走通的路径分布；`alternate` 占比高说明规范动词不是模型的自然选择，是工具接口信号而非模型失分。命令 trace 同时记录 `route`，且 runner 执行的是**模型实际写的那一条**而不是 plan 的规范命令，否则报告里的 trace 与真正跑过的 CLI 不是同一个命令。

本地 `ai_eval_tests` 25/25、`m58_cnb_ai_runner_tests` 41 通过（1 个 live 测试按 token 门控 ignored），未触发 CI。

#### 工具接口缺陷修复（2026-08-02，经用户显式授权）

下面三项原本记在「明确不做」里（不改 SKILL.md / 输出 schema / answer_contract），由用户显式授权后修复。它们都是**工具接口缺陷**，不是模型能力问题：

1. **`--advise-query` 双前缀**：`--question-kind availability` 对已带前缀的 target 再拼一层，产出 `model:comp:app/x.spg|button2` 这种无效 target。改为统一走 `scoped_target()`：已带 `comp:`/`field:`/`model:`/`page:`/`action:`/`dataflow:`/`cond:` 前缀的原样返回，裸 ID 才按 question-kind 补前缀。同一个缺陷在 display/value-source/writer 上也存在（page_scope 与带前缀 target 同时给出时），一并修掉。
2. **`--advise-query` 不发标准信封**：载荷原本是顶层裸 JSON，任何按 `AiOutput` 转发的消费方（例如 M58 runner 的 `filter_cli_output`）只会拿到 `{}`。现在统一走 `AiOutput`，新增 `OutputKind::QueryAdvice`，路由载荷原样放进 `details`（键名不变），`summary` 给出 `question_kind`/`primary_command`/`primary_target`，`next_queries` 给出可直接执行的命令串。`docs/reference/function-calling-runtime.md` 早就写着该工具返回 `result.summary`、`result.details`——这次是实现向已发布契约对齐。顺带：未识别的 question-kind 此前静默回退到 auto，现在会带 `UNKNOWN_QUESTION_KIND` 诊断。
3. **补齐 action 轴**：新增 `TraversalIntent::Action`、`--intent action`、`--question-kind action` 与 `answer_facts.action_facts`。事实全部来自既有图与既有 blocking_conditions，不新增解析：`Triggers` 边给出目标挂了哪些动作，`owner_type = Action` 的条件给出门禁表达式。关键在于它把用户问句里不可能出现的 action id 以可直接执行的 `action:<页面>|<组件>|<动作>` 交回给模型，渐进披露因此变成工具保证而不是运气。

修复过程中发现并一并修掉一个更早的缺陷：动作条件的 `owner_node_id` 此前算成 `action:<文件>|<组件>`，丢掉了动作段，指向一个不存在的节点——模型照着查必然落空。

一个刻意的区分：组件被别的动作的门禁条件引用时（例如 `input1.value=='test'` 挡住 `button2` 的 action1），这些动作放在 `related_actions[]` 而不是 `actions[]`。它们是「为什么点不动」的关键证据，但把它们说成 input1 自己的动作就是幻觉。实测 `--intent action` 打在 `comp:...|input1` 上返回 `action_count=0`、`related_action_count=1`。

`--intent action` 未并入 `auto`：既有 auto 查询的输出体积保持不变，action_facts 只在显式请求时出现。

#### 阻塞项（已于 2026-08-02 修复，保留记录）

`--advise-query --question-kind {display|value-source|availability|writer|page-logic|model-relationships}` 本可作为确定性路由预言机，直接验证「把推理搬进 Rust」这一主张，但两处输出层缺陷挡住了实验，且都落在「不得擅自修改输出逻辑」的约束内，故只记录不修改：

1. `--question-kind availability` 的 `primary_target` 是 `model:comp:app/actions_test.spg|button2`（双前缀），模型照此构造的 target 无效。其余五个 kind 干净；而 `availability` 恰好是上述 case 涉及的那条轴。
2. `--advise-query` 不发标准 `AiOutput` 信封（载荷是顶层 `primary_command`/`primary_target`/`followup_rules`），而 runner 的 `filter_cli_output` 只转发 `schema_version`/`kind`/`query_target` 加 `{summary, details, evidence, diagnostics, next_queries}`，模型会收到 `{}`。

同时记录一个结构性观察：组件的三条属性轴中，display 与 value 由 `--intent` 在组件 target 上寻址，**actions 没有对应的 intent**，必须同时换动词（`--explain`）和换 target 文法（`action:`）；`--advise-query --question-kind` 同样缺 action 一类。这是路由不自解释的根因之一，是否补齐待决策。

上述两项阻塞与 action 轴缺失已在 2026-08-02 全部修复（见上一节），路由预言机实验不再被挡住。下一次 CNB live run 可以直接观察 `command_route_usage` 与 `command_routing_confusion` 的变化。
