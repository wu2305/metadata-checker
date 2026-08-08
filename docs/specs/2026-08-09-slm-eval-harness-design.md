# SLM 评测 harness 设计（M58.2）

> 状态：draft（2026-08-09；待批准）
> 范围：把 M58 自研 JSON-command runner 冻结，改以真实 agent harness（kimi-code CLI）+ 外移判分作为评测主路径；固化此前只存在于代码与 commit 里的约束与结论。
> 编号：ai-eval 线 **M58.2**；前序 [2026-07-17-cheap-model-comprehension-eval-design.md](2026-07-17-cheap-model-comprehension-eval-design.md)（approved）。

## 为什么需要这份 spec

自 2026-07-17 spec 批准以来，评测路径发生了方向性变化，但**这些变化只存在于代码和 commit message 里**：六问真实项目冒烟、语义 judge 取代关键词统计、CNB tool calling 探测结论、AI 网关的调用位置限制、单模型约束。最新的 spec 仍停在 2026-07-17，描述的是一个已被取代的 runner。

任何人（包括 agent）从 `docs/` 进入这条工作线，读到的都是过时结论。本文档的第一目的是消除这个文档债，第二目的才是描述新设计。

## 既有事实（此前未落文档）

### 1. CNB AI 网关只能在 pipeline 内调用

本地使用 access_token 调用 AI 网关返回 **403**（`OpenAPI only allowed in pipeline`）。只有 pipeline 注入的 `CNB_TOKEN` 能调通。

推论：**被测 agent 必须在 CNB pipeline 内运行**。任何"本地跑一遍评测"的方案不成立。这条约束单独决定了评测只能是两阶段形态（pipeline 内产出 → 之后判分），不是实现选择。

### 2. 可用模型只有 deepseek-v4-flash

CNB AI Chat 当前只提供 `deepseek-v4-flash`（外部实测，2026-08-08；本仓库内无自动化举证，若 CNB 后续放开模型列表需重新核对本节）。推论：

- **被扫的变量是 prompt（SKILL.md），不是 model。** 模型是钉死的常量。
- 无法用"换个强模型看是否也失败"来区分「提示词不清楚」与「模型能力不足」。唯一的归因通道是**同一模型下的 prompt 配对 A/B**。
- 因此评测输出的第一公民是**逐 case 配对网格**，不是单一聚合通过率。

### 3. CNB AI Chat 支持原生 tool calling

`.cnb.yml` 的 `probe CNB AI Chat endpoint` stage 实测：`tools` + `tool_choice:required` 返回 200 且响应含 `tool_calls`。这推翻了 2026-07-17 spec 的假设（"当前契约未公开 tools / tool_calls 字段"），也是 kimi-code 能作为 harness 的前提。

同时确认 `stream:false` 返回 400，必须 `stream:true` + `Accept: text/event-stream`。

### 4. 真实项目图必须串行、可共享只读

`xiaoshouyi_large_real_cases.json` 的 `execution_notes` 明确：串行运行 case，并发容易触发 **redb lock**；`graph_db_path_strategy` 为 `temp_per_run_or_shared_readonly`。

推论：多 variant × 多 trial 的墙钟成本是本设计的一等约束，不能等撞上再处理。

### 5. 语料现状与 provenance 缺口

`xiaoshouyi_large_real_cases.json`：8 个 case，难度分布 3 easy / 2 hard / 3 expert。`verified_with` 只记录**校验**信息（日期、corpus @ `6920ac51`、binary @ `3e49dc6`）。

提取过程此前完全无记录，现已补进 fixture 的顶层 `provenance` 字段：这批 case 由 Kimi-K3 提取并自答，经 python 脚本与 metadata-checker CLI 实跑校验，再由人确认。**但该字段是事后追记的口述，不是可追溯的证据**——校验脚本、CLI 输出和人工确认记录都不在仓库里，无法复核。新增 case 必须逐 case 写 `provenance`，并保留可复核的产物。

推论：n=8 且每个难度层只有 2–3 个 case。这足以做调试仪表，**不足以回答产品问题**（"廉价模型 + SKILL.md 到底够不够用"）。语料扩充是与本设计并行的独立轨道，并且新 case 必须写入 `provenance`。

### 6. CNB stage 脚本按 sh 执行，本镜像 sh = dash

stage 脚本默认由 `sh` 解释，`browser-wasm-ci` 基于 `node:22-bookworm`（Debian 系），`/bin/sh` 是 dash。**所有 stage 脚本必须是 POSIX sh，不能用 bash 专属语法。**

这条以最坏的方式暴露：冒烟循环最初写成 `while IFS=$'\t' read -r ...`。dash 不认 ANSI-C quoting，把 `$'\t'` 当成字面的 `$`、`'`、`\`、`t` 四个字符，于是按字母 `t` 切分——case_id 被切碎、transcript 互相覆盖、问题文本变成垃圾，而**退出码全 0、记录数断言通过、token 检查通过，整条流水线全绿**。下游 judge 找不到对应文件也只记 infra 占位，不判红。

两个直接教训，已写进实现：

- `bash -n` 和 bash 下的 stub 干跑**都查不出这类缺陷**。shell 兼容性必须用 `dash` 实跑验证。
- stage 脚本必须显式 `set -eu`。CNB 未文档化是否启用 errexit；实测 dash 无 `-e` 时，记录数硬断言与空子集拒绝都只打印错误然后继续，一样是假绿。

## 冻结：M58 JSON-command runner

`tests/support/m58_ai_eval.rs`（2,608 行）+ `tests/m58_cnb_ai_runner_tests.rs`（2,840 行）= **5,448 行**自研 agent loop，即日**冻结**：保留既有 baseline 记录，不再新增 case，不再作为主路径演进。

必须同时记录对其分数的正确解读：最后一次 baseline `trial_pass_rate=0.2308`，其中 23/39 trial 死在 `wrong_command`（命令被 `policy.validate` 拒绝，在执行 CLI 之前）。**这个数字主要测量的是该 runner 自身的路由约束，不是模型的理解力。** 把它当作模型能力结论引用是错误的。

该 runner 回答的是一个真实但不同的问题：没有 tool calling 的模型能否驱动白名单协议。事实 3 已使这个问题脱离主路径。

## 设计

### 形态：两阶段

```
阶段 A（CNB pipeline 内，必须）
  kimi-code CLI + SKILL.md skill + metadata-checker 二进制
  → 逐 case 产出 transcript
  → CNB 内抽取 final answer
  → 归档 records + answers 为 artifact

阶段 B（判分，独立 pipeline）
  消费阶段 A 的 artifact
  → 从 metadata-checker-keys 导入 judge 凭证
  → 调用判分模型
  → 产出配对网格
```

阶段 A 的 agent 是 kimi-code，不是自研 loop：它自己读 SKILL.md、自己从命令错误里恢复、自己决定是否建图。这才是要验证的部署配置。没有命令白名单，没有 trial 截杀。

### 阶段 B 必须也是 pipeline

判分**不能**退化为"本地手动跑一下"。`docs/ai-eval-runs/` 里三份孤立的 `2026-05-08` 人工快照就是这么来的——那正是 2026-07-17 spec 列为"尚未闭环"的问题之一，如果判分靠手动，它会原样复发。

"外部判分"中的**外部只指 API endpoint，不指搬出 CI**。

### 判分规则

沿用现有 Rust judge 已确立的设计，逐条移植而非重新发明：

- **逐条断言判定**，每个 `must_mention` / `must_not_mention` 是一次独立的窄二元判断，不是对整份 `standard_answer` 的一次整体 rubric。
- **确定性聚合**：`pass = 所有 must 条目 supported ∧ violations == 0`。**不信任模型给出的 overall verdict。**
- 语义判定，不做关键词 / regex 命中——`must_mention` 是语义短句（如 `继承 panel35.visibleCondition`、`model11.totalRowCount__ > 0`），关键词匹配对措辞变体过脆，这一点已在替换关键词统计时确立。

### final answer 抽取留在 CNB 内且留在 Rust

`extract_final_answer` 现有 4 个单测覆盖：取最后一段 assistant 文本、拼接并跳过空串、无 assistant 时返回空、嵌套 content 与坏行容忍。

其中"无 assistant 时返回空串"意味着 **`""` 是合法返回值**。因此一个实现有缺陷的抽取器（例如用 jq 草率重写）会**静默**产出空答案，判分结果是全部 `not_mentioned`——一个看起来完全合理的零分。这是最难发现的失效模式。

设计决定：**不重写抽取器**。拆分 `kimi_smoke_judge.rs`，抽取逻辑连同其单测保留在 Rust 并运行在 CNB 内，只退役判分半边。

### 墙钟与图预算

- graphdb 每次 pipeline 运行**构建一次**，case 之间共享只读（`temp_per_run_or_shared_readonly` 已允许）。
- case **串行**执行（`execution_notes` 要求）。
- **variant 拆分到不同的 pipeline 触发**，使单次运行是 `cases × trials` 而非 `variants × cases × trials`。
- trial 数不靠估算：先在六问冒烟中记录逐问墙钟，再据实测与 CNB job 超时确定。

## 未决策：判分数据边界

判分模型若在 CNB 之外，xiaoshouyi（真实客户项目）的答案文本要出 CNB 闭环送第三方 API。

必须明确的是：**限制为只送 final answer 减少的是暴露量，不是暴露种类。** `must_mention` 条目本身就是 `model11.totalRowCount__ > 0`、`继承 panel35.visibleCondition` 这类内容——最终答案按设计必须包含字段名与表达式，否则无从判分。字段名级的项目信息出境是本方案的**固有成本**，不是可选成本。

两个选项，二选一，无第三条路径：

| 选项 | 得到 | 代价 |
|------|------|------|
| A：判分模型在 CNB 外 | 摆脱 judge 与被测模型同源的偏差 | 字段名级项目信息送第三方 API |
| B：判分留在 CNB（deepseek） | 数据不出闭环 | judge 与被测模型同源 |

选项 B 的代价需要展开：judge 的失败模式（对措辞流畅的回答判得更松）是**输入相关**的，而 prompt variant 恰好改变措辞流畅度。因此偏差不会在 A/B 两侧对消，它会伪装成正信号。选项 B 下的配对 A/B 结论上限因此明显受限。

**本决策留待人工拍板，不由实施方默认。** 阶段 A（含语料轨道）不依赖此决策。

## 非目标

- 引入 promptfoo 等 npm 评测框架。评估结论：CNB 网关的 pipeline-only 限制（事实 1）迫使两阶段，框架的 run 半边用不上；而语料 JSON 字段完备意味着换任何 runner 成本都低，这是"harness 选择很廉价"的理由，不是采纳某个框架的理由。六块重复 bash 的正解是把问题列表数据化 + 一个循环。
- 再造任何自研 agent loop。
- 用 M58 冻结 runner 的分数做模型能力结论。
- 在语料扩充到位前，用配对网格回答产品问题。

## 验收

1. 本 spec 批准后，`docs/milestones/INDEX.md` 与 M58 journal 反映冻结与新路径。
2. 六问冒烟中 `.cnb.yml` 不再含任何问题文本。
3. 逐 trial 结构化 record（含墙钟）成为下游唯一输入。
4. 判分外移后，阶段 B 是 pipeline 而非手动步骤。
