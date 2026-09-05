# 图数据库更换计划复核（2026-09-05）

> 状态：**评估意见，非计划本体**
> 复核对象：[2026-09-04-local-query-plane-and-backend-research-plan.md](2026-09-04-local-query-plane-and-backend-research-plan.md)（approved）
> 触发：M58.3 deferred 范围（F3/F4/F5/F6 + PR6）与「随 redb 退役」的一批 P2 决议
> 复核人立场：只读复核，不改计划正文；结论按「该保留 / 该改 / 该补」三类给出

## 0. 一句话结论

**换后端解决不了计划想解决的问题。**

本文第 1 版基于计划正文与仓库 grep，把痛点重新框定为冷启动。补充证据（2026-08-30/31
两个 codex session 的完整命令记录，见 §0.1）到手后，结论要往前推一步：jq 回退是真实
的、量化的、可分类的，但**把 315 条 jq 调用逐条分类之后，只有 4 条是图形状的查询**。
其余 311 条落在三个和存储后端无关的桶里：工具输出契约、属性覆盖、关系型表投影。

因此计划里那句「先研究与原型，后决定是否替换 redb」应当收敛为一个更早、更便宜的
结论：**先把这三个桶填了；填完之后 jq 回退还剩多少，才是后端选型的真实输入。**

## 0.1 补充证据：315 条 jq 调用的逐条分类

来源：`~/.codex/sessions/2026/08/30/…01a05233-5b52-7da2-acf0-eae776264098.jsonl`（96 MB）
与 `…/31/…01a056da-5e7c-7fa0-8f62-ff24bf05075e.jsonl`（51 MB）。这两个 session 正是
计划 §2.1 引用为需求证据的那两个。原始命令含真实业务路径，按计划的脱敏要求**不入库**，
此处只登记形态与统计。

从中提取到 632 条含 `jq` 的命令，去掉 `/bin/zsh -lc` 包装造成的重复后 **315 条**，
另有 468 条 `metadata-checker` 调用。315 条分类如下：

| 桶 | 条数 | 占比 | 形态 | 换后端能否解决 |
|---|---:|---:|---|---|
| **A 裁剪工具自身输出** | 100 | 32% | `metadata-checker … \| jq '{query_target,summary:{conclusion,page_role,entrypoint_count,…},details:{entrypoints,action_flows,data_sources,write_targets,navigation,visibility_rules,risk_diagnostics}}'` | **否**。这是输出契约问题 |
| **B4 跨文件扁平表投影** | 110 | 35% | `jq -r '.. \| objects \| select(.actions?) \| .actions[] \| [$c,.actionType,.dataSet,…] \| @tsv'`；`for f in <6 个 spg>; do …` | **否**（部分是覆盖问题） |
| **B1 按 id 取节点全部属性** | 52 | 17% | `jq '[.. \| objects \| select(.id? == "button6") \| {id,type,value,text,label,title,tooltip,visible,disableCondition,action_count}]'` | **否**。定位已经有了，缺的是把原始属性集交回来 |
| **B5 浏览** | 24 | 8% | 纯投影 | 否 |
| **B3 计数/聚合** | 21 | 7% | `\| length`、`group_by`、`unique` | 部分 |
| **B2 结构筛选** | 8 | 2.5% | `select((.components? // []) \| any(.id? == "combobox3"))` | 部分 |
| **真·图形状（递归/关系遍历）** | **4** | **1.3%** | `walkmenu` 递归菜单树、`recurse`、`children[]` | 是 |

B 类（215 条，直接 jq 原始文件）的形态统计：

- **46% 要求 `@tsv` 扁平表输出**，111 条带 `jq -r`——想要的是**表**，不是 JSON 树，
  更不是图遍历；
- **33% 一条命令跨多个文件**（多参数或 `for f in …` 循环整个目录）；
- 最常投影的字段（top 25）：`id(75) type(52) value(39) visibleCondition(38)
  disableCondition(37) actions(35) name(34) submitField(33) title(28) notNull(24)
  path(15) filter(14) dataSet(10) …`

### 0.1.1 五个高频字段，图里根本没有

对上表里的字段做全仓 `grep '"<字段>"' src/`：

| 字段 | src 命中文件数 | 出现在哪 |
|---|---:|---|
| `businessDesc` | **0** | `.tbl` 的 `dimensions[]`，字段的**业务描述** |
| `notNull` | **0** | 组件必填标记（jq 投影里出现 24 次） |
| `validMessage` | **0** | 校验失败文案 |
| `resourceRoot` | **0** | **应用菜单树**（分析者手写递归 `walkmenu` 重建菜单路径） |
| `modelDataType` | **0** | `sources[].content.properties` |
| `validExp` | 4 | 已覆盖 |
| `submitField` / `dbfield` / `dimensions` | 5 / 7 / 7 | 已覆盖 |

这就是「工具没有暴露」那一半：**不是查不出来，是根本没抽进图。** 任何后端都读不到
一个不存在的事实。`businessDesc` 尤其致命——对一次业务分析而言，字段的业务描述大概是
整个语料里最值钱的字符串，而图不带它。`resourceRoot` 是应用菜单树，属一等结构，也没抽。

### 0.1.2 对计划的三条修正

1. **A 桶（32%）根本不是查询问题。** 分析者拿到工具输出后手工重投影，保留
   `query_target` + `summary` 的若干子键 + `details` 的若干子键。这是「给多了 / 给的
   切片不对」，属输出契约（`--budget` 档位、字段选择、profile），**在 `output.rs` 里
   解决，一行后端代码都不用改**。
2. **想要的形状是表，不是图。** 46% 显式 `@tsv`，33% 跨文件。这直接**反对**计划
   §4 把「Grafeo 原生 Cypher」定为首选路线——需求形状更接近
   `SELECT comp_id, action_type, data_set, condition_exp FROM actions WHERE page = ?`。
   若最终真要选后端，这条证据把天平推向关系型投影而非 Cypher。
3. **1.3% 才是图查询。** 315 条里 4 条。以此为据启动一次图数据库迁移，投入产出比
   是失衡的。

### 0.1.3 一个被忽略的张力

填 §0.1.1 的覆盖缺口意味着**往图里抽更多字段**，图会变大，而冷启动已经是
**1,008 秒**（89,094 节点 / 199,575 边 / 514 MiB，`performance-baseline.md:775-790`）。
覆盖与冷启动相互拉扯：不解决「查询在存储里执行还是在内存图里执行」，补覆盖只会让
冷启动更糟。这两件事必须一起决策，但**它们都不是「选哪个数据库」**。

## 0.2 冷启动框定（原第 0 节，保留为并列问题之一）

基线里最硬的事实是 **stdio 启动加载 1,008 秒**。表达力问题换个引擎就能解，17 分钟
冷启动换引擎解不了——那是「全量 hydrate 进内存再查」这个架构决定的。**真正的决策变量
不是选哪个库，而是查询到底在存储里执行还是在内存图里执行。** 计划把这一条写成了
阶段 4 的一个验收 bullet，应当提升为主假设之一。

结合 §0.1，最终框定是**三个独立问题**，只有第三个可能需要碰后端：

| 问题 | 证据 | 归属 |
|---|---|---|
| 输出给多了 / 切片不对 | 100/315 条 jq 在裁剪工具输出 | `output.rs` 输出契约 |
| 事实没抽进图 | 5 个高频字段 src grep = 0 | scanner 覆盖 |
| 冷启动 1,008 s + 表投影缺口 | 基线实测；46% `@tsv`、33% 跨文件 | 查询执行模型（可能换后端） |

## 1. 该保留的（不要在后续修订里丢掉）

1. **「没有真实样例，不进入后端选型结论」这道门槛**（阶段 0）。这是全篇最值钱的一句。
   图数据库选型最常见的失败模式就是先爱上一个引擎再去找需求。
2. **两个平面的切分**：图查询返回结构化关系 + 文件定位，原始 JSON 由既有文件读取
   链路提供。拒绝把 raw body 复制进候选后端是对的——那等于用图数据库重新实现一个
   很差的文档库，而项目已经有能直接读 `.spg`/`.tbl` 的链路。
3. **阶段 4 的不侵入原型 + 那条可证伪的检查**：「查询-only 进程必须验证不打开、不
   hydrate redb，不能从已 hydrate 的 redb 再生成投影」。这条把「看起来变快了」和
   「真的绕开了 hydrate」区分开，是全篇最硬的验收设计。
4. **阶段 6 拒绝把 M58.3 遗留算作已解决**。迁移项目最典型的死法就是把旧身份问题
   原样复制到新后端。

## 2. 该改的

### 2.1 主假设错位：把「绕开全量 hydrate」提为头号指标

现状：`89,094` 节点 / `199,575` 边 / graphdb `514 MiB`，stdio 启动加载
**1,008.1 s / 1,011.8 s**（2 轮实测，`performance-baseline.md:775-790`）。

这个数字的成因是启动时把整张图 hydrate 进内存，不是 redb 的 KV 读写慢。因此：

- 若新后端仍然「启动即 hydrate、查询在内存图里跑」，换 redb 为 DuckDB/Grafeo
  **不会改善这 1,008 秒**，只会换一种编码格式；
- 若查询下推到存储引擎里执行（按需读，不 hydrate），那么**redb 也未必需要换**——
  需要换的是 `GraphReadStore` 之上的查询执行模型。

**建议**：阶段 5 的头条指标改为「冷启动 → 首条查询返回」的端到端 wall time，
数据库体积降为次要指标（计划第 5 节已经说「决策不能只看数据库大小」，但阶段 5 的
指标清单仍把体积排在延迟前面）。同时在阶段 2 给每个候选加一列显式表态：
**该候选是否支持查询下推、下推到什么粒度**。

### 2.2 F4 / PR4a 的排序反了：迁移机制目前根本不存在

计划阶段 6 写「须先重新处理 M58.3 尚未完成的页面局部身份/schema 与重建验收，
**或**明确记录兼容适配器」。这个「或」是个陷阱：

- 适配器保住全局 `model:<name>` 身份，等于把缺陷平移进新后端；
- 而按 M58.3 spec 的迁移矩阵（`:191-230`），改身份要动**每一类边的端点**：
  `Contains`(model→field)、`Reads`/`FieldWrite`、`FieldAlias`、
  `DataflowInput`/`DataflowOutput`、`DependsOn`(cond→owner)、
  `DependsOn`(cond→符号)，外加 `next_queries` 的 **57 处调用 / 16 个文件**审计。

**这套迁移做两遍（一遍在 redb 上、一遍在新后端上）严格劣于做一遍。**

更要紧的是一条**已核实的事实**：schema 版本强制重建机制**没有实现**。全仓
`grep graph_schema_version` / `GRAPH_SCHEMA_STALE` 只命中
`src/diagnostics.rs:39` 的一句「PR4a 的 GRAPH_SCHEMA_STALE 待该 PR 再引入」。
而 spec 已写明：bump `REDB_V2_SCHEMA_VERSION` **不会**重建旧 v1 节点——版本不匹配
只是 `read_v2_layout` 返回 `None` 回退 v1，`--build-graph` 又按文件 hash 跳过未变
文件。

结论：**今天这个仓库没有任何能强制全量重建的开关。** 任何 schema 变更——换后端也
好、F4 局部身份也好——都会撞上「旧库静默按旧口径继续服务」。

**建议**：把 **PR4a（`graph_schema_version` 权威键 + hydrate 前校验 + fail-closed
`GRAPH_SCHEMA_STALE` + 强制全量重建路径）提为阶段 4 的前置**，而不是留在 deferred
里。它不依赖任何后端选型结论，成本远小于迁移本身，且**无论最终换不换后端都必须有**。
阶段 4 的原型投影 schema 应当直接按 **F4 之后**的局部身份文法生成，即使此时 redb
运行时仍跑旧身份——否则原型验证的是一个已知要废弃的 schema。

### 2.3 二进制体积应当是阶段 2 的淘汰规则，不是打分项

约束：release binary ≤ 10 MB（AGENTS.md），计划记录当前约 3.6 MiB。

计划把「二进制体积」列为研究维度之一，与「维护状态」「许可证」平级打分。但这不是
打分项，是**硬淘汰线**：静态链接的 DuckDB 体量是这个预算的数量级以上。

**建议**：阶段 2 开篇写死一条淘汰规则——任何候选把 release binary 推过 10 MB，
直接出局，不再投入 spike 成本。这很可能在花任何原型成本之前就淘汰掉 DuckDB，
从而把阶段 4 的两个 spike 缩成一个。（具体体积须实测确认，不要引用二手数字。）

### 2.4 缺一个硬约束：browser-wasm 目标

计划的研究维度写了「四平台」，指的是 `windows_amd64` / `macos_arm64` /
`linux_amd64` / `linux_arm64` 四个 native 平台。**`wasm32` 不在里面。**

但这个仓库是有 wasm 目标的：`Cargo.toml` 的 `browser-wasm` feature 独立成栈，
`redb` 只挂在 `cli-local` 下，`src/persistence/` 里 `redb.rs` 与 `indexeddb.rs`
并列，AGENTS.md 明写「browser-wasm 不得引入 redb」。

也就是说，今天的架构**已经**是「native 用 redb，wasm 用 IndexedDB stub，两边共用
`GraphReadStore` 抽象」。如果新后端编不到 wasm32，结果不是「换掉 redb」，而是
**多养一套查询引擎**——native 一套 Cypher/SQL，wasm 一套手写遍历。那比现状更差。

**建议**：阶段 2 增加一条硬维度「wasm32 可用性」，并明确表态：

- 若候选可编 wasm32 → 有机会统一两个平面；
- 若不可 → 候选只能定位为「native 侧的可选加速层」，`GraphReadStore` 抽象必须保留，
  且必须回答「wasm 侧的等价查询能力谁提供」。

这条不解决，阶段 6 会撞墙。

### 2.5 Grafeo 被指定为首选路线，但它的存在性/维护状态尚未验证

计划第 4 节把「Grafeo 原生 Cypher + `with_read_store`」定为阶段 4 的优先 spike，
而 Grafeo 的维护状态、许可证、Rust API 恰恰是**阶段 2 才要去核实**的内容。这是个
顺序倒置：先钦定了首选，再去做尽调。

我无法从本仓库或既有文档中确认 Grafeo 是一个真实、在维护的嵌入式 Rust Cypher crate；
计划里也没有版本号、仓库链接或日期。按计划自己的「证据规则」（优先官方文档和官方
仓库、记录版本日期、区分事实与未验证项），**Grafeo 目前属于未验证项，不该占据
「推荐初始技术路线」的位置**。

**建议**：阶段 2 完成前，第 4 节的推荐降级为「候选之一」；核实通过（有仓库、有
license、有近期 release、能编到目标平台）后再恢复首选地位。

### 2.6 基线臂应当是「表投影 over 现有 GraphReadStore」，而不是受限 Cypher

四个候选里，第 3 个（基于现有 `GraphReadStore` 加一层查询）是**唯一完全不碰持久化
层**的方案：零迁移风险、零 schema 变更、零二进制膨胀、天然同时支持 native 和 wasm。
它应当是**基线（control）**，其余候选必须证明相对它有可测量的优势，才值得承担迁移
成本。

但 §0.1 的证据改了这层查询该长什么样。计划把候选 3 写成「受限 Cypher」，而 215 条
直接 jq 里 **46% 要 `@tsv` 扁平表、33% 跨文件、只有 4 条是递归/关系遍历**。真实需求
是「把某一类事实按行摊平成表，跨文件筛一下」，不是「沿边走几跳」。

**建议**：基线臂的第一版做**表投影**而不是 Cypher 子集——按事实类型暴露若干只读
视图（`components`、`actions`、`sources`、`conditions`、`writes`、`fields`），
支持 `WHERE` 等值/前缀/正则过滤、字段投影、`LIMIT`、TSV/JSON 双输出。这直接对着
B1/B3/B4/B5（**207/315，66%**）。`MATCH`/多跳留到确有需求再说——目前证据只有 4 条。

三个问题与归属方案的对应：

| 问题 | 证据规模 | 归属方案 | 碰后端吗 |
|---|---:|---|---|
| 输出给多了 / 切片不对 | 100/315 | `output.rs` 字段选择 / profile | 否 |
| 事实没抽进图 | 5 个字段 grep=0 | scanner 覆盖扩展 | 否 |
| 表投影缺口 | 207/315 | 表投影层 over `GraphReadStore` | 否 |
| 图形状查询 | 4/315 | 受限 `MATCH`，可延后 | 或许 |
| 冷启动 1,008 s | 基线实测 | 查询执行模型（下推 vs hydrate） | **是** |

计划把这些捆在一个决策里，于是「要不要换数据库」被表达力需求推着走——而表达力需求
里 66% 换不换都能解，32% 压根不是查询问题。

## 3. 该补的

1. **v2 shadow 的处置必须现在表态。** `src/graph_redb_v2.rs` 至今是 shadow-only
   （`REDB_V2_SCHEMA_VERSION = "m52-redb-v2-shadow"`，文件头注明「M53 再切入
   hydrate / incremental persist」），而 deferred 里又把「v2 shadow 族三处」「坏行
   hydrate 计数断言」列为**随 redb 退役**处理。这是最坏的中间态：既在付维护成本，
   又不交付收益。建议在阶段 5 决策门产出里显式二选一——**冻结**（明确不再投入，
   代码打上 deprecation 注释）或**落地**（切 hydrate/incremental persist）；在此
   之前不再新增 v2 代码。
2. **一个明确的「不换」出口。** 计划第 5 节第 8 条写「独立复核确认后，才决定是否
   移除或降级 redb」，但全篇没有描述「redb 留下」这个结局长什么样、需要补哪些工作。
   建议补一节「保留 redb 的收尾清单」（至少含 PR4a 强制重建、v2 处置、受限 Cypher
   查询层），让「不换」成为一个有交付物的结论，而不是「研究没做完」的默认状态。
3. **诊断计数基线需要在阶段 0 一并重新钉。** `performance-baseline.md` 记录
   `SCANNER_UNRECOGNIZED_CONTAINER_KEY = 312`（交叉验证口径 `moreFields 279 +
   params 33`）与 `SCANNER_DUPLICATE_COMPONENT_ID = 1,105`；复核返修 P1-7 改了组件
   身份判定，两个数都会漂移，312 的「精确吻合」交叉验证已不成立（见
   [2026-09-05 spec](../specs/2026-09-05-m58-3-condition-symbol-normalization-design.md)
   第 5 节）。阶段 0 既然要固定 `snapshot_id` 和内容 hash，顺带把这两个数重测回登，
   否则阶段 5 拿不到可比的前后对照。

## 4. 建议的执行顺序

§0.1 的分类把「先做什么」变得相当明确：**三件不碰后端的事覆盖了 315 条里的 311 条，
而且都比迁移便宜一个数量级。** 先做它们，再看还剩多少 jq。

```
第 1 步     输出契约：--budget 之外补字段选择 / profile          → 接住 A 桶 100 条
            └── 只动 output.rs，零后端改动，成本最低收益最直接

第 2 步     scanner 覆盖：businessDesc / notNull / validMessage   → 接住"没有暴露"那半
            / resourceRoot(菜单树) / modelDataType
            └── 先量化每个字段进图后的体积增量（见 §0.1.3 的张力）

第 3 步     表投影层 over GraphReadStore（基线臂）                → 接住 B1/B3/B4/B5 207 条
            └── 只读视图 + WHERE + 投影 + LIMIT + TSV/JSON
            └── 跑完之后重测：jq 回退还剩多少条？这才是后端选型的真实输入

并行        PR4a（schema 版本键 + fail-closed + 强制全量重建）
            ├── 不依赖任何选型结论，换不换后端都必须有
            └── 是 F4 与任何迁移的共同前置

然后        阶段 0（jq 缺口冻结，按 §0.1 的分类轴而非只按查询意图）
            + 诊断计数基线重测，同一个 snapshot_id 一次跑完

之后        只有在第 3 步之后 jq 回退仍显著，才进阶段 2 尽调。
            先执行两条淘汰规则：
            ① release binary > 10 MB → 出局
            ② 不能编 wasm32 → 降级为 native 可选加速层，不得替换抽象
            并核实 Grafeo 的存在性与维护状态

再之后      F4 / PR4b（页面局部身份），schema 一次到位
            └── 阶段 4 原型的投影直接按 F4 后的身份文法生成

最后        阶段 4/5 原型与决策门，头条指标 = 冷启动到首条查询
```

注意第 1–3 步**没有一步需要先决定用哪个数据库**。这正是本文的核心主张：后端选型
不是这条路的起点，是它的可选终点。

## 5. 复核范围声明

本文基于：计划正文、M58.3 spec、`performance-baseline.md`、`AGENTS.md`、`Cargo.toml`、
仓库内 grep 结果，以及 §0.1 所述两个 codex session 的完整命令记录（共 147 MB
jsonl，机器解析全量命令，非抽样阅读）。以下为**已在本仓库核实**的事实：PR4a 未实现
（`graph_schema_version` / `GRAPH_SCHEMA_STALE` 全仓无实现）、redb 仅挂 `cli-local`
feature、`persistence/` 下 redb 与 indexeddb 并列、v2 仍为 shadow-only。

以下为**未核实项**，不作为结论依据：Grafeo 的存在性与维护状态、DuckDB 静态链接的
实际体积、任何候选在 wasm32 上的可编译性。这些是阶段 2 的输入。
