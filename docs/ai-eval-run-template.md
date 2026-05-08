# AI Eval 评测记录模板

用于记录空上下文小模型（如 5.4-mini）基于 SKILL.md + CLI 输出的真实评测结果。

## 记录原则

- 不要提交大段模型原始回答，只提交结构化结果和失败分类。
- 每次评测应记录：模型版本、case filter、通过/失败数量、关键失败项。
- 支持记录多个模型结果，包括 5.4-mini、5.3-codex、本地模型。

## 评测记录格式

```json
{
  "eval_run_id": "run-2026-05-05-5.4-mini",
  "model": "gpt-5.4-mini",
  "skill_md_version": "1.2.0",
  "cases_json": "tests/fixtures/corpus/ai_eval/ai_eval_cases.json",
  "binary_path": "/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker",
  "project_dir": "tests/fixtures/test_project",
  "real_project_dir": "/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi",
  "case_filter": "all | fixture-only | real-only",
  "timestamp": "2026-05-05T12:00:00Z",
  "summary": {
    "total_cases": 10,
    "passed": 8,
    "failed": 2,
    "skipped": 0
  },
  "results": [
    {
      "case_id": "page_purpose_actions_test",
      "status": "pass",
      "commands_used": ["--query-page-logic page:app/actions_test.spg"],
      "assertion_pass_count": 4,
      "assertion_fail_count": 0,
      "missed_facts": [],
      "forbidden_claims_hit": [],
      "diagnostics_handling": "n/a"
    },
    {
      "case_id": "dataflow_chain_trace",
      "status": "fail",
      "commands_used": ["--query-dataflow df_a", "--explain field:df_b.id"],
      "assertion_pass_count": 3,
      "assertion_fail_count": 2,
      "missed_facts": ["physical_x 作为中间节点"],
      "forbidden_claims_hit": [],
      "diagnostics_handling": "correctly_conservative",
      "failure_category": "missed_fact",
      "notes": "模型未能从 lineage 中识别 physical_x 节点"
    }
  ]
}
```

## 失败分类

| 分类 | 说明 |
|------|------|
| missed_fact | 遗漏 expected_fact |
| hallucination | 出现 forbidden_claim 或无依据推断 |
| wrong_command | 选用的 CLI 命令与 minimal_command_plan 偏差过大 |
| ignored_diagnostic | 遇到诊断未降级，仍做确定性结论 |
| over_read_details | 未先读 summary 而直接读取大量 raw details |
| plan_mismatch | 未按 minimal_command_plan 执行，导致断言失败 |

## 运行方式

### 机器自动运行

```bash
cargo test --test ai_eval_tests
```

自动执行所有 active case 的 expected_output_assertions 校验。

### 真实项目评测（M15）

1. 确认真实项目路径存在：
2. 使用独立 graphdb 路径：
3. 串行执行，避免 redb 锁冲突
4. 记录中标注  和 

### 5.4-mini 空上下文验收

- 使用全新空上下文 5.4-mini，只提供 SKILL.md + 二进制路径 + 项目路径 + case question
- 至少覆盖 5 类问题：页面用途、按钮行为、文本统计、模型被 DataFlow 消费、.tbl 单文件理解
- 记录结构化结果，不提交大段模型原文
- 失败项必须归类：missed_fact / hallucination / wrong_command / ignored_diagnostic

### 人工/半自动评测流程

1. 准备空上下文环境（不给源码，不给历史对话）。
2. 给 SKILL.md 全文 + 二进制路径 + 项目路径。
3. 按 case_id 顺序提问，记录模型选用的命令和回答。
4. 对照 answer_assertions.must_include / must_not_include 判分。
5. 记录结果到本模板 JSON。

### 独立 runner 设计（未来 CLI 子命令）

```bash
metadata-checker --run-ai-eval \
  --cases tests/fixtures/corpus/ai_eval/ai_eval_cases.json \
  --project-dir tests/fixtures/test_project \
  --output eval-results.json \
  --model 5.4-mini
```

runner 输入：
- `cases_json`：评测集路径
- `project_dir`：项目目录
- `binary_path`：metadata-checker 二进制路径（默认当前二进制）
- `case_filter`：可选 case_id 白名单

runner 输出：
- 结构化 eval result JSON，字段见上方模板
- 失败信息包含 case_id、命令、失败断言路径、实际值、期望值

runner 约束：
- 串行执行（避免 redb 锁冲突）
- 隔离 graphdb（临时目录复制）
- 复用 expected_output_assertions 断言执行逻辑
