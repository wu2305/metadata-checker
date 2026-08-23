# M58.3 缺口修复与可观测性打底设计

> 状态：**draft**
> 里程碑：M58.3（三动词命令表面收敛，active）
> 上游证据：[M58 journal 2026-08-23 节](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md)（5 项缺口实跑记录）+
> `docs/ai-eval-runs/2026-08-23-m58-2-gold-verification/`（逐 case 证据产物）
> 根因分析：2026-08-23 三路并行代码解剖（scanner/图构建、查询/解释、横切指标），结论见本文第 1 节。

## 1. 根因分析结论（修复的事实基础）

5 个上层缺口不是孤立 bug，是三个系统性模式的切面：

- **模式 1：静默降级惯例。** redb 反序列化失败静默跳边（`graph_redb.rs:227/237/582/592`）、
  v2 失败静默回退 v1（`:175-179`）、`.ok().flatten()` 把图查询错误折叠成「无数据」、
  62 处 `unwrap_or("?")`、scanner 对未识别类型 `_ => {}` 静默丢弃。错误答案以 high
  confidence 正常输出，零 TODO 意味着降级从未被承认。
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
2. `graph_redb` 节点/边反序列化失败计数入 `--status` / 查询 diagnostics；v2→v1 回退
   显式记一笔。
3. 页面限定解析回退全局模型时（`model_scope.rs:126/370`）输出显式 diagnostic
   （参照 `page_logic.rs:165` 已有的 `page_scoped_target_not_resolved_fallback_to_global_model`，
   explain 主路径接上同类信号）。

### Phase 2：P0 单点修复（每条都有实跑证据可复现）

| # | 缺口 | 根因位置 | 修法 |
|---|------|---------|------|
| F1 | 容器子键白名单丢组件（1049 处） | `superpage/mod.rs:313-324,543-554`、`scanner/spg.rs:107` | 枚举扩展到实测存在的子键（`panel`/`columns`/`tabs`/`operateButtons`/`grid` 等，以语料实测清单为准）；同页重复组件 id 加诊断（`spg.rs:82-103`） |
| F2 | 无表达式组件的祖先条件继承断（button13） | `conditions.rs:379-416` 不读节点 meta json_path（`spg.rs:468` 白写）、`:429` 硬编码 `.components[` | `component_json_paths` 补读节点 meta；`is_ancestor_json_path` 放宽到全部容器子键；同时在 scanner 补建 comp→comp Contains 边让 `component_ancestor_chain` 真正生效 |
| F3 | `${IF(...)}` 表达式模型引用截断 | `expr_ast.rs:204-219` tokenizer 吞整个 `${}` + `:823-831` 盲 `split('.')` → `model:IF(model6` 垃圾节点 | `classify_identifier` 的 `${}` 分支校验内部是否纯点分路径，否则对 inner 递归走表达式 AST |
| F4 | 页面限定降级 / 协议矛盾 | `route.rs:429-434`（剥页面段）vs `page_logic.rs:115-122`（构造页面段）；`model_scope.rs:126/370` 静默回退 | 二选一统一协议方向（保留页面段，归一化不得剥掉）；回退路径接 Phase 1 的 diagnostic；移除 `model_scope.rs:5` 硬编码表名偏好 |
| F5 | calc 表达式不进 value-source | `value_source.rs:235-247` 只认单裸 `${FIELD}` | value-source 事实提取放宽到一般表达式：图里已有带 `source_expr` 的 Reads 边（`spg.rs:559-585`），直接透出 raw_expr + 引用清单，不再仅限裸字段 |
| F6 | 写动作 conditionExp 不暴露 | 写边 meta 不带（`spg.rs:842-851,872-881`），输出层不回查 action 节点 | scanner 把 `condition`/`conditionExp` 抄进写边 meta（与 action 节点 meta 同源），`--relations` writers 与 writer intent 透出 |

每条修复的验收 = 对应证据目录里的失败命令重放通过 + 新增回归测试（黑盒走真实二进制，
落在 `tests/m58_3_command_surface_tests.rs` 或新文件），覆盖「之前有表达式才碰巧正确」
之外的形状（无表达式组件、`${IF(...)}`、页面限定 model、calc 组件、带 conditionExp 的写动作）。

### Phase 3：页面内模型节点身份（核心设计，不推迟）

#### 为什么必须现在做

页面内模型是三重身份的交汇点，缺一维全部失真：

- **查询逻辑**：`model:modelN` 的 availability gates 应该来自**本页这份** filter，不是别页合并进全局节点的表路径；
- **写入场景**：同一个动作写哪张表、带什么 conditionExp 门控，是页面级事实；
- **同表异 filter**：绑定车辆.spg 的 model6/7/8 同指 `fact_customAutoMyAutoList.tbl` 但 filter 完全不同——全局按名合并后，三个逻辑视图塌成一个节点，谁的 filter 都答不对。

#### 现状 schema 的精确病灶

- `model:` 命名空间混装两类节点：物理表（`scanner/tbl.rs:29`，按表名）与页面局部逻辑模型（`scanner/spg.rs:149`，按局部 id）。
- 局部模型全局按名合并（`spg.rs:149` + `upsert_node` 后写覆盖 `utils.rs:15`），两页同 id 时 path/meta 由扫描顺序决定。
- 局部→物理的映射靠 `FieldAlias` 边（`spg.rs:1472-1473`），页面绑定只在扫描期 `source_path_map`（`spg.rs:325`），扫完即丢。

#### 设计：id 分段 + 双写聚合

**节点 id 方案**：页面局部模型节点 id 改为 `model:<PAGE_PATH>|<modelN>`（竖线分段）。
这个格式不是新发明——`page_logic.rs:115` 的 `page_scoped_model_target` 已经在生产它、
`model_scope.rs:12` 已经在解析它，本次是让图 schema 与既有 target 文法对齐。物理表节点
保持 `model:<tableName>`（无竖线），**竖线即两类节点的判据**。

**边与 meta**：

- 局部模型节点携带**本页自己的** filter/meta（不再 upsert 覆盖）→ F4 的 availability
  gates 直接读节点自身，启发式重建（路径兜底/主页投票/硬编码表名）整体拆除；
- 局部→物理的 `FieldAlias` / DataFlow 边改从 `model:PAGE|modelN` 发出，血缘不断；
- **写入双写**：动作写物理字段时同时落两条边——`ActionWrites → model:PAGE|modelN`
  （带本页 conditionExp，承接 F6）与既有物理表聚合边。`--relations 'model:fact_testDrive'`
  的跨页扇出计数（84/248 类）语义不变；新增「这个页面怎么写这张表」的精确视图。

**解析与兼容**：

- `model:PAGE|modelN` 精确命中页面局部节点（O(1)，不再归一剥离——`route.rs:432` 的
  `by_overqualified` 档对 model 前缀禁用，F4 并入本 Phase）；
- 裸 `model:modelN` 若只命中一个页面局部节点则直达；多页面同名 → 走既有歧义模式
  **如实交回全部候选**（与 `test_ambiguous_bare_target_answers_every_candidate` 同契约），
  绝不替调用方猜；
- 旧 graphdb 不兼容：graph schema 版本号 +1，加载旧库给明确「请重建」诊断（不静默回退）。

**与评测语料的相互校验**：13 个 case 的计数类断言（read_by_count 等）在重建后重放；
physical 聚合语义不变所以多数应保持一致，变了的按「重新校验并记录」流程处理。

### 明确推迟（记录但不修）

- facts 块信封统一、巨型函数拆分（`page_logic.rs` 831 行等）、`.clone()` 治理——可演进性
  改进，不与正确性修复混在一个里程碑。
- 旧动词（`--explain-condition`/`--query-*` 等 10 个）的删除决策：维持「隐藏但可用」，
  待 M58.3 验收 run 后单独决策。

## 3. 非目标

- 不接 typed PPR / 图召回到任何默认路径（M58.4 结论：收益未证明，曝光瓶颈在先）。
- 不改 SKILL.md 的三动词结构；F4 的协议统一若影响 SKILL.md 示例则同步最小修订。
- 不动已冻结的 JSON-command runner。
- 不重排既有 case 的 gold；若 F1–F6 修复改变了扩充 case 证据里的输出（如 model10
  物理表解析），按「重新校验 case 后更新」流程处理并记录。

## 4. 验收

1. Phase 1 的三类计数/diagnostic 在真实语料构建与查询输出中可见，且正常路径不产生噪音。
2. F1–F6 各自的复现命令（取自证据目录）修复后输出正确事实，回归测试绿。
3. Phase 3 落地后：裸 `model:modelN` 多页同名时如实交回候选、单页直达；
   `model:PAGE|modelN` 精确命中且 gates 来自本页 filter；`--relations 'model:表名'`
   的跨页聚合计数语义不变；旧 graphdb 得到明确「请重建」诊断而非静默回退。
4. 远端 `cargo fmt --check`、`m58_3_command_surface_tests`、`kimi_harness_judge_tests`、
   `ai_eval_tests` 绿；本地干跑五条路径符合预期。
5. 修复后对 pin 语料重建 graphdb，重放 13 个 case 的 `expected_output_assertions`：
   允许断言值随修复**变好**而更新（按流程重新校验），不允许静默变坏。
6. 完成后跑一次 11-case `api_trigger_kimi_harness_smoke` 拿扩充后首轮基线，与
   `command_route_usage` / `command_routing_confusion` 的历史基线对比——这是 M58.3
   标 done 的验收门。

## 5. PR 切分建议

- PR1：Phase 1 可观测性（独立可验收，为后续所有修复提供「修复前后丢了多少数据」的对照）
- PR2：F1 + F2（scanner/继承链，同层）
- PR3：F3（表达式解析，小切口先行）
- PR4：**Phase 3 页面内模型节点身份**（含 F4 协议统一；schema 版本 +1，需全量重建验证）
- PR5：F5 + F6（facts 暴露，其中 F6 的写边双写在 PR4 的 schema 上落地）
- PR6：graphdb 重建 + 13 case 重放 + 11-case live run 验收
