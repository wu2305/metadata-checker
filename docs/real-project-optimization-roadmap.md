# 真实项目 AI 使用效果优化里程碑

本文档记录在真实项目 `/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi` 上，用空上下文 `5.4-mini` 和主线程交叉验证后暴露的问题。目标是让任意 AI 只依赖 `SKILL.md` 与 `metadata-checker` 二进制，也能低噪声、可验证地理解 `.spg` 与 `.tbl` 元数据逻辑。

## M10：表与 DataFlow 单文件输出可理解

状态：已收敛。剩余非阻塞观察项已归并到 M13 和 M14。

### 目标

让 AI 在没有项目图数据库时，也能从单个 `.tbl` 文件获得正确的表/DataFlow 语义，而不是被误导为 SuperPage。

### 问题清单

- 单文件解析真实 `.tbl` 时输出 `kind=SuperPage`，`component_count=0`，证据为 `Legacy page metadata parsed`，语义错误。
- `.tbl --detail` 只输出空 `components/data_bindings/settings`，无法支撑 AI 理解物理表字段、DataFlow 输入、DataFlow 输出或维度加工。
- 默认 `SuperPage` schema 与 `.tbl` 实际语义混用，会让模型以为表也是页面。
- 没有单文件 DataFlow 摘要，AI 无法在 graphdb 缺失时回答“这个表从哪里来”“输出到哪里”“字段怎么加工”。
- 缺少 `.tbl` 单文件 evidence，无法回到 `dimensions`、`inputField`、`exp`、`target/output` 等原始 JSON 路径。
- 缺少 `.tbl` 单文件诊断，解析不到输出节点时没有区分“确实无输出”和“当前解析能力不足”。

### 验收目标（M15 已收敛）

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

### 验收目标（M15 已收敛）

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

### 验收目标（M15 已收敛）

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
- M10 遗留观察：单文件 `.tbl` / DataFlow 的 `details.fields`、`details.field_lineage`、`evidence` 在真实 DataFlow 上仍可能很长，低噪声预算不能只覆盖项目级查询，也要覆盖单文件 Table/DataFlow 输出。
- `--budget compact|normal|full` 已存在，但不是所有查询都能体现清晰预算语义。
- 默认 `SuperPage` 输出没有 `page_role/what_is_it`，AI 要从数据源和表达式中自行归纳页面用途。
- `--priority` 在真实页面上对 AI 的增量信息不明显，容易让模型白跑命令。

### 验收目标（M15 已收敛）

- CLI 增加或强化 `--budget compact`/`--ai-brief`，输出只包含 `summary`、`key_findings`、`risk_diagnostics`、`evidence_summary`、`next_queries`。
- 长列表输出必须包含 `total_count`、`shown_count`、`truncated`、`remaining_count`。
- 单文件 `.tbl` / DataFlow 输出必须支持低噪声摘要，默认避免让 AI 直接读取完整字段和 evidence 大数组。
- `query-model` summary 增加 DataFlow 角色计数，如 `consumed_by_dataflow_count`、`produced_by_count`。
- `query-page-logic` summary 增加 `top_entrypoints`、`top_writes`、`top_navigation` 的低噪声摘要。
- 默认单页输出增加 `what_is_it` 与 `page_role`，不要求项目图也能给出保守页面意图。
- `--priority` 若无有效优先级发现，应输出明确 `NO_PRIORITY_RULES` 或 `priority_rule_count=0`。

## M14：目标定位与命令规范防错

### 目标

减少 AI 在真实项目中找不到目标、写错 target、shell 命令被管道拆开的概率。

### 问题清单

- `next_queries` 中 `comp:...|button1` 未加引号，shell 会把 `|` 当管道。
- M10 遗留观察：单文件 `.tbl` 输出的 `next_queries` 也需要纳入统一命令规范，不能只修页面/组件 target。
- `SKILL.md` 虽有 ID 格式说明，但没有强制规定所有含 `|`、空格、中文路径的 target 必须加单引号。
- 真实项目存在大量同名或局部名模型，如 `model1/model5/model74`，全项目查询容易命中错误节点。
- 缺少 `find`/`search` 类命令，AI 要先依赖 `rg --files` 或猜路径。
- 缺少“从页面内局部模型 ID 解析到真实表/DataFlow”的命令。
- 查询错误时缺少候选目标建议，例如“你可能想查 page:... 或 comp:...|...”。
- `--query-model` 接收裸模型名，但 `--explain` 接收 `model:` 前缀，规范差异容易让模型混用。

### 验收目标（M15 已收敛）

- 所有 `next_queries` 对含 `|`、空格、中文路径的 target 自动加单引号。
- 所有 `next_queries` 都应输出可直接复制执行的命令片段；无法确定 `<DIR>` 或 `<MODEL>` 时必须明确标注占位符来源和解析方式。
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

### 验收目标（M15 已收敛）

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

### 验收目标（M16 已收敛）

- `SKILL.md` 增加“真实项目项目级查询决策树”：check graph、build graph、query、fallback。
- `SKILL.md` 增加“证据强弱分级”：真实 JSON 路径强，graph-derived 中，缺路径弱。
- `SKILL.md` 增加“长输出读取预算”：先 brief，再按需 details，不默认 full。
- `SKILL.md` 增加 `.tbl` 和 DataFlow 问题的命令路径。
- `SKILL.md` 增加冲突处理规则：summary 不完整时结合 details 中角色字段，但必须说明口径。
- `SKILL.md` 增加命令引用规则：所有 target 使用单引号。
- 空上下文 `5.4-mini` 能按协议完成至少 5 个真实项目问题，不出现已知误判。

## M17：条件抽取基础层

### 目标

从 `.spg` 中稳定抽取条件表达式，形成可复用的条件记录，而不是让 AI 直接阅读大体积 raw metadata。

### 问题清单

- `5.4-mini` 分析 `合同协议.spg` 的“正常显示数据条件”时，需要直接读取 5MB `.spg` 才能定位 `visibleCondition`、`disableCondition` 和 `filter.clauses[]`。
- `--query-page-logic` 能给出页面规模和 Top-N 入口，但不能完整表达页面显示/禁用/数据过滤条件。
- 条件表达式分散在组件、动作、数据源、默认值中，缺少统一结构。
- 多行表达式、`IF`、`CASE`、`CONCAT`、系统变量和中文路径容易导致抽取遗漏。

### 工作清单

- 抽取 `visibleCondition`、`disableCondition`、`conditionExp`、`filter.clauses[].exp`、`defaultValueExp`、`exp`。
- 为每条条件记录 `source_file`、`json_path`、`component_id/model_id/action_id`、`condition_type`。
- 解析条件中引用的 `param*`、`$user.*`、`model*.field`、`input*.value`、`text*.value`、`model*.totalRowCount__`。
- 定义统一结构 `ConditionRecord`，并写入 `docs/schema.md`。
- 增加 fixture 覆盖空条件、多行条件、`IF/CASE/CONCAT`、中文路径、系统变量。

### 验收目标

- 给定 `.spg`，工具能列出所有条件表达式及引用对象。
- 每条条件都有可核验位置，不允许缺少 `source_file` 和 `json_path`。
- 本阶段只做抽取和定位，不输出业务解释。

## M18：条件依赖图

### 目标

把条件表达式与上游数据来源、下游组件状态串起来，形成可查询的条件依赖路径。

### 问题清单

- 当前工具无法直接表达 `param5 -> model10.filter -> model10.totalRowCount__ -> panel13.visibleCondition` 这类链路。
- 隐藏计算字段如 `input3.exp = model22.phoneNumber` 会成为条件中间层，但 PageLogic 摘要不会显式串联。
- 模型过滤条件与 `model*.totalRowCount__` 的显示门控之间缺少结构化关系。
- 页面局部模型写入没有归并到物理表字段：例如 `潜客信息跟进.spg` 的 `model6.phoneNumber` 实际写入 `$DATA:/主数据/fact_qwSidebar.tbl` 的 `phoneNumber`，但 `query-model model:fact_qwSidebar` 无法发现该跨页面 writer，导致 `合同协议.spg` 中 `input3 = model22.phoneNumber` 的生成链断在物理表读取处。

### 工作清单

- 建立 `param/input/text/model/system_user` 到 `condition` 的依赖边。
- 将隐藏计算字段纳入依赖链，例如 `input3.exp`、`text41.value`、`input8.exp`。
- 将 `model.filter` 与 `model.totalRowCount__` 建立关系。
- 将 `visibleCondition/disableCondition` 与组件显示/禁用状态建立关系。
- 输出条件依赖路径，保留每段 path 的 evidence。
- 建立 `.spg` 页面内 `sources[]` 局部模型到真实 `$DATA:/.../*.tbl` 物理表的别名映射，覆盖大小写、相对路径和中文目录。
- 在 `submitData/updateData/insertData/deleteData` 等数据写入 action 中，同时保留页面局部模型写入边，并补充归并后的物理表字段写入边。
- 物理表字段写入边必须携带原始 action 证据，包括 `source_file`、`json_path`、`component_id`、`action_id`、局部 `dataSet` 和归并后的物理表字段。
- `query-model <physical_table>`、`--explain action:...`、`--query-page-logic <page>` 需要能从不同入口看到同一条写入事实，且不能把 `<graph-edge-derived>` 当作唯一强证据。
- 增加最小 fixture：一个页面通过局部 `model6` 写入 `$DATA:/主数据/fact_qwSidebar.tbl.phoneNumber`，另一个页面通过局部 `model22` 读取同一物理字段，验证跨页面 writer 可被物理表查询发现。

### 验收目标

- 能回答“某个组件显示依赖哪些参数/模型/系统变量”。
- 能回答“某个参数影响哪些模型和组件”。
- 条件链中每个节点都能追溯到 `ConditionRecord` 或原始组件/模型定义。
- 给定物理表 `model:fact_qwSidebar`，`query-model` 能列出来自 `潜客信息跟进.spg` 的 `button1.action1/action4` 对 `phoneNumber` 的写入。
- 给定 `合同协议.spg` 的 `input3`，工具能串联 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.model6.phoneNumber`，并标明读取边与写入边各自证据位置。

## M19：页面数据可用性摘要

### 目标

从条件依赖图生成低噪声页面级摘要，让 AI 不读 raw `.spg` 也能理解页面数据正常显示的主要前置条件。

### 问题清单

- `5.4-mini` 需要手动归纳 `param5 + text41 + input3 + $user.dept_id + model*.totalRowCount__`。
- 当前 PageLogic 的 `data_source_count`、`entrypoint_count`、`write_target_count` 不能直接说明“页面为什么没数据”。
- `visibleCondition`、`disableCondition` 和数据源过滤条件没有按影响范围排序。
- M18 已能建出条件图和物理表写入归并，但 `--context comp:...|input3 --depth 2` 会访问数百个邻近节点，混入同名局部模型、旁路 DataFlow、其他页面组件等低相关关系，容易让小模型从主链路漂移。

### 工作清单

- 聚合页面级关键输入：入口参数、系统变量、隐藏计算字段、用户输入字段。
- 聚合主要数据源过滤条件。
- 聚合主要显示门控：`visibleCondition`。
- 聚合主要交互门控：`disableCondition` 和 enabled condition。
- 按影响范围排序：影响模型数量、影响组件数量、是否影响入口/面板/按钮。
- 输出 `summary.display_prerequisites`、`summary.data_prerequisites`、`summary.action_prerequisites`。
- 从 M18 的条件图中抽取 `primary_paths` / `key_paths`，优先展示与目标页面、目标组件、目标物理表直接相关的链路，例如 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- 对跨页面或同名局部模型扩散出的旁路关系降级到 `related_context`，并在 summary 中只给计数和升级建议，不默认展开。

### 验收目标

- AI 只读 summary 就能说出页面数据正常显示的主要条件。
- compact 模式下输出 Top-N 条件，normal/full 模式可展开完整条件列表。
- 对 `OUTPUT_TRUNCATED` 场景给出明确升级建议。
- 小模型默认读取 summary 时能优先复述主链路，不会把同名局部模型的其他页面引用、DataFlow 旁路或低相关组件误认为页面正常显示的必要条件。

## M19-FIX：可扩展主链路计算框架

**状态：主体实现已收敛，真实项目回归测试待验证。**

### 背景

M19 已接入页面级 prerequisites、`primary_paths`、`key_primary_paths` 与 `related_context`，但当前 `primary_paths` 仍偏向“高置信图边列表”，不是围绕页面数据可用性目标的因果路径。典型症状：

- `key_primary_paths` 容易被 `$user.id`、`model19.brand`、普通模型读取边占据。
- `合同协议.spg` 的关键链路 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4` 没有稳定进入 summary Top-N。
- 单一打分/排序容易掩盖问题，需要预留后续 PRA、Personalized PageRank、Steiner Tree 或学习型 selector 接口。

### 目标

把 `primary_paths` 从“边集合”重构为“候选路径 + 分类 + 可替换选择器”的主链路计算框架。当前阶段不把分数写死为唯一依据，而是输出可解释的路径候选、分类理由、选择理由和诊断信息。

### 任务清单

- 定义路径领域模型：`PathNodeRef`、`PathEdgeRef`、`PathSegment`、`PathCandidate`、`PathTerminal`、`PathQuery`、`PathSelectionResult`。
- 每条 `PathCandidate` 至少包含 `path_id`、`purpose`、`terminals`、`segments`、`evidence`、`selection_reason`、`rank_features`、`classification`、`confidence`、`diagnostics`。
- 区分路径分类：`primary_path`、`candidate_path`、`supporting_path`、`related_context`、`rejected_path`。
- 拆分路径发现与路径选择接口：
  - `PathFinder::find_candidates(&PathQuery) -> Vec<PathCandidate>`
  - `PathSelector::select(&PathQuery, Vec<PathCandidate>) -> PathSelectionResult`
- 先实现 `BoundedCausalPathFinder`，限制深度、边类型和页面作用域，避免全图扩散。
- 先实现 `RuleBasedPathSelector`，但预留 `WeightedPathSelector`、`NoScorePathSelector`、`DebugAllPathSelector`、`FutureLearningPathSelector`。
- 实现 `AnchorExtractor`，从页面条件、隐藏计算字段、物理表读取字段、跨页面 writer 中提取 `target_anchors`、`source_anchors`、`sink_anchors`、`bridge_anchors`、`excluded_anchors`。
- 将字段级物理表归并纳入路径计算，候选路径必须能表达页面局部模型字段、物理表字段、跨页写入 action 和原始 `json_path`。
- 禁止直接把所有 `Reads` / `ActionWrites` 塞进 `primary_paths`；候选路径必须至少连接两个 anchor，或连接 condition、组件、模型过滤、`totalRowCount__`、物理表 writer 等因果对象。
- 输出 `rank_features` 作为特征，不作为唯一真伪判断；至少包含 `contains_target_component`、`contains_physical_field`、`contains_cross_page_writer`、`field_name_match`、`same_page`、`has_condition_node`、`has_json_path`、`edge_type_sequence`、`path_length`、`source_file_count`、`contains_only_structural_edges`、`contains_dataflow_side_branch`。
- `key_primary_paths` 采用“分组保底”而不是简单 Top-K：
  - 至少保留 1 条 value source path。
  - 至少保留 1 条 data prerequisite path。
  - 至少保留 1 条 display gate path。
  - 至少保留 1 条 action/write source path。
  - 如果存在跨页 writer，必须保留至少 1 条 writer path。
  - 如果存在目标组件，例如 `input3`，必须优先保留包含该组件的路径。
- `related_context` 只承载相关但非必要的旁路关系；同名局部模型的其他页面引用、DataFlow 旁路、结构边和字段不匹配跨页边不得进入 `key_primary_paths`。
- 修复 `related_context_summary` 统计时机，保证 `summary.related_context_count == details.related_context_summary.total_count`。
- 支持调试输出层级：
  - compact：只输出 `key_primary_paths`、Top-N prerequisites 和计数。
  - normal：输出 `primary_paths`、`supporting_paths`、`related_context_summary`。
  - full：输出 `candidate_paths`、`rejected_paths`、`path_selection_diagnostics`。
- 更新 `docs/schema.md`，定义 `PathCandidate`、`PathSegment`、`PathTerminal`、`rank_features`、`classification`、`selection_reason` 和 `PathSelectionResult`。
- 更新 `SKILL.md`，要求条件/页面数据可用性问题优先读取 `summary.key_primary_paths`，只有诊断提示时再看 `candidate_paths` 或 `related_context`。

### 测试清单

- 增加 fixture：简单读链 `component -> local model -> physical model`。
- 增加 fixture：跨页 writer `other page action -> same physical field`。
- 增加 fixture：同名局部模型噪声，其他页面 `model22` 不同物理表或不同字段。
- 增加 fixture：DataFlow 旁路相关但不应进入 primary。
- 增加 fixture：`condition -> component -> model.filter -> totalRowCount__`。
- 增加真实项目回归：`合同协议.spg` 的 `key_primary_paths` 必须包含 `input3`、`model22.phoneNumber`、`fact_qwSidebar.phoneNumber`、`潜客信息跟进.spg|button1|action1` 或 `action4`。
- 增加负例断言：`key_primary_paths[0..5]` 不得全部被 `$user.id`、`model19.brand` 等普通读边占据。
- 增加负例断言：外页同名局部模型不得进入 `primary_path`，除非它写入同一物理字段。
- 增加一致性断言：`related_context_count` 与 `related_context_summary.total_count` 必须一致。
- 增加输出稳定性断言：compact 不展开大数组，normal 输出候选分类，full 输出 rejected paths。
- 全量验证必须通过 `cargo test`，且不得新增 Rust warning。

### 验收目标

- `--query-page-logic 'page:app/销售.app/销售/合同协议.spg' --budget compact` 的 `summary.key_primary_paths` 能优先呈现 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- 小模型只读 summary 能复述主链路，并能区分主链路、补充路径和相关但非必要的旁路关系。
- 路径选择策略可替换，不依赖写死的单一分数；后续可以接入 PRA、Personalized PageRank、Steiner Tree 或学习型 selector。
- `related_context` 不再污染 `primary_paths`，且其统计、诊断和升级建议一致。

## M20：Why 条件查询能力

### 目标

支持面向目标对象的条件解释，回答“为什么不显示、为什么按钮灰、为什么数据为空”。

### 问题清单

- 当前只能通过 PageLogic + raw `.spg` 人工推导某个面板或按钮的门控条件。
- `model*.totalRowCount__` 作为数据命中行数门控没有专门解释。
- target 不存在或写错时，条件查询需要沿用 M14 的候选建议和 shell-safe 命令规范。
- 通用 `--context` 适合探索，但不适合作为“为什么不显示 / 为什么没数据”的默认回答入口；它会把主因、旁路依赖和低相关邻居混在一起，增加注意力漂移风险。

### 工作清单

- 新增条件解释命令，命名可采用 `--explain-condition <TARGET>` 或拆分为 `--why-visible`、`--why-disabled`、`--why-data-empty`。
- 输出目标条件链、上游依赖、失败原因候选。
- 对 `model*.totalRowCount__` 特殊处理，解释为“数据集命中行数门控”。
- 支持 page、component、model、field 四类 target。
- target 不存在时返回候选目标和下一条查询命令。
- 输出目标化 `primary_reason` / `primary_path`，把与目标无直接因果关系的邻居放入低优先级 `related_context`，并标明其不是必要条件。

### 验收目标

- 能解释“这个面板为什么不显示”。
- 能解释“这个按钮为什么灰”。
- 能解释“这个数据源为什么可能为空”。
- 输出必须区分确定条件和推断条件。
- 对 `合同协议.spg|input3` 这类目标查询，默认输出聚焦主因链路；不需要 AI 再从数百个 context 节点中自行筛选。

## M21：未知 Action 语义归类

### 目标

减少 `UNKNOWN_ACTION_TYPE` 对复杂页面理解的干扰，让 AI 能识别附件查看、接口发送、消息提示和脚本执行的基础语义。

### 问题清单

- `合同协议.spg` 中大量 `script`、`webAPI`、`showMessage`、`showFilesGallary` 被标记为 `UNKNOWN_ACTION_TYPE`。
- 未知 action 会淹没 `key_findings`，导致 AI 只能保守地说“工具无法理解部分动作”。
- 附件查看、接口发送、消息提示是低代码页面的常见关键行为。

### 工作清单

- 为 `script` 增加 `script_execution` 语义分类。
- 为 `webAPI` 增加 `api_call` 语义分类，并保留接口参数证据。
- 为 `showMessage` 增加 `message_prompt` 语义分类。
- 为 `showFilesGallary` 增加 `file_gallery` 语义分类，识别附件数据集和附件字段。
- 将这些 action 纳入 `action_category` 和 PageLogic action flow。
- 保留无法解析参数的 diagnostic，但不再输出高噪声 `UNKNOWN_ACTION_TYPE`。

### 验收目标

- 复杂页面摘要不再被同类 `UNKNOWN_ACTION_TYPE` 重复刷屏。
- AI 能识别附件查看、接口发送、消息提示、脚本执行。
- 未解析脚本内容时仍保持保守，不编造脚本内部语义。

## M22：条件类 AI 评测与协议收敛

### 目标

把页面条件分析能力固定成长期回归，验证空上下文小模型不读 raw `.spg` 也能回答条件类问题。

### 问题清单

- 当前真实项目评测主要覆盖页面用途、按钮行为、DataFlow、目标定位和 `.tbl` 单文件。
- “页面为什么没数据 / 按钮为什么灰 / 面板为什么不显示”尚未形成固定评测。
- `SKILL.md` 没有明确条件类问题的命令路线。
- M18 残留的注意力漂移风险尚未进入评测：模型可能把 `--context` 中的同名局部模型、旁路 DataFlow、其他页面组件当成主链路证据。

### 工作清单

- 增加真实项目 eval case：`合同协议.spg 正常显示数据的条件是什么？`
- 增加真实项目 eval case：`合同协议.spg 的 input3 来源是什么？`，要求答案优先输出 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`，并禁止把旁路 context 当作必要条件。
- 增加 fixture case：简单显示条件、链式条件、按钮禁用、数据源为空、系统变量依赖。
- 更新 `SKILL.md`，增加“页面没数据 / 按钮灰 / 面板不显示”的命令路线。
- 空上下文 `5.4-mini` 验证时禁止读取 raw `.spg`，只能使用工具输出回答。
- 评测断言必须覆盖 `param`、`$user`、隐藏字段、`model.filter`、`totalRowCount__`、`visibleCondition/disableCondition`。
- 评测断言必须包含注意力漂移负例：同名局部模型的其他页面引用、旁路 DataFlow、低相关组件不得被写成主因或必要条件。

### 验收目标

- 小模型能回答条件类问题，且不需要直接读取 5MB 原始元数据。
- 回答包含证据链、截断说明和保守口径。
- 禁止误判：不能把条件门控说成渲染故障，不能把未参与初始展示的参数说成必需条件。
- 禁止漂移：当工具同时输出主链路和相关上下文时，小模型必须优先引用主链路，并显式区分“相关但非必要”的旁路关系。

## M23：Hot Graph Runtime 基座 ✅

### 目标

让项目级查询能够在同一进程内复用已加载的 `GraphDB`，为后续 function calling / stdio server / MCP 长驻工具形态打基础。

M23 只解决一件事：把“加载图”和“执行查询”从 CLI 分支中解耦，证明同一个 runtime 连续执行多次查询时，第二次不再全量加载 graphdb。

### 非目标

- 不实现 Lazy GraphDB。
- 不新增 redb 邻接索引。
- 不实现 MCP server。
- 不改 `metadata-checker` skill 调用协议。
- 不做 pathfinder 深度性能优化。
- 不改变现有 CLI 输出协议。

### 背景问题

- AI 通过 skill 调用 CLI 时，每次都是新进程冷启动。
- 当前项目级查询会在 `src/main.rs` 中先打开 graphdb，再通过 `GraphDB::load_from_db` 全量反序列化 nodes/edges 并构建 `petgraph`。
- `samply` 对真实项目 `xiaoshouyi` 的 `input3 --explain-condition` 采样显示，冷查询的主要成本集中在 graphdb 全量读取、redb range 迭代、`serde_json::from_slice`、`serde_json::Value::deserialize` 和内存图构建。
- 如果目标转向 function calling / 长驻进程，全量加载并不是错误方向；问题是不能每次 function call 都重复加载。

### 设计原则

- 保留现有 `GraphDB` 全量加载模型，作为热查询和复杂关系遍历的主路径。
- 先建立运行时复用边界，再考虑服务协议。
- 查询逻辑应能被 CLI 和 runtime 共同调用，避免继续把业务查询逻辑写死在 `main.rs` 分支里。
- 阶段耗时必须可观测，否则后续无法判断优化是否真正命中冷启动问题。

### 工作清单

- 新增运行时模块，例如 `src/runtime.rs`。
- 新增 `GraphRuntime` 结构，至少包含：
  - `graph: GraphDB`
  - `graph_db_path: PathBuf`
  - `loaded_at: SystemTime`
  - `graph_file_mtime: Option<SystemTime>`
  - `graph_file_size: u64`
  - `load_count: usize`
- 实现 `GraphRuntime::load(graph_db_path: impl AsRef<Path>) -> Result<Self>`。
- 实现 `GraphRuntime::query(request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse>`。
- 定义 `RuntimeQueryRequest`，至少支持：
  - `command: RuntimeQueryCommand`
  - `target: String`
  - `budget: String`
  - `human: bool`
- 定义 `RuntimeQueryCommand`，M23 最小范围只要求：
  - `ExplainCondition`
- 定义 `RuntimeQueryResponse`，至少包含：
  - `result: serde_json::Value`
  - `timing: RuntimeTiming`
  - `diagnostics: Vec<String>`
- 定义 `RuntimeTiming`，至少包含：
  - `graph_load_ms`
  - `query_compute_ms`
  - `serialize_ms`
  - `total_ms`
- 将 `explain_condition_target` 的核心逻辑拆出为“返回 `serde_json::Value`”的函数。
  - CLI 仍负责 `println!("{}", serde_json::to_string_pretty(...))`。
  - runtime 直接复用该函数，避免通过 stdout 解析结果。
- 保留当前 `explain_condition_target` 作为 CLI 包装函数，避免破坏现有调用。
- 增加 runtime 单元测试或集成测试：
  - 加载 fixture graphdb。
  - 连续执行两次同一 `ExplainCondition` 查询。
  - 断言 `load_count == 1`。
  - 断言两次查询结果的关键字段一致。
- 增加真实项目 ignored 测试或手动验收命令：
  - 目标：`comp:app/销售.app/销售/合同协议.spg|input3`
  - 要求结果仍包含 `model22.phoneNumber`、`fact_qwSidebar.phoneNumber`、`action1`、`action4`。
- 增加阶段耗时日志或测试可读输出，用于区分：
  - 第一次 runtime 查询：包含 graph load。
  - 第二次 runtime 查询：`graph_load_ms == 0` 或接近 0。
- 更新 `main.rs` 内部调用路径时保持 CLI 行为兼容：
  - 单次 CLI 项目级查询仍可继续冷启动。
  - M23 不要求 CLI 自动进入长驻模式。

### 落地任务拆解

#### M23.1：确认当前查询入口和可复用边界

- 阅读 `src/main.rs` 中项目级查询分支，确认 `--explain-condition` 当前调用链。
- 阅读 `src/explain.rs` 中 `explain_condition_target`，标记所有直接 `println!`、`serde_json::to_string_pretty`、`human` 输出分支。
- 阅读 `src/graph.rs` 中 `GraphDB::open_or_diagnostic`、`GraphDB::open_readonly`、`GraphDB::load_from_db`，确认全量加载发生点。
- 记录 M23 不改动的范围：
  - 不改 `GraphDB` 序列化格式。
  - 不改 redb 表结构。
  - 不改 `PathFinder` 算法。
  - 不改 CLI JSON 输出 shape。
- 输出一份简短实现备注，说明 M23 的最小代码改动路径。

验收：

- 能指出 `GraphRuntime` 应复用的最小查询函数。
- 能指出哪些 CLI 输出逻辑必须留在 wrapper 层。

#### M23.2：拆出 ExplainCondition 纯查询函数

- 在 `src/explain.rs` 中新增返回结构化结果的函数，建议命名：
  - `build_explain_condition_output(graph: &GraphDB, target_id: &str, budget: &str) -> Result<serde_json::Value>`
- 将 `explain_condition_target` 中构建 JSON 的主体逻辑迁移到新函数。
- `build_explain_condition_output` 不允许直接 `println!` / `eprintln!`。
- `build_explain_condition_output` 不处理 human 输出。
- 原 `explain_condition_target` 保留为 CLI wrapper：
  - 调用 `build_explain_condition_output`。
  - `human == false` 时继续 pretty JSON 输出。
  - `human == true` 时保持现有行为或显式沿用当前 JSON 输出，不能破坏编译。
- target 不存在时仍返回当前 `TARGET_NOT_FOUND` 结构，不能退化为 error。
- 保持 `details.primary_path`、`details.related_context`、`details.data_empty_gates` 等字段不变。

验收：

- 现有 `--explain-condition` CLI 输出与重构前等价。
- `cargo test --test regression_tests test_real_project_query_page_logic_input3_chain -- --ignored --nocapture` 仍通过，若该测试名调整则运行现有 input3 真实项目回归。
- 不新增 `#[allow(dead_code)]`。

#### M23.3：新增 Runtime 类型与请求响应协议

- 新增 `src/runtime.rs`。
- 在 `src/lib.rs` 或模块入口注册 `pub mod runtime;`，若当前项目没有 `lib.rs`，按现有模块组织在 `main.rs` 中声明 `mod runtime;`。
- 定义 `GraphRuntime`：
  - `graph: GraphDB`
  - `graph_db_path: PathBuf`
  - `loaded_at: SystemTime`
  - `graph_file_mtime: Option<SystemTime>`
  - `graph_file_size: u64`
  - `load_count: usize`
- 定义 `RuntimeQueryCommand`：
  - `ExplainCondition`
- 定义 `RuntimeQueryRequest`：
  - `command: RuntimeQueryCommand`
  - `target: String`
  - `budget: String`
  - `human: bool`
- 定义 `RuntimeQueryResponse`：
  - `result: serde_json::Value`
  - `timing: RuntimeTiming`
  - `diagnostics: Vec<String>`
- 定义 `RuntimeTiming`：
  - `graph_load_ms: u128`
  - `query_compute_ms: u128`
  - `serialize_ms: u128`
  - `total_ms: u128`
- 为上述类型添加中文文档注释，文档注释放在 `#[derive(...)]` 之前。
- 只为确实需要序列化的请求/响应类型派生 `Serialize` / `Deserialize`。

验收：

- `cargo check` 无 warning。
- 类型命名和字段命名符合 Rust 规范。
- 没有为了消除 warning 添加 `#[allow(dead_code)]`。

#### M23.4：实现 GraphRuntime 加载与查询

- 实现 `GraphRuntime::load(graph_db_path: impl AsRef<Path>) -> Result<Self>`。
- `load` 内部调用当前稳定的 graphdb 打开方式，优先使用现有 CLI 项目级查询同路径。
- `load` 记录 graphdb 文件 metadata：
  - `mtime`
  - `size`
  - `loaded_at`
- `load_count` 初始化为 `1`。
- 实现 `GraphRuntime::query(&self, request: RuntimeQueryRequest) -> Result<RuntimeQueryResponse>`。
- M23 只支持 `RuntimeQueryCommand::ExplainCondition`。
- `query` 调用 `build_explain_condition_output(&self.graph, &request.target, &request.budget)`。
- `query` 的 `graph_load_ms` 必须为 `0`，因为 runtime 已经加载。
- `query_compute_ms` 只统计查询构建耗时。
- `serialize_ms` 可以先统计 `serde_json::to_vec(&result)` 的耗时，但不能改变返回的 `result`。
- `total_ms` 覆盖整个 `query` 调用。
- 对不支持的命令不预留空分支；M23 只有一个 enum variant。

验收：

- 同一个 `GraphRuntime` 调用多次 `query`，`load_count` 保持 `1`。
- `query` 不调用 `GraphDB::open_or_diagnostic` / `GraphDB::load_from_db`。
- `query` 返回的 `result.kind == "Explain"`。

#### M23.5：补测试项目与 runtime 回归

- 新增测试文件或扩展现有 regression 测试，建议命名：
  - `tests/runtime_tests.rs`
- 测试 fixture graphdb 可以在测试内基于 `tests/fixtures/test_project` 构建到 `/tmp` 或临时目录。
- 增加测试：`test_graph_runtime_reuses_loaded_graph_for_explain_condition`。
- 测试步骤：
  - 构建 fixture graphdb。
  - `GraphRuntime::load(&graph_db_path)`。
  - 第一次执行 `ExplainCondition`。
  - 第二次执行同一 `ExplainCondition`。
  - 断言 `runtime.load_count == 1`。
  - 断言两次结果的 `kind`、`query_target`、关键 `summary` 字段一致。
  - 断言第二次响应的 `timing.graph_load_ms == 0`。
- 增加真实项目 ignored 测试，建议命名：
  - `test_real_project_runtime_input3_explain_condition_reuses_graph`
- 真实项目测试目标：
  - project：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
  - target：`comp:app/销售.app/销售/合同协议.spg|input3`
- 真实项目测试断言：
  - `details.primary_path` 中包含 `model22.phoneNumber`
  - 包含 `fact_qwSidebar.phoneNumber`
  - 包含 `action1`
  - 包含 `action4`
  - 连续两次查询 `load_count == 1`

验收：

- fixture 测试默认运行。
- 真实项目测试标记 `#[ignore]`，避免 CI 依赖本机路径。
- 测试断言使用 `assert_eq!` / 明确 helper，避免只 `assert!(json.to_string().contains(...))` 的弱断言；真实项目长 JSON 可使用已有 `array_any_contains_substring` 类 helper。

#### M23.6：保留 CLI 兼容并避免范围外扩散

- 不新增 CLI 参数。
- 不新增 `--serve-stdio`。
- 不更新 `SKILL.md`。
- 不改变 `--explain-condition` 当前命令输出。
- 如必须调整 `main.rs`，只做调用函数名变化，不改变分支顺序和参数语义。
- 不把 `GraphRuntime` 接入默认 CLI 单次查询路径；M23 只提供 runtime 能力和测试证明。

验收：

- 现有命令仍可运行：
  - `./target/release/metadata-checker --project-dir <PROJECT> --graph-db-path <DB> --explain-condition '<TARGET>' --budget compact`
- 输出字段兼容 M22 eval。

#### M23.7：性能与验证记录

- 运行 `cargo check`。
- 运行受影响测试：
  - `cargo test --test runtime_tests`
  - `cargo test --test regression_tests`
  - 如改动 explain 输出，再运行 `cargo test --test explain_tests` 或仓库中对应 explain 测试文件。
- 运行真实项目 ignored 测试或手动命令，记录结果。
- 可选：用 `samply` 对 runtime 测试或后续 M24 stdio 查询再采样；M23 不强制。
- 在提交信息或交付说明中明确：
  - 第一次 runtime 查询包含 graph load。
  - 第二次 runtime 查询复用内存 graph。
  - CLI 单次查询仍是冷启动，留给 M24/M26 接入解决。

验收：

- `cargo check` 0 warning。
- 默认测试通过。
- 真实项目 input3 主链路不回退。
- git diff 只包含 runtime、explain 查询函数拆分、测试和必要模块注册。

### 验收目标

- `cargo check` 无 warning。
- 现有测试不回退。
- `GraphRuntime` 可以在一个进程内复用同一份内存 `GraphDB`。
- 同一个 runtime 连续执行两次 `ExplainCondition`，第二次不调用 `GraphDB::load_from_db`。
- `input3` 真实项目查询的主链路不回退，仍能输出 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`。
- M23 完成后，才能进入 M24 的 stdio function calling server。

### 后续拆分

- M24：Stdio Function Calling Server。把 `GraphRuntime` 暴露为 JSONL stdin/stdout 长驻服务。
- M25：Runtime Cache 与 Reload。处理 graphdb 文件变更检测、手动 reload、reload 失败降级。
- M26：Skill / Function Calling 接入与性能验收。更新 skill 决策树，固化冷 CLI、首次 stdio、第二次 stdio 的真实项目性能基线。

## M24：Stdio Function Calling Server ✅

### 目标

把 M23 的 `GraphRuntime` 暴露为长驻 JSONL stdio 服务，让 AI/function calling 包装层可以在一个进程内连续执行多个项目级查询。

M24 只解决协议和进程生命周期：启动一次、加载一次 graphdb、stdin 每行一个请求、stdout 每行一个响应。

### 非目标

- 不做 MCP server。
- 不做后台 daemon。
- 不做 graphdb 自动 reload。
- 不更新 `metadata-checker` skill 默认调用策略。
- 不新增 Lazy GraphDB。
- 不改变单次 CLI 查询输出。

### 工作清单

- 在 `src/cli.rs` 新增参数：
  - `--serve-stdio`
  - `--serve-graph-db-path <PATH>` 或复用现有 `--graph-db-path`
- 在 `src/main.rs` 中增加 `--serve-stdio` 分支。
- 新增服务模块，建议命名：
  - `src/stdio_server.rs`
- 定义 JSONL 请求结构：
  - `request_id: String`
  - `command: String`
  - `target: String`
  - `budget: Option<String>`
  - `human: Option<bool>`
- 定义 JSONL 响应结构：
  - `request_id: String`
  - `ok: bool`
  - `result: Option<serde_json::Value>`
  - `error: Option<String>`
  - `diagnostics: Vec<String>`
  - `timing: RuntimeTiming`
- 服务启动时：
  - 解析 graphdb path。
  - 调用 `GraphRuntime::load`。
  - stderr 输出启动日志。
  - stdout 不输出启动日志，避免污染 JSONL 协议。
- 服务循环：
  - 从 stdin 逐行读取。
  - 空行跳过。
  - 非法 JSON 返回 `ok=false`，不能 panic。
  - 未知 command 返回 `ok=false` 和明确错误。
  - 支持 `explain_condition` 命令，映射到 `RuntimeQueryCommand::ExplainCondition`。
  - 每个响应必须单行 JSON。
  - stdout 每次响应后 flush。
- 错误处理：
  - 可恢复错误转为 JSONL error。
  - 不在库代码中 `println!` / `eprintln!`。
  - server 层可以用 stderr 输出运行日志。
- 增加测试：
  - 启动 `metadata-checker --serve-stdio --graph-db-path <fixture_db>` 子进程。
  - 连续写入两个 `explain_condition` 请求。
  - 读取两行响应。
  - 断言两个响应 `ok=true`。
  - 断言第二个响应 `timing.graph_load_ms == 0`。
  - 断言两次响应不包含非 JSON 日志。
- 增加负例测试：
  - 非法 JSON 行。
  - 未知 command。
  - 缺 target。

### 落地任务拆解

#### M24.1：CLI 参数和入口

- 修改 `src/cli.rs`，增加 `serve_stdio: bool`。
- 确认 help 文本说明这是机器协议，不是人类 REPL。
- 修改 `src/main.rs`：
  - 在项目级查询分支前处理 `--serve-stdio`。
  - 缺少 graphdb path 时返回结构化错误或 `anyhow::bail!`。
  - 不要求 `<FILE>` input。

验收：

- `metadata-checker --help` 能看到 `--serve-stdio`。
- 不影响现有 `--project-dir --query-*` 分支。

#### M24.2：JSONL 协议实现

- 新增 `StdioRequest` / `StdioResponse` 类型。
- 类型字段使用 snake_case。
- command 先用字符串解析，不急于暴露复杂 enum 到外部协议。
- 内部再转换到 `RuntimeQueryRequest`。
- 所有响应必须包含原始 `request_id`；若请求 JSON 都无法解析，则使用空字符串或生成 `"unknown"`。

验收：

- 一行输入只产生一行输出。
- stdout 只包含 JSONL。
- stderr 才允许包含启动和错误日志。

#### M24.3：子进程集成测试

- 新增 `tests/stdio_server_tests.rs`。
- 使用 `std::process::Command` 启动当前测试 binary 对应的 `metadata-checker` 可执行文件；如果现有测试已有 helper，复用 helper。
- 子进程 stdin/stdout 使用 pipe。
- 测试结束必须 kill/wait 子进程，避免遗留进程。
- 测试 fixture graphdb 在临时目录中构建，避免污染仓库。

验收：

- `cargo test --test stdio_server_tests` 通过。
- 测试失败时不会挂住。

#### M24.4：协议文档

- 新增或更新 docs，建议：
  - `docs/stdio-server.md`
- 文档包含：
  - 启动命令。
  - 请求示例。
  - 响应示例。
  - 错误响应示例。
  - stdout/stderr 边界。

验收：

- 文档示例可以直接复制执行。

### 验收目标

- `cargo check` 无 warning。
- `cargo test --test stdio_server_tests` 通过。
- stdio server 连续两个请求只加载一次 graphdb。
- stdout 协议对 function calling 包装层稳定可解析。
- M24 完成后，才能进入 M25 的 reload/cache。

## M25：Runtime Cache 与 Reload

### 目标

让长驻 runtime 能识别 graphdb 文件变化，并在不崩溃的前提下刷新内存图，避免 AI/function calling 使用过期图。

M25 只解决 runtime 生命周期可靠性：状态查询、变更检测、手动 reload、失败降级。

### 非目标

- 不做自动后台文件监听。
- 不做多项目 graph runtime 池。
- 不做 MCP server。
- 不更新 skill 默认策略。
- 不做 Lazy GraphDB。

### 工作清单

- 扩展 `GraphRuntime`：
  - `reload_count: usize`
  - `last_reload_error: Option<String>`
  - `graph_fingerprint: GraphFingerprint`
- 新增 `GraphFingerprint`：
  - `path: PathBuf`
  - `mtime: Option<SystemTime>`
  - `size: u64`
- 实现 `GraphRuntime::current_fingerprint() -> Result<GraphFingerprint>`。
- 实现 `GraphRuntime::is_graph_changed() -> Result<bool>`。
- 实现 `GraphRuntime::reload_if_changed() -> Result<bool>`。
- 实现 `GraphRuntime::reload() -> Result<()>`。
- reload 策略：
  - 新图加载成功后再替换旧 graph。
  - 新图加载失败时保留旧 graph。
  - 记录 `last_reload_error`。
  - 返回 diagnostic，不能静默失败。
- stdio server 增加命令：
  - `status`
  - `reload`
- stdio server 每次普通查询前可选调用 `reload_if_changed()`；如果担心开销，M25 可先只在请求显式带 `check_reload: true` 时执行，但协议必须明确。
- `status` 输出：
  - graphdb path
  - loaded_at
  - load_count
  - reload_count
  - node_count
  - edge_count
  - graph_file_mtime
  - graph_file_size
  - last_reload_error
- 增加测试：
  - graphdb 未变化时 `reload_if_changed()` 返回 false。
  - graphdb 替换后 `reload_if_changed()` 返回 true。
  - reload 失败时旧 runtime 仍可查询。
  - stdio `status` 返回当前 runtime 状态。
  - stdio `reload` 成功后 `reload_count` 增加。

### 落地任务拆解

#### M25.1：Fingerprint 与状态输出

- 在 `src/runtime.rs` 中新增 `GraphFingerprint`。
- 为 `GraphRuntime` 增加 `status()` 方法，返回 JSON 或强类型 `RuntimeStatus`。
- `status` 不触发 reload。

验收：

- 单元测试能读取 node/edge 数和 graphdb 文件状态。

#### M25.2：安全 reload

- reload 不允许先清空当前 graph。
- 使用局部变量加载新 `GraphRuntime` 或新 `GraphDB`。
- 加载成功后再替换 `self.graph` 和 fingerprint。
- 加载失败时保留旧 graph，返回错误并记录。

验收：

- 损坏 graphdb reload 失败后，旧查询仍可返回结果。

#### M25.3：stdio 命令扩展

- `status` 不要求 target。
- `reload` 不要求 target。
- 普通查询响应 diagnostics 中可以包含 `"GRAPH_RELOADED"` 或 `"GRAPH_RELOAD_FAILED"`。
- 错误响应仍保持单行 JSON。

验收：

- 连续请求：`status` -> `explain_condition` -> `reload` -> `status` 都可解析。

### 验收目标

- 长驻进程不会静默使用过期 graphdb。
- reload 失败不会导致服务崩溃或丢失旧图。
- `status` 能让 AI 判断当前 runtime 是否加载了预期 graphdb。
- M25 完成后，才能进入 M26 的 skill/function calling 接入。

## M26：Skill / Function Calling 接入与性能验收

### 目标

把 M23-M25 的长驻查询能力接入 AI 使用路径，并用真实项目性能基线验证“第二次查询不再冷启动”。

M26 只解决 AI 使用协议和性能验收，不再大改 runtime 架构。

### 非目标

- 不新增新的查询语义。
- 不重写 pathfinder。
- 不做 Lazy GraphDB。
- 不做 MCP server，除非 M24 stdio 已稳定且另开后续里程碑。

### 工作清单

- 更新 `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md` 或仓库内同步文档，增加 stdio server 决策树。
- 决策规则：
  - 单次、低成本、临时查询可继续 CLI。
  - 同一项目连续多个项目级查询优先 stdio server。
  - 条件类、context、query-model 连续追问应复用 stdio server。
  - graphdb 变更后先发 `status` / `reload`。
- 增加 function calling wrapper 示例文档，建议：
  - `docs/function-calling-runtime.md`
- 文档包含：
  - 如何启动 stdio server。
  - 如何发送 JSONL 请求。
  - 如何处理 `request_id`。
  - 如何处理错误响应。
  - 如何关闭进程。
- 建立性能基线文档，建议：
  - `docs/performance-baseline.md`
- 基线至少包含：
  - CLI 冷查询 `input3 --explain-condition` 耗时。
  - stdio server 首次查询耗时。
  - stdio server 第二次查询耗时。
  - graph load 阶段耗时。
  - query compute 阶段耗时。
  - 输出大小。
- 用真实项目 `xiaoshouyi` 验收：
  - `comp:app/销售.app/销售/合同协议.spg|input3`
  - `model:fact_qwSidebar`
  - 一个 `--context` 或后续已接入 command。
- 更新 ai-eval 或手工验收清单：
  - 记录 stdio 模式下输出仍满足 M22 主链路断言。
  - 记录模型不会因为新增 timing/status 字段发生注意力漂移。
- 可选运行 `samply`：
  - 对 stdio 第二次查询采样。
  - 确认热点不再是 `GraphDB::load_from_db`。

### 落地任务拆解

#### M26.1：Skill 决策树更新

- 更新 skill 文档时保持 summary-first 原则。
- 把 stdio server 放在“同一项目连续多问”路径，不替代所有 CLI。
- 明确 stdout JSONL 不能混入日志。
- 明确单次查询仍可用 CLI，避免模型为了简单问题启动长驻服务。

验收：

- skill 文档能指导 AI 在连续追问时复用服务。

#### M26.2：性能基线采集

- 记录命令：
  - CLI 冷查询。
  - stdio 启动。
  - stdio 第一次请求。
  - stdio 第二次请求。
- 每个命令记录：
  - real/user/sys 或 runtime timing。
  - 输出大小。
  - graphdb 路径。
  - git commit。
- 基线数字不要求绝对固定，但必须能证明第二次 stdio 查询绕过 graph load。

验收：

- `docs/performance-baseline.md` 有真实项目数据。

#### M26.3：AI 输出抗漂移检查

- 用 M22 的 input3 问题重新跑一轮工具输出。
- 确认新增 timing/status 不被模型当作业务证据。
- 如果输出新增字段会干扰 AI，调整 skill 读取策略：
  - 先读 `summary`
  - 再读 `details.primary_path`
  - timing 只用于性能判断

验收：

- 业务回答仍优先引用主链路，不引用 runtime timing 作为业务原因。

### 验收目标

- AI 连续项目级查询不再天然冷启动。
- stdio 第二次查询的 `graph_load_ms == 0`。
- 真实项目 input3 主链路不回退。
- skill 文档明确 CLI 与 stdio server 的选择边界。
- 性能基线文档记录可复现命令和结果。
