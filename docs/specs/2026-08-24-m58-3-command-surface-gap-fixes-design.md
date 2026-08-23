# M58.3 缺口修复与可观测性打底设计

> 状态：**draft**（2026-08-23/24 独立评审三轮后修订，修订点见文末「评审修订记录」）
> 里程碑：M58.3（三动词命令表面收敛，active）
> 配套计划：`docs/plans/2026-08-24-m58-3-command-surface-gap-fixes-plan.md`（多 PR 硬规则要求，
> 见 `docs/governance/planning.md:34`）
> 上游证据：[M58 journal 2026-08-23 节](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)（5 项缺口实跑记录）+
> `docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/`（逐 case 证据产物）
> 根因分析：2026-08-23 三路并行代码解剖（scanner/图构建、查询/解释、横切指标），结论见本文第 1 节。

## 1. 根因分析结论（修复的事实基础）

5 个上层缺口不是孤立 bug，是三个系统性模式的切面：

- **模式 1：静默降级惯例。** redb 反序列化失败静默跳边（`graph_redb.rs:227/237/582/592`）、
  `.ok().flatten()` 把图查询错误折叠成「无数据」、62 处 `unwrap_or("?")`、scanner 对
  未识别类型 `_ => {}` 静默丢弃。错误答案以 high confidence 正常输出。v2 持久化路径
  细分两层：`graph_redb.rs:184-191` 的 hydrate 失败其实已写 `v2_hydrate_warning` 并有
  访问器（`:267`），但**全仓无生产调用方**，只有测试断言它为 `None`——造好的管道没接；
  真正静默的是 `:178` 的 `if let Ok(Some(layout))`，`read_v2_layout` 报错与返回 `None`
  被折叠成同一条路径。
- **模式 2：图 schema 缺「页面内模型」身份，且不止 model 节点。** `model:modelN` 全局按名
  合并（`scanner/spg.rs:149` + upsert 覆盖）；**字段节点同样全局键控**
  （`field:<model>.<field>`，构造点 `spg.rs:150/:378/:1415`），两页 `model6.name` 即使
  model 节点分开也仍塌成一个 field 节点；`route.rs:432` 剥掉页面段与 `page_logic.rs:115`
  构造同格式直接矛盾；`model_scope.rs:5` 把真实客户表名硬编码进通用打分。comp→comp
  Contains 边从未建，`component_ancestor_chain` 是恒返回空的结构性死代码。
- **模式 3：白名单驱动 + 各自手写。** 容器子键只枚举 `components/panels/steps/comps`
  （实测 pin 语料 1049 处 `panel`/`columns`/`tabs` 等组件对象被静默丢弃，逐键逐形态
  测量见附录 A）；value-source 只认单裸 `${FIELD}`；写边 meta 缺 conditionExp；六个
  facts 块各自手写信封。

**测试覆盖缺口的精确表述**（第三轮评审更正）：`m58_3_command_surface_tests.rs:18-38`
已启动真实二进制并对 fixture 项目构建 redb graphdb；`scanner_tests.rs`（20 处
`GraphDB::open`）、`graph_store_tests.rs:11/:46`、`m53_redb_v2_hydrate_tests.rs` 都走
redb 真实路径。真正缺的是两类：**(a) 坏行 hydrate**（redb 里塞入反序列化失败/悬挂端点的
记录，验证加载期的计数与降级信号）；**(b) pin 真实语料的端到端行为**（构建 + 查询 +
增量刷新的全链路断言）。「白盒走 MemoryGraphStore 所以静默丢边活到评测」的说法对
(a) 成立、对既有 happy-path 测试不成立，本文一律按 (a)/(b) 两类精确引用。

## 2. 范围

### Phase 1：可观测性打底（元修复，先于一切单点修复）

**统一诊断信封。** 所有新增与既有诊断共用一个信封，字段写死：

```
{ code, severity, count, sample_location, answer_impact, first_seen_phase }
```

- `code`：稳定枚举字符串（初值见下表），进契约测试，不允许实现期自由发挥；
- `severity`：`warning` / `error`；`answer_impact`：`none` / `partial` / `blocking`；
- `sample_location`：首个样例的文件/节点定位（计数即可，不逐条刷屏）；
- 落点三处都必须出现：构建输出 envelope、`--status` 的 `RuntimeStatus`、查询响应的
  `diagnostics` 数组。

**初始诊断类（Phase 1 落地）**：

| code | 触发点 | answer_impact |
|------|--------|---------------|
| `SCANNER_UNRECOGNIZED_CONTAINER_KEY` | 容器子键/对象形态未识别（F1 修复后转永久安全网） | partial |
| `SCANNER_DUPLICATE_COMPONENT_ID` | 同页重复组件 id（`spg.rs:82-103`） | partial |
| `GRAPH_DB_NODE_DECODE_FAILED` | `graph_redb.rs:227` | partial |
| `GRAPH_DB_EDGE_DECODE_FAILED` | `:237` | partial |
| `GRAPH_DB_EDGE_DANGLING_ENDPOINT` | `:238-239` 悬挂边（实际更常见，与反序列化失败拆开） | partial |
| `GRAPH_DB_V2_LAYOUT_UNREADABLE` | `:178` 的 `read_v2_layout` Err/None 折叠路径 | none |
| `GRAPH_DB_PARTIAL_HYDRATE` | 上面三类 hydrate 损失的**汇总闸门**：任一计数 > 0 即在该实例**所有**后续响应置顶（悬挂边无法归因到具体查询，不做按目标归因），confidence 降为 partial——缺失的边会把「未知」变成假的「无关系」，只计数不够 | partial |
| `PAGE_SCOPED_TARGET_FALLBACK` | 页面限定回退全局模型（真实回退点 `page_logic.rs:155-170/:300-311`；既有信号 `page_scoped_target_not_resolved_fallback_to_global_model` 归入此 code） | partial |

**传播与归属**（评审 P1-4 要求的硬设计）：

- runtime 加载诊断目前只在局部构造、不存进 `GraphRuntime`（`runtime.rs:338`），
  `RuntimeStatus` 无诊断字段（结构体 `runtime.rs:260`，`status()` `:1222`）——Phase 1 把加载诊断**存入 GraphRuntime**，
  `status()` 透出；查询响应从 runtime 取同一份，**不在查询层另造第二套信号**。
- 查询时聚合已有先例（`query/page_logic/diagnostics.rs:305` 的 `UNKNOWN_ACTION_TYPE`）：同类诊断的
  归属规则 = **加载期归 GraphRuntime，查询期归 page_logic diagnostics**，同一 code
  只在一处产生，跨来源不去重（code 本身即来源标识）。
- `graph_redb.rs:267` 的 `v2_hydrate_warning` 死访问器接进上述信封，不新建平行管道。

**坏行 hydrate 测试**（覆盖缺口 a）：构造含坏节点行/坏边行/悬挂边的 redb，断言三类
计数 + `GRAPH_DB_PARTIAL_HYDRATE` 出现在 status 与查询响应中。

### Phase 2：P0 单点修复

| # | 缺口 | 根因位置 | 修法 |
|---|------|---------|------|
| F1 | 容器子键白名单丢组件（1049 处，逐键逐形态测量见附录 A） | `superpage/mod.rs:313-324,543-554`、`scanner/spg.rs:107` | **形态感知递归**，不扩展枚举：递归规则为「value 是数组、非空、且**每个**元素都是带字符串 `id` 和字符串 `type` 的对象 → 子组件数组」。多态键安全：字符串形态（如 action 的 `panel: "nextPage"`，`raw_types.rs:87`）天然不匹配；混合形态数组（部分元素缺 `id`/`type`）整体判非组件、进 `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 计数；已知非组件键进排除列表（初值以实测为准，PR2 描述登记）。`RawComponent` 加 `#[serde(flatten)]` extra map（`raw_types.rs:210-217` 无 `deny_unknown_fields`，兼容），scanner 侧 `spg.rs:107` 裸 `Value` 递归平行修改；json_path 逐层保留。同页重复组件 id 诊断（`spg.rs:82-103`）。**证据性质更正**：F1 不是 journal 五个实跑缺口之一，其证据是语料测量（附录 A），验收按「测量数 → 0 丢失」而非「重放命令」 |
| F2 | 无表达式组件的祖先条件继承断（button13） | `conditions.rs:379-416` 不读节点 meta json_path、`:429` 硬编码 `.components[` | `component_json_paths` 补读节点 meta；`is_ancestor_json_path` 放宽到全部容器子键；scanner 补建 comp→comp Contains 边。**不变式与选父规则见说明 A** |
| F3 | `${IF(...)}` 表达式模型引用截断 | `superpage/expr_ast.rs:204-219` tokenizer 吞整个 `${}` + `:823-831` `classify_identifier` 盲 `split('.')` | **修复点改到能合并递归结果的层**：`classify_identifier` 只返回单个 `RefType`（`:818-819`），无法扩展 refs/resolved_refs/diagnostics——内嵌表达式的处理移到 tokenizer 的 `${}` 分支或 `extract_refs_from_ast`（`:644-649`，已有 refs/resolved_refs/diagnostics 三个可变累积参数），在其中对 inner 递归走 AST 并**合并**结果。纯点分路径保持现状直出。**已知边界**：tokenizer 贪婪吃到第一个 `}`，嵌套 `${...${...}...}` 词法层已截断，本修复只覆盖单层；嵌套形态若实测出现（Phase 1 计数器暴露）另立项。测试矩阵：普通 `${model.field}` 兼容、`${IF(...)}` 多引用、缺右花括号、嵌套调用、纯字面量 |
| F4 | 页面限定降级 / 协议矛盾 | `route.rs:429-434`（剥页面段）vs `page_logic.rs:115-122`（构造页面段）；真实回退点 `page_logic.rs:155-170/:300-311` | **方向已定**：保留页面段，`route.rs:432` 的 `by_overqualified` 档对 model/field 前缀禁用；回退接 `PAGE_SCOPED_TARGET_FALLBACK`；协议统一随 Phase 3 落地（归属 PR4b）。硬编码表名移除的平局替代见说明 B |
| F5 | calc 表达式不进 value-source | `explain/condition_facts/value_source.rs:235-247` 只认单裸 `${FIELD}` | **硬依赖 F3**（calc case 的 reads/lineage 为空正是 `${CASE WHEN...}` 提取失败的下游，证据 `facts_review.md:6`——F3 不落地 F5 无米下锅，PR 次序已保证 PR3 < PR5）。图里已有带 `source_expr` 的 Reads 边（`spg.rs:559-585`），透出规则与输出契约见说明 C |
| F6 | 写动作 conditionExp 不暴露 | 写边 meta 不带（`spg.rs:842-851,872-881`），输出层不回查 action 节点 | scanner 把 `condition`/`conditionExp` 抄进写边 meta（与 action 节点 meta 同源）。输出契约见说明 C；Phase 3 双写落地后抄到页面局部写边 |

**说明 A（F2 的祖先链不变式）**：`spg.rs:512` 给**每个**组件（含深层嵌套）建 page→comp
Contains 边，补 comp→comp 后嵌套组件有两条 incoming Contains；`component_ancestor_chain`
（`conditions.rs:308-320`）用 `.find()` 取第一条且 Page 终止（`:325-327`）——不修选父逻辑
就会顺序依赖（redb 与 `MemoryGraphStore` 迭代序可能不同，落进覆盖缺口 a 的盲区）。

**写死的不变式**：每个嵌套组件**恰好一个** Component 父（scanner 建边时保证，重复/多父
进 `SCANNER_DUPLICATE_COMPONENT_ID` 或新诊断）；`component_ancestor_chain` 选父**按
NodeType 确定性排序**——Component 父优先、Page 仅兜底，与迭代序无关。page→comp 边保留
（按 page Contains 枚举组件的消费者不动）。备选「page 只 Contains 根组件」被否决：改动面
覆盖所有按 page 枚举组件的消费者，收益与确定性选父相同。comp meta 已有 `parent_id`
（`spg.rs:469-470`）作为平行事实通道，选建边不读 meta 的取舍同前（图为唯一事实源）。

**PR2 测试电池**：唯一父断言、无环、近到远序稳定、page_logic 遍历路径、增量重建后
祖先链不变（覆盖缺口 b 的增量维度）。

**说明 B（F4 硬编码表名的平局替代）**：`FACT_AUTO_CUSTOMER_AUTO_REL_TABLE`
（`model_scope.rs:5`）在 `pick_best_dataflow_candidate` 的三处调用点作为
`preferred_input_path` 打分偏好（`:296/:304/:388`）。Phase 3 消歧「哪个局部模型」，而
「哪个 DataFlow 喂这条路径」是另一个轴。移除后三处逐一改为：`preferred_input` 由页面段
解析结果提供（命中页面局部模型时取其 DataflowOutput 指向的物理路径）；无页面上下文或
仍平局时**如实交回候选**（同歧义契约），不用客户表名偏好替调用方猜。

**说明 C（F5/F6 的事实输出契约）**——以下字段进契约测试，不允许实现期自由发挥：

- F5 value-source facts：`raw_expr`（string）、`refs`（数组，元素 `{raw, kind, target}`）、
  `source_field`（产出该 Reads 边的组件字段名）；**按 `source_field` 过滤**，display/enable
  类表达式不进 value-source facts；同一表达式产生多条 Reads 边时**按 raw_expr 分组去重**
  （一条事实 + 引用清单，不重复计边）；证据不足时给 `VALUE_SOURCE_EVIDENCE_MISSING`
  诊断而非空数组。
- F6 writer facts：`condition` 与 `conditionExp` 是两个字段（前者结构化条件对象、后者
  表达式字符串，语义不同源，不得合并）；每条 writer 事实带 `evidence_refs`（指向边/节点
  id）与 `confidence`；Phase 3 双写后**页面局部写边 + 物理聚合写边计为一个逻辑 writer**
  （计数去重按 (action, 物理字段) 键），缺失 conditionExp 的 writer 给
  `WRITER_CONDITION_MISSING` 诊断。

### Phase 3：页面内模型节点身份——完整局部子图（核心设计，不推迟）

#### 为什么必须现在做

页面内模型是三重身份的交汇点，缺一维全部失真：

- **查询逻辑**：`model:modelN` 的 availability gates 应该来自**本页这份** filter，不是别页合并进全局节点的表路径；
- **写入场景**：同一个动作写哪张表、带什么 conditionExp 门控，是页面级事实；
- **同表异 filter**：绑定车辆.spg 的 model6/7/8 同指 `fact_customAutoMyAutoList.tbl` 但 filter 完全不同——全局按名合并后，三个逻辑视图塌成一个节点，谁的 filter 都答不对。
  证据：`docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/xiaoshouyi_bindcar_input5_pending_reason_calc/facts_review.md:7`。
- **增量刷新误删/泄漏（比答案不准更硬）**：`model:modelN`/`field:modelN.x` 进该页
  `node_ids`（内嵌 DataFlow 路径 `spg.rs:370-371/387-388`；dwtable source 路径
  `spg.rs:1485-1486`——model6/7/8 的主流触发路径），indexer 按 `previous_node_ids`
  批删（`scanner/indexer.rs` `apply_incremental_changes`）——**编辑任意一页会删掉别的
  页也在用的跨页共享节点**；反方向，read（`spg.rs:196/:219`）/write（`:260/:283`）路径
  经 `ensure_model_field` 建的节点**不进** `node_ids`，增量下是泄漏。两个方向同源于
  「节点归属粒度 ≠ 文件粒度」，分段后一并消解。验收：增量改一页后另一页 model 事实不掉。

#### 完整 id 文法与迁移矩阵（评审 P1-1：分段必须覆盖整条局部子图，不是只改 model 节点）

| 节点类 | 现状 id（全局塌陷） | 新 id | 构造点 |
|--------|--------------------|-------|--------|
| 页面局部模型 | `model:modelN` | `model:<PAGE>\|<modelN>` | `spg.rs:149` |
| 局部字段 | `field:modelN.x` | `field:<PAGE>\|<modelN>.<x>` | `spg.rs:150`（`ensure_model_field`） |
| 内嵌 DataFlow 模型 | `model:<sourceId>`（meta 仅记 `embeddedIn`，:368） | `model:<PAGE>\|<sourceId>` | `spg.rs:360` |
| 内嵌 DataFlow 字段 | `field:<sourceId>.<dim>` | `field:<PAGE>\|<sourceId>.<dim>` | `spg.rs:378` |
| 隐式行数字段 | `field:modelN.totalRowCount__` | `field:<PAGE>\|<modelN>.totalRowCount__` | `spg.rs:1415` |
| 条件节点 | `cond:<PAGE>\|<condId>`（**已带页面段**，:1437-1438，不变） | 同左 | — |
| 物理表模型 | `model:<tableName>`（`tbl.rs:105`，**保持全局**，聚合语义所系） | 同左 | — |
| 物理表字段 | `field:<tableName>.<name>`（`tbl.rs:129`、`spg.rs:218-219`，保持全局） | 同左 | — |
| 组件/动作/页面 | 已带页面段 | 不变 | — |

**竖线即判据**：带 `|` 的是页面局部节点，不带的是全局/物理节点。

**边端点迁移**（以全仓 `EdgeType::Contains`/`DependsOn` 实际构造点为准，逐类列全）：

- `Contains`：实际只有两类——model→field（`spg.rs:167/:394/:1432`）与 page→comp
  （`:512`）。**不存在** page→model / page→cond 的 Contains 边（更正本文早先的虚构
  表述）。迁移：model→field 的两端随局部 model/field 分段重指；page→comp 不变；
- `Reads`/`FieldWrite`/写边（comp/action → field/model）：指向**局部** field/model id；
- `FieldAlias`（字段级 field→field，`spg.rs:238-245/:302-309`）：**从局部 field 节点发出**，
  指向物理 field——更正本文早先「FieldAlias 从局部 model 节点发出」的错误表述，
  FieldAlias 从来不是 model→model；
- `DataflowInput`/`DataflowOutput`（模型级，`spg.rs:1497-1513`）：局部 model ↔ 物理 model；
- `DependsOn`（cond → owner）：`OwnerType::ModelSource` 分支构造全局
  `format!("model:{}", cond.owner_id)`（`spg.rs:1345-1347`，建边 `:1361-1368`）——
  这是「条件归属于哪个 model」的**承重边**，F4「gates 来自本页 filter」就压在它上面；
  分段后必须改指局部 model id，漏改则悬挂到不存在的 `model:modelN`；
- `DependsOn`（cond → upstream 符号）：`"model"` 符号分支构造全局 id
  （`spg.rs:1379-1382`，建边 `:1396-1403`），同样改指局部 model id；
- `DependsOn`（cond → `field:...totalRowCount__`，`spg.rs:1447-1454`）：指向分段后的行数字段；
- **写入双写**：动作写物理字段时落两条边——`ActionWrites → model:<PAGE>|modelN`
  （带本页 conditionExp，承接 F6）与既有物理表聚合边；`--relations 'model:fact_testDrive'`
  跨页扇出计数语义不变（说明 C 的去重键保证）。

**解析与 next_queries**：

- `model:<PAGE>|modelN` / `field:<PAGE>|modelN.x` 精确命中（O(1)，不归一剥离）；
- 裸 `model:modelN` / `field:modelN.x`：单页命中直达；多页同名 → 如实交回全部候选
  （同 `test_ambiguous_bare_target_answers_every_candidate` 契约）；
- `next_queries` 生成的 id 必须与事实来源同粒度：页面局部事实生成页面段 id，物理聚合
  事实生成全局 id。全仓生成点逐一审计（口径：`format_next_query(` 调用 57 处/16 文件，
  含定义与循环内调用；已知合成全局 field id 的：
  `query/model.rs:294`、`main.rs:407`），PR4b 验收含「next_queries 指向的 id 可直接
  执行且命中正确粒度节点」。

#### schema 版本与迁移机制（评审 P1-3：版本号必须真的能触发迁移）

bump `REDB_V2_SCHEMA_VERSION`（`graph_redb_v2.rs:17`）**不会**重建旧 v1 节点——版本
不匹配只是 `read_v2_layout` 返回 `None` 回退 v1（`:385-387`），而 `--build-graph` 按
文件 hash 跳过未变文件（`indexer.rs:161-168`）。因此迁移机制显式定义：

- **权威键**：`META_TABLE` 新增 `graph_schema_version`（`META_TABLE` 现仅存 diff-refresh
  checkpoint，`graph_redb.rs:36-37`）。写入侧每次 persist 盖当前版本。
- **显式枚举**：键不存在 = 显式归类 `v1`（旧库），**不是**兼容性默认——与
  `read_v2_shadow_state`「无记录视为 Current」（`graph_redb.rs:172-174`）的先例相反，
  禁止照抄。
- **校验时机**：hydrate 之前校验，**v1、v2 两条路径都校验**（v2 shadow 自己的 layout
  版本是另一层，不替代图 schema 版本）。
- **fail-closed**：期望版本 > 库版本时，查询拒绝服务并返回稳定错误码
  `GRAPH_SCHEMA_STALE`（进统一诊断信封，`answer_impact: blocking`），exit code 非零；
  `--status` 透出当前/期望版本。
- **重建路径**：`--build-graph` 检测到版本不匹配时**强制全量**——清空图与 file-state
  表（或将全部源文件标 dirty），不依赖文件 hash；diff-refresh checkpoint 随版本失效，
  否则重建后的图会被旧 checkpoint 增量打补丁。
- **PR4a 行为不变**：PR4a 引入键与校验机制，期望版本 = v1，缺键/旧库正常加载；
  PR4b bump 到 v2，此后旧库触发 `GRAPH_SCHEMA_STALE`。
- **迁移测试矩阵**：v1-only 库、含 v2 shadow 的库、graphdb-only（无源文件）场景、
  runtime reload 路径、增量刷新路径、`--non-human` 输出下的旧 schema 失败形态。

**与评测语料的相互校验**：13 个 case 的计数类断言在重建后重放；physical 聚合语义不变
所以多数应保持一致；变了的按验收第 5 条的变迁台账流程处理。

### 明确推迟（记录但不修）

- facts 块信封统一、巨型文件/函数拆分（`page_logic.rs` 共 2820 行等）、`.clone()` 治理——
  可演进性改进，不与正确性修复混在一个里程碑。
- 旧动词（`--explain-condition`/`--query-*` 等 10 个）的删除决策：维持「隐藏但可用」，
  待 M58.3 验收 run 后单独决策。

## 3. 非目标

- 不接 typed PPR / 图召回到任何默认路径（M58.4 结论：收益未证明，曝光瓶颈在先）。
- 不改 SKILL.md 的三动词结构；F4 的协议统一若影响 SKILL.md 示例则同步最小修订。
- 不动已冻结的 JSON-command runner。
- 不重排既有 case 的 gold；断言值变化走验收第 5 条的变迁台账。

## 4. 验收

1. Phase 1 的诊断信封在构建输出、`--status`、查询响应三处可见；每个 code 有契约测试。
   硬指标：pin 语料（`xiaoshouyi-corpus @ 6920ac51`）全量构建的**正常路径诊断 = 0**；
   实现期确认不可避免的良性计数，给出白名单基线数并在本文件登记。
2. F1–F6 各自的证据（实跑重放命令或附录 A 测量）修复后通过，回归测试绿；F1 的判据是
   「测量数 → 0 丢失」，F2 的判据含说明 A 的测试电池。
3. Phase 3 落地后：裸 id 多页同名如实交回候选、单页直达；页面段 id 精确命中且 gates
   来自本页 filter；`--relations 'model:表名'` 跨页聚合语义不变；next_queries 生成的 id
   粒度正确且可直接执行；PR4b 后旧库触发 `GRAPH_SCHEMA_STALE`（含 `--non-human` 形态）
   而非静默回退；**增量刷新修改一页后，另一页的 model 事实不掉**。
4. 远端测试矩阵（评审 P2-1 补齐）：`cargo fmt --check`、`m58_3_command_surface_tests`、
   `kimi_harness_judge_tests`、`ai_eval_tests`，以及按影响面点名的 focused 套件——
   `superpage_tests`（解析）、表达式解析（`expr_ast` 相关）、`scanner_tests`（层级）、
   conditions/value-source、`graph_store_tests`（parity）、`m53_redb_v2_hydrate_tests`
   （v1/v2 hydrate）、stale rebuild、增量刷新/runtime invalidation。本地干跑五条路径
   逐字为：`scripts/cnb-smoke-dry-run.sh`（期望 exit 0、66 records）+
   `--fault empty|dup|noeat|judgeleak`（各期望判红）。
5. **断言变迁台账**（评审 P1-8，防自我背书）：13 case 重放中任何断言值变化必须记入
   台账（`case_id / 断言 / 旧值 / 新值 / 源证据命令 / 复核人决定`），随 PR 附证据目录
   重放输出 diff，走 M58.2 重新校验流程（`test_fixture_cases_carry_per_case_provenance`
   守卫保持绿），并由**独立验收者**复核——不允许「数值变好」自动通过。
6. 基线策略：历史 6 题绝对分与扩充语料**不可比**（journal:281 规则）。修复后的 11-case
   `api_trigger_kimi_harness_smoke` 记为**扩充后首轮基线**本身，不与历史绝对分对比；
   与历史唯一可比的是重叠 case 的 `command_route_usage` / `command_routing_confusion`
   路由指标，对比时固定分母。若实现期成本允许，可在 PR1 后先跑一次修复前 11-case
   baseline 形成同语料前后对照（可选，不阻塞）。

## 5. PR 切分（摘要；权威版本在配套 plan）

- PR1：Phase 1 诊断信封 + 计数 + 传播 + 坏行 hydrate 测试
- PR2：F1（形态感知递归）+ F2（继承链，含说明 A 不变式与测试电池）
- PR3：F3（表达式递归合并，F5 的硬依赖）
- PR4a：图 schema 版本键 + 校验 + fail-closed + 强制全量重建机制（期望版本 = v1，行为不变）
- PR4b：Phase 3 完整局部子图（id 文法 + 边迁移 + 双写 + F4 协议统一 + next_queries 审计）
- PR5：F5 + F6（事实输出契约，说明 C）
- PR6：graphdb 重建 + 13 case 重放 + 变迁台账 + 11-case 首轮基线

## 附录 A：F1 容器子键测量（pin 语料 @ 6920ac51）

> 待 PR2 开工时用测量脚本回填逐键逐形态计数（键 × 字符串/对象数组/混合形态 × 命中
> `id`+`type` 谓词比例），当前总数 1049 来自 2026-08-23 根因解剖。回填后本节即为
> 「测量数 → 0 丢失」验收的对照基线。

## 评审修订记录

第一轮（2026-08-23）：v2 管道事实修正；page_logic.rs 行数 2820；P0-A 选父决定；
P0-B 版本键；P0-C 增量误删论据；F1 结构识别表态；F4 方向写死 + 平局替代；计数器拆因；
F3 词法边界；PR4 拆分；验收 1/5 机械判据；model6/7/8 证据路径。

第二轮（同日复验）：P1-1 版本兼容矩阵消除「行为不变」矛盾；FieldAlias 层级/行号修正
（字段级，`spg.rs:238-245/:302-309`；模型级 DataflowInput/Output `:1497-1513`）；
dwtable 路径 `spg.rs:1485-1486` 与 `ensure_model_field` 泄漏方向；回退点改正
`page_logic.rs:155-170/:300-311`；路径前缀补全；PR2「Component 父唯一」断言；
F1 排除列表登记；F4 归属 PR4b。

第三轮（2026-08-24）：**页面局部身份扩为完整局部子图**（id 文法 + 迁移矩阵 + 边端点 +
next_queries 审计，P1-1）；F2 不变式与测试电池写死（P1-2）；迁移机制补全（权威键、
v1/v2 双路校验、fail-closed `GRAPH_SCHEMA_STALE`、强制全量重建、迁移测试矩阵，P1-3）；
统一诊断信封 + `GRAPH_DB_PARTIAL_HYDRATE` 闸门 + 传播归属（P1-4）；F1 改形态感知递归
（panel 多态安全）+ 附录 A 测量归档（P1-5）；F3 修复点上移到 `extract_refs_from_ast`
合并层（P1-6）；F5/F6 事实输出契约（说明 C）+ F5 硬依赖 F3（P1-7）；断言变迁台账 +
基线可比性策略 + 配套 plan 文档（P1-8）；测试覆盖缺口表述更正（P2-2）；验证矩阵补齐、
干跑五条路径逐字列出（P2-1）。

第四轮（复验第三轮重写）：边端点迁移清单纠错——`Contains` 实际只有 model→field
（`spg.rs:167/:394/:1432`）与 page→comp（`:512`）两类，不存在 page→model/cond 边；
补三类遗漏的 `DependsOn` 边迁移（cond→owner ModelSource 承重边 `spg.rs:1345-1347/
:1361-1368`、cond→upstream `"model"` 符号 `:1379-1382/:1396-1403`、cond→totalRowCount__
`:1447-1454`）；`GRAPH_DB_PARTIAL_HYDRATE` 明确「所有后续响应」不按查询目标归因；
next_queries 审计口径改为 `format_next_query(` 57 处/16 文件（删不可复现的「31 处」）；
`RuntimeStatus` 与 `diagnostics.rs` 引用补全行号/路径前缀。
