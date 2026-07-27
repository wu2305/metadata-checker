# M58 AI Eval 运行报告模板

用于记录 CI tester 内的空上下文小模型评测。runner 只把 `SKILL.md`、固定项目路径、metadata-checker release binary 和当前问题提供给模型；模型通过 CNB AI Chat API 的 SSE 流返回 JSON `command` / `final` 消息。

## 记录原则

- JSON `RunReport` 是唯一事实源，Markdown 只由同一对象生成。
- 不写入 CNB token、Authorization header、完整 prompt、模型原始回答或大段 CLI 输出。
- 每次记录 provider、model、CNB build、case 状态、失败分类和命令轨迹统计。
- `fixture_structural` 不调用模型；`fixture_llm` 使用 fixture 项目；`real_manual` 只在明确的人工/手动入口执行。

## RunReport 格式

```json
{
  "schema_version": "1.0.0",
  "provider": "cnb-ai-chat",
  "model_id": "gpt-5.4-mini",
  "cnb_build_id": "build-20260727",
  "started_at": "2026-07-27T00:00:00Z",
  "cases": [
    {
      "case_id": "page_purpose_actions_test",
      "status": "pass",
      "passed": true,
      "max_command_count": 2,
      "failure_classes": [],
      "judge_notes": [],
      "command_trace": [
        {
          "command_kind": "--query-page-logic",
          "target": "page:app/actions_test.spg",
          "args": [],
          "budget": null,
          "plan_step_index": 0,
          "accepted": true,
          "detail_request": false,
          "budget_upgrade": false,
          "output_sections": ["summary"]
        }
      ]
    }
  ],
  "pass_rate": 1.0,
  "failure_classes": {},
  "command_trace_stats": {
    "total_commands": 1,
    "accepted_commands": 1,
    "rejected_commands": 0,
    "cases_with_commands": 1,
    "average_commands_per_case": 1.0,
    "max_command_count_exceeded_cases": 0,
    "budget_upgrade_count": 0
  }
}
```

`judge_notes` 只能是短的确定性诊断，不得复制模型回答。`command_trace` 只保留参数、plan step 和 section 元数据，不保留 stdout 全文。

## 失败分类

| 分类 | 说明 |
|------|------|
| `missed_fact` | 遗漏 `answer_assertions.must_include` |
| `hallucination` | 命中 `must_not_include` 或无依据禁用结论 |
| `wrong_command` | 命令未通过 `minimal_command_plan` 精确匹配 |
| `ignored_diagnostic` | 存在诊断但回答没有保守表达 |
| `over_read_details` | 请求 `--detail`、`--budget full` 或未允许的细节路径 |
| `needs_human_review` | case assertion schema 或自动证据规则无法确定 |
| `runner_error` | adapter、CLI 或 case workspace 的运行时失败，未伪造业务答案 |

一次 case 可以有多个失败分类；`pass_rate` 只按 `passed` 计数，不按失败分类去重。

## 本地验证

普通开发验证只运行结构和 fake adapter，不访问 CNB：

```bash
cargo test --features cli-local --test m58_cnb_ai_runner_tests
```

## CNB 手动/定时运行

真实 `fixture_llm` baseline 通过 `.cnb.yml` 的 `api_trigger_m58_llm` pipeline 运行，不进入 PR 阻塞 CI。pipeline 必须提供：

- `CNB_TOKEN`：仅用于 `Authorization: Bearer ...`，属于 pipeline-only 密钥；
- `M58_CNB_REPO`：可选覆盖项；未显式设置时回退到 `CNB_REPO_SLUG`；
- `M58_CNB_MODEL`：可选覆盖项；未显式设置时默认 `deepseek-v4-flash`；
- `M58_CNB_API_BASE`：可选，仅用于测试 endpoint 覆盖。

`M58_METADATA_CHECKER_BIN` 由 build stage 生成的 release binary 路径提供，不是本地凭据或手工配置要求。运行前先构建 release binary，使用每个 case 独立的 graphdb，并串行执行 case，避免 redb 锁冲突。报告默认写入 `target/m58-ai-eval/`；发布或归档前只上传结构化 JSON/Markdown。

## 空上下文边界

bootstrap 只包含仓库 `SKILL.md`、固定 binary/project 路径、JSON 协议和 case question。不得读取源码、历史运行记录、CNB Knowledge Base 或其他隐藏上下文；CLI stdout 以新的 `user` 消息回传给模型。
