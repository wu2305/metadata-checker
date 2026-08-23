# M58.3 修订包：附录 A 测量回填与三处更正

> 状态：**amendment packet**（不是 plan，也不是 handoff）
> 目标文档：[spec](../specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md)（draft，四轮评审）
> 与 [plan](2026-08-24-m58-3-command-surface-gap-fixes-plan.md)（draft），二者当前状态为 commit `b5bead4`。
> 本包**不新建 plan**——plan 已存在。本包只做三件事：
> (1) 回填 plan PR2 前置的「附录 A 逐键逐形态测量」；
> (2) 更正上一轮评审中三处被我夸大或错层的判断；
> (3) 提三条针对现有 PR 切分的具体修订建议。
> 证据：`tools/corpus-shape-audit.py`（脚本 sha256 前缀 `9d2f2d3f`）+
> `docs/ai-eval-runs/2026-08-24-m58-3-corpus-shape-audit.json`（语料 `6920ac51`，501 个 `.spg`）。

---

## 0. 先更正我自己

上一轮我给出的三个数字**沿用了错误的测量口径**，现已全部重测并作废：

| 我原先的说法 | 实际 | 错在哪 |
|---|---|---|
| 「293 处 `${comp.value}` 被记成 model 读」 | **4 处** | 我只读了 `classify_identifier`，没看到 `superpage/mod.rs:59` 的上下文解析（`resolve_expression_refs_with_context`）会把 head 命中已抽取组件 id 的 `ModelField` 改写成 `ComponentProperty`（`mod.rs:122`）。`classify_identifier` 不是终态 |
| 「图里有 250 个垃圾 model 节点」 | **117 个候选名 / 471 次**，且仍非图内实测 | 我扫的是原始 JSON 字符串，把不可达记录上的表达式也算了进来。要断言图内节点数，必须重建后从图里数 |
| 「说明 B 在 50% 的内联 dataFlow source 上无法定偏好」 | **撤回** | 我把「一个 DataFlow 消费几张物理表」当成了「有几个竞争候选」。候选集在 `model_scope.rs:264+` 由 `DataflowInput` 出边 + `is_dataflow_model` 过滤决定，一个 DataFlow 连三张表仍是一个候选 |

**共同根因**：我用手写谓词测原始语料，而没有跟着生产调用链走。本包所有数字改由
`tools/corpus-shape-audit.py` 复刻真实遍历与分类规则产出，并在脚本头声明测量边界。

另：上一版本文件把接收方指向「创建 plan」，而 plan 已存在（`b5bead4`），spec 也已是
四轮评审而非一轮。按原文执行会覆盖现有成果。本次整体改写为修订包。

---

## 1. 附录 A 测量回填（plan PR2 的前置项）

复现：

```bash
python3 tools/corpus-shape-audit.py \
  --corpus <succ-definitions>/projects/xiaoshouyi \
  --json docs/ai-eval-runs/2026-08-24-m58-3-corpus-shape-audit.json
```

**测量边界**（引用数字时必须一并引用）：脚本读原始 JSON、复刻
`superpage::extract_components` 的现役遍历与 `is_expr` 分类，**不构建 graphdb**。
产出的是**候选量**，不是图中实际节点/边数。

### 1.1 逐键逐形态

现役遍历到达 **26,007**；漏掉候选 **8,653**，其中带行为证据 **8,310** / 无行为证据 **343**。

| 容器键 | 漏掉 | 带表达式 | 带 actions | 带子容器 | 判读 |
|---|---:|---:|---:|---:|---|
| `columns` | 5226 | 26 | 136 | 143 | 组件 |
| `components` | 2144 | 1054 | 459 | 642 | 组件（祖先在非白名单键下而整棵子树不可达） |
| `panel` | 245 | 0 | 48 | 245 | 组件 |
| `grid` | 245 | 9 | 165 | 0 | 组件 |
| `attrFields` | 240 | 10 | 0 | 0 | 组件（有表达式） |
| `tabs` | 179 | 12 | 0 | 0 | 组件（有表达式） |
| `operateButtons` | 31 | 19 | 31 | 0 | 组件 |
| `effectStyles` | 226 | 0 | 0 | 0 | **建议进排除列表** |
| `conditionStyles` | 61 | 0 | 0 | 0 | **建议进排除列表** |
| `labelFields` | 35 | 0 | 0 | 0 | **建议进排除列表** |
| `stateFields` | 20 | 0 | 0 | 0 | **建议进排除列表** |
| `buttons` | 1 | 0 | 0 | 0 | **建议进排除列表**（样本过少，需人工确认） |

**排除列表初值建议**：`effectStyles` / `conditionStyles` / `labelFields` / `stateFields`
（合计 342），判据是三项行为证据全为 0。`buttons`（1 个）样本不足以自动定性，请人工看一眼。
样本形态：`{"id":"effectStyle1","type":"hover"}` —— 带 `id`+`type` 的样式记录，
`type` 取值是 `hover`/`active`/`disabled` 这类状态名，不是组件类型。

**混合形态数组**：`conditionStyles` 有 15 处是「含组件样元素但不是每个元素都带 `id`+`type`」，
正好落进 spec F1 写的整体判非组件 + `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 计数分支。
这条分支有真实样本，PR2 的测试可以直接取用。

### 1.2 对 spec / plan 的直接影响

- **spec:30 与 spec:87 的「1049 处」应更新为 8,653 候选 / 8,310 带行为证据**。
  plan 已预留这条（「附录 A 测量若与 1049 总数出入大，F1 验收基线以回填后的逐键计数为准」），
  本包即为该回填。
- **组件数量级变化**：若排除列表取上述初值，组件数 26,007 → 34,317（**+32%**）。
  这个数随排除列表变动，plan 应把它写成「排除列表定值后的推论」，不是独立事实。
- **F1 验收基线**：按 spec 的「测量数 → 0 丢失」口径，目标是
  `漏掉候选 - 排除列表命中数 == 0`，而不是我上一版提的
  `reached == 所有 id+type 对象`——后者会把 343 个样式记录一并吃进图里，与 spec F1
  的排除列表要求直接冲突。**这条请以本包为准，废弃我上一版的不变量表述。**

---

## 2. 三处更正后的判断

### 2.1 F3 的范围是对的；真正的缺口在 scanner，且很小

沿真实调用链的表达式终态（只统计解析器真正抽到的组件）：

```
  961  ModelField（正确）
  471  畸形 ${} → 仍是 ModelField（117 个不同的垃圾 model 名）  ← F3 的目标
   44  head 未知 → 仍是 ModelField
    4  → ComponentProperty → 被 scanner 通配臂丢弃           ← spec 未覆盖
```

spec F3 把修复点定在 tokenizer `${}` 分支或 `extract_refs_from_ast`（`:644-649`）
是对的，471 次正是它的目标。**我上一版提的「`${}` 与裸标识符共用判据」是错的**——
那会绕开设计好的 `ComponentProperty` 路径，不是修它。

剩下的真缺口：`scanner/spg.rs:559-704` 的 match 处理
`ModelField`/`ComponentValue`/`Param`/`UserProperty`/`SystemVar`，
`ComponentProperty` 落进 `:705` 的 `_ => {}` 被静默丢弃；
而 `dependency.rs:48` 同时处理 `ComponentValue` 与 `ComponentProperty`。
**同一个 RefType，两个消费者契约不一致。**

建议：作为 PR2 或 PR5 的一个小项，明确 `ComponentProperty` 的图契约
（建 comp→comp `DependsOn` 边并带属性名，还是显式记诊断），并补一条
`parse → 上下文解析 → scanner 建边` 的贯通测试。当前量只有 4 处（后缀 `.txt`×3、
`.seconds`×1），**但 F1 放开容器后须重测**——届时会有新的组件 id 进入
`component_ids` 集合，改写为 `ComponentProperty` 的比例会变。

### 2.2 DataFlow 解析深度差是真的；歧义结论撤回

事实部分成立且可复核：

```
dwtable sources        2278
内联 dataFlow sources   399   共 1994 个节点
spg.rs:402-434 只取 moduleTablePath；tbl.rs:214+ 深度解析同一 schema
未被 .spg 侧解析：fields 1994、alias 1994、inputNodes 1259、joinConditions 329、steps 218
```

**但「50% 无法定偏好」撤回**（见 §0）。在没有具体失败 resolver trace 之前，
不能声称说明 B 有覆盖率缺口。

建议按治理走：deep parse 属于新模块（`dataflow::parse_nodes`），需 spec 改动而非
plan 条目。**建议记入 M58.5 候选**，除非有人能给出具体的 resolver 失败 trace ——
有 trace 再回来改 spec。本包不建议往 M58.3 塞。

### 2.3 PR2/PR3：代码可并行，验收不独立

我上一版说「实测无硬顺序、可并行」，**证明是循环的**：那个脚本只抽取白名单可达的组件，
再只扫这些组件上的引用，因此按构造就看不见 PR2 才暴露出来的记录——而同一份测量里
这些记录带 1,054 处表达式（`components` 键一行）。

修正表述：**PR2 与 PR3 代码层面可并行开发，但 PR3 的语料断言必须在 PR2 合入后重测才能关闭。**

同理，我上一版写的「允许 13-case 断言在 PR2 到 PR6 之间处于已知待更新状态」是错的。
应按 spec 验收 5 的要求：**在改变断言的那个 PR 里就更新，并附变迁台账**。
把 PR2 与 PR4b 相邻放置能省一次上下文切换，但两个 PR 只要各自独立可验收，
各自的重建就都省不掉——这一点我上一版也说过头了。

---

## 3. 对现有 plan 的三条具体修订建议

| # | 位置 | 建议 |
|---|---|---|
| 1 | plan PR2「前置：附录 A 逐键逐形态测量回填」 | 标记为**已完成**，指向 `tools/corpus-shape-audit.py` 与 `docs/ai-eval-runs/2026-08-24-m58-3-corpus-shape-audit.json`；排除列表初值取 §1.1 四键（`buttons` 待人工确认） |
| 2 | plan PR2 或 PR5 写入范围 | 增加 `ComponentProperty` 图契约小项（§2.1），含贯通测试；注明 F1 落地后需重测该计数 |
| 3 | plan PR2 验收 | 增加性能基线义务：组件数预计 +32%，按 [performance-baseline.md](../milestones/performance/performance-baseline.md) 的 M50 标准真实项目 runner 采集，记录节点/边数、graphdb 体积、全量构建与加载耗时、查询 P50/P95。**不要**用它反推预期值，采完为准 |

另有一处仓库层面的不一致：[INDEX.md:68](../milestones/INDEX.md) 的 M58.3 行 plan 列仍是 `—`，
但 plan 已存在于 `docs/plans/2026-08-24-m58-3-command-surface-gap-fixes-plan.md`。
未在本包内改动 INDEX，请 plan 转 approved 时一并回填。

---

## 4. 一个仍然成立的负面结论（省一个 PR）

属性层不是同一个「模式 3」，**不建议动**：

- `RawComponent` 未覆盖、且带 `${}`/`=` 的属性键只有 **45 处**，分布在 7 个键：
  `params` 19、`operator` 10、`title` 9、`validMessage` 4、`dynamicColumns` 1、
  `endLocationExp` 1、`resPathExp` 1；量最大的未覆盖键全是样式
  （`width` 22425、`templateName` 19436、`height` 18706、`minHeight` 14260），正确地忽略了。
- 已知键中值为对象/数组、被 `_ => continue` 跳过且内含 `${` 的，全语料仅 8 处。
  （以上三项均由 `tools/corpus-shape-audit.py` 的 `properties` 段产出，可复现。）
- `raw_types.rs` 的 49 个字段 / 两张分类表 / `fields` 元组三份平行清单目前完全同步，
  唯一异常是 `defaultValueExp` 同时在 always 与 conditional 里（无害，always 先命中）。

**适用范围限制**：以上只测了**当前可达**的组件。F1 放开容器后，新进入的
`columns` / `grid` / `attrFields` / `tabs` 等类型可能带不同的属性键，
**PR2 合入后须重跑本节测量**再下结论。这是维护成本问题（加一个键要改三处、
无机制保证同步），不是现存 bug。
