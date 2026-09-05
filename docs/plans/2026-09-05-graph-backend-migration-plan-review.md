# 图数据库更换计划复核（2026-09-05）

> 状态：**评估意见，非计划本体**
> 复核对象：[2026-09-04-local-query-plane-and-backend-research-plan.md](2026-09-04-local-query-plane-and-backend-research-plan.md)（approved）
> 触发：M58.3 deferred 范围（F3/F4/F5/F6 + PR6）与「随 redb 退役」的一批 P2 决议
> 复核人立场：只读复核，不改计划正文；结论按「该保留 / 该改 / 该补」三类给出

## 0. 一句话结论

计划的**方法论是对的**（先冻结真实缺口、再选型、原型不侵入运行时），但**问题框定
偏了一档**：它把痛点定义为「查询表达力不够，分析者被迫回到 jq」，而基线里最硬的
事实是 **stdio 启动加载 1,008 秒**。表达力问题换个引擎就能解，17 分钟冷启动换引擎
解不了——那是「全量 hydrate 进内存再查」这个架构决定的。**真正的决策变量不是选哪个
库，而是查询到底在存储里执行还是在内存图里执行。** 计划把这一条写成了阶段 4 的一个
验收 bullet，应当提升为整个计划的主假设。

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

### 2.6 「受限 Cypher over 现有 GraphReadStore」应当是基线，不是第三候选

四个候选里，第 3 个（基于现有 `GraphReadStore` 的受限 Cypher）是**唯一完全不碰
持久化层**的方案：零迁移风险、零 schema 变更、零二进制膨胀、天然同时支持 native
和 wasm。

它也直接对准计划自己定义的痛点——`GraphReadStore` 今天只能按 ID 取节点、取邻边、
遍历和计数，表达不了过滤/投影/聚合/路径。补一层受限 `MATCH`/`WHERE`/`RETURN`/
`ORDER BY`/`LIMIT` 就把阶段 0 的大部分 jq 样例接住了。

**建议**：把它设为**基线（control）**，其余候选必须证明自己相对它有可测量的优势
才值得承担迁移成本。注意它**不解决** 2.1 的 1,008 秒冷启动——所以这恰好把两个问题
干净地分开了：

| 问题 | 归属方案 |
|---|---|
| 表达力缺口（分析者回退 jq） | 受限 Cypher over `GraphReadStore`，不换后端 |
| 冷启动 1,008 s / 全量 hydrate | 查询下推架构改造，可能换后端 |

计划把这两件事捆在一个决策里，导致「要不要换数据库」这个问题被表达力需求推着走，
而表达力需求其实换不换都能解。

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

```
现在        PR4a（schema 版本键 + fail-closed + 强制全量重建）
            ├── 不依赖任何选型结论，换不换后端都必须有
            └── 是 F4 与任何迁移的共同前置

然后        阶段 0（真实 jq 缺口冻结）+ 诊断计数基线重测
            └── 同一个 snapshot_id，一次跑完

并行        受限 Cypher over GraphReadStore 的最小 spike（基线臂）
            └── 直接检验「表达力缺口是否换后端也能解」

之后        阶段 2 尽调，先执行两条淘汰规则：
            ① release binary > 10 MB → 出局
            ② 不能编 wasm32 → 降级为 native 可选加速层，不得替换抽象
            并核实 Grafeo 的存在性与维护状态

再之后      F4 / PR4b（页面局部身份），schema 一次到位
            └── 阶段 4 原型的投影直接按 F4 后的身份文法生成

最后        阶段 4/5 原型与决策门，头条指标 = 冷启动到首条查询
```

## 5. 复核范围声明

本文基于：计划正文、M58.3 spec、`performance-baseline.md`、`AGENTS.md`、`Cargo.toml`
与仓库内 grep 结果。以下为**已在本仓库核实**的事实：PR4a 未实现
（`graph_schema_version` / `GRAPH_SCHEMA_STALE` 全仓无实现）、redb 仅挂 `cli-local`
feature、`persistence/` 下 redb 与 indexeddb 并列、v2 仍为 shadow-only。

以下为**未核实项**，不作为结论依据：Grafeo 的存在性与维护状态、DuckDB 静态链接的
实际体积、任何候选在 wasm32 上的可编译性。这些是阶段 2 的输入。
