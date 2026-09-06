# M58.3 条件符号归一与悬挂节点收口设计

> 状态：**approved**（2026-09-05 用户批准：「修待办 2 的相邻缺口」）
> 里程碑：M58.3（复核返修之后的相邻缺口，评审外新发现）
> 上游：`docs/specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md`（closed，
> 本文只做其 P1-4/P1-5/P1-6 修复留下的**同族**缺口收口，不扩大 M58.3 范围）
> 发现来源：2026-09-05 复核返修 worker 报告的「相邻缺口」四条 + 本轮实现回归中
> 新发现的第五条（C2）

## 1. 背景

复核返修（`b5d70ac`）修掉了 cond→component 符号边的属性后缀、动作条件的驱动源、
裸 `${paramN}` 的归一和组件身份判定。这四处修完之后，**同一类问题在相邻位置仍然
成立**：符号串的产生侧（`conditions.rs`）与消费侧（`scanner/spg.rs`）之间，仍有
若干条路径会产出「指向不存在节点的符号」或「不存在的 JSON 路径」。

这些缺口的共同形态是：**图里悄悄多出一个不存在的东西，或悄悄少掉一条边**，
两者都不报错。存储层对端点缺失的边是静默丢弃，因此缺陷不会在任何测试里冒头。

## 2. 缺口清单与判据

| 编号 | 缺口 | 现象 | 判据（修复后必须成立） |
|---|---|---|---|
| A | 条件符号未经页面上下文归一 | `scan_conditions` 的动作条件与 source filter 走裸 `parse_expression_ast`，`${txtB.txt}`（txtB 是组件）判成 `ModelField("txtB","txt")` → 符号 `model:txtB.txt` → 指向永不存在的节点 | 三条路径（组件表达式 / 动作条件 / source filter）共用同一份 `resolve_ref_type`，同一符号在三处产出一致 |
| B | 空字段名尾点 | 裸 `${modelN}` → `ModelField(modelN, "")` → 符号 `model:modelN.`，`ensure_model_field` 进而造出 `field:modelN.` 节点与 model→field `Contains` 边 | 图中不存在以 `.` 结尾的 `field:` 节点；模型级节点与模型级边保留 |
| C | cond→param/user/system 整类悬挂 | 这三类目标节点只在**组件表达式**路径上创建；只出现在条件里的符号没有节点，边被两端存储静默丢弃 | 只在条件里出现的 param/user/system 符号也有节点，且 cond→目标边真的落库 |
| C2 | `${$user.x}` / `${$now}` 误判 | `classify_identifier` 的 `${...}` 分支直接按 `.` 拆分，`${$user.deptId}` 判成 `ModelField("$user","deptId")` → `model:$user` + `field:$user.deptId` 垃圾节点，用户属性血缘整条丢失 | `${$user.x}` 与裸 `$user.x` 分类完全一致；图中不存在 `$` 前缀的 `model:` / `field:` 节点 |
| D | json_path 合成兜底 | `collect_json_paths` 只从 `canvas.components` 起步，`extract_components` 从 `canvas` 对象本身起步（含 `canvas.panels/steps/comps` 与 canvas 级 extra 组件数组）。差集里的组件查不到路径，退回 `canvas.components[id='...']` 合成串 | 任何条件记录的 `json_path` 都能在原始 JSON 里逐段定位；不出现 `canvas.components[id=` |

C2 是实现 A/B 的回归测试时暴露出来的：真实语料
（`tests/fixtures/browser-offscreen-real-project/`）里有 `${$user.WXWORK_USER_ID}`、
`${$user.id}`、`${$user.dept_id}`、`${$user.LEADER.USER_NAME}` 四种写法。

## 3. 设计决策

### 3.1 归一函数只有一份（缺口 A）

不在 `conditions.rs` 里另写一套启发式。新增 `PageRefContext`（组件 id / 数据源 id /
参数 id 三个集合），直接复用 `superpage::resolve_ref_type`——它就是组件表达式路径
在 `resolve_expression_refs_with_context` 里用的那一份。`resolve_ref_type` 的可见性
从私有提升为 `pub(crate)`，不新增公开 API。

组件表达式那条路径（`spg.expressions`）的 refs 在 superpage 解析末尾已经归一过，
再走一次是幂等的，因此三条路径可以无差别地共用。

### 3.2 空字段名不是字段（缺口 B）

`ensure_model_field` 的返回值第二项改为 `Option<String>`：字段名为空时只确保
`model:{model}` 节点，不造 `field:` 节点、不建 `Contains` 边，调用方据此跳过所有
字段级边（`Reads` / `FieldWrite` / `FieldAlias`）。边的 `field_path` 文案统一走新的
`model_field_path`，字段名为空时只写模型名。

**取舍**：也可以在 `expr_ast` 层把 `ModelField(m, "")` 直接归一成别的 RefType，但那会
改变 `parse_expression_refs` 的公开语义并波及既有断言。选择在建图侧收口，语义变化
局限在「不造没有字段语义的字段节点」这一件事上。

### 3.3 补建节点而不是丢边（缺口 C）

param / user / system 三类符号本身就是事实（它们写在条件表达式里），补建节点不存在
「凭空造节点」的问题。补建方式与组件表达式路径**逐字段一致**：同样的 id 形态、
同样的 `NodeType::Field`、同样的 `meta.kind`，`add_node` 幂等。

`component` / `model` 两类**不**补建：component 目标经缺口 A 归一后只可能是已知组件
id（节点必然已在 pass1 注册），model 目标的节点归 sources 处理与跨文件链接负责，
在条件路径上凭空造模型节点会污染模型清单。

### 3.4 遍历起点对齐提取侧（缺口 D）

`collect_json_paths` 拆成两层：`collect_json_paths_from_node`（单节点：登记 id→path，
再递归四个白名单键 + 形态吻合的 extra 键）与 `collect_json_paths`（数组遍历，逐元素
调前者）。两个入口（`conditions.rs::scan_conditions`、`output.rs`）同步从
`canvas.components` 数组上移到 `canvas` 对象本身，与 `extract_components` 同起点。

## 4. 验收

- 新增 `tests/m58_3_adjacent_gap_tests.rs`，六个用例逐缺口锁定（A、A+B、B、C、C2、D）；
- 影响面回归：conditions / scanner / superpage / core_feature / regression /
  corpus_snapshot / dataflow_lineage / explain / output_parser 等 30 个套件 + lib；
- **快照影响**：缺口 B/C/C2 改变图的节点与边集合。fixture 语料若触发这些形态，
  `corpus_snapshot_tests` 会失败并需要按 spec 验收第 5 条逐断言登记到
  `docs/ai-eval-runs/2026-08-26-m58-3-snapshot-change-ledger.md`。
- **真实项目诊断计数基线**：本轮不重测。见第 5 节。

## 5. 遗留与未决

1. **真实项目诊断计数基线已陈旧**（继承自复核返修 P1-7，非本轮引入）：
   `performance-baseline.md` 记录 `SCANNER_UNRECOGNIZED_CONTAINER_KEY = 312`
   （交叉验证口径 `moreFields 279 + params 33`）与 `SCANNER_DUPLICATE_COMPONENT_ID
   = 1,105`。P1-7 把「id 有 / type 无」的对象计入 UNRECOGNIZED 并使其不再注册组件
   上下文，两个计数都会漂移，且 312 的「精确吻合」交叉验证不再成立。需要在真实项目
   上重测后回登，属 M58.3 收尾项。
2. `RefType::Param` 的命名启发式（`starts_with("param")`）会把 `paramount` 之类的
   模型名误判为参数。该启发式是既有行为，本轮不动。
3. `model:` 目标在条件路径上仍可能悬挂（引用了本页 sources 之外、且跨文件链接也没
   补上的模型）。这属于「数据实情 vs 实现缺陷」未定性的一类，需要先有诊断口径再修，
   不在本轮范围。
