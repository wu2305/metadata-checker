# M35：Answer Contract 与工具内思考护栏

| milestone | M35 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

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

- [x] M35.0：模块化拆分前置包
  - 目标：先拆低耦合模块，避免 M35 新字段继续堆入 `src/explain.rs` / `src/query.rs`。
  - 第一阶段只做无行为变更拆分，不改变 JSON 输出契约。
  - 拆分阶段：
    - [x] M35.0a：新增 `src/answer_contract.rs`，迁移 `TraversalIntent` 与 fact block 启用规则，并通过 `explain` re-export 保持旧调用路径。
    - [x] M35.0b：新增 `src/model_scope.rs`，迁移 page-scoped model target 解析、页面后代扫描、DataFlow path 匹配和页面内局部 model resolver。
    - [x] M35.0c：不新增空 `src/followup.rs`；required followup 生成器在 M35.3 出现实际调用点时再落模块，避免 dead code。
    - [x] M35.0d：拆分 `src/query/page_logic.rs` 与 `src/query/model.rs`，避免 `src/query.rs` 继续膨胀。
    - [x] M35.0e：拆分 `src/explain/importance.rs` 与 `src/explain/evidence.rs`，迁移重要性分类与 evidence 提升 helper。
    - [x] M35.0f：拆分 `src/explain/condition_facts.rs`，迁移条件收集、条件去重、值来源上下文、answer facts 与路径分区 helper。
    - [x] M35.0g：拆分 `src/explain/handlers.rs`，迁移图节点 explain handler；`src/explain.rs` 只保留单文件解释、条件输出装配和 handler 分发。
    - [x] M35.0h：继续拆分 `src/explain/handlers/` 子模块，将 condition、component/action、model/field、page/dataflow handler 分组维护。
    - [x] M35.0i：继续拆分 `src/explain/condition_facts/` 子模块，将 conditions、value_source、answer_facts、path_partition 分组维护。
    - [x] M35.0j：拆分 `src/query/page_logic/graph_collect.rs` 与 `src/query/page_logic/metadata.rs`，迁移页面节点递归收集和原始 `.spg` 文件补充元数据读取。
    - [x] M35.0k：拆分 `src/query/page_logic/path_summary.rs`，迁移主链路发现、字段因果路径保底、路径选择和跨页旁路上下文补充。
    - [x] M35.0l：拆分 `src/query/page_logic/prerequisites.rs`，迁移页面级条件 prerequisite 构建、分类和 impact 排序。
    - [x] M35.0m：拆分 `src/query/page_logic/diagnostics.rs`，迁移风险诊断、主路径截断降级、旁路统计和 evidence 采样上限。
    - [x] M35.0n：拆分 `src/query/page_logic/evidence.rs`，迁移 PageLogic evidence 采样和 next_queries 生成。
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

- [x] M35.1：统一 `answer_contract` schema
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

- [x] M35.2：`thinking_frame` 输出
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

- [x] M35.3：`required_followups` 生成器
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

- [x] M35.7：`--query-page-logic` 内嵌 `key_model_availability`
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

- [x] M35.9：high-fanout `truncation_guard`
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

- [x] M35.10：轻量 `--advise-query`
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

- [~] M35.11：stdio/function-calling 同步（explain-condition / advise-query 已同步，query-page-logic / query-model / context 待同步）
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
