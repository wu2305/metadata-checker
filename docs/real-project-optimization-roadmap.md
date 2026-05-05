# 真实项目 AI 使用效果优化里程碑

本文档记录在真实项目 `/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi` 上，用空上下文 `5.4-mini` 和主线程交叉验证后暴露的问题。目标是让任意 AI 只依赖 `SKILL.md` 与 `metadata-checker` 二进制，也能低噪声、可验证地理解 `.spg` 与 `.tbl` 元数据逻辑。

## M10：表与 DataFlow 单文件输出可理解

### 目标

让 AI 在没有项目图数据库时，也能从单个 `.tbl` 文件获得正确的表/DataFlow 语义，而不是被误导为 SuperPage。

### 问题清单

- 单文件解析真实 `.tbl` 时输出 `kind=SuperPage`，`component_count=0`，证据为 `Legacy page metadata parsed`，语义错误。
- `.tbl --detail` 只输出空 `components/data_bindings/settings`，无法支撑 AI 理解物理表字段、DataFlow 输入、DataFlow 输出或维度加工。
- 默认 `SuperPage` schema 与 `.tbl` 实际语义混用，会让模型以为表也是页面。
- 没有单文件 DataFlow 摘要，AI 无法在 graphdb 缺失时回答“这个表从哪里来”“输出到哪里”“字段怎么加工”。
- 缺少 `.tbl` 单文件 evidence，无法回到 `dimensions`、`inputField`、`exp`、`target/output` 等原始 JSON 路径。
- 缺少 `.tbl` 单文件诊断，解析不到输出节点时没有区分“确实无输出”和“当前解析能力不足”。

### 验收目标

- 真实 `.tbl` 单文件输出 `kind=Table | DataFlow`，不再输出 `SuperPage`。
- `summary` 至少包含 `table_id/name`、`table_type`、`field_count`、`input_count`、`output_count`、`what_is_it`。
- `details` 至少包含 `fields`、`dataflow_inputs`、`dataflow_outputs`、`field_lineage`。
- `evidence` 能定位到真实 `source_file` 与 JSON 路径。
- 对无输出、无法识别输出、表达式无法解析分别产生明确 diagnostics。
- 以 `xiaoshouyi/data/tables/销售/fact_saleContract.tbl` 和至少 2 个真实 DataFlow `.tbl` 建立回归样例。

## M11：图数据库路径、只读与并发可用性

### 目标

让项目级查询在真实只读项目、沙箱、多 Agent 并行场景下可稳定使用。

### 问题清单

- 项目级查询依赖项目根目录 `.metadata-checker.graphdb`，不存在时直接失败。
- `--build-graph` 默认写真实项目根目录，沙箱或只读目录下会失败。
- 图数据库创建成功后，普通查询仍可能因沙箱权限无法读取。
- 多个项目级查询并发执行时会出现 `Database already open. Cannot acquire lock`。
- `SKILL.md` 没有把 graphdb 缺失、权限失败、锁冲突定义为明确 fallback 流程。
- 没有临时图数据库路径，空模型只能在真实项目中写入副产物。
- 没有“只读检查 graphdb 是否可用”的轻量命令，AI 需要尝试真实查询才知道是否阻塞。

### 验收目标

- CLI 增加 `--graph-db-path <PATH>`，支持将 graphdb 放在 `/tmp` 或工作区内。
- CLI 增加只读检查能力，例如 `--check-graph`，输出图数据库是否存在、可读、可写、是否需要 rebuild。
- 项目级查询在只读图数据库上支持并发读，或至少返回明确可恢复诊断与重试建议。
- `SKILL.md` 明确 graphdb 缺失时的 fallback：先检查、再构建到可写路径、最后退回单文件模式。
- 错误输出进入统一 JSON diagnostic，而不是只输出裸 `Error:`。
- 真实项目上可连续运行页面逻辑、模型查询、组件 explain，不因锁冲突失败。

## M12：Explain 语义摘要与证据质量收敛

### 目标

让 `--explain` 对组件、动作、模型、字段输出真正可回答业务问题的语义摘要，并避免伪证据误导模型。

### 问题清单

- 单文件 `--explain button1` 仍可能输出 `button component button1`，业务语义不足。
- 项目级 `--explain` 的 `type_detail` 可能出现组件 ID（如 `button1`），而不是真实组件类型。
- 文本统计类组件可能被误判为 `entrypoint`，空模型已指出 `text11` 这类组件容易把 AI 带向错误方向。
- evidence 中存在 `node_id="?"`、`json_path="<graph-edge-derived>"`、`raw_expr="n/a"` 或 `raw_expr="?"`，看似证据但无法核验。
- `LINEAGE_SOURCE_MISSING` 对没有字段血缘需求的按钮/普通组件也频繁出现，形成噪声。
- `triggered_by` 与 `affects` 的关系命名对 AI 不够直观，容易把“页面包含组件”理解成业务触发。
- Action 的 `semantic_summary` 在 `PageLogic` 中较好，但 `Explain` 的 component/action 摘要没有完全复用。

### 验收目标

- `--explain comp:...|button1` 输出 `semantic_summary`，明确“点击后打开哪个对话框/跳转/写入/刷新”。
- `--explain` 的 `type_detail` 使用真实类型，不使用 ID。
- `importance` 区分 `entrypoint`、`calculated_display`、`container`、`static_display`，文本统计不能默认算入口。
- evidence 若无法定位 JSON 路径，必须降级为 `confidence=low` 或移动到 `diagnostics`。
- 删除或替换 `node_id="?"`，不能定位时输出明确诊断 `EVIDENCE_LOCATION_MISSING`。
- 仅在用户询问字段来源或目标确实有字段 lineage 时输出 `LINEAGE_SOURCE_MISSING`。
- 增加真实项目回归：`首页.spg` 的 `button1`、`text11`、`button2`，以及一个表单输入组件。

## M13：面向 AI 的低噪声 brief 输出模式

### 目标

降低大页面、大模型查询输出体积，让小模型优先抓住结论、风险和下一步，而不是陷入 details/evidence 大数组。

### 问题清单

- `query-page-logic` 在真实页面上同时展开 `action_flows`、`visibility_rules`、`data_sources`、`evidence`，输出过长。
- `EVIDENCE_SAMPLED` 提示存在，但 AI 仍需在 details 与 evidence 之间自行判断覆盖范围。
- `query-model fact_saleContract` 输出 52 个 `consumed_by_dataflows`，summary 只说读写为 0，模型容易忽略 DataFlow 消费方。
- `details` 中同类长列表缺少分组、排序依据、Top-N 摘要与剩余计数。
- `--budget compact|normal|full` 已存在，但不是所有查询都能体现清晰预算语义。
- 默认 `SuperPage` 输出没有 `page_role/what_is_it`，AI 要从数据源和表达式中自行归纳页面用途。
- `--priority` 在真实页面上对 AI 的增量信息不明显，容易让模型白跑命令。

### 验收目标

- CLI 增加或强化 `--budget compact`/`--ai-brief`，输出只包含 `summary`、`key_findings`、`risk_diagnostics`、`evidence_summary`、`next_queries`。
- 长列表输出必须包含 `total_count`、`shown_count`、`truncated`、`remaining_count`。
- `query-model` summary 增加 DataFlow 角色计数，如 `consumed_by_dataflow_count`、`produced_by_count`。
- `query-page-logic` summary 增加 `top_entrypoints`、`top_writes`、`top_navigation` 的低噪声摘要。
- 默认单页输出增加 `what_is_it` 与 `page_role`，不要求项目图也能给出保守页面意图。
- `--priority` 若无有效优先级发现，应输出明确 `NO_PRIORITY_RULES` 或 `priority_rule_count=0`。

## M14：目标定位与命令规范防错

### 目标

减少 AI 在真实项目中找不到目标、写错 target、shell 命令被管道拆开的概率。

### 问题清单

- `next_queries` 中 `comp:...|button1` 未加引号，shell 会把 `|` 当管道。
- `SKILL.md` 虽有 ID 格式说明，但没有强制规定所有含 `|`、空格、中文路径的 target 必须加单引号。
- 真实项目存在大量同名或局部名模型，如 `model1/model5/model74`，全项目查询容易命中错误节点。
- 缺少 `find`/`search` 类命令，AI 要先依赖 `rg --files` 或猜路径。
- 缺少“从页面内局部模型 ID 解析到真实表/DataFlow”的命令。
- 查询错误时缺少候选目标建议，例如“你可能想查 page:... 或 comp:...|...”。
- `--query-model` 接收裸模型名，但 `--explain` 接收 `model:` 前缀，规范差异容易让模型混用。

### 验收目标

- 所有 `next_queries` 对含 `|`、空格、中文路径的 target 自动加单引号。
- `SKILL.md` 增加命令引用硬规则：target 一律单引号包裹。
- CLI 增加 `--find-page <KEYWORD>`、`--find-model <KEYWORD>`、`--find-component <KEYWORD>`。
- CLI 增加页面作用域解析能力，例如 `--resolve-model page:... model5`。
- 查询目标不存在时，返回候选列表与推荐下一条命令。
- 统一说明 `--query-model` 与 `--explain model:` 的参数差异，或在 CLI 内兼容两种写法。

## M15：真实项目 AI 评测集扩展

### 目标

把真实项目中已暴露的误判模式纳入自动评测，确保后续优化不是只在人工检查中成立。

### 问题清单

- 当前 AI eval 主要覆盖 fixtures，真实项目问题没有形成长期回归。
- 缺少空上下文小模型对真实项目的固定任务验证。
- 缺少文本统计/仪表盘组件误判为入口的测试。
- 缺少 `.tbl` 单文件真实输出测试。
- 缺少 graphdb 不存在、只读、锁冲突的行为测试。
- 缺少 `next_queries` shell 安全测试，无法防止未加引号的 `|` 回归。
- 缺少针对 `fact_saleContract` 这类高扇出模型的摘要质量测试。

### 验收目标

- 在不复制整个真实项目的前提下，建立真实项目选择清单，引用 `xiaoshouyi` 中固定文件和目标。
- 每个真实项目评测 case 包含问题、命令计划、期望摘要、禁止误判、证据要求。
- 空上下文 `5.4-mini` eval 至少覆盖：页面用途、按钮行为、文本统计、模型被 DataFlow 消费、`.tbl` 单文件理解。
- 自动测试覆盖 graphdb 缺失和锁冲突的 JSON diagnostic。
- 自动测试覆盖 `next_queries` 中 target 引号。
- 高扇出模型输出必须验证 `consumed_by_dataflow_count` 出现在 summary。

## M16：SKILL.md 真实使用协议收敛

### 目标

把工具能力边界、fallback 路径和保守回答规范写成模型不会误解的任务协议。

### 问题清单

- `SKILL.md` 对 graphdb 缺失、权限失败、锁失败没有明确决策树。
- `SKILL.md` 没有把 `<graph-edge-derived>`、`node_id="?"` 定义为弱证据。
- `SKILL.md` 没有强调 `details` 长数组默认不可全读。
- `SKILL.md` 没有针对单文件 `.tbl`、项目级 `.tbl`、页面内局部 DataFlow 的不同路径给出清晰命令选择。
- `SKILL.md` 没有明确“summary 与 details 冲突时如何回答”，例如模型读写 0 但 DataFlow 消费 52。
- `SKILL.md` 没有要求 AI 在目标 ID 不确定时先使用 find/resolve，而不是猜测。
- `SKILL.md` 对 `--human` 与机器模式边界已经有说明，但没有真实项目故障处理示例。

### 验收目标

- `SKILL.md` 增加“真实项目项目级查询决策树”：check graph、build graph、query、fallback。
- `SKILL.md` 增加“证据强弱分级”：真实 JSON 路径强，graph-derived 中，缺路径弱。
- `SKILL.md` 增加“长输出读取预算”：先 brief，再按需 details，不默认 full。
- `SKILL.md` 增加 `.tbl` 和 DataFlow 问题的命令路径。
- `SKILL.md` 增加冲突处理规则：summary 不完整时结合 details 中角色字段，但必须说明口径。
- `SKILL.md` 增加命令引用规则：所有 target 使用单引号。
- 空上下文 `5.4-mini` 能按协议完成至少 5 个真实项目问题，不出现已知误判。

