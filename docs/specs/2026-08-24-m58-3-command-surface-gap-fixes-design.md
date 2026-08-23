# M58.3 缺口修复与可观测性打底设计

> 状态：**draft**（2026-08-23 独立评审两轮后修订，修订点见文末「评审修订记录」）
> 里程碑：M58.3（三动词命令表面收敛，active）
> 上游证据：[M58 journal 2026-08-23 节](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)（5 项缺口实跑记录）+
> `docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/`（逐 case 证据产物）
> 根因分析：2026-08-23 三路并行代码解剖（scanner/图构建、查询/解释、横切指标），结论见本文第 1 节。

## 1. 根因分析结论（修复的事实基础）

5 个上层缺口不是孤立 bug，是三个系统性模式的切面：

- **模式 1：静默降级惯例。** redb 反序列化失败静默跳边（`graph_redb.rs:227/237/582/592`）、
  `.ok().flatten()` 把图查询错误折叠成「无数据」、62 处 `unwrap_or("?")`、scanner 对
  未识别类型 `_ => {}` 静默丢弃。错误答案以 high confidence 正常输出，零 TODO 意味着
  降级从未被承认。v2 持久化路径细分两层：`graph_redb.rs:184-191` 的 hydrate 失败其实
  已写 `v2_hydrate_warning` 并有访问器（`:267`），但**全仓无生产调用方**，只有测试断言
  它为 `None`——造好的管道没接；真正静默的是 `:178` 的 `if let Ok(Some(layout))`，
  `read_v2_layout` 报错与返回 `None` 被折叠成同一条路径。
- **模式 2：图 schema 缺「页面内模型」身份。** `model:modelN` 全局按名合并
  （`scanner/spg.rs:149` + upsert 覆盖），页面限定只能靠查询时启发式重建；
  `route.rs:432` 剥掉页面段与 `page_logic.rs:115` 构造同格式直接矛盾；
  `model_scope.rs:5` 把真实客户表名硬编码进通用打分。comp→comp Contains 边从未建，
  `component_ancestor_chain` 是恒返回空的结构性死代码。
- **模式 3：白名单驱动 + 各自手写。** 容器子键只枚举 `components/panels/steps/comps`
  （实测 pin 语料 1049 处 `panel`/`columns`/`tabs` 等组件对象被静默丢弃）；
  value-source 只认单裸 `${FIELD}`；写边 meta 缺 conditionExp；六个 facts 块各自手写信封。

测试形态提供了庇护所：白盒测试走 `MemoryGraphStore` 绕过 redb 序列化路径，真实数据
端到端覆盖基本只靠 M58 评测 harness。

## 2. 范围

### Phase 1：可观测性打底（元修复，先于一切单点修复）

让降级自己暴露，不再等评测发现：

1. scanner 对未识别的容器子键 / action_type / condition symbol 计数，进图构建输出的
   diagnostics（计数即可，不逐条刷屏）。
2. `graph_redb` hydrate 静默点拆因计数，入 `--status` / 查询 diagnostics：
   - `node_decode_failed`（`graph_redb.rs:227` 节点反序列化失败）；
   - `edge_decode_failed`（`:237` 边反序列化失败）；
   - `edge_dangling_endpoint`（`:238-239` 端点不在 `node_indices` 的悬挂边——实际更常见，
     与反序列化失败合成一个数会无法定位，必须拆开）；
   - `:178` 的 `read_v2_layout` Err/None 折叠路径新增计数；
   - hydrate 失败分支**接上**已有的 `v2_hydrate_warning` 访问器（`:267`）——这不是新建
     管道，是把一个只有测试在断言的死访问器接进 `--status` 输出。
3. 页面限定解析回退全局模型时输出显式 diagnostic。真正的回退点在
   `page_logic.rs:155-170/:300-311`（已有 `page_scoped_target_not_resolved_fallback_to_global_model`
   信号，explain 主路径接上同类信号）；`model_scope.rs:120-130/:365-375` 是候选路径
   收集里的路径兜底（`unwrap_or(fallback_model_path)`），同样计数但不混为一谈。

Phase 1 的计数器不是过渡措施：F1 改为结构识别后（见 Phase 2），它们转为**永久安全网**，
任何新语料形状进来都会先在计数上显形。

### Phase 2：P0 单点修复（每条都有实跑证据可复现）

| # | 缺口 | 根因位置 | 修法 |
|---|------|---------|------|
| F1 | 容器子键白名单丢组件（1049 处） | `superpage/mod.rs:313-324,543-554`、`scanner/spg.rs:107` | **不再扩展枚举**——那是换一份语料再丢一次。改为结构识别：`RawComponent` 用 `#[serde(flatten)]` 收 extra map，递归规则为「任何是对象数组且元素带 `id`+`componentType` 的 value 都当子组件」；既有四键白名单降级为**排除列表**（已知非组件键不进递归；排除列表初值以实测非组件键为准，在 PR2 描述中登记）。注意 scanner 侧 `spg.rs:107` 走裸 `Value` 递归、不经 `RawComponent`，需平行修改。Phase 1 计数器兜底未知形状。同页重复组件 id 加诊断（`spg.rs:82-103`） |
| F2 | 无表达式组件的祖先条件继承断（button13） | `conditions.rs:379-416` 不读节点 meta json_path（`spg.rs:468` 白写）、`:429` 硬编码 `.components[` | `component_json_paths` 补读节点 meta；`is_ancestor_json_path` 放宽到全部容器子键；scanner 补建 comp→comp Contains 边。**配套硬决定**（见表格下方说明 A）：`component_ancestor_chain` 改为「优先 Component 父、Page 仅兜底」 |
| F3 | `${IF(...)}` 表达式模型引用截断 | `superpage/expr_ast.rs:204-219` tokenizer 吞整个 `${}` + `:823-831` 盲 `split('.')` → `model:IF(model6` 垃圾节点 | `classify_identifier` 的 `${}` 分支校验内部是否纯点分路径，否则对 inner 递归走表达式 AST。**已知边界**：tokenizer（`:206-217`）贪婪吃到第一个 `}`，嵌套 `${...${...}...}` 在词法层就已截断，本修复只覆盖单层 `${IF(...)}`；嵌套形态若在语料实测中出现（Phase 1 计数器会暴露），另立项处理，不算回归 |
| F4 | 页面限定降级 / 协议矛盾 | `route.rs:429-434`（剥页面段）vs `page_logic.rs:115-122`（构造页面段）；页面限定回退全局模型发生在 `page_logic.rs:155-170/:300-311`（`model_scope.rs:120-130/:365-375` 只是候选路径收集的路径兜底） | **方向已定，无决策空间**：保留页面段，`route.rs:432` 的 `by_overqualified` 档对 model 前缀禁用；回退路径接 Phase 1 的 diagnostic；协议统一随 Phase 3 的节点 id 分段一并落地（并入 PR4b）。`model_scope.rs:5` 硬编码表名移除的平局替代规则见下方说明 B |
| F5 | calc 表达式不进 value-source | `explain/condition_facts/value_source.rs:235-247` 只认单裸 `${FIELD}` | value-source 事实提取放宽到一般表达式：图里已有带 `source_expr` 的 Reads 边（`spg.rs:559-585`），直接透出 raw_expr + 引用清单，不再仅限裸字段 |
| F6 | 写动作 conditionExp 不暴露 | 写边 meta 不带（`spg.rs:842-851,872-881`），输出层不回查 action 节点 | scanner 把 `condition`/`conditionExp` 抄进写边 meta（与 action 节点 meta 同源），`--relations` writers 与 writer intent 透出；Phase 3 双写落地后抄到页面局部写边 |

**说明 A（F2 的边冲突与迭代序陷阱）**：`spg.rs:512` 给**每个**组件（含深层嵌套）都建
page→comp Contains 边。补建 comp→comp 后，嵌套组件有**两条** incoming Contains，而
`component_ancestor_chain`（`conditions.rs:308-319`）用 `.find()` 取第一条匹配边、并把
`NodeType::Page` 当链终止（`:325-327`）——存储迭代序若先给出 page 边，链在第一跳就断，
结果与今天的「恒返回空」相同，但变成**顺序依赖的不确定行为**；且 redb 迭代序与
`MemoryGraphStore` 可能不同，正好落进第 1 节诊断的测试庇护所。决定：**嵌套组件保留
page→comp 边**（按 page Contains 计组件数的消费者不动），`component_ancestor_chain`
改为按「Component 父优先、Page 兜底」排序选父。另注：comp 节点 meta 已存 `parent_id`
（`spg.rs:469-470`），祖先链不建边也能走 meta——取舍为：建边让图查询/可视化/血缘
统一走边（与 Phase 3 双写同哲学，图为唯一事实源），读 meta 零 schema 变更但又多一条
平行事实通道；本设计选建边。PR2 验收加一条「嵌套 comp 的 Component 父唯一」断言——
若某形状出现多条 comp→comp incoming Contains（多父），顺序依赖会从后门回来，必须在
测试层挡住。

**说明 B（F4 硬编码表名的平局替代）**：`FACT_AUTO_CUSTOMER_AUTO_REL_TABLE`
（`model_scope.rs:5`）在 `pick_best_dataflow_candidate` 的三处调用点作为
`preferred_input_path` 打分偏好（`:296/:304/:388`）。Phase 3 消歧的是「哪个局部模型」，
而「哪个 DataFlow 喂这条路径」是另一个轴，不会被页面分段自动解决。移除硬编码后三处
逐一改为：`preferred_input` 由**页面段解析结果**提供——命中页面局部模型时取其
FieldAlias/DataFlow 出边指向的物理路径作为偏好；无页面上下文或仍平局时**如实交回候选**
（同歧义契约），不再用客户表名偏好替调用方猜。

每条修复的验收 = 对应证据目录里的失败命令重放通过 + 新增回归测试（黑盒走真实二进制，
落在 `tests/m58_3_command_surface_tests.rs` 或新文件），覆盖「之前有表达式才碰巧正确」
之外的形状（无表达式组件、`${IF(...)}`、页面限定 model、calc 组件、带 conditionExp 的写动作）。

### Phase 3：页面内模型节点身份（核心设计，不推迟）

#### 为什么必须现在做

页面内模型是三重身份的交汇点，缺一维全部失真：

- **查询逻辑**：`model:modelN` 的 availability gates 应该来自**本页这份** filter，不是别页合并进全局节点的表路径；
- **写入场景**：同一个动作写哪张表、带什么 conditionExp 门控，是页面级事实；
- **同表异 filter**：绑定车辆.spg 的 model6/7/8 同指 `fact_customAutoMyAutoList.tbl` 但 filter 完全不同——全局按名合并后，三个逻辑视图塌成一个节点，谁的 filter 都答不对。
  证据：`docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/xiaoshouyi_bindcar_input5_pending_reason_calc/facts_review.md:7`
  （cmd4_* availability 把 model6/7/8 分别降级解析到 `szsys_4_users.tbl` /
  `fact_saleContract.tbl` / `fact_warrantyperiod.tbl`，源文件证实三者同表）。
- **增量刷新误删（比答案不准更硬：是数据丢失）**：`process_spg_file_from_value` 把
  `model:modelN` 和 `field:modelN.x` 放进该页文件的 `node_ids` 返回值（内嵌 DataFlow
  路径 `spg.rs:370-371/387-388`；dwtable source 路径 `spg.rs:1485-1486`——model6/7/8
  场景的主流触发路径），indexer 增量更新按 `previous_node_ids` 批删
  （`scanner/indexer.rs` `apply_incremental_changes` → `merge_removed_node_ids`，
  `:296-301` 区域）。今天 `model:model6` 是**跨页共享**的全局节点——**编辑任意一页会
  删掉别的页也在用的那个节点**，直到那些页被重扫。id 分段后删除粒度才与文件粒度对齐。
  反方向同样存在：普通 read/write 路径 `ensure_model_field`（`spg.rs:260/:283`）建的
  model/field 节点**不进** `node_ids`，增量下是泄漏而非误删——两个方向都源于「节点
  归属粒度 ≠ 文件粒度」，分段后一并消解。验收加一条：增量刷新改一页后，另一页的
  model 事实不掉。

#### 现状 schema 的精确病灶

- `model:` 命名空间混装两类节点：物理表（`scanner/tbl.rs:29`，按表名）与页面局部逻辑模型（`scanner/spg.rs:149`，按局部 id）。
- 局部模型全局按名合并（`spg.rs:149` + `upsert_node` 后写覆盖 `utils.rs:15`），两页同 id 时 path/meta 由扫描顺序决定。
- 局部→物理的映射分两层：模型级靠 `DataflowInput`/`DataflowOutput` 边
  （dwtable source 块，`spg.rs:1497-1504` 起），字段级靠 `FieldAlias` 边
  （read/write 路径，`spg.rs:238-245/:302-309`）；页面绑定只在扫描期
  `source_path_map`（`spg.rs:325`），扫完即丢。

#### 设计：id 分段 + 双写聚合

**节点 id 方案**：页面局部模型节点 id 改为 `model:<PAGE_PATH>|<modelN>`（竖线分段）。
这个格式不是新发明——`page_logic.rs:115` 的 `page_scoped_model_target` 已经在生产它、
`model_scope.rs:12` 已经在解析它，本次是让图 schema 与既有 target 文法对齐。物理表节点
保持 `model:<tableName>`（无竖线），**竖线即两类节点的判据**。

**边与 meta**：

- 局部模型节点携带**本页自己的** filter/meta（不再 upsert 覆盖）→ F4 的 availability
  gates 直接读节点自身，启发式重建（路径兜底/主页投票/硬编码表名）整体拆除；
- 局部→物理的模型级 `DataflowInput`/`DataflowOutput` 与字段级 `FieldAlias` 边改从
  `model:PAGE|modelN` 发出，血缘不断；
- **写入双写**：动作写物理字段时同时落两条边——`ActionWrites → model:PAGE|modelN`
  （带本页 conditionExp，承接 F6）与既有物理表聚合边。`--relations 'model:fact_testDrive'`
  的跨页扇出计数（84/248 类）语义不变；新增「这个页面怎么写这张表」的精确视图。

**解析与兼容**：

- `model:PAGE|modelN` 精确命中页面局部节点（O(1)，不再归一剥离——`route.rs:432` 的
  `by_overqualified` 档对 model 前缀禁用，F4 并入本 Phase）；
- 裸 `model:modelN` 若只命中一个页面局部节点则直达；多页面同名 → 走既有歧义模式
  **如实交回全部候选**（与 `test_ambiguous_bare_target_answers_every_candidate` 同契约），
  绝不替调用方猜；
- 旧 graphdb 不兼容：见下方「schema 版本与旧库拒绝」。

**schema 版本与旧库拒绝**（PR4a 先行落地）：

- `graph_redb` 的 `META_TABLE` 目前只存 diff-refresh checkpoint（`:36-37/:959-963/:1040+`），
  没有可「+1」的图 schema 版本键——PR4a **先引入** `graph_schema_version` 键（写入侧
  每次 persist 盖当前版本；读取侧显式解析）。
- **键不存在必须显式归类为「schema version 1（旧库）」**——这是代码里显式枚举的已知
  版本，不是兼容性默认。仓里既有先例正好相反：`read_v2_shadow_state` 对旧库无记录
  「视为 Current，向后兼容」（`graph_redb.rs:172-174` 注释）——**不得照抄**，照抄会让
  旧库静默通过，精确复现模式 1。
- 版本兼容矩阵写死在代码里：PR4a 期间期望版本 = 1，故缺键/版本 1 的库**正常加载、
  行为不变**（纯机制落地）；PR4b 把期望版本 bump 到 2，此后加载版本 1/缺键的库给明确
  「graph schema 过旧，请重建」诊断并拒绝静默继续。「键不存在 = 旧库」的判定从 PR4a
  起就生效，只是 PR4a 阶段旧库恰好兼容——拒绝动作由版本 bump 触发，不靠实现期临时
  决定。
- checkpoint 随版本失效：版本不匹配时忽略 diff-refresh checkpoint，强制全量重建，
  否则重建后的图会被旧 checkpoint 增量打补丁。

**与评测语料的相互校验**：13 个 case 的计数类断言（read_by_count 等）在重建后重放；
physical 聚合语义不变所以多数应保持一致，变了的按「重新校验并记录」流程处理（验收第 5 条的机械判据）。

### 明确推迟（记录但不修）

- facts 块信封统一、巨型文件/函数拆分（`page_logic.rs` 共 2820 行等）、`.clone()` 治理——
  可演进性改进，不与正确性修复混在一个里程碑。
- 旧动词（`--explain-condition`/`--query-*` 等 10 个）的删除决策：维持「隐藏但可用」，
  待 M58.3 验收 run 后单独决策。

## 3. 非目标

- 不接 typed PPR / 图召回到任何默认路径（M58.4 结论：收益未证明，曝光瓶颈在先）。
- 不改 SKILL.md 的三动词结构；F4 的协议统一若影响 SKILL.md 示例则同步最小修订。
- 不动已冻结的 JSON-command runner。
- 不重排既有 case 的 gold；若 F1–F6 修复改变了扩充 case 证据里的输出（如 model10
  物理表解析），按「重新校验 case 后更新」流程处理并记录。

## 4. 验收

1. Phase 1 的计数/diagnostic 在真实语料构建与查询输出中可见。硬指标：pin 语料
   （`xiaoshouyi-corpus @ 6920ac51`）全量构建的**正常路径 diagnostics 条数 = 0**；
   若实现期确认存在不可避免的良性计数，给出白名单基线数并在本文件登记，不得留「不产生
   噪音」这种无判据表述。
2. F1–F6 各自的复现命令（取自证据目录）修复后输出正确事实，回归测试绿。
3. Phase 3 落地后：裸 `model:modelN` 多页同名时如实交回候选、单页直达；
   `model:PAGE|modelN` 精确命中且 gates 来自本页 filter；`--relations 'model:表名'`
   的跨页聚合计数语义不变；PR4b 版本 bump 后，旧 graphdb（含无 `graph_schema_version`
   键的库）得到明确「请重建」诊断而非静默回退；**增量刷新修改一页后，另一页的 model
   事实不掉**。
4. 远端 `cargo fmt --check`、`m58_3_command_surface_tests`、`kimi_harness_judge_tests`、
   `ai_eval_tests` 绿；本地干跑五条路径符合预期。
5. 修复后对 pin 语料重建 graphdb，重放 13 个 case 的 `expected_output_assertions`：
   允许断言值随修复**变好**而更新，但任何断言值变化必须在对应 PR 中附**证据目录命令
   重放的输出 diff**，并走 M58.2 的重新校验流程（重放命令 → 更新 `docs/ai-eval-runs/`
   证据 → 更新 fixture provenance，`test_fixture_cases_carry_per_case_provenance`
   守卫保持绿）；不允许静默变坏。
6. 完成后跑一次 11-case `api_trigger_kimi_harness_smoke` 拿扩充后首轮基线，与
   `command_route_usage` / `command_routing_confusion` 的历史基线对比——这是 M58.3
   标 done 的验收门。

## 5. PR 切分建议

- PR1：Phase 1 可观测性（独立可验收，为后续所有修复提供「修复前后丢了多少数据」的对照）
- PR2：F1 + F2（scanner/继承链，同层；含说明 A 的祖先链选父改造）
- PR3：F3（表达式解析，小切口先行）
- PR4a：图 schema 版本键机制（引入 `graph_schema_version`、缺键显式归类 version 1、
  checkpoint 随版本失效）；期望版本仍为 1，**加载行为不变**，拒绝诊断由 PR4b 的
  版本 bump 触发
- PR4b：Phase 3 节点 id 分段 + 解析 + 写入双写，**F4 协议统一归属本 PR**
  （`route.rs:432` 禁用 model 前缀剥离、说明 B 的平局替代）；`format!("model:` 30 处 /
  `strip_prefix("model:")` 12 处散布 15+ 文件，与 PR4a 切开后各自可验收（`format!("model:` 30 处 /
  `strip_prefix("model:")` 12 处散布 15+ 文件，与 PR4a 切开后各自可验收）
- PR5：F5 + F6（facts 暴露，其中 F6 的写边双写在 PR4b 的 schema 上落地）
- PR6：graphdb 重建 + 13 case 重放 + 11-case live run 验收

## 评审修订记录（2026-08-23 独立评审一轮）

- 事实修正：`v2_hydrate_warning` 管道已存在但未接（`:187/:267`），真正静默的是 `:178`
  折叠路径；`page_logic.rs` 行数 831 → 2820。
- P0-A：F2 补 comp→comp 边与既有 page→comp 边的迭代序冲突，写死「Component 父优先、
  Page 兜底」决定，并记录 `parent_id` meta 备选取舍（说明 A）。
- P0-B：明确 `META_TABLE` 无版本键，PR4a 先引入；「键不存在 = 旧库」写进设计，禁止照抄
  `read_v2_shadow_state` 的向后兼容先例；checkpoint 随版本失效。
- P0-C：增量刷新误删跨页共享 model 节点列为 Phase 3 最硬论据，并加验收条。
- F1 方向表态：结构识别替代枚举扩展，白名单降级为排除列表，Phase 1 计数器转永久安全网。
- F4 消除表述矛盾：方向已定（保留页面段），补硬编码表名移除后的平局替代规则（说明 B）。
- Phase 1 计数器拆因（`node_decode_failed` / `edge_decode_failed` / `edge_dangling_endpoint`）；
  F3 补嵌套 `${}` 词法边界；PR4 拆 PR4a/PR4b；验收 1/5 改机械判据；model6/7/8 补证据路径。

第二轮（复验 P1/P2）：

- P1-1：消除「PR4a 行为不变」与「缺键判旧库」的矛盾——缺键从 PR4a 起显式归类
  version 1，PR4a 期望版本 = 1 故加载行为不变；拒绝诊断由 PR4b 版本 bump 触发，
  版本兼容矩阵写死在代码里，不留实现期临时决定。
- P2-1：「局部→物理映射」改述为模型级 `DataflowInput`/`DataflowOutput`
  （`spg.rs:1497-1504`）+ 字段级 `FieldAlias`（`spg.rs:238-245/:302-309`），
  原引 `spg.rs:1472-1473` 实为 id 构造行。
- P2-2：增量刷新论据补 dwtable source 路径（`spg.rs:1485-1486`，model6/7/8 主流触发）
  与反向泄漏（`ensure_model_field` `spg.rs:260/:283` 的节点不进 `node_ids`）。
- P2-3：回退点引用改正——真正的页面限定→全局回退在 `page_logic.rs:155-170/:300-311`；
  `model_scope.rs:120-130/:365-375` 是候选路径收集的路径兜底，两处分别表述。
  （复验引的 `explain.rs:523` 实为 target-not-found 分支，亦不采用。）
- P2-4：路径前缀补全（`superpage/expr_ast.rs`、`explain/condition_facts/value_source.rs`）。
- 残留建议采纳：PR2 验收加「嵌套 comp 的 Component 父唯一」断言；F1 排除列表初值
  在 PR2 描述登记；PR 切分节给 F4 单独归属指针（PR4b）。
