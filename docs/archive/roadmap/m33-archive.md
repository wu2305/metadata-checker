# M33：目标节点中心的受控多跳因果遍历

| milestone | M33 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

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
