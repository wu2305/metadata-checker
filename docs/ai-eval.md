# AI 问答验收层（M9-D）

M9-D 验证目标：空上下文小模型（如 5.4-mini）只读 SKILL.md + metadata-checker 二进制输出，能否正确回答真实业务问题。

## 与 M9-A/B/C 的关系

| 阶段 | 产物 | 职责 |
|------|------|------|
| M9-A | `tests/fixtures/corpus/manifest.json` | 原始语料元信息索引（1290+ 样本） |
| M9-B | `tests/fixtures/corpus/selection.json` | 精选 12 条代表样本进入仓库 |
| M9-C | `tests/fixtures/corpus/snapshots/` | 结构化 snapshot，捕获输出契约变化 |
| M9-D | `tests/fixtures/corpus/ai_eval/ai_eval_cases.json` | AI 问答评测集，验证模型能否基于 CLI 输出回答业务问题 |

M9-A/B/C 保证**语料和输出契约稳定**；M9-D 保证**AI 能基于稳定契约做出正确语义判断**。

## 空上下文验证流程

1. 拉起一个**全新空上下文**的 5.4-mini 实例。
2. 只给以下信息，不给源码解释、不给历史对话：
   - `SKILL.md` 全文
   - 二进制路径：`/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker`
   - 项目路径（如需）：`tests/fixtures/test_project`
   - 具体问题（从 `ai_eval_cases.json` 中抽取 `question`）
3. 模型必须按 SKILL.md 的**回答协议**执行：先选命令 → 读 summary → 按需读 details/evidence → 保守处理 diagnostics。
4. 收集模型回答，对照 `expected_facts` 和 `forbidden_claims` 判分。

## 判分规则

- **命中 expected_facts**：每个事实必须出现在回答中，允许同义表达。
- **无 forbidden_claims**：出现任何 forbidden claim 即判失败。
- **关键结论必须引用 evidence**：不能凭空推断，必须说明来源是 CLI 输出的哪个字段。
- **diagnostics 保守处理**：如果输出含 `UNRESOLVED_*`、`EVIDENCE_SAMPLED`、`LINEAGE_EXPR_UNPARSED` 等，模型必须降级表达，不能假装诊断不存在。

## 失败分类

| 类型 | 说明 |
|------|------|
| missed_fact | 遗漏 expected_fact |
| hallucination | 出现 forbidden_claim 或无依据推断 |
| wrong_command | 选用的 CLI 命令与 minimal_command_plan 偏差过大 |
| ignored_diagnostic | 遇到诊断未降级，仍做确定性结论 |
| over_read_details | 未先读 summary 而直接读取大量 raw details |

## 维护

- 新增 eval case：编辑 `ai_eval_cases.json`，同步更新 `tests/ai_eval_tests.rs`。
- 发现模型在某 case 持续失败：先检查 M9-C snapshot 是否已捕获输出变化；再检查 SKILL.md 协议是否足够明确；最后考虑补充 minimal_command_plan 或修改 expected_facts。
- 不要把模型的大段回答提交进仓库，只在 `ai_eval_runs/`（如有）记录结论和分类。
