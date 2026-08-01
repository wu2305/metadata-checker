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
| no_command | 模型没有执行任何 CLI 命令 |
| command_rejected | 命令未通过 `minimal_command_plan` 精确匹配而被拒绝，trial 立即结束 |
| wrong_command | 命令已执行但没有对应的 plan step |
| ungrounded_answer | 要求证据但没有任何被接受的命令可作为依据 |
| ignored_diagnostic | 遇到诊断未降级，仍做确定性结论 |
| over_read_details | 未先读 summary 而直接读取大量 raw details |

`command_rejected` 是**工具接口指标，不是模型能力指标**：它统计的是模型按 `SKILL.md`
选出的命令与人工最小计划不一致的次数，即 CLI 动词表面对读者是否自解释。这类失败会在
执行 CLI 前终止 trial，因此它与 `missed_fact` 互斥，两者把 trial 分成「路由阶段失败」
和「路由成功但理解失败」两段，可分别归因到命令表面设计和输出可读性。

## 维护

- 新增 eval case：编辑 `ai_eval_cases.json`，同步更新 `tests/ai_eval_tests.rs`。
- 大型真实项目探究 case：优先放入 `tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json`，它用于人工/Agent 评估小模型能否理解真实项目多跳关系，不默认纳入普通 CI。
- 发现模型在某 case 持续失败：先检查 M9-C snapshot 是否已捕获输出变化；再检查 SKILL.md 协议是否足够明确；最后考虑补充 minimal_command_plan 或修改 expected_facts。
- 不要把模型的大段回答提交进仓库，只在 `ai_eval_runs/`（如有）记录结论和分类。

## 真实项目 case 分层（M15）

ai_eval_cases.json 支持两类 case：

### fixture case（默认）
- `project_dir`: `tests/fixtures/test_project`
- `requires_real_project`: false（或省略）
- 测试在隔离临时目录执行，自动复制 fixture 项目并构建 graphdb

### 真实项目 case
- `project_dir`: `/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- `requires_real_project`: true
- `graph_db_path_strategy`: `temp_per_case` 或 `none_single_file`
- `real_project_target`: 真实项目中的目标 ID（如 `page:app/售后.app/首页.spg`）
- `source_path`: 真实项目中的源文件相对路径（如 `app/售后.app/首页.spg`）
- 测试在真实项目目录执行，项目级命令使用独立 graphdb 避免锁冲突；单文件 `.tbl` case 使用 `none_single_file`

真实项目 case 至少覆盖以下场景：
- 页面用途（page_logic）
- 按钮行为（explain + entrypoint）
- 文本统计/仪表盘组件误判（text/dashboard，不把 `data_source_display` 当成主要写入入口）
- 高扇出模型（dataflow + consumed_by_dataflow_count）
- 目标查找（find-page / find-model）
- 拼写错误候选（typo + candidate_targets）
- `.tbl` 单文件理解（PhysicalTable，不当成 SuperPage 或 DataFlow）
- DataFlow .tbl 理解（dataflow + input/output）

### 大型真实项目探究集

`tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json` 是面向 5.4-mini 等小模型的高难度人工评测集。它不追求覆盖所有 CLI schema，而是验证模型能否在真实 xiaoshouyi 项目中完成以下探究任务：

- 区分显示条件、数据可用性和值来源，不把 DataFlow 物理来源误当 visibleCondition。
- 对裸字段值表达式向上寻找最近数据容器的 `dataSet`，再追踪 DataFlow 字段级 `originalNode` / `originalField`。
- 使用 page-scoped model target 解释页面内模型的数据门控，避免混入其他页面同名 model。
- 通过字段别名和物理表规范化找到跨页面 writer，例如 `合同协议.spg|input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber -> 潜客信息跟进.spg`。
- 遇到高扇出模型和截断输出时升级到 `--budget normal`，避免在 compact 截断输出上做全量结论。

## 串行执行要求

AI eval 测试必须串行执行。原因：
- redb 嵌入式 KV 数据库同一时间只能被一个进程打开。
- 多个 eval case 同时访问同一个 `.metadata-checker.graphdb` 会触发 `Database already open` 锁冲突。
- `tests/ai_eval_tests.rs` 使用全局 `CLI_LOCK: Mutex<()>` 保证 CLI 调用串行。

## GraphDB 隔离要求

- eval 测试不依赖已有的 `.metadata-checker.graphdb`（该文件通常被 git ignore）。
- 每次 eval 测试运行时，在临时目录复制一份 `tests/fixtures/test_project`，并在临时目录内独立构建图数据库。
- 临时目录路径：`std::env::temp_dir().join("metadata-checker-ai-eval-test")`。
- 测试结束后自动清理临时目录，避免磁盘膨胀。
- 这种隔离确保 eval 测试和 snapshot/corpus 测试互不干扰，不抢锁。

## 结构化断言执行器

M9-E 引入 `expected_output_assertions`，机器自动判分：

```json
{
  "path": "summary.entrypoint_count",
  "op": "gt",
  "value": 0,
  "description": "页面必须有用户入口"
}
```

支持的 ops：
- `gt` / `gte` / `lt` / `lte` / `eq` / `ne` — 数值/字符串比较
- `not_empty` / `empty` — 数组/对象空值检查
- `contains` — 字符串包含
- `array_any` — 数组中至少一个对象满足 field == value
- `array_contains` — 数组中至少一个元素等于 value（支持字符串数组）
- `contains_field_path` — 对象包含指定字段
- `exists` — 路径存在且非 null
- `manual` — 跳过机器校验，留给人工判分

失败信息格式：
```
case <case_id> 命令 '<cmd>' 断言失败 [<description>]: path '<path>' <原因>
  期望: <expected>
  实际: <actual>
```

## M9-E 覆盖检查

M9-E 至少覆盖以下七类风险：
- PageLogic（页面级逻辑摘要）
- Explain（单对象语义解释）
- Context（邻居上下文）
- DataFlowQuery（DataFlow 链路）
- Diagnostics（诊断降级处理）
- Condition（条件动作执行）
- Lineage（字段来源追溯）

当前 10 个 case 覆盖情况：
| case_id | 覆盖类型 |
|---------|----------|
| page_purpose_actions_test | PageLogic |
| button_submit_effect | Explain |
| field_lineage_model1_name | Explain + Lineage |
| dataflow_output_source | DataFlowQuery + Explain |
| readonly_page_check | Explain + Diagnostics |
| param_passing_link | PageLogic |
| diagnostic_affects_answer | PageLogic + Diagnostics |
| context_button1_neighbors | Context |
| condition_action_behavior | Explain + Condition |
| dataflow_chain_trace | DataFlowQuery + Explain + Lineage |

## command_index 限定

`expected_output_assertions` 支持可选字段 `command_index`，用于限定断言只针对特定命令执行。

当某个 case 的 `required_commands` 包含多个命令且输出结构不同时，可以为每个命令指定独立的断言：

```json
{
  "path": "summary.input_count",
  "op": "gt",
  "value": 0,
  "description": "df_a summary.input_count > 0",
  "command_index": 0
}
```

- `command_index: 0` 表示只针对 `required_commands` 中第 1 个命令执行该断言。
- 省略 `command_index` 时，断言对所有命令都执行。

示例：dataflow_chain_trace 使用两个命令：
- 命令 0 `--query-dataflow df_a` 验证 `summary.input_count > 0`（模型级 DataFlow 链路）
- 命令 1 `--explain field:df_b.id` 验证 `details.lineage` 非空且 `source_fields` 包含 `field:df_a.id`（字段级链式追溯）

## M9-E 断言增强记录

| case_id | 增强前 | 增强后 |
|---------|--------|--------|
| dataflow_output_source | evidence/summary/details 非空 | summary.input_count > 0, details.inputs/outputs 非空 |
| dataflow_chain_trace | evidence/summary/kind 非空 | command_index 区分：query-dataflow 验证 input_count/inputs 非空；explain field 验证 lineage_count > 0, lineage 非空, source_fields 包含 field:df_a.id |

所有 case 的 evidence_requirements 已从 manual 自然语言迁移到结构化断言。

## M9-F：结构化命令计划与回答判分

### 结构化 minimal_command_plan

M9-F 将 `minimal_command_plan` 从字符串数组升级为结构化对象：

```json
{
  "command_kind": "--query-page-logic",
  "target": "page:app/actions_test.spg",
  "args": [],
  "requires_project_dir": true,
  "budget": null
}
```

字段说明：
- `command_kind`：主命令，如 `--query-page-logic`、`--explain`、`--context`、`--query-dataflow`
- `target`：目标 ID，如 `page:app/foo.spg`、`model:model1`、`field:model1.name`
- `args`：额外参数数组，如 `["--depth", "2"]`；预算不能重复放入 args
- `requires_project_dir`：是否必须配合 `--project-dir`
- `budget`：命令顶层预算值，合法值为 `compact` / `normal` / `full`

`required_commands` 由 `minimal_command_plan` 确定性展开得到：
```
--project-dir <project_dir> <command_kind> <target> [<args...>]
```

### answer_assertions 回答判分

```json
{
  "must_include": ["button1", "submitData", "model1"],
  "must_not_include": ["只读", "无动作"],
  "diagnostic_disclaimer_required": false,
  "evidence_reference_required": true
}
```

### risk_tags 风险覆盖

全部 active case 必须覆盖以下标签：
`page_logic`、`explain`、`context`、`dataflow`、`lineage`、`condition`、`diagnostic`

### case_status 与 difficulty

- `case_status`：`active` / `quarantined` / `needs_fixture`
- `difficulty`：`basic` / `intermediate` / `hard`
- `max_command_count`：模型最多允许的 CLI 调用次数（默认 <= 3）
- `allowed_output_sections`：模型允许读取的输出字段（默认包含 `summary`，按需加入 `details`/`evidence`）

### M58 Skill 理解维度

`fixture_llm` case 还可以声明 `evaluation_dimensions`，用于解释模型为何失败以及按能力切片：

```json
{
  "task_family": "dataflow",
  "target_resolution": "explicit",
  "distractor_count": 2,
  "dependency_depth": 3,
  "stateful": false,
  "error_injection": false,
  "output_truncation": true
}
```

- `task_family`：主要能力族，如 `page_logic`、`navigation`、`dataflow`、`condition`、`diagnostic`。
- `target_resolution`：目标是显式给出、需要搜索，还是跨文件解析。
- `distractor_count`：相似命令/关系带来的干扰数量。
- `dependency_depth`：需要跟踪的依赖链深度。
- `stateful`、`error_injection`、`output_truncation`：分别表示状态保持、错误/诊断和输出预算是否是任务难点。

一次对话只适合作为 smoke，不足以证明 Skill 被稳定理解。M58 live runner 默认每个 active `fixture_llm` case 执行 3 个独立 trial，并同时报告 `trial_pass_rate`、`case_stable_pass_rate` 和 `cases_with_flaky_trials`；单 trial 结果只证明该次路径可用。

### answer_style_policy 回答风格

所有 active case 共享同一套回答风格约束：
- 先给结论，再列依据
- 引用 CLI 输出的 summary/details/evidence 作为依据
- 遇到 diagnostics 时保守表达
- 禁止引用源码或历史上下文
- 证据不足时明确说明不确定

## 评测记录

见 [ai-eval-run-template.md](ai-eval-run-template.md)：结构化评测记录格式、失败分类、运行方式。
