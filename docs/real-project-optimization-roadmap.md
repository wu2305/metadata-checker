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

## M25：Runtime Cache 与 Reload ✅

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

## M26：Skill / Function Calling 接入与性能验收 ✅

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
  - 同一项目连续多个 `explain_condition` 查询优先 stdio server。
  - `context`、`query-model`、`query-page-logic` 连续追问当前仍走 CLI；除非后续里程碑显式扩展 stdio command，不能在 skill/function calling 文档中宣称已支持。
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
  - `field:fact_qwSidebar.phoneNumber`（通过当前已接入的 `explain_condition` 验证）
  - `--context` / `--query-model` / `--query-page-logic` 作为 CLI 边界记录，不纳入当前 stdio 验收。
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

- AI 连续 `explain_condition` 查询不再天然冷启动。
- stdio 第二次查询的 `graph_load_ms == 0`。
- 真实项目 input3 主链路不回退。
- skill 文档明确 CLI 与 stdio server 的选择边界。
- 性能基线文档记录可复现命令和结果。

## M27：Stdio 查询命令面扩展

### 目标

把当前 AI 高频项目级查询接入 stdio server，让 function calling 在同一进程内复用 `GraphRuntime`，不再因为 `context` / `query-model` / `query-page-logic` 退回 CLI 而重复冷启动。

M27 只扩展 stdio command surface，不改变各查询本身的业务语义和输出结构。

### 非目标

- 不重写 pathfinder。
- 不改变 `--explain-condition` 主链路选择规则。
- 不引入 Lazy GraphDB。
- 不把 stdio server 升级成 MCP server。
- 不为了 stdio 扩展重写 CLI 输出 schema。

### 工作清单

- 新增 stdio command：`context`
  - 对齐 CLI：`--context <ID> --depth <N> --budget <compact|normal|full>`。
  - request 字段：`target`、`depth`、`budget`、`human`、`check_reload`。
  - 默认 `depth=1`，默认 `budget=normal`，AI 使用建议优先 `compact` / `normal`。
- 新增 stdio command：`query_model`
  - 对齐 CLI：`--query-model <MODEL>`。
  - request 字段：`target`、`budget`、`human`、`check_reload`。
  - 真实项目验收目标必须包含 `model:fact_qwSidebar`。
- 新增 stdio command：`query_page_logic`
  - 对齐 CLI：`--query-page-logic <PAGE>`。
  - request 字段：`target`、`budget`、`human`、`check_reload`。
  - compact 输出必须保留 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- action1/action4` 主链路。
- 新增 stdio command：`explain`
  - 对齐 CLI：`--explain <ID>`。
  - 用于普通对象解释，不替代 `explain_condition`。
- 抽出统一 stdio command registry。
  - 建议放在 `src/stdio.rs` 或 `src/runtime/stdio.rs`。
  - 每个 command 必须声明 required fields、默认参数、输出 kind、错误码。
  - 避免在 `main.rs` 中继续堆叠大 match 分支。

### 落地任务拆解

#### M27.1：Stdio command registry

- 抽出 `StdioCommand` / `StdioRequest` / `StdioResponse` 处理入口。
- 保留既有 `explain_condition` / `status` / `reload` 行为不变。
- unknown command 返回单行 JSON 错误，不 panic。
- missing required field 返回结构化错误。

验收：

- 既有 `stdio_server_tests` 全部通过。
- unknown command / missing target 负例测试通过。

#### M27.2：接入 `context`

- 在 runtime 层复用当前 graph 调用 context 查询。
- 支持 `depth`、`budget`、`human`。
- `check_reload=true` 时沿用 M25 reload 检测逻辑。

验收：

- stdio `context` 输出结构与 CLI JSON 结构一致。
- 第二次 stdio `context` 查询 `timing.graph_load_ms == 0`。
- malformed depth / budget 有稳定错误码。

#### M27.3：接入 `query_model`

- 在 runtime 层复用当前 graph 调用 model 查询。
- 支持 physical table target 和普通 model target。
- 不降低 M19/M22 主链路与 field alias 回归质量。

验收：

- 真实项目 `model:fact_qwSidebar` 能看到 `潜客信息跟进.spg|button1|action1` / `action4` 写入。
- 第二次 stdio `query_model` 查询 `timing.graph_load_ms == 0`。

#### M27.4：接入 `query_page_logic`

- 在 runtime 层复用当前 graph 调用页面逻辑查询。
- 支持 compact/normal/full budget。
- 输出过大时仍遵守已有截断和 diagnostics 规则。

验收：

- 真实项目 `page:app/销售.app/销售/合同协议.spg` compact 输出包含 input3 主链路。
- 第二次 stdio `query_page_logic` 查询 `timing.graph_load_ms == 0`。

#### M27.5：接入 `explain`

- 在 runtime 层复用当前 graph 调用 explain 查询。
- 保持普通 explain 与 explain_condition 的用途区分。

验收：

- `explain comp:...|input3` 与 CLI JSON 输出结构一致。
- `explain_condition` 既有测试不回退。

### 验收目标

- stdio 支持 `explain_condition`、`explain`、`context`、`query_model`、`query_page_logic`、`status`、`reload`。
- 所有新增 command 的第二次查询 `graph_load_ms == 0`。
- 新增 command 错误响应保持单行 JSON。
- 真实项目 `input3` 与 `fact_qwSidebar` 主链路不回退。
- skill 文档不再要求 AI 为这些查询退回 CLI，除非 graphdb 不可用。

## M28：Stdio 请求/响应契约收敛

### 目标

统一 stdio 的 request / response / error schema，让 function calling wrapper 可以稳定消费，不靠字符串猜测或每个 command 特判。

### 非目标

- 不新增查询命令。
- 不改变 CLI human 输出。
- 不重写已有 JSON 结果中的业务字段。

### 工作清单

- [x] 定义统一 request schema：
  - `request_id`
  - `command`
  - `target`
  - `budget`
  - `depth`
  - `human`
  - `check_reload`
- [x] 定义统一 success response：
  - `request_id`
  - `ok: true`
  - `result`
  - `diagnostics`
  - `timing`
- [x] 定义统一 error response：
  - `request_id`
  - `ok: false`
  - `error.code`
  - `error.message`
  - `diagnostics`
  - `timing`
- [x] 统一错误码：
  - `INVALID_JSON`
  - `UNKNOWN_COMMAND`
  - `MISSING_TARGET`
  - `INVALID_TARGET`
  - `INVALID_BUDGET`
  - `INVALID_DEPTH`
  - `GRAPH_RELOAD_FAILED`
  - `QUERY_FAILED`
- [x] 更新文档：
  - `docs/function-calling-runtime.md`
  - `docs/schema.md`
  - `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`
- [x] 增加负例测试：
  - malformed JSONL
  - unknown command
  - missing target
  - invalid depth
  - invalid budget
  - reload failed 后旧 graph 仍可用

### 验收目标

- 所有 stdio command 使用同一响应 envelope。
- stdout 只输出 JSONL，stderr 只输出日志。
- function calling wrapper 不需要按 command 猜测错误格式。
- schema 文档和测试断言一致。

## M29：Function Calling 工具层优化

### 目标

基于 M27-M28 的 stdio 能力，为 AI 提供更不易误用的工具描述、调用边界和读取策略。

### 非目标

- 不实现 MCP server。
- 不新增底层图查询能力。
- 不把所有 CLI 参数暴露给 AI。

### 工作清单

- [x] 设计工具拆分：
  - `metadata_explain_condition`
  - `metadata_explain`
  - `metadata_context`
  - `metadata_query_model`
  - `metadata_query_page_logic`
  - `metadata_runtime_status`
  - `metadata_runtime_reload`
- [x] 明确每个 tool 的使用边界：
  - “为什么不显示 / 为什么没数据 / 值从哪来” → `metadata_explain_condition`
  - “这个对象是什么” → `metadata_explain`
  - “周围关系是什么” → `metadata_context`
  - “模型读写全貌” → `metadata_query_model`
  - “页面整体逻辑” → `metadata_query_page_logic`
- [x] 更新 skill 决策树：
  - 单次问题可用 CLI。
  - 同一项目连续追问优先 stdio/function calling。
  - graphdb 变更后先 `status` / `reload`。
  - graphdb 不可用时才单文件 fallback。
- [x] 增加 anti-drift 约束：
  - `timing` 只能用于性能判断。
  - 业务回答优先读 `summary`。
  - 证据核查读 `details.primary_path` / `evidence`。
  - `related_context` 默认不是必要条件。
- [x] 增加 function calling 示例：
  - 连续解释 `input3`、`model:model22`、`field:fact_qwSidebar.phoneNumber`。
  - 查询 `model:fact_qwSidebar`。
  - 查询 `合同协议.spg` 页面逻辑。

### 验收目标

- AI 能根据问题类型选择正确 tool。
- tool 描述不引导 AI 读取 timing/status 作为业务证据。
- skill 文档与 stdio 已支持命令完全一致。
- 真实项目样例能覆盖 input3 主链路、fact_qwSidebar writer、页面逻辑三类问题。

## M30：Stdio 性能与容量治理

### 目标

在 stdio command 扩展后，防止大查询把性能瓶颈从 GraphDB 冷加载转移到 pathfinder、evidence 组装或 JSON 序列化上。

### 非目标

- 不优化 GraphDB 构建速度。
- 不做 Lazy GraphDB。
- 不牺牲主链路正确性换取性能。

### 工作清单

- [x] 所有 stdio 查询记录统一 timing：
  - `graph_load_ms`
  - `query_compute_ms`
  - `serialize_ms`
  - `total_ms`
  - `output_size_bytes`
- [x] 输出预算治理：
  - 默认 compact 或 normal，禁止默认 full。
  - full 必须显式指定。
  - 超预算返回 `OUTPUT_TRUNCATED` diagnostics。
  - 截断不能删除 primary path。
- [x] 建立真实项目性能基线：
  - `explain_condition input3`
  - `context input3 depth=2`
  - `query_model fact_qwSidebar`
  - `query_page_logic 合同协议.spg`
  - 对比 CLI 冷启动与 stdio 第二次请求。
- [x] 热点分析：
  - stdio 第二次查询不得进入 `GraphDB::load_from_db`。
  - 若慢，优先检查 pathfinder、evidence 组装、JSON serialize、大数组排序和 clone。
  - 可选使用 `samply` 对真实项目第二次查询采样。
- [x] 回归门槛：
  - 第二次 stdio 查询 `graph_load_ms == 0`。
  - 小查询 `total_ms < 50ms`。
  - 中等查询 `total_ms < 300ms`。
  - 大查询必须可被 budget 截断，且输出不发生注意力漂移。

### 验收目标

- stdio 扩展后，第二次查询不再冷启动。
- 大输出有预算和截断机制，不依赖 AI 自行忽略噪声。
- 性能基线文档记录可复现命令、commit、graphdb 路径、输出大小和 timing。
- 真实项目主链路验收不因性能截断回退。

## M31：条件因果方向与继承去重

### 背景

真实问题“`会员已注册.spg` 中 `text41` 哪些情况会显示？”暴露出两类缺口：

- 工具能找到祖先容器 `panel35.visibleCondition = model11.totalRowCount__ > 0`，但 AI 没有继续展开 `model11` 的当前页面 filter。
- 组件自身与父组件可能声明相同限制条件，若不去重会导致回答重复或误判为多个独立必要条件。

### 目标

让 `explain_condition` 对显示/隐藏问题输出面向因果解释的条件链：

```text
目标组件自身条件
→ 祖先容器继承条件
→ totalRowCount__ 数据命中门控
→ 当前页面模型 filter 条件
```

### 非目标

- 不重写 GraphDB 存储边方向。
- 不把所有 related_context 自动升为必要条件。
- 不做跨页面同名 model 的业务推断。

### 功能点

- [x] 条件作用域标注：
  - `condition_scope = direct`
  - `condition_scope = inherited`
  - `condition_scope = expanded_from_total_row_count`
  - 输出 `owner_node_id`、`inherited_from`、`ancestor_distance`、`expanded_from_condition`。
- [x] 祖先容器条件收集：
  - 目标为组件时，沿 `Contains` 入边向上查找父组件。
  - 收集祖先 `visibleCondition` / `disableCondition` / action condition。
  - 祖先条件进入主 `blocking_conditions`，不放入 `related_context`。
- [x] `totalRowCount__` 门控展开：
  - 从 blocking condition 的 `referenced_symbols` / `raw_expr` 中识别 `modelX.totalRowCount__`。
  - 在当前页面作用域下查找 `modelX` 的 `SourceFilterExp` / `SourceFilterClause`。
  - 展开结果进入 `data_empty_gates`，并标注来源条件。
- [x] 条件去重：
  - 按 `source_file + condition_type + effect_type + normalized_expr/raw_expr + referenced_symbols` 生成 `condition_key`。
  - 自身与祖先条件等价时只保留一条。
  - 去重后保留 `deduped_condition_ids` / `deduped_scopes` / `deduped_owner_node_ids`。
- [ ] 真实项目验收：
  - `/app/售后.app/绑定车辆/会员已注册.spg|text41`
  - 必须回答：自身无直接 visible，继承祖先容器门控，`model11.totalRowCount__ > 0` 继续展开为当前页面 `model11` filter。
- [ ] SKILL 决策树更新：
  - 显示问题不能只查目标组件。
  - 遇到 `modelX.totalRowCount__` 必须继续展开当前页面 `modelX` filter。
  - 明确区分 direct / inherited / expanded / related_context。

### 细化任务清单

- [x] Fixture 补齐：
  - 新增父容器 `panel_total_gate.visibleCondition = model1.totalRowCount__ > 0`。
  - 新增子组件 `text_total_child`，自身无 visible 条件。
  - 新增子组件 `text_total_child_duplicate`，自身与父容器声明相同 visible 条件。
- [x] `explain_condition` 数据结构：
  - 为条件对象补充 `condition_scope`。
  - 为条件对象补充 owner/继承/展开来源字段。
  - 不删除既有字段，保持兼容。
- [x] 继承条件遍历：
  - 实现组件祖先链遍历。
  - 避免跨页面条件进入主条件。
- [x] 数据门控展开：
  - 实现 `totalRowCount__` 模型识别。
  - 从当前页面模型条件中抽取 filter。
- [x] 去重实现：
  - 实现条件 key。
  - 合并重复条件并保留证据来源。
- [x] Fixture 回归测试：
  - 子组件继承父容器 totalRowCount 门控。
  - totalRowCount 门控展开为当前页面 filter。
  - 子/父重复 visibleCondition 去重。
- [ ] 真实项目回归测试：
  - ignored 测试覆盖 `text41`。
  - 断言 `panel35#visibleCondition#8` 与当前页面 `model11` filter 同时出现。
- [ ] 文档与 skill：
  - 更新 `SKILL.md` / `docs/schema.md`。
  - 增加弱指向显示问题回答规则。

### 验收目标

- AI 问“组件哪些情况显示”时，不停在目标组件直接条件。
- 祖先容器门控被标为继承必要条件，而不是 related_context。
- `modelX.totalRowCount__` 不再作为终点，必须展开到当前页面数据源 filter。
- 相同条件不会重复污染主答案，但证据来源仍可追溯。

## M32：裸字段值来源继承容器数据上下文

### 背景

真实问题“`会员已注册.spg` 中 `text41` 如果显示，显示的数据来源于哪张物理表？”暴露出新缺口：

- `text41.value = ${CUSTOMAUTOMYAUTOLIST}` 只有字段名，没有模型名前缀。
- 字段所属数据集不在 `text41` 自身，而在祖先容器 `sliderpanel2.source = data` / `sliderpanel2.dataSet = model11`。
- AI 若只看 `text41.value`，会误把 `CUSTOMAUTOMYAUTOLIST` 当成全局模型或物理表。

### 目标

让 `explain_condition` 对组件裸字段值输出稳定链路：

```text
目标组件 value = ${FIELD}
→ 最近带 dataSet 的祖先数据容器
→ 页面 source 中的 dataSet 模型
→ 表路径 / DataFlow 加工表
→ 字段级原始输入表（能证明时）
```

### 非目标

- 不把所有裸字段都强行解析为页面 source；只有存在最近数据容器时才输出确定结论。
- 不用业务名称猜物理表；字段级来源必须来自 DataFlow 元数据或图边证据。
- 不替代 M31 显示条件链，M32 只补“显示后值从哪里来”。

### 细化任务清单

- [x] 扫描组件数据上下文：
  - 从原始 `.spg` JSON 递归记录组件 `json_path`。
  - 记录组件 `source`、`dataSet`、`parent_id`。
  - 将组件 `properties`、`json_path`、`source`、`dataSet` 写入 Component 节点 meta。
- [x] 保留页面 source 模型节点：
  - 对 `dwtable` source 显式创建局部 `model:<source.id>` 节点。
  - 保留 `sourcePath`，建立局部 model 与表路径的关系。
- [x] 识别裸字段值：
  - 只识别 `${FIELD}` 且 `FIELD` 不含 `.` 的表达式。
  - 输出 `bare_symbol`，避免把字段名误当模型名。
- [x] 最近数据容器解析：
  - 用 `json_path` 前缀查找最近祖先组件。
  - 只接受带 `source` 或 `dataSet` 的祖先容器。
  - 输出 `nearest_data_context`。
- [x] 表路径解析：
  - 用容器 `dataSet` 找页面 source model。
  - 输出 `field_path = dataSet.FIELD` 与 `table_source_path`。
- [x] DataFlow 字段来源解析：
  - 在 `.tbl` DataFlow meta 中保留 `nodeTablePaths`。
  - 通过 `nodeFields.dbfield/name` 匹配裸字段。
  - 通过 `originalNode` / `aliasMap` / `nodeTablePaths` 输出 `module_table_path`。
- [x] Fixture 回归测试：
  - `slider_data_context.dataSet = model1`。
  - 子组件 `text_bare_field_child.value = ${name}`。
  - 断言解析为 `model1.name` 和 `data/table1.tbl`。
- [x] 真实项目回归测试：
  - `text41.value = ${CUSTOMAUTOMYAUTOLIST}`。
  - 断言最近数据容器是 `sliderpanel2`。
  - 断言 dataSet 是 `model11`。
  - 断言表路径包含 `$DATA:/加工表/小程序/绑车.tbl`。
  - 能证明字段级来源时断言 `fact_autoCustomerAutoRel.tbl`。
- [x] 文档与 skill：
  - `docs/schema.md` 记录 `value_source_context` 契约。
  - `SKILL.md` 要求“值来源”问题先读 `value_source_context`，不要把裸字段当表。

### 验收目标

- AI 问“组件显示的数据来源于哪张表”时，不再停在 `${FIELD}`。
- 对裸字段必须先找最近数据容器，再解释 `dataSet.field`。
- DataFlow 字段级来源可证明时输出原始输入表；不可证明时只输出输入表候选。

## M33：目标节点中心的受控多跳因果遍历

### 背景

M31/M32 已经分别解决了两个真实问题：

- 显示条件不能只看目标组件自身，需要沿祖先容器和 `totalRowCount__` 门控做必要展开。
- 裸字段值来源不能把 `${FIELD}` 当模型名，需要继承最近数据容器 `dataSet` 并继续追到 DataFlow 字段来源。

但这两个能力仍暴露出一个更通用的问题：用户问题通常会指向某个节点，但答案常常需要多跳才能判断。如果 CLI 只给目标节点 1-hop，会丢失答案；如果直接给整页/全图，又会让 AI 在 `related_context`、候选路径、页面其它组件和 DataFlow 候选中注意力漂移。

因此需要新增一层“目标节点中心的受控多跳因果遍历”：

```text
target node
→ 按问题意图选择允许的边类型
→ 在有限 path budget 内展开多跳
→ 遇到明确停止条件后形成 answer_facts
→ details/evidence 只作为核查材料
```

### 核心原则

- 默认以用户指定的 `target_id` 为锚点，不做页面级全量展开。
- 多跳不是按单纯 `depth` 展开，而是按 `intent + allowed_edge_types + stop_conditions` 展开。
- 每条保留路径必须说明 `why_included`，避免 AI 把普通邻居误读为必要证据。
- 输出必须区分证明链路、候选链路和被排除链路。
- compact 输出优先给 `answer_facts`，不是把大数组截断后交给模型判断。

### 非目标

- 不替代现有 `--context` 的自由探索能力。
- 不删除 `details.primary_path` / `related_context` 等兼容字段。
- 不让 LLM 在运行时自由决定图遍历规则；规则必须由 CLI 内置。
- 不做无界跨页搜索；跨页只在 writer / field lineage 等 intent 明确需要时展开。

### 功能要求

- [x] 新增 `TraversalIntent`：
  - `Display`：回答显示/隐藏/禁用条件。
  - `ValueSource`：回答组件值、字段值、来源表。
  - `Writer`：回答字段/模型被谁写入或生成。
  - `Availability`：回答数据源为什么可能为空、row count 门控。
  - `Context`：回答周边关系，允许更宽但仍受 budget 控制。
- [x] 新增 `TraversalPolicy`：
  - `intent`
  - `allowed_edge_types`
  - `directions`
  - `max_paths`
  - `max_steps_per_path`
  - `stop_conditions`
  - `rank_rules`
- [x] 新增 `AnswerPath`：
  - `intent`
  - `result`
  - `confidence`
  - `steps[]`
  - `evidence_refs[]`
  - `why_complete`
  - `stop_condition_hit`
- [x] 新增 `answer_facts` 输出层：
  - `display_facts`
  - `value_source_facts`
  - `writer_facts`
  - `availability_facts`
  - `context_facts`
- [x] compact budget 默认输出：
  - `summary`
  - `answer_facts`
  - 少量 `evidence_refs`
  - 必要 diagnostics
  - 不默认输出大体量 `related_context` / `candidate_paths`
- [x] normal/full budget 保留现有 details，并增加 traversal debug 信息：
  - `traversal_policy`
  - `rejected_paths`
  - `candidate_paths`
  - `path_rank_reason`

### Intent 策略细化

#### Display

- [x] 允许边/关系：
  - `Condition -> Component`
  - `Condition -> Action`
  - `Condition -> Model`
  - `Condition -> Field totalRowCount__`
  - 祖先容器条件（通过 `Contains` 或 `json_path` 前缀）
  - 当前页面 model filter 展开
- [x] 停止条件：
  - 找到 direct/inherited display/disable/action condition。
  - 若 condition 引用 `modelX.totalRowCount__`，只继续展开当前页面 `modelX` filter。
  - 展开到当前页面 filter 后停止，不进入 DataFlow、action writer、value reads。
- [x] 明确排除：
  - `value` / `exp` 作为显示证据。
  - `value_source_context`。
  - 其他页面同名 model 条件。
  - `referenced_by_model_filter` 作为必要显示条件。

#### ValueSource

- [x] 允许边/关系：
  - target outgoing `Reads`
  - M32 派生 Reads：`resolution = inherited_container_data_context`
  - `FieldAlias`
  - `DataflowOutput`
  - DataFlow `nodeFields` / `dimensions` 字段来源
  - `DataflowInput` 仅作为字段级来源不可证明时的候选
- [x] 停止条件：
  - 找到非 DataFlow 物理表字段。
  - 找到 `dataflow_field_origin.module_table_path`。
  - 无字段级证明时停在 `dataflow_inputs` candidates。
- [x] 明确排除：
  - visible/disable/action conditions。
  - 页面其它组件 reads。
  - unrelated model filters。
  - action flows / navigation。

#### Writer

- [x] 允许边/关系：
  - incoming `Writes`
  - incoming `ActionWrites`
  - incoming `FieldWrite`
  - reverse `FieldAlias`
  - cross-page writer action。
  - writer action 的触发组件和证据页面。
- [x] 停止条件：
  - 找到具体 action writer + field evidence。
  - 找到组件 submitField 写入证据。
  - 找到跨页 writer 页面和 action 后停止。
- [x] 明确排除：
  - writer action 的 unrelated reads / navigation。
  - 同页其它 action flows。
  - 与目标字段无关的同 model 写入。

#### Availability

- [x] 允许边/关系：
  - model filter condition。
  - `totalRowCount__` gate。
  - filter 中引用的 param/user/system var。
  - 当前 model 若是 DataFlow，可列出上游 input candidates。
- [x] 停止条件：
  - 已列出决定数据为空/非空的 filter 条件。
  - param/user/system var 只列名称和表达式，不继续扩到所有使用处。
  - DataFlow 上游仅列候选，不展开到全链路，除非用户追问来源。

#### Context

- [ ] 允许边/关系：
  - 目标节点 1-hop incoming/outgoing。
  - 经过 rank 后的 selected 2-hop。
  - action/model/page/component 邻居。
- [ ] 停止条件：
  - 达到 `max_paths` / `max_steps_per_path`。
  - 相同 result 去重。
  - 低置信或无 `why_included` 的路径进入 `candidate_paths`，不进入 `answer_facts`。

### 排序与去重要求

- [ ] 字段级路径优先于模型级路径。
- [ ] 有 `json_path` / `source_file` / `action_id` 的路径优先。
- [ ] proven path 优先于 candidate path。
- [ ] 同一 `result` 只保留最短高置信路径。
- [ ] 同一 action/page/source_file 的重复证据合并。
- [x] 候选和证明必须分开：
  - `proven_paths`
  - `candidate_paths`
  - `rejected_paths`
- [x] 每个 rejected path 必须有 `reject_reason`，例如：
  - `edge_type_not_allowed_for_intent`
  - `outside_current_page_scope`
  - `candidate_only_without_field_origin`
  - `related_context_not_required`

### CLI / Runtime 要求

- [x] `--explain-condition` 新增可选参数：
  - `--intent auto|display|value-source|writer|availability|context`
  - 默认 `auto`
- [x] function-calling / stdio command 支持 `intent` 字段。
- [x] `auto` 模式下：
  - 如果 target 是 `comp:`，默认同时生成短小 `display_facts` 与 `value_source_facts`。
  - 如果 target 是 `field:`，默认生成 `value_source_facts` 与 `writer_facts`。
  - 如果 target 是 `model:`，默认生成 `availability_facts` 与 writer/read summary。
  - 如果用户问题由 wrapper 可传入 intent，则优先使用 wrapper intent。
- [x] compact budget：
  - 默认隐藏 `related_context` 大数组。
  - 默认隐藏 `primary_path`、`value_source_context` 和 traversal debug 明细。
  - 保留 `answer_facts.paths[].steps[]` 的短证据。
- [x] normal/full budget：
  - 输出完整 details。
  - 输出 traversal debug，便于冷脸验收。

### Schema 要求

- [x] 更新 `docs/schema.md`：
  - `summary.intent`
  - `summary.answer_facts_count`
  - `details.answer_facts`
  - `details.traversal_policy`
  - `details.proven_paths`
  - `details.candidate_paths`
  - `details.rejected_paths`
- [x] `AnswerPath.steps[]` 至少包含：
  - `step`
  - `node_id`
  - `node_type`
  - `edge_type`
  - `direction`
  - `field_path`
  - `source_file`
  - `json_path`
  - `why_included`
- [x] 每个 fact 必须包含：
  - `result`
  - `confidence`
  - `evidence_refs`
  - `paths`
  - `missing_evidence`（没有证明时必须显式说明）

### 测试任务清单

- [x] Fixture：Display intent
  - 目标组件自身无 visible，但祖先有 `visibleCondition`。
  - compact `display_facts` 只包含 direct/inherited/expanded gates。
  - 不包含 value source。
- [x] Fixture：ValueSource intent
  - 子组件 `${name}` 继承容器 `dataSet=model1`。
  - `value_source_facts.result` 指向 `data/table1.tbl`。
  - 路径 steps 包含 `inherited_container_data_context`。
- [x] Fixture：Writer intent
  - 字段被 action 写入。
  - `writer_facts` 包含 action id、component trigger、field path。
  - 不包含同 model 其它字段写入。
- [x] Fixture：Availability intent
  - model filter 引用 param/user/system。
  - 输出 filter 条件与引用变量，不扩散到变量所有使用处。
- [ ] Fixture：候选/证明分离
  - DataFlow 字段级 origin 不可证明时，只输出 `candidate_paths`。
  - 不得把 DataFlow input candidate 放进 proven result。
- [x] 真实项目回归：`text41`
  - target：`comp:app/售后.app/绑定车辆/会员已注册.spg|text41`
  - `display_facts` 只包含：
    - 无 direct visibleCondition。
    - inherited `panel35#visibleCondition`。
    - expanded `model11.ISSHOW == 1`。
  - `value_source_facts` 只包含：
    - `bare_symbol = CUSTOMAUTOMYAUTOLIST`
    - `nearest_data_context = sliderpanel2`
    - `field_path = model11.CUSTOMAUTOMYAUTOLIST`
    - `table_source_path = $DATA:/加工表/小程序/绑车.tbl`
    - `proven_physical_input = $DATA:/主数据/fact_autoCustomerAutoRel.tbl`
  - 不得把 `model2.name=text41` 当必要显示条件。
  - 不得把 `CUSTOMAUTOMYAUTOLIST` 当模型或表。
- [x] 真实项目回归：`input3`
  - target：`comp:app/销售.app/销售/合同协议.spg|input3`
  - `value_source_facts` / `writer_facts` 保留跨页 writer 链路。
  - 必须包含 `model22.phoneNumber`、`fact_qwSidebar.phoneNumber`、`action1`、`action4`。
  - compact 输出不得包含大量无关页面 related_context。
- [x] stdio / function-calling 回归
  - `explain_condition` 支持 `intent`。
  - `human=true` 行为不回退。
  - 外部 graphdb + project_dir 不回退。
- [x] Snapshot / 体积基线回归
  - 更新 compact 输出 snapshot。
  - 确认 `answer_facts` 稳定，不因数组顺序随机漂移。
  - `text41 --intent display --budget compact` 已增加真实项目低噪声断言，并已写入 `docs/performance-baseline.md` 作为长期基线。

### Skill / 文档任务

- [x] `SKILL.md` 改为更薄的默认协议：
  - 默认读 `summary` + `answer_facts`。
  - 只有核查时读 `details`。
  - 只有争议或缺证时读 `evidence` / `rejected_paths`。
- [x] function-calling 文档更新：
  - wrapper 应传入 `intent`。
  - 小模型默认不读 `related_context`。
- [x] `docs/schema.md` 更新 M33 字段契约。
- [x] `docs/performance-baseline.md` 增加 M33 compact 输出体积基线。

### 验收目标

- 用户明确指向节点时，compact 输出不再像页面级全量解释。
- 多跳答案不会丢失，但路径数量受 `max_paths` / `max_steps_per_path` 控制。
- 每条进入 `answer_facts` 的路径都有 `why_included` 和停止原因。
- 小模型无需理解全图即可回答 `text41` 显示条件和值来源。
- 候选不会被误报为证明。
- `related_context` 不再成为 compact 输出中的注意力漂移来源。

## M34：DataFlow 内部投影与字段级来源展开

### 背景

M32/M33 已经能处理页面组件裸字段来源：

```text
text41.value = ${CUSTOMAUTOMYAUTOLIST}
→ 最近数据容器 sliderpanel2.dataSet = model11
→ model11.CUSTOMAUTOMYAUTOLIST
→ $DATA:/加工表/小程序/绑车.tbl
→ $DATA:/主数据/fact_autoCustomerAutoRel.tbl
```

但真实项目中，页面 model 经常指向 `modelDataType=DataFlow` 的 `.tbl`。这类 `.tbl` 内部存在 `ModelTable`、`Join`、`Union`、`Filter`、`Output` 等节点，实际字段来源、行可用性条件和输出前过滤条件隐藏在 DataFlow 内部。若只输出 DataFlow 输入表候选，小模型仍会遗漏：

- 字段到底来自哪一个输入节点、哪张物理表。
- Join 只是匹配上下文，还是会过滤行。
- Union 是多分支任一输出，还是字段只来自某一分支。
- DataFlow filter 是 model 数据可用性条件，不是组件 direct visibleCondition。

同时，M33 已经证明 compact 输出必须低噪，不能把完整 DataFlow JSON 或全量内部拓扑交给 AI。M34 因此要实现 **target-relevant DataFlow projection**：按 `intent + target field/model` 只展开和当前问题有关的 DataFlow 事实。

### 核心原则

- `originalNode` / `originalField` 是字段级来源的最高优先级证明。
- `originalNode` 必须先解析为 DataFlow 内部节点 alias，再由该节点的 `moduleTablePath` 落到真实物理表。
- `inputNode` / `inputField` / `exp` / `unionMapArray` / join clauses 只能作为递归或降级证据；缺少字段级 origin 时不得把输入表候选当成 proven。
- Join / Union / filter 必须被压缩成 AI 可读的事实，不输出完整 DataFlow 节点树。
- `value-source` 只解释目标字段来源；`availability` 只解释目标 model 是否有数据；`display` 只在 `totalRowCount__` gate 依赖 model 时引用 availability 摘要。
- compact 输出只能进入 `details.answer_facts` 的短事实；normal/full 才允许审计路径和 rejected/candidate 细节。

### 非目标

- 不做完整 DataFlow 执行器。
- 不模拟聚合、排序、窗口函数、脚本表达式的业务含义。
- 不把所有 DataFlow 节点默认展开给 AI。
- 不把 DataFlow filter 改写成组件自身显示条件。
- 不要求第一阶段覆盖所有复杂表达式；无法证明时必须降级为 candidate，并说明缺失证据。

### 真实项目锚点

- 页面：`app/售后.app/绑定车辆/会员已注册.spg`
- 组件：`text41`
- 容器：`sliderpanel2`
- 页面 model：`model11`
- DataFlow 表：`$DATA:/加工表/小程序/绑车.tbl`
- 目标字段：`CUSTOMAUTOMYAUTOLIST`
- 已验证字段 origin：
  - `dbfield = CUSTOMAUTOMYAUTOLIST`
  - `originalField = 车辆VIN`
  - `originalNode = FACT_AUTOCUSTOMERAUTOREL`
  - `FACT_AUTOCUSTOMERAUTOREL.moduleTablePath = $DATA:/主数据/fact_autoCustomerAutoRel.tbl`
- 已验证输入过滤：
  - `[是否展示] == 1`
  - `[关系类型] == 车主关系`
- 已验证输出前过滤：
  - `[粉丝ID]=$user.WECHAT_UNIONID OR ([使用人粉丝ID] = $user.WECHAT_UNIONID and [人员手机号] = [使用人手机])`

### 数据结构解析任务

- [x] 扩展 DataFlow meta 预解析结构：
  - `alias_map`: alias -> node_id。
  - `id_to_alias`: node_id -> alias。
  - `node_types`: node_id -> `ModelTable` / `Join` / `Union` / `Output` / 其它。
  - `module_table_paths`: node_id -> `moduleTablePath`。
  - `node_fields`: node_id -> field name/dbfield/inputField/inputNode/originalNode/originalField/exp/dimensionPath。
  - `node_filters`: node_id -> filter clauses / exp。
  - `join_clauses`: node_id -> joinType / leftTable / rightTable / clauses。
  - `union_maps`: node_id -> unionMapArray / inputNodes。
  - `output_nodes`: Output 节点及其 inputNodes。
- [x] 字段索引必须同时支持：
  - `name` 匹配，如 `车辆VIN`。
  - `dbfield` 匹配，如 `CUSTOMAUTOMYAUTOLIST`。
  - `inputField` 匹配。
  - 大小写不敏感的 `dbfield` fallback。
- [ ] `originalNode` 解析规则：
  - 先按 alias 精确匹配。
  - 再按 node_id 匹配。
  - 支持别名中带括号或重复后缀的情况，匹配失败时进入 candidate。
- [ ] `originalField` 解析规则：
  - 优先用 `originalField` 找原始节点字段。
  - 若原始节点是 `ModelTable` 且字段不存在于 node fields，可用 `originalField` 直接作为物理表字段名。
  - 若 `originalField` 缺失，降级到 `inputField` / `name` / `dbfield`。
- [x] filter 字段引用解析：
  - 支持 `[字段]`。
  - 支持 `[节点].[字段]`。
  - 支持 filter `clauses[]` 的 `leftExp` / `rightExp` / `rightValue`。
  - 支持 filter `clauses[]` 的自由 `exp`。
  - 识别 `$user.*`、`$param.*`、`param*` 等变量引用，只列引用，不递归扩散。

### 字段来源 projection 任务

- [x] 新增或扩展 DataFlow 字段来源追踪函数，输入：
  - DataFlow model/table path。
  - target output field name/dbfield。
  - intent。
  - budget。
- [ ] 输出 `DataflowFieldOrigin`：
  - `target_field`
  - `dataflow_table_path`
  - `output_node`
  - `output_field`
  - `source_node_alias`
  - `source_node_type`
  - `source_module_table_path`
  - `source_field`
  - `source_dbfield`（能解析则填）
  - `via`: `originalNode/originalField` / `inputNode/inputField` / `unionMapArray` / `expression`
  - `confidence`: `proven` / `candidate`
  - `missing_evidence`
- [ ] `value-source` intent 行为：
  - 只追目标字段相关路径。
  - 遇到 `Join`，只说明目标字段来自左/右/中间输入；Join 条件进入 `join_context`，不抢占 value result。
  - 遇到 `Union`，只列能产生目标字段的分支；每个分支独立标注 proven/candidate。
  - 找到 `ModelTable.moduleTablePath` 后停止字段来源递归。
  - 无字段级 origin 时，只输出 `dataflow_inputs` candidates，不得写入 `proven_physical_input`。
- [ ] `details.answer_facts.value_source_facts` 增加短事实：
  - `dataflow_table`
  - `dataflow_output_field`
  - `physical_source_fields[]`
  - `dataflow_branches[]`
  - `join_context[]`
  - `candidate_inputs[]`

### 可用性 projection 任务

- [x] 新增或扩展 DataFlow availability 追踪函数，输入：
  - DataFlow model/table path。
  - target model。
  - 当前页面 filter / totalRowCount gate 上下文。
- [ ] 输出 `DataflowAvailabilityFacts`：
  - `source_filters[]`
  - `output_filters[]`
  - `join_rules[]`
  - `union_rules[]`
  - `referenced_vars[]`
  - `physical_input_tables[]`
- [x] filter role 分类：
  - `source_filter`: ModelTable 输入节点 filter。
  - `join_condition`: Join 匹配条件。
  - `output_filter`: Output 或输出前节点 filter。
  - `branch_filter`: Union 分支内部 filter。
  - `field_expression`: 字段表达式引用，不直接作为行可用性条件。
- [x] Join 行语义：
  - `InnerJoin`: 左右都必须匹配，影响 row availability。
  - `LeftJoin`: 左表行保留，右表字段可能为空。
  - `RightJoin`: 右表行保留，左表字段可能为空。
  - `FullJoin`: 任一侧可保留，字段可能为空。
  - 未识别类型进入 candidate，并保留原始 joinType。
- [x] Union 行语义：
  - `Union`: 任一分支有数据即可输出。
  - 字段按 `unionMapArray` 标注来自哪些分支。
  - 分支内 filter 必须保留为 branch_filter。
- [ ] `display` intent 行为：
  - 只有当 direct/inherited condition 引用 `model.totalRowCount__` 时，才引用 DataFlow availability 摘要。
  - 不把 DataFlow filter 写成组件 direct/inherited visibleCondition。
  - `data_empty_gates` 可包含展开后的 DataFlow source/output filters，但必须标注 `condition_scope = expanded_from_total_row_count` 或等价字段。

### 图模型与边任务

- [ ] 评估是否新增边类型；若新增，必须同步 `graph.rs` 序列化和测试：
  - `DataflowFieldOrigin`
  - `DataflowFilter`
  - `DataflowJoinCondition`
  - `DataflowUnionBranch`
- [ ] 若第一阶段不新增边类型，也必须在 node meta 中持久化足够 projection 所需字段，确保冷启动 graphdb 查询无需重新读全项目文件。
- [x] 图节点/边不得把 DataFlow 内部节点污染为普通页面 model。
- [x] DataFlow 内部字段节点命名必须稳定：
  - 推荐：`dataflow-field:<table_path>|<node_alias>|<field_name_or_dbfield>`。
  - 物理表字段继续使用既有 `field:<model>.<field>` 或 canonical 字段节点。
- [ ] `--context` 可看到 DataFlow 内部相关边，但 compact explain-condition 不默认展开。

### 输出契约任务

- [ ] 更新 `docs/schema.md`：
  - `details.answer_facts.value_source_facts.dataflow_table`
  - `physical_source_fields[]`
  - `dataflow_branches[]`
  - `join_context[]`
  - `candidate_inputs[]`
  - `details.answer_facts.availability_facts.dataflow_availability`
  - `source_filters[]`
  - `output_filters[]`
  - `join_rules[]`
  - `union_rules[]`
  - `referenced_vars[]`
- [ ] 更新 `docs/function-calling-runtime.md`：
  - function calling 问值来源时传 `intent=value-source`。
  - 问 model 是否有数据或显示条件中有 `totalRowCount__` 时传 `intent=availability` 或 `intent=display`。
  - compact 默认只读 `answer_facts` 的 DataFlow projection。
- [ ] 更新 `SKILL.md` 和已安装 skill：
  - 明确 DataFlow projection 读取顺序。
  - 明确 `details.answer_facts.<fact_block>` 路径，不允许写成 `details.value_source_facts`。
  - 明确 Join/Union/filter 的解释模板。
- [x] compact 输出体积基线：
  - `text41 --intent value-source --budget compact`。
  - `text41 --intent display --budget compact`。
  - `model11 --intent availability --budget compact`。

### 测试任务清单

- [ ] Fixture：`originalNode/originalField` 直连 `ModelTable`
  - 输出字段 `CUSTOMAUTOMYAUTOLIST` 证明到 `fact_autoCustomerAutoRel.tbl.车辆VIN`。
  - `confidence = proven`。
- [ ] Fixture：`originalNode` 指向中间节点
  - 递归追到上游 `ModelTable`。
  - steps 中保留中间节点 alias/type。
- [ ] Fixture：缺少 `originalNode/originalField`
  - 只能输出 candidate input，不得输出 proven physical source。
- [x] Fixture：LeftJoin 字段来自左表
  - value-source result 来自左表。
  - join_context 标注右表只参与匹配或补充字段。
  - availability 标注 `left_rows_preserved_right_fields_nullable`。
- [x] Fixture：InnerJoin
  - availability 标注左右都必须匹配。
  - join_condition 进入 row rule。
- [x] Fixture：Union
  - value-source 按目标字段列出分支来源。
  - availability 标注 `any_branch_can_output`。
  - 不把无目标字段的分支写入 value-source proven。
- [x] Fixture：source/output/branch filters
  - source_filter、output_filter、branch_filter 分类正确。
  - `$user.*` / param 引用只列名，不递归扩散。
- [ ] 真实项目回归：`text41 value-source`
  - 必须输出：
    - `dataflow_table = $DATA:/加工表/小程序/绑车.tbl`
    - `physical_source_fields` 包含 `$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN`
    - `via = originalNode/originalField`
    - `originalNode = FACT_AUTOCUSTOMERAUTOREL`
    - `originalField = 车辆VIN`
  - 不得把 `CUSTOMAUTOMYAUTOLIST` 当表名。
- [ ] 真实项目回归：`text41 display`
  - 仍保留 M31/M33 display 结论。
  - `model11.totalRowCount__ > 0` 展开后补充 DataFlow availability 摘要。
  - DataFlow filter 不得冒充组件 direct visibleCondition。
- [ ] 真实项目回归：`model11 availability`
  - 必须包含：
    - `$DATA:/主数据/fact_autoCustomerAutoRel.tbl`
    - `[是否展示] == 1`
    - `[关系类型] == 车主关系`
    - 输出前 `$user.WECHAT_UNIONID` 过滤。
- [x] stdio 回归：
  - `explain_condition` + `intent=value-source` 返回 DataFlow projection。
  - `explain_condition` + `intent=availability` 返回 DataFlow availability。
  - compact 不返回完整 internal topology。

### 实施切分

- [x] M34.1：DataFlow meta 解析增强
  - 在现有 `DataFlowMeta` 或独立模块中补齐 node/module/filter/join/union 索引。
  - 增加 fixture 单元测试。
- [x] M34.2：字段级 origin projection
  - 实现 `originalNode/originalField` 优先追踪。
  - 接入 `value_source_facts`。
  - 覆盖 `text41 value-source`。
- [x] M34.3：availability projection
  - 收集 source/output/branch filters。
  - 收集 Join/Union 行规则。
  - 接入 `availability_facts` 和 display 的 row-count gate 展开。
- [x] M34.4：输出契约与 skill 同步（schema/function-calling/performance 已完成，SKILL.md 待手动同步）
  - 更新 schema/function-calling/SKILL/performance baseline。
  - 确保小模型读取路径稳定。
- [x] M34.5：冷脸验收与性能检查（全量测试通过，部分 ignored 真实项目测试通过，compact 基线已测量）
  - 跑普通全量测试。
  - 跑 ignored 真实项目测试。
  - 检查 compact 输出体积不回退。

### 小模型执行微任务包

> 下面任务包供 5.3-codex-spark 这类小上下文模型逐包执行。每包只处理一个能力面，必须完成测试和提交后再进入下一包。不要一次性实现 M34 全部内容。

#### M34-P0：只读定位包

- 目标：不改代码，只确认现有入口。
- 允许读取文件：
  - `src/query/dataflow.rs`
  - `src/explain.rs`
  - `src/scanner/tbl.rs`
  - `tests/regression_tests.rs`
- 必须输出：
  - 现有 DataFlow meta 从哪里来。
  - `value_source_context.dataflow_field_origin` 当前在哪里生成。
  - `answer_facts.value_source_facts` 当前在哪里组装。
  - 最小改动应落在哪些函数。
- 禁止：
  - 修改文件。
  - 重构。
  - 新增边类型。

#### M34-P1：DataFlow meta 最小索引包

- 目标：只让已有 DataFlow meta 能解析 `moduleTablePath` 与 output 字段 `originalNode/originalField`。
- 允许修改文件：
  - `src/query/dataflow.rs`
  - 如现有 explain 侧有独立解析逻辑，可只修改 `src/explain.rs` 中对应私有结构。
  - `tests/regression_tests.rs` 或更合适的现有测试文件。
- 必须实现：
  - 字段记录保留 `originalField`、`originalNode`、`inputNode`。
  - DataFlow 节点索引保留 `moduleTablePath`。
  - 支持 output 字段按 `dbfield` 匹配，例如 `CUSTOMAUTOMYAUTOLIST`。
  - 支持 output 字段按 `name` 匹配，例如 `车辆VIN`。
- 暂不实现：
  - filter。
  - Join。
  - Union。
  - availability。
  - schema 文档。
- 测试要求：
  - 增加一个 fixture 或现有 fixture 用例，证明 output field 可读到 `originalNode/originalField`。
  - `cargo check`。
  - 相关测试。
- 提交：
  - `feat: index dataflow field origins`

#### M34-P2：originalNode 直连 ModelTable 来源包

- 目标：把 output 字段通过 `originalNode/originalField` 追到 `ModelTable.moduleTablePath`。
- 允许修改文件：
  - `src/query/dataflow.rs` 或新增 `src/query/dataflow_projection.rs`。
  - `src/explain.rs` 中接入点。
  - 对应测试文件。
- 必须实现：
  - 输入：DataFlow meta + target field。
  - 若 output field 有 `originalNode`：
    - 用 `originalNode` 匹配 node alias。
    - 找到该 node 的 `moduleTablePath`。
    - 输出 proven origin。
  - 若 `originalField` 存在：
    - 作为物理表字段名输出。
  - 若找不到 node 或 moduleTablePath：
    - 输出 candidate/missing_evidence，不得输出 proven。
- 输出字段最小要求：
  - `dataflow_table`
  - `dataflow_output_field`
  - `physical_source_fields[]`
  - `original_node`
  - `original_field`
  - `via = originalNode/originalField`
  - `confidence = proven`
- 暂不实现：
  - 中间节点递归。
  - Join/Union。
  - filter。
- 测试要求：
  - 直连 `ModelTable` proven case。
  - 找不到 `originalNode` candidate case。
  - `cargo check`。
  - 相关测试。
- 提交：
  - `feat: project dataflow original field origins`

#### M34-P3：接入 `value_source_facts` 包

- 目标：让 `--explain-condition ... --intent value-source --budget compact` 在 `details.answer_facts.value_source_facts` 中暴露 DataFlow projection 短事实。
- 允许修改文件：
  - `src/explain.rs`
  - 可能需要引用 P2 的 projection helper。
  - `tests/regression_tests.rs`
- 必须实现：
  - 保留 M33 既有字段：
    - `raw_expr`
    - `bare_symbol`
    - `nearest_data_context`
    - `field_path`
    - `table_source_path`
    - `proven_physical_input`
  - 增加 DataFlow 短事实：
    - `dataflow_table`
    - `dataflow_output_field`
    - `physical_source_fields[]`
    - `candidate_inputs[]`
  - compact 仍不输出完整 DataFlow internal topology。
  - `details.answer_facts.value_source_facts` 是唯一 AI 直读位置；不要新增 `details.value_source_facts`。
- 测试要求：
  - fixture compact value-source 包含新增字段。
  - 真实项目 ignored：`text41 value-source` 包含 `$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN`。
  - `cargo check`。
  - 相关测试。
- 提交：
  - `feat: expose dataflow origins in value facts`

#### M34-P4：DataFlow filter 解析包

- 目标：只解析 filter，不接 Join/Union。
- 允许修改文件：
  - DataFlow projection helper 文件。
  - `src/explain.rs`
  - 测试文件。
- 必须实现：
  - ModelTable 节点 filter clauses：
    - `leftExp`
    - `operator`
    - `rightValue`
    - `rightExp`
  - 自由 `exp` filter。
  - 字段引用提取：
    - `[字段]`
    - `[节点].[字段]`
  - 变量引用提取：
    - `$user.*`
    - `$param.*`
    - `param*`
  - role：
    - `source_filter`
    - `output_filter`
- 暂不实现：
  - Join 行语义。
  - Union 分支语义。
- 测试要求：
  - fixture source_filter。
  - fixture output_filter + `$user.*`。
  - 真实项目可选：`绑车.tbl` 包含 `[是否展示] == 1`。
- 提交：
  - `feat: parse dataflow filters`

#### M34-P5：接入 `availability_facts` 包

- 目标：让 model availability 或 display row-count gate 能看到 DataFlow filter 摘要。
- 允许修改文件：
  - `src/explain.rs`
  - DataFlow projection helper 文件。
  - `tests/regression_tests.rs`
- 必须实现：
  - `details.answer_facts.availability_facts.dataflow_availability`。
  - 包含：
    - `physical_input_tables[]`
    - `source_filters[]`
    - `output_filters[]`
    - `referenced_vars[]`
  - display intent 中，如果组件显示条件引用 `model.totalRowCount__`：
    - 可在 data_empty_gates 或 display_facts 中补充 DataFlow availability 摘要。
    - 必须标注它来自 row-count expansion。
    - 不得标注为 direct/inherited visibleCondition。
- 测试要求：
  - fixture availability。
  - 真实项目 ignored：`model11 availability` 包含 `fact_autoCustomerAutoRel.tbl`、`[是否展示] == 1`、`[关系类型] == 车主关系`、`$user.WECHAT_UNIONID`。
- 提交：
  - `feat: add dataflow availability facts`

#### M34-P6：Join 最小语义包

- 目标：只做 Join 摘要，不改变字段来源主结果。
- 必须实现：
  - 解析 Join node 的 `joinType`、`leftTable`、`rightTable`、`clauses`。
  - 输出 `join_context[]` 到 value-source。
  - 输出 `join_rules[]` 到 availability。
  - 行语义：
    - `InnerJoin` -> `both_sides_required`
    - `LeftJoin` -> `left_rows_preserved_right_fields_nullable`
    - `RightJoin` -> `right_rows_preserved_left_fields_nullable`
    - `FullJoin` -> `either_side_preserved_fields_nullable`
- 测试要求：
  - LeftJoin 字段来自左表。
  - InnerJoin availability。
- 提交：
  - `feat: summarize dataflow join rules`

#### M34-P7：Union 最小语义包

- 目标：只做 Union 分支摘要。
- 必须实现：
  - 解析 `inputNodes` 与 `unionMapArray`。
  - value-source 只列目标字段相关分支。
  - availability 输出 `union_rules[]`，语义为 `any_branch_can_output`。
  - 没有目标字段的分支不得进入 proven value source。
- 测试要求：
  - Union 两分支字段来源。
  - 无目标字段分支被排除。
- 提交：
  - `feat: summarize dataflow union branches`

#### M34-P8：文档与 skill 包

- 目标：同步 AI-facing contract。
- 允许修改文件：
  - `docs/schema.md`
  - `docs/function-calling-runtime.md`
  - `docs/performance-baseline.md`
  - `SKILL.md`
  - `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`
- 必须写清：
  - DataFlow projection 字段都在 `details.answer_facts.<fact_block>` 下。
  - `value-source` 读 `value_source_facts.physical_source_fields[]`。
  - `availability` 读 `availability_facts.dataflow_availability`。
  - Join/Union/filter 是 DataFlow row/value context，不是组件 direct visibleCondition。
- 测试要求：
  - 如有 skill/docs 测试则跑。
  - `cargo check`。
- 提交：
  - `docs: document dataflow projection facts`

#### M34-P9：验收与清理包

- 目标：冷脸验收前自检。
- 必须运行：
  - `cargo check`
  - `cargo test`
  - M34 ignored 真实项目测试。
  - M33 关键 ignored 真实项目测试，防止回退。
- 必须检查：
  - compact 输出未恢复大数组。
  - candidate 不会被写成 proven。
  - `text41 display` 没把 DataFlow filter 当 direct visibleCondition。
  - `text41 value-source` 能说明 `originalNode/originalField`。
- 提交：
  - 若只有测试/文档修正，用 `test:` 或 `docs:`。
  - 若无改动，只输出验收报告，不提交。

### 验收目标

- `text41` 值来源能够从裸字段稳定追到 `fact_autoCustomerAutoRel.tbl.车辆VIN`，并注明 `originalNode/originalField` 证据。
- `text41` 显示条件仍以组件 direct/inherited/row-count gate 为主，不把 DataFlow filter 混成显示条件。
- `model11 availability` 能解释 DataFlow 输入过滤、输出前过滤、Join/Union 行语义。
- Join/Union 只作为 target-relevant projection 输出，小模型无需读完整 DataFlow JSON。
- 字段级来源不可证明时，输出 candidate，不输出 proven。
- compact 输出继续遵守 M33 低噪策略。

## M35：Answer Contract 与工具内思考护栏

### 背景

空上下文 5.4-mini 已能在 `text41 value-source`、`input3 writer` 这类目标明确问题中读懂关键链路，但在弱指向、多跳、需主动追问的问题上仍不稳定：

- `text41 display` 虽然回答了 `panel35.visibleCondition` 与 `model11.ISSHOW == 1`，但混入了 value-source 相关证据，说明只靠 Skill 文字无法稳定隔离证据用途。
- `model11 什么时候有数据` 没有主动执行 page-scoped model availability 查询，漏掉 DataFlow 内部 filters、Join/Union 语义和物理输入表。
- `fact_qwSidebar` 高扇出关系使用 compact 输出后，没有按截断风险升级到 normal，仍倾向给全量结论。

M35 的目标不是继续堆长 Skill，而是把 Skill 中的分析思考模式固化到 CLI/stdio 输出协议里，让模型每次都先读结构化 `answer_contract` / `thinking_frame`，再按工具给出的 `required_followups` 和 `truncation_guard` 行动。

### 目标

- CLI 输出直接告诉模型：当前问题应读哪个 fact block、哪些字段禁止作为主证据、是否必须继续追问。
- 当出现 `totalRowCount__`、裸字段、DataFlow origin 缺失、page-local model、高扇出截断时，工具自动给出下一步命令或完整性状态。
- `--query-page-logic` 对页面关键模型内嵌 availability 摘要，降低弱指向问题对模型主动探索能力的依赖。
- Skill 从命令说明书收敛为“元数据分析思考协议”：先读 contract，再读 facts，再执行 followup。
- CLI 与 stdio/function-calling 字段保持一致。

### 非目标

- 不做自然语言问题理解器；`--advise-query` 只接受结构化 `question-kind`。
- 不把完整 DataFlow JSON 或全量图邻居塞回 compact。
- 不把所有相关上下文都提升为主证据；仍遵守 M33/M34 的 target-centric、low-noise 输出原则。
- 不用 Skill 文本替代代码层契约；Skill 只描述如何消费契约。
- 不对 `model11`、`model22`、`model6` 等局部 model id 做硬编码特殊处理；这些 id 只作为真实项目回归样本。

### 术语与边界说明

- **局部 model id**：页面 `sources[]` 内定义的 id，例如 `model1`、`model6`、`model11`、`model22`。这些 id 在不同页面会重复，单独的 `model11` 没有全局业务语义。
- **page-scoped model target**：由“页面路径 + 局部 model id”组成的稳定目标，例如 `model:app/售后.app/绑定车辆/会员已注册.spg|model11`。凡是问题或证据来自某个页面，都应优先输出这个 target。
- **resolved_model_target**：图内兼容旧查询或规范化后的节点 id，例如 `model:model11`。它只能作为内部定位结果或兼容字段，不应鼓励 AI 在缺少页面上下文时直接查询它。
- **physical_or_dataflow_path**：局部 model 在当前页面实际绑定的数据来源路径，可能是物理表，也可能是 DataFlow `.tbl`。它来自当前页面 metadata 的 source 定义，不来自 model 数字本身。
- **特别覆盖/真实样本**：路线图中提到的 `会员已注册.spg|model11`、`合同协议.spg|model22`、`潜客信息跟进.spg|model6` 只表示这些真实案例必须进入回归测试，不能在代码中写成 `if model_id == "model11"` 这类特殊分支。
- **关键模型**：`key_model_availability` 中的模型不是人工白名单，而是从当前页面条件、显示门控、数据门控、Action 条件、组件读写引用中自动发现。发现规则必须基于当前页面上下文。
- **followup**：`required_followups` 是“当前输出不足以完整回答时必须继续执行的查询”，不是普通 next query 推荐；`must_run_for_complete_answer=true` 时，模型不应直接给最终确定结论。
- **compact 摘要**：compact 可以给方向性结论，但当 `truncation_guard.safe_to_answer_full_relationships=false` 时，不能回答“全部/有哪些完整关系”。此时必须升级到建议 budget。

### 任务清单

- [ ] M35.0：模块化拆分前置包
  - 目标：先拆低耦合模块，避免 M35 新字段继续堆入 `src/explain.rs` / `src/query.rs`。
  - 第一阶段只做无行为变更拆分，不改变 JSON 输出契约。
  - 拆分阶段：
    - [x] M35.0a：新增 `src/answer_contract.rs`，迁移 `TraversalIntent` 与 fact block 启用规则，并通过 `explain` re-export 保持旧调用路径。
    - [x] M35.0b：新增 `src/model_scope.rs`，迁移 page-scoped model target 解析、页面后代扫描、DataFlow path 匹配和页面内局部 model resolver。
    - [ ] M35.0c：新增 `src/followup.rs`，在 required followup 生成器有实际调用点时落代码。
    - [ ] M35.0d：拆分 `src/query/page_logic.rs` 与 `src/query/model.rs`，避免 `src/query.rs` 继续膨胀。
  - 拆分原则：
    - 先迁移纯类型和纯函数。
    - 每次迁移都必须有调用点，避免新增未消费代码。
    - 不为拆分而改变输出字段。
    - 不引入循环依赖。
  - 验证：
    - `cargo check` 0 warning。
    - 相关 regression tests 通过。
  - 提交：
    - `refactor: split answer contract primitives`

- [ ] M35.1：统一 `answer_contract` schema
  - 所有核心查询输出增加顶层或 `details` 内稳定字段 `answer_contract`。
  - 最小字段：
    - `intent`
    - `target_scope`
    - `primary_fact_path`
    - `forbidden_fact_paths[]`
    - `completion.status`
    - `completion.missing[]`
    - `completion.next_commands[]`
  - `completion.status` 枚举：
    - `complete`
    - `needs_followup`
    - `partial_due_to_truncation`
    - `partial_due_to_unresolved_target`
    - `partial_due_to_missing_origin`
  - 覆盖命令：
    - `--explain-condition`
    - `--query-page-logic`
    - `--query-model`
    - `--context`
  - 测试：
    - fixture JSON 断言 `primary_fact_path` 指向当前 intent 的 fact block。
    - display intent 的 `forbidden_fact_paths` 包含 value-source 相关路径。
    - writer intent 的 `primary_fact_path` 指向 `details.answer_facts.writer_facts`。

- [ ] M35.2：`thinking_frame` 输出
  - 输出模型可直接照读的分析框架：
    - `question_kind`
    - `target_scope`
    - `answer_with`
    - `do_not_use_as_primary_evidence[]`
    - `required_followups[]`
    - `completion_status`
  - 与 `answer_contract` 保持语义一致，但面向 AI 可读。
  - 测试：
    - `text41 --intent display` 的 `thinking_frame.answer_with` 必须是 display facts。
    - `text41 --intent value-source` 的 `thinking_frame.answer_with` 必须是 value-source facts。
    - 不同 intent 下 `do_not_use_as_primary_evidence` 不同。

- [ ] M35.3：`required_followups` 生成器
  - 触发条件：
    - display 条件引用 `modelX.totalRowCount__`。
    - availability 目标是 page-local model，且当前输出尚未包含其完整 availability facts。
    - page-local model 可解析到 DataFlow，但当前输出尚未包含 DataFlow filters / physical inputs / Join/Union 摘要。
    - value-source 遇到裸字段 `${FIELD}` 且未完成最近数据容器 `dataSet` 解析。
    - value-source 遇到 DataFlow table 但缺字段级 proven origin。
    - writer 链路只到 page-local model，未到物理表字段。
    - compact 输出发生截断且用户问题需要全量关系。
  - 每条 followup 包含：
    - `reason`
    - `command`
    - `must_run_for_complete_answer`
    - `expected_fact_path`
  - 测试：
    - `text41 display` 必须生成 page-scoped `model11 availability` followup。
    - 裸字段无法证明 origin 的 fixture 必须生成 value-source followup 或 missing origin 状态。
    - compact 截断的 model query 必须生成 normal budget rerun。

- [ ] M35.4：intent 证据隔离硬化
  - `display` intent 默认不输出 value-source 细节，或仅以 `answer_usage=not_primary_evidence` 放入 related/supporting summary。
  - `value-source` intent 不把 blocking/display condition 当主证据。
  - `availability` intent 不把 unrelated same-name model 条件当主证据。
  - `writer` intent 不把普通 Reads 当生成动作，除非后续有 FieldAlias/FieldWrite/ActionWrites。
  - 测试：
    - `text41 display` 输出不得让 value-source 路径进入 primary evidence。
    - answer contract 的 forbidden path 与实际输出位置一致。

- [ ] M35.5：page-scoped model resolver 标准化
  - 解析原则：
    - 从表达式提取 `modelX.field` 或 `modelX.totalRowCount__`。
    - 读取该表达式所属的 `source_file` / page path。
    - 在该页面 metadata 的 `sources[]` 中查找 `id == modelX`。
    - 生成 `page_scoped_target = model:{page_path}|{modelX}`。
    - 再解析该 source 的物理表或 DataFlow 表路径。
  - 禁止：
    - 禁止按 `model11`、`model22`、`model6` 等具体字符串分支。
    - 禁止在缺少 page path 时把 `model:model11` 视为唯一可靠目标。
  - 所有出现局部 model 引用的位置补充：
    - `local_model_id`
    - `page_scoped_target`
    - `resolved_model_target`
    - `physical_or_dataflow_path`
    - `scope_warning`
  - 真实项目回归样本应覆盖 `model11.totalRowCount__`、`model22.phoneNumber`、`model6.phoneNumber`，但实现必须适用于任意页面局部 model id。
  - 测试：
    - `text41 display` 中 `model11.totalRowCount__` 能给出 `model:app/售后.app/绑定车辆/会员已注册.spg|model11`。
    - `input3 writer` 中 `model22.phoneNumber` 能保留 page-local 到 physical field 的别名链。

- [ ] M35.6：`totalRowCount__` availability 自动展开
  - 当 display/condition 中引用 `modelX.totalRowCount__`：
    - 自动展开当前页面 model filter。
    - 自动生成 model availability followup。
    - 如果 model 指向 DataFlow，输出 target-relevant DataFlow availability 摘要。
  - 必须区分：
    - inherited display gate
    - expanded data gate
    - DataFlow availability context
  - 测试：
    - `text41 display` 包含 inherited `panel35` 与 expanded `model11.ISSHOW == 1`。
    - `model11 availability` 包含 DataFlow filters、Join/Union、physical inputs。
    - DataFlow filters 不得被标成 direct visibleCondition。

- [ ] M35.7：`--query-page-logic` 内嵌 `key_model_availability`
  - 页面级输出增加 `key_model_availability[]`。
  - 关键模型发现规则：
    - display / hidden / disabled 条件中引用 `modelX.totalRowCount__` 或模型字段。
    - data source filter 中引用模型字段或组件字段。
    - action condition / conditionExp 中引用模型字段。
    - 页面组件 value/exp 明确读取 page-local model 字段。
  - 对由上述规则自动发现的模型，输出：
    - `page_scoped_target`
    - `direct_filters[]`
    - `dataflow_table`
    - `source_filters[]`
    - `output_filters[]`
    - `physical_inputs[]`
    - `join_rules[]`
    - `union_rules[]`
    - `row_semantics`
  - 边界：
    - 不输出全页面所有 sources；只输出与页面关键条件、数据门控或用户目标相关的模型。
    - 不把同名局部 model 的其他页面条件混入当前页面。
  - 测试：
    - `会员已注册.spg` page logic 中必须包含 `model11` availability 摘要。
    - 摘要必须包含 `$DATA:/加工表/小程序/绑车.tbl`、`[是否展示] == 1`、`$user.WECHAT_UNIONID`。

- [ ] M35.8：裸字段与 DataFlow origin contract
  - 对 `${FIELD}` 输出硬护栏：
    - `bare_symbol_is_not_table=true`
    - `candidate_inputs_are_not_proven=true`
  - DataFlow 字段来源统一输出：
    - `dataflow_table`
    - `dataflow_output_field`
    - `original_node`
    - `original_field`
    - `proven_physical_input`
    - `candidate_inputs[]`
    - `origin_confidence`
  - 测试：
    - `text41 value-source` 必须从 `CUSTOMAUTOMYAUTOLIST` 追到 `sliderpanel2 -> model11 -> fact_autoCustomerAutoRel.tbl.车辆VIN`。
    - candidate-only fixture 不得输出 proven physical input。

- [ ] M35.9：high-fanout `truncation_guard`
  - compact 输出增加：
    - `truncation_guard.is_complete`
    - `truncation_guard.safe_to_answer_full_relationships`
    - `truncation_guard.required_budget_for_complete_answer`
    - `truncation_guard.truncated_sections[]`
    - `recommended_rerun`
  - high-fanout 判定建议：
    - 任一 readers/writers/related/dataflow 数组超过 compact limit。
    - 或同一模型跨页面读写数量超过普通样本展示能力。
    - 具体阈值应复用现有 budget/limit 配置，不另造散落常量。
  - 高扇出模型 compact 改为分组摘要：
    - `readers_by_page`
    - `writers_by_page`
    - `writes_by_field`
    - `consumed_by_dataflow_summary`
    - `sample_readers`
    - `sample_writers`
  - 边界：
    - 分组摘要用于方向判断，不等价于完整明细。
    - `safe_to_answer_full_relationships=false` 时，回答“有哪些全部关系”必须 rerun normal/full。
  - 测试：
    - `fact_qwSidebar --budget compact` 必须标记不能回答全量关系。
    - `fact_qwSidebar --budget normal` 可用于完整关键读写验收。

- [ ] M35.10：轻量 `--advise-query`
  - 新增结构化规划命令：
    - `--advise-query`
    - `--page <PAGE>`
    - `--target <TARGET>`
    - `--question-kind <display|value-source|availability|writer|page-logic|model-relationships>`
  - target 语义：
    - 如果 target 是组件 id，必须结合 `--page` 生成 `comp:{page}|{component}`。
    - 如果 target 是局部 model id，必须结合 `--page` 生成 page-scoped model target。
    - 如果 target 是物理表/model 名，允许生成 `--query-model`，但应标注它不是页面局部模型。
  - 输出：
    - 推荐主命令。
    - 推荐 budget。
    - 是否需要 graphdb。
    - `primary_fact_path`。
    - `forbidden_fact_paths[]`。
    - followup 规则。
  - 不做自然语言问题分类，只消费结构化参数。
  - 测试：
    - `text41 + display` 推荐 explain-condition display。
    - `model11 + availability` 推荐 page-scoped model availability。
    - `fact_qwSidebar + model-relationships` 推荐 query-model normal 或 compact+guard。

- [ ] M35.11：stdio/function-calling 同步
  - stdio 输出同步包含：
    - `answer_contract`
    - `thinking_frame`
    - `required_followups`
    - `truncation_guard`
  - function-calling 文档同步更新。
  - CLI/stdio 字段一致，不能出现 CLI 有 contract、stdio 缺 contract。
  - 测试：
    - stdio `query_page_logic` 与 CLI 在 contract 关键字段上等价。
    - stdio `query_model fact_qwSidebar` 能输出 truncation guard。
    - stdio `explain_condition text41 display` 能输出 forbidden fact paths。

- [ ] M35.12：Skill 重写为“元数据分析思考协议”
  - 压缩命令说明，突出固定流程：
    - 识别 intent。
    - 读取 `answer_contract`。
    - 只读 `primary_fact_path`。
    - 执行 `required_followups`。
    - 遇到 `truncation_guard.safe_to_answer_full_relationships=false` 必须升级 budget。
    - 回答中声明使用的 evidence block。
  - 明确禁止：
    - value-source 回答 display。
    - related_context 回答必要条件。
    - candidate_inputs 回答 proven source。
    - compact sample 回答全量 readers/writers。
  - 同步：
    - 仓库 `SKILL.md`
    - `/Users/wuhaocheng/.codex/skills/metadata-checker/SKILL.md`
    - `docs/schema.md`
    - `docs/function-calling-runtime.md`

- [ ] M35.13：真实项目评测与失败分类
  - 使用 `tests/fixtures/corpus/ai_eval/xiaoshouyi_large_real_cases.json` 作为验收集。
  - 增加或记录失败分类：
    - `wrong_intent`
    - `missed_followup`
    - `used_forbidden_fact_path`
    - `ignored_truncation_guard`
    - `confused_page_scoped_model`
  - 5.4-mini 空上下文验收重点：
    - `text41 display` 不再混用 value-source 证据。
    - `model11 availability` 必须拿到 DataFlow filters、Join/Union、physical inputs。
    - `fact_qwSidebar` high-fanout 不得在 compact 截断上给全量结论。

### 验收目标

- 空上下文 5.4-mini 能按 `answer_contract` 选择证据域，回答中不再把 value-source 当 display evidence。
- `text41 display` 输出明确：自身无 direct visibleCondition，继承 `panel35.visibleCondition = model11.totalRowCount__ > 0`，并展开 `model11.ISSHOW == 1`。
- `text41 value-source` 输出明确：裸字段不是表名，链路为 `sliderpanel2 -> model11.CUSTOMAUTOMYAUTOLIST -> $DATA:/加工表/小程序/绑车.tbl -> fact_autoCustomerAutoRel.tbl.车辆VIN`。
- `model11 availability` 即使从 page logic 弱指向问题进入，也能拿到 DataFlow filters、output filters、physical inputs、Join/Union 语义。
- `input3 writer` 保留 `model22.phoneNumber -> fact_qwSidebar.phoneNumber -> 潜客信息跟进.spg action1/action4` 字段级跨页链路。
- `fact_qwSidebar` compact 输出能阻止模型给全量关系结论，并明确推荐 normal budget。
- CLI 与 stdio/function-calling 的 contract 字段一致。
- docs/schema、function-calling 文档、Skill 与测试同步更新。
