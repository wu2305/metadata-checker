# M58.2 LLMOps 评测闭环计划

> 状态：**closed（实施交付范围收口，2026-09-06）**；2026-08-23 原批准范围与 kimi 浮动策略保留。
> 范围：把已落地的 Stage A 冒烟收成「改 SKILL/CLI → 再跑 → 可比对」的评测闭环。不扩语料、不改 gold 断言、不重写 SKILL.md。
> 编号：ai-eval 线 **M58.2** 续作；spec [2026-08-09-slm-eval-harness-design.md](../specs/2026-08-09-slm-eval-harness-design.md)（approved）。

## 产品定义

1. `deepseek-v4-flash` 在 kimi-code 里带着 `metadata-checker` 答题。
2. `glm-5.2` 逐条对照 fixture `standard_answer`；Rust 算 pass（全部 must supported ∧ violations==0）。
3. 结果可复现到「除被改的那一个变量外，pins 相同」。kimi-code 故意浮动：始终 `curl install.sh`，只把 `kimi_version` 记进 `run.json`。SKILL/CLI A/B 必须共享同一次 kimi 安装（同一 pipeline job，或两份附件的 `kimi_version` 相同）。

跨日、跨 kimi 版本的分数差（如 0.37.2 的 3/6 vs 0.38.0 的 2/6）是 harness 增量，不当成 prompt 信号。

## 三个率（固定分母，不筛行为）

| 率 | 含义 | 分母 |
|---|---|---|
| `answer_quality`（即原 `task_score`） | 语义通过 | 全部非 INFRA trial |
| `tool_adherence` | `mc>0` | 有匹配 record 的非 INFRA trial |
| `tool_assisted_quality` | `mc>0` 且语义 PASS | 同上。**LLMOps 主指标** |

raw-only PASS 计入 `answer_quality`，不计入 `tool_assisted_quality`。禁止再引入「按行为筛分母」的 `tool_score`。

## PR 切分

### PR1 — 身份 + 三个率（本计划落地的第一刀）

- `run.json` 增加 `fixture_path` / `fixture_sha256`、`agent_temperature` / `judge_temperature`（当前 API 未设则 `null`）、`kimi_pin_policy: "float"`。
- `judge.md` 输出上表三率；保留行为分层作诊断。
- 干跑断言新字段；Rust 单测锁三率口径。
- journal / INDEX 链到本计划。

### PR2 — 阶段 B 同 pipeline 下一 stage（复用 runner env）

- **不**新增 `api_trigger_kimi_harness_judge`。A（generate）与 B（judge）都在 `api_trigger_kimi_harness_smoke` 里顺序执行。
- CNB 同一 pipeline 的 stage 共用 workspace；`docker.volumes` 里的 `target/cnb/kimi-harness-build` 与 `target/kimi-harness-smoke` 跨 stage 保留。B 不安装 kimi、不编 release、不拉语料。
- 语义 FAIL 不判红；缺产物 / `fixture_sha256` 对不上 / judge API 失败才 INFRA。
- 同一 job 内跑两个 `KIMI_SMOKE_VARIANT` 是后续 SKILL A/B 的触发方式，不在本 PR 改 SKILL。

## 非目标

- 钉死 kimi-code 版本。
- 本轮验证或改写 8 条 gold（`recalled_unverifiable` 仍在；gold 变更是单独实验）。
- 本轮改 SKILL.md 或 CLI 动词。
- 把 n=6 写成产品结论。

## 验收

1. 干跑绿；`kimi_harness_judge_tests` 覆盖三率与 `run.json` 字段守卫。
2. 下一轮 live smoke 的 `run.json` 含 `fixture_sha256` 与 `kimi_pin_policy=float`。
3. 同一次 `api_trigger_kimi_harness_smoke` 里 smoke run 之后有 `kimi-code harness judge`；干跑先 A 后 B、A 失败则跳过 B。


## 收口记录（2026-09-06）

PR1 身份/三率与 PR2 同 pipeline judge 已交付，实现与历史运行坐标见
[M58 journal](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)。本次不重新执行
收费 live baseline，也不把早于 2026-08-23 字段扩展的 run 当作新字段的实跑证明。
原验收中“下一轮 live smoke”的新字段/产物核验与扩充后 11-case 首轮基线一起转交
[M59-BASELINE](2026-09-06-m59-grafeo-implementation-plan.md)，因此标为范围关闭，非全部验收 done。
