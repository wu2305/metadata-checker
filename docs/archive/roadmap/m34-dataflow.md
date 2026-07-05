# M34：DataFlow 内部投影与字段级来源展开

| milestone | M34 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

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
