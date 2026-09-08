# 历史提案：SKILL 规则覆盖与加权评分

> 状态：**历史草稿，未批准，未实现，不占里程碑编号**。2026-09-08 从本地归档恢复。
> 来源：`archive/m52-redb-v2-and-ci`，提交 `2f9dff1925ec1e74e5b9e049c089b8a706efb990`，
> 原路径：`docs/m58-cheap-model-understanding-eval-design.md`，Git blob `c347bfe6c8267e68fa2318b2b8d11a161ec50371`。
> 下方「历史原文」按原始字节保存；原文 SHA-256：`6115bee5468c2b429d3db39d0b551498d45559edeffbb2129ae38f820826b4dc`。

本稿提出规则注册表、case 的 `rule_ids` 标注、覆盖测试及四维加权趋势分数。
在 `main@3d595d3` 中未找到该稿指定的 `skill_rule_registry.json`、
`skill_rule_coverage_tests.rs`、`ai_eval_trend_report.rs`；既有 M58 approved spec
采用空上下文 runner 与确定性回答判分，不能据同属“评测”就认定此稿全部内容已经落地。

原文的 M58.1–M58.4 是旧草稿阶段号，**不对应现行 M58 子里程碑**。其“用户明确”等表述
仅作为当时的历史记录保留，不代表对现在开工、评分权重或门禁的授权。任何实施均须
另行按当前规划规范审批；不改现有判分器、题集或验收标准。

重新讨论时至少需要先解决：

- 规则被某 case 声明引用，不证明该 case 的断言能捕获违反规则的回答；需反例验证。
- must-include 等文本断言的通过率不等于语义正确性，不能用高效率得分抵消事实错误。
- 缺 diagnostics 的 conservatism 如何计分、不同 case 如何加权、未知/缺失记录如何处理，
  原稿没有完整定义。示例权重和 overall_score 不能直接作为可比较基线。
- 旧路径、现行 schema 与实现范围需重核。历史原文保留失效指针，不能作为当前导航。

本文件位于 `docs/archive/`，不进入知识库；不是代码恢复或已批准计划。
现行入口为 [M58 approved spec](../../specs/2026-07-17-cheap-model-comprehension-eval-design.md)。
**正文内所有相对链接（如 `docs/m56-dashboard-report-support-design.md`）均为旧归档路径语境，只作历史记录保留，不回写、不更新为当前文档树路径。**

## 历史原文

# M58 面向小模型/低成本模型的理解度评测设计

> 记录日期：2026-07-05。
> 本文档是 brainstorming 阶段产出的设计稿，尚未开始实现。

## Summary

`docs/ai-eval.md`（M9-D/E/F）已经把"空上下文小模型（如 5.4-mini）只读 SKILL.md + CLI 输出能否正确回答业务问题"确立为验收目标，并已经有结构化断言（`expected_output_assertions`）、结构化回答判分（`answer_assertions`）、结构化命令计划（`minimal_command_plan`）和失败分类。这些都是好的地基。

但当前框架缺两块：

1. **陷阱覆盖不系统**：`SKILL.md` 里有大量显式规则（Negative Constraints Checklist ~18 条、Diagnostics & Fallback Matrix、Bare Symbol Protocol、Display Logic Gating Hierarchy、DataFlow Projection intent 切换），但 `ai_eval_cases.json` 目前只有 10 个命名 case（`docs/ai-eval.md` M9-E 表），没有证据表明每条规则都至少有一个 case 能在模型违反该规则时捕获它。
2. **没有量化的"理解程度"信号**：现有判分是 pass/fail，看不出"这一版 SKILL.md 改动后，小模型的理解程度是变好还是变差，具体哪个维度变差"。

本设计只解决这两块（案例陷阱覆盖 + 量化评分/趋势追踪），不涉及"如何自动调用真实小模型跑评测"——那部分由 Codex/cnb.cool subagent 或 CI 单独解决，本设计只约定这类执行者需要产出什么格式的记录，评分从记录里确定性算出。

## Goals

- 系统性审计 `SKILL.md` 的每条可检查规则，确保至少有一个 eval case 能捕获违反该规则的回答。
- 定义一个可复现、确定性计算的复合分数，反映"这次 SKILL.md/CLI 改动是让小模型更容易理解，还是更容易犯错"。
- 分数可以按模型、按 SKILL.md 版本、按维度（risk_tags/difficulty/rule_ids）切片，用于诊断"哪里变差了"，而不是只给一个模糊的总分。
- 复用已有的 `docs/ai-eval-runs/*.json` 记录格式，只做扩展，不引入新的存储机制。

## Non-Goals

- 不做真实模型自动调用的 harness（`--run-ai-eval` runner）。这是已经在 `docs/ai-eval-run-template.md` 里标注为"未来"的独立问题，用户已明确要通过 Codex/cnb.cool subagent 或 CI pipeline 解决，不在本设计范围内。
- 不把分数做成 CI 硬门禁。这是一个诊断/趋势信号，用来证明"CLI 输出更容易被 LLM 理解了"和"SKILL.md 指令更精确了"，不用于自动拦截提交。
- 不引入 LLM-as-judge 主观打分。本仓库一直倾向确定性断言（M9-E 已经把 `evidence_requirements` 从自然语言迁移成结构化断言），本设计延续这个方向。
- 不做分层强制通过门槛（如"basic 必须 100% 通过才算达标"）。用户明确只需要趋势信号，不需要 go/no-go 判定。

## 架构方案对比

| 方案 | 描述 | 优点 | 缺点 |
|---|---|---|---|
| **1. 陷阱覆盖审计 + 加权复合分数（采用）** | 把 SKILL.md 每条规则映射到至少一个 case；复合分数由已有结构化字段（`answer_assertions`/`expected_output_assertions`/`minimal_command_plan`/`max_command_count`）算出 | 基本是扩展已有确定性基础设施，改动面小、可复现 | 案例编写工作量不小（但这是必要投入，不是可省略的基础设施成本） |
| 2. 分层强制通过门槛 | 按 difficulty 定最低通过率，作为 go/no-go 判定 | 简单直观 | 用户已明确不需要硬门禁；且会丢失渐进式趋势信息（"差一点没过"和"完全错"看起来一样） |
| 3. LLM-as-judge 主观打分 | 用更强模型对回答整体打分 0-100 | 能覆盖断言难以捕捉的开放式回答质量 | 非确定性、每个 case 多一次模型调用成本，且与本仓库一贯的确定性判分方向相反 |

选择方案 1。

## 陷阱覆盖审计

### 规则注册表

新增 `tests/fixtures/corpus/ai_eval/skill_rule_registry.json`，枚举从 `SKILL.md` 提取的可检查规则：

```json
{
  "rules": [
    {
      "rule_id": "bare_symbol_no_table_guess",
      "source_section": "Bare Symbol Protocol",
      "statement": "当组件值形如 ${FIELD} 且 FIELD 没有模型前缀时，不能直接回答'来自 FIELD 表'",
      "priority": "high"
    },
    {
      "rule_id": "display_gating_scope_order",
      "source_section": "Display Logic Gating Hierarchy",
      "statement": "必须按 condition_scope 的 if-else 顺序读取（direct -> inherited -> expanded_from_total_row_count -> referenced_by_model_filter），不能跳步或混用",
      "priority": "high"
    },
    {
      "rule_id": "candidate_not_proven",
      "source_section": "Negative Constraints Checklist",
      "statement": "禁止把 candidate_inputs 当作 proven_physical_input",
      "priority": "medium"
    }
  ]
}
```

覆盖范围：Negative Constraints Checklist 全部条目、Diagnostics & Fallback Matrix 全部 code、Bare Symbol Protocol 步骤、Display Logic Gating Hierarchy 步骤、DataFlow Projection intent 切换规则。每条规则标注 `priority`（`high`/`medium`/`low`），`high` 优先分配新 case。

### case 标注

`ai_eval_cases.json` 每个 case 新增 `rule_ids: []` 字段（与现有 `risk_tags` 并列的独立维度），标注该 case 设计用来捕获哪些规则的违反。

### 覆盖检查测试

新增 `tests/skill_rule_coverage_tests.rs`：

- 读取 `skill_rule_registry.json` 和 `ai_eval_cases.json`。
- 断言每个 `rule_id` 在至少一个 `case_status = active` 的 case 的 `rule_ids` 中出现。
- 未覆盖的规则测试失败，报错信息包含 `rule_id`、`source_section`、`statement`，直接告诉开发者要补哪个 case。
- 这是机器可验证的门禁（覆盖率本身可以强制，不是"分数"，两者不冲突：分数是模型表现的诊断信号，覆盖率是测试完整性的工程门禁）。

### 优先填补的 case 缺口

按现有 10 个命名 case 反推，以下规则大概率缺 case，优先补齐：

- Bare Symbol Protocol 的完整链路（`raw_expr` → `nearest_data_context` → `field_path` → `table_source_path` → `dataflow_field_origin`）。
- Display Logic Gating Hierarchy 的 `expanded_from_total_row_count` 和 `referenced_by_model_filter` 分支。
- `candidate_inputs` vs `proven_physical_input` 的混淆陷阱。
- medium/low confidence evidence 被误当 high confidence 使用。
- `read_by_count=0` 被误判为"模型未使用"（忽略 `consumed_by_dataflow_count`/`dataflow_role`）。

## 复合分数

### 单 case 分数

```
case_score = 0.5 * correctness        (answer_assertions + expected_output_assertions 通过率)
           + 0.2 * protocol_adherence  (commands_used 与 minimal_command_plan 匹配，无 wrong_command)
           + 0.15 * conservatism       (存在 diagnostics 时正确降级，无 ignored_diagnostic)
           + 0.15 * efficiency         (commands_used <= max_command_count，无 over_read_details)
```

四个维度全部可以从已有字段确定性计算，不需要新的主观判断：

- `correctness`：`answer_assertions.must_include`/`must_not_include` 是否满足 + `expected_output_assertions` 通过比例。
- `protocol_adherence`：实际使用命令序列与 `minimal_command_plan` 展开的 `required_commands` 的匹配度（完全匹配=1.0，命令类型对但目标/参数偏差=0.5，命令类型错=0）。
- `conservatism`：case 输出含 diagnostics 时，回答是否包含降级表达（可用 `answer_assertions.diagnostic_disclaimer_required` 已有机制判断）。
- `efficiency`：`commands_used` 数量是否超过 `max_command_count`，以及是否有 `over_read_details` 迹象（读取了 `allowed_output_sections` 之外的字段）。

### 聚合

- **按模型聚合**：`model_score = mean(case_score for case in active_cases)`，用于比较 5.4-mini vs. 其他候选小模型。
- **按 SKILL.md 版本聚合**：同一模型在不同 `skill_md_version` 下的分数对比，用于验证"这次 SKILL.md 改动是不是真的提升了理解度"。
- **按维度切片**：可按 `risk_tags`、`difficulty`、新增的 `rule_ids` 分组计算子集平均分，用于定位"哪一类规则/哪一类难度在退化"。

## 记录与趋势追踪

### 扩展现有 run 记录格式

`docs/ai-eval-run-template.md` 的 run JSON 里，每个 `results[]` 条目新增：

```json
{
  "case_id": "...",
  "status": "pass",
  "commands_used": ["..."],
  "case_score": 0.92,
  "dimension_scores": {
    "correctness": 1.0,
    "protocol_adherence": 0.8,
    "conservatism": 1.0,
    "efficiency": 1.0
  }
}
```

顶层 `summary` 新增 `overall_score`（全部 active case 的加权平均）。这是对已有 schema 的纯增量扩展，不破坏现有字段。

### 趋势报告

新增 `src/bin/ai_eval_trend_report.rs`（复用 `m51_profile_report.rs` 的既有模式：一个读取历史记录、打印对比表的小 bin，不新建存储系统、不做 UI dashboard）：

- 读取 `docs/ai-eval-runs/*.json` 全部记录。
- 按 `model` + `skill_md_version` 分组，打印 `overall_score` 随版本变化的表格。
- 按 `risk_tags`/`rule_ids` 维度打印子集分数，方便定位退化来源。
- 纯只读报告工具，不修改任何记录文件。

## 执行契约（与本设计解耦）

不管由谁产出评测记录（人工、Codex/cnb.cool subagent、CI pipeline），只需要满足：

- 每条记录包含 `case_id`、`model`、`skill_md_version`、`commands_used`（有序）、模型的最终回答原文。
- 记录写入符合 `docs/ai-eval-run-template.md` 扩展后的 schema。
- 分数从记录确定性计算，不依赖执行者是谁、怎么调用模型。

## 里程碑与测试

- **M58.1**：`skill_rule_registry.json` + `skill_rule_coverage_tests.rs`，先把审计门禁立起来（此时大概率大量规则未覆盖，测试会失败，这是预期的、用来驱动后续补 case 的信号）。
- **M58.2**：按优先级补齐缺口 case（先 `high` priority），每补一批重跑覆盖测试直至全绿。
- **M58.3**：`case_score`/`dimension_scores` 计算逻辑（可以是 `ai_eval_tests.rs` 里的辅助函数，也可以是独立可复用函数），扩展 run 记录 schema。
- **M58.4**：`ai_eval_trend_report.rs` 趋势报告工具。
- 测试要求：覆盖测试本身是强制 CI 项（防止新增 case 不打 `rule_ids` 导致覆盖率虚高）；分数计算逻辑需要单元测试覆盖四个维度的边界情况（如 diagnostics 存在但回答未降级、命令数超出 `max_command_count`）。

## 后续队列（本次不做，但已识别）

1. **真实模型自动调用 harness**——用户计划通过 Codex/cnb.cool subagent 或 CI 单独解决，本设计只约定了它需要产出的记录格式。
2. **元数据创建/修改能力**——仍排在"理解"完全站稳之后（详见 `docs/m56-dashboard-report-support-design.md` 的后续队列）。
