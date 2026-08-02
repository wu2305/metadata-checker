# M58 Gemma4 直接 Agent 测试启动协议

你正在参加 `metadata-checker` M58 空上下文小模型评测。

本文件是完整的启动内容。你可以直接使用 terminal 工具执行本文件规定的 CLI 命令，不需要 runner，也不需要返回 `kind=command` 或 `kind=final` JSON。

你只能根据本文件和 terminal 返回的 CLI JSON 回答当前 case。

## 评测目标

在没有源码、历史运行记录、答案键和隐藏知识库的情况下，选择最小且可核验的 CLI 查询，读取机器输出，并给出有证据边界的回答。

本评测关注：

- 是否选对查询类型和目标；
- 是否直接、安全地执行最小 CLI 命令；
- 是否先读取 `summary`，再读取必要的 `details` / `evidence`；
- 是否识别诊断、截断和证据不足；
- 是否避免臆测、过度读取和无依据结论。

## 禁止读取的内容

除本文件和 terminal 返回的 CLI JSON 外，不得读取或使用：

- metadata-checker 源代码；
- 测试代码；
- M58 设计文档、计划和历史运行报告；
- `ai_eval_cases.json` 中的 `expected_facts`、`forbidden_claims`、`answer_assertions` 或其他答案键；
- CNB Knowledge Base；
- 其他隐藏上下文、缓存或模型记忆。

## 固定运行环境

- 项目目录：`/Users/wuhaocheng/Documents/repos/metadata-checker/tests/fixtures/test_project`
- CLI 程序：`/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker`
- graphdb 根目录：`/tmp/m58-gemma4`

如果 CLI 程序不存在，不要读取源码或修改代码；直接报告测试环境缺少 release binary。

## 每个 case 的准备步骤

每个 case 必须使用独立 graphdb，不能复用其他 case 的 graphdb。先创建临时目录，再构建图：

```bash
mkdir -p /tmp/m58-gemma4
"/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker" \
  --project-dir "/Users/wuhaocheng/Documents/repos/metadata-checker/tests/fixtures/test_project" \
  --build-graph \
  --graph-db-path "/tmp/m58-gemma4/<case_id>.graphdb"
```

图构建完成后，再使用同一个 case 的 graphdb 执行查询。图构建是测试准备步骤，不是业务答案。

## 可用查询能力

### 页面逻辑

回答“页面做什么、有哪些入口、会写入什么、有哪些动作”时使用：

```bash
"/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker" \
  --project-dir "/Users/wuhaocheng/Documents/repos/metadata-checker/tests/fixtures/test_project" \
  --query-page-logic 'page:<relative-page-path>' \
  --graph-db-path "/tmp/m58-gemma4/<case_id>.graphdb" \
  --budget compact
```

### 组件解释

回答“组件是什么、点击后发生什么、触发了什么动作”时使用：

```bash
"/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker" \
  --project-dir "/Users/wuhaocheng/Documents/repos/metadata-checker/tests/fixtures/test_project" \
  --explain 'comp:<relative-page-path>|<component-id>' \
  --graph-db-path "/tmp/m58-gemma4/<case_id>.graphdb" \
  --budget compact
```

### 字段写入来源

回答“字段由谁写入、值从哪里来”时使用：

```bash
"/Users/wuhaocheng/Documents/repos/metadata-checker/target/release/metadata-checker" \
  --project-dir "/Users/wuhaocheng/Documents/repos/metadata-checker/tests/fixtures/test_project" \
  --explain 'field:<model>.<field>' \
  --graph-db-path "/tmp/m58-gemma4/<case_id>.graphdb" \
  --budget compact
```

如果 terminal 工具支持 argv 数组，优先使用 argv 数组；如果只能执行 shell，必须给含有 `|`、中文、空格或括号的 target 加单引号。

## 目标定位规则

- 页面目标使用 `page:<相对页面路径>`；
- 组件目标使用 `comp:<页面路径>|<组件 ID>`；
- 字段目标使用 `field:<模型>.<字段>`；
- 不要猜测 model、page 或 component ID；
- 不要把裸字段名当成表名；
- 不要把页面包含关系当成用户触发逻辑；
- 不要把值来源、写入来源和显示条件混为一谈。

## CLI 输出阅读规则

收到 CLI JSON 后，严格按照以下顺序阅读：

1. 先看顶层 `summary`；
2. 再看与问题直接相关的 `details` 主证据块；
3. 需要核验时再看 `evidence`；
4. 检查 `diagnostics`、`next_queries` 和截断提示；
5. 最终回答只使用实际出现的事实。

特别注意：

- compact 输出中的数组可能只是 Top-N，不代表完整全集；
- `read_by_count=0` 不能单独证明模型未被使用；
- `details.upstream` 为空不能单独证明没有上游依赖；
- `related_context` 和 `supporting_context` 默认不是必要证据；
- 看到 `OUTPUT_TRUNCATED` 或 `safe_to_answer_full_relationships=false` 时，只有在确有必要时才升级到 `normal`；
- 不要默认请求 `full`、`raw` 或完整 metadata；
- 看到诊断时必须在最终回答中保守说明，不得假装诊断不存在；
- 目标不存在时依据 `TARGET_NOT_FOUND` 和候选目标回答，不要自动替用户猜测。

## Terminal 执行约束

- 每个 case 最多执行两个业务查询命令；图构建不计入业务查询次数；
- 初始使用 `--budget compact`；
- 只有输出明确不足时才升级到 `--budget normal`；
- 不要使用 `--budget full`、raw 输出或无关查询；
- 只执行本文件规定的 metadata-checker 命令；
- 不要执行 `rm`、网络请求、凭证读取、源码搜索或任意文件写入；
- 不要把用户问题直接拼接成 shell 代码；
- 不要把 token、密码、cookie、Authorization header 或其他凭证放进命令或回答；
- 如果一次查询已经足够回答问题，应立即停止查询并回答。

## 最终回答规则

terminal 是工具调用，不需要把命令包装成 JSON，也不要输出 `kind=command` / `kind=final`。

最终回答使用自然语言，必须：

- 先给出一句话结论；
- 说明关键事实及对应的 CLI 字段或 evidence；
- 明确诊断、截断或不确定项；
- 不复制大段 CLI 原文；
- 不声称读取过源码、历史资料或隐藏上下文；
- 不编造 CLI 输出中不存在的关系。

## 当前测试 cases

以下三个 case 必须分别使用全新的 Agent 会话，使用同一份本文件。前一个 case 的 terminal 输出、回答和推理不得带入下一个 case。

### Case 1

- case_id：`page_purpose_actions_test`
- question：`页面 actions_test 主要做什么？用户能触发哪些逻辑？会影响哪些数据？`

### Case 2

- case_id：`button_submit_effect`
- question：`点击 actions_test 页面中的 button1 会发生什么？`

### Case 3

- case_id：`field_lineage_model1_name`
- question：`字段 model1.name 的值从哪里来？`

每个新会话只处理一个 case：先完成该 case 的图构建，再根据问题选择最小查询，读取 CLI JSON，最后给出自然语言回答。
