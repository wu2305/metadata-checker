# M58 AI Eval 运行报告模板

用于记录 CI tester 内的空上下文小模型评测。runner 只把 `SKILL.md`、固定项目路径、metadata-checker release binary 和当前问题提供给模型；模型通过 CNB AI Chat API 的 SSE 流返回 JSON `command` / `final` 消息。

## 记录原则

- JSON `RunReport` 是唯一事实源，Markdown 只由同一对象生成。
- 不写入 CNB token、Authorization header、完整 prompt、模型原始回答或大段 CLI 输出。
- 每次记录 provider、model、CNB build、trial/case 稳定性、失败分类和命令轨迹统计。
- `fixture_structural` 不调用模型；`fixture_llm` 使用 fixture 项目；`real_manual` 只在明确的人工/手动入口执行。

## RunReport 格式

```json
{
  "schema_version": "1.1.0",
  "provider": "cnb-ai-chat",
  "model_id": "gpt-5.4-mini",
  "cnb_build_id": "build-20260727",
  "started_at": "2026-07-27T00:00:00Z",
  "cases": [
    {
      "case_id": "page_purpose_actions_test",
      "trial_index": 0,
      "task_family": "page_logic",
      "difficulty": "basic",
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
          "budget": "compact",
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
  "trial_pass_rate": 1.0,
  "trial_count": 1,
  "case_count": 1,
  "case_stable_pass_rate": 1.0,
  "cases_with_flaky_trials": 0,
  "failure_classes": {},
  "command_trace_stats": {
    "total_commands": 1,
    "accepted_commands": 1,
    "rejected_commands": 0,
    "cases_with_commands": 1,
    "average_commands_per_case": 1.0,
    "average_commands_per_trial": 1.0,
    "max_command_count_exceeded_cases": 0,
    "budget_upgrade_count": 0
  }
}
```

`judge_notes` 只能是短的确定性诊断，不得复制模型回答。`command_trace` 只保留参数、plan step 和 section 元数据，不保留 stdout 全文。

多 trial 报告中，`cases_with_commands` 按去重后的 `case_id` 计数；`average_commands_per_case` 的分母是去重后的 `case_count`，`average_commands_per_trial` 的分母是报告中的 trial 行数，避免 trial 数增加后 case 维度被重复放大。

## 失败分类

| 分类 | 说明 |
|------|------|
| `missed_fact` | 遗漏 `answer_assertions.must_include` |
| `hallucination` | 命中 `must_not_include` 或无依据禁用结论 |
| `wrong_command` | 命令未通过 `minimal_command_plan` 精确匹配 |
| `ignored_diagnostic` | 存在诊断但回答没有保守表达 |
| `over_read_details` | 请求 `--detail`、`--budget full` 或未允许的细节路径 |
| `needs_human_review` | case assertion schema 或自动证据规则无法确定 |
| `runner_error` | adapter、CLI 或 case workspace 的运行时失败，未伪造业务答案；包含 CNB HTTP/transport/SSE 级别异常或 JSON 未返回前就失败的适配层问题 |

一次 trial 可以有多个失败分类；`trial_pass_rate` 按 trial 行计数，`case_stable_pass_rate` 只有同一 case 的所有 trial 都通过才计为通过。为兼容旧消费者，`pass_rate` 与 `trial_pass_rate` 保持同值；`cases_with_flaky_trials` 单独标出同一 case 试验结果不稳定的情况。

`task_family` 和 `difficulty` 用于按能力切片；case 原始的 `evaluation_dimensions`（如 `target_resolution`、`distractor_count`、`dependency_depth`、`stateful`、`error_injection`、`output_truncation`）保留在评测集，不进入模型 bootstrap。

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
- `M58_AI_EVAL_TRIALS`：live 默认 `3`；本地/fake runner 默认 `1`，每个 trial 使用独立 history、workspace 和 graphdb；
- `M58_CNB_API_BASE`：可选，仅用于测试 endpoint 覆盖。

`api_trigger_m58_llm` 先跑 runtime contract preflight：在独立 subshell 中执行与 live stage 完全相同的 `export M58_CNB_REPO="${M58_CNB_REPO:-${CNB_REPO_SLUG:?}}"` 与 `export M58_CNB_MODEL="${M58_CNB_MODEL:-deepseek-v4-flash}"`，验证仅有 `CNB_TOKEN + CNB_REPO_SLUG` 时空/缺失的 `M58_CNB_REPO` 会回退到 `CNB_REPO_SLUG`、空/缺失的 `M58_CNB_MODEL` 会回退到 `deepseek-v4-flash`，同时确认显式覆盖不会被默认值覆盖。

`M58_METADATA_CHECKER_BIN` 由 build stage 生成的 release binary 路径提供，不是本地凭据或手工配置要求。运行前先构建 release binary，使用每个 case/trial 独立的 graphdb，并串行执行 case，避免 redb 锁冲突。报告默认写入 `target/m58-ai-eval/`；发布或归档前只上传结构化 JSON/Markdown。

## 空上下文边界

bootstrap 只包含仓库 `SKILL.md`、固定 binary/project 路径、JSON 协议和当前问题。不得读取源码、历史运行记录、CNB Knowledge Base 或其他隐藏上下文；也不得注入 `expected_facts`、`must_include`、`minimal_command_plan`、`CNB_TOKEN`、答案键或隐藏 case 数据。CLI stdout 以新的 `user` 消息回传给模型。Bootstrap 不得包含任何实际 case 的 target/answer。

bootstrap 必须显式约束：

- 每轮只返回一个 raw JSON object；禁止 Markdown fence、解释性前缀或后缀；`final` 只能有 `kind` 和 `answer` 两个键，`answer` 使用简短字符串且单行输出。
- 先按问题意图选命令，再选最小查询范围：单组件/按钮/动作（包括“点击后发生什么”）-> `--explain`，不得误用 `--query-page-logic`；页面整体逻辑 -> `--query-page-logic`；writer/value-source/condition -> `--explain-condition` 且携带 `--intent`（如 `display`、`value-source`、`writer`）。
- 裸 `field:<model>.<field>` 的值/来源/写入问题优先使用 `--explain field:<model>.<field>`；不要因为“值从哪里来”把直接字段关系误选成 `--explain-condition`。
- 按钮/点击问题必须使用 `--explain comp:app/<relative-file>.spg|<component-id>`，不得使用 `--query-page-logic`。
- command JSON 只给固定 schema / shape example，例如 `{"kind":"command","command_kind":"--query-page-logic","target":"page:<relative-page-path>.spg","args":[],"budget":"compact"}`；这不是当前 case 的答案，也不是允许 target。
- 实际 command turn 必须依据 `SKILL.md` 和 question 自主选择 `command_kind`、`target`、`args`、`budget`；不要从 case metadata 注入最小计划。
- 页面目标必须保留标准相对路径：`page:app/<relative-file>.spg`。
- target 构造固定为 `page:app/<relative-file>.spg`、`comp:app/<relative-file>.spg|<component-id>` 或 `field:<model>.<field>`；不得删除 `app/` 或 `.spg`。`budget` 只能放在 command JSON 顶层字段，不得塞入 `args`。
- final answer 至少引用一个 literal section name（`summary` / `details` / `evidence` / `diagnostics`）；page final 必须 literal 包含“用户入口、按钮、写入目标、action”，field final 必须 literal 包含“页面 action、写入、字段”，并以 CLI 实际事实填充名称和数量。
- 裸 field 的一次 compact `--explain` 已有 `summary` 和 `evidence` 后立即返回 final，不要再发第二条命令。
- 第一轮先走最小允许查询：默认使用 `--budget compact`；仅在 `diagnostics`/`OUTPUT_TRUNCATED` 不足时再升级到 `normal` 或 `full`。`budget` 仅表示查询深度，不是路径或目标前缀。
- 后续 `user` 消息只作为 CLI 证据读取；先看 `summary`，再读声明的主证据块。若主证据块为空或 `result=null`，不要直接作答；仍有查询机会时，用同一 `target` 执行 `--explain`、`args=[]`、`budget=compact` 作为受限 fallback，否则明确说明证据不足。
- 遇到 `diagnostics`、`OUTPUT_TRUNCATED` 或其他不确定性时，final answer 必须保守说明，禁止猜测。

runner 保持 strict parse：malformed JSON、Markdown fence、解释性前缀等都记为 `protocol_error`，不会自动剥离或重试后继续执行。

页面 final 使用固定标签“用户入口：...；按钮：...；写入目标：...；action：...”，不要用同义词替换这些标签。

边界说明：

- `parse_agent_turn` 只接收 **adapter 已成功返回的 assistant content**，对内容执行 strict JSON 协议解析。
- 每轮只输出一行 JSON；`final` 对象只能包含 `kind` 和 `answer` 两个键，`answer` 是简短字符串，不得添加 `sources`、`evidence` 等额外键。
- `protocol_error` 仅表示模型返回内容不符合 `command/final` 协议（如 fenced JSON、非 JSON、解释性前缀）。
- `runner_error` 表示在模型内容到达 runner 之前失败，典型是 `CnbChatAdapter` 的 HTTP 请求/SSE 解析失败、provider 运行时不可达、超时、CLI 执行失败、以及 workspace 构建失败等；这类不属于模型协议层错误。
