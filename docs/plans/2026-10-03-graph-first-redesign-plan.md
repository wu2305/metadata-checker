# 图优先重设计：实施计划（脚本线 + 图事实线并行）

> 状态：**draft**（待用户批准；批准前不写实现代码）
> 上位 spec：[图优先重设计](../specs/2026-10-02-graph-first-redesign-design.md)（draft，PR #6）
> 方向来源：2026-10-03 用户在决策卡上选「Both, parallel」，并说明最关心**数据处理逻辑与数据库操作逻辑**
> 已完成的前置：PR #8（扫描诊断逐次记录，已合入）、PR #9（阶段 1.0：空 `dbTableName` 与跨页引用解析）

## 0. 做什么、不做什么

两条线同时推进，各自拆成可独立评审的小 PR：

- **图事实线（spec 阶段 1）**：把「数据怎么流、动作做什么」沉进图——动作类型、取值模式、血缘边、诊断节点。
- **脚本线（spec 阶段 5）**：把 141 个后端 `.action.ts` 与每应用一个 `custom.ts` 里的**数据处理与数据库操作**沉进图。只存能成边或成事实的内容（读写哪张表、哪些字段、什么操作、调哪个外部地址或脚本、数据怎么加工），**不存脚本原文与无关字面量**。

不做：解析 `.fapp` / `.meta` / `.wfl`；删除任何旧动词（阶段 4 另批）；MCP；评测（阶段 2）；`.dash` / `.rpt`。

## 1. 共同前置：schema 注册表（PR #5）

两条线都要新增节点或边类型。PR #4 / #5 的 `graph_schema.rs` 是这些类型的单一登记处（`--graph-schema`、`--gql` help、生成文档、一致性测试都从它出）。
**PR #5 目前 on hold。** 它不放行，两条线只能先在 `graph.rs` 里加枚举，等 PR #5 合入后再补登记，这是返工。所以第一件事是用户决定是否放行 PR #5（见本计划的待决项）。

## 2. 图事实线

| 步 | 内容 | 门禁 |
|----|------|------|
| L1 | `action_kind`（动作类型）、`eval_mode`（取值模式）、循环标记入图 | 夹具上 `--explain` 的对应 fact block 可由 GQL 复现；B5 全量/增量逐属性相等 |
| L2 | 血缘边 `DerivesFrom`，带 `confidence` / `transform` / `lineage_kind`（借用 OpenLineage 的 `DIRECT` / `INDIRECT`，值域实现时对照现有 `transform` 文本） | 同上；真实语料上边数与抽样核对 |
| L3 | 诊断入图：`Diagnostic` 节点挂到所属节点（依赖 PR #8 的逐次记录） | 每个 `SCANNER_*` 记录对应一个节点；增量重扫后随文件退场 |
| L4 | 低成本边：`dimensionPath`（字段→表；先对账 26,994 与 32,231 的差）、`properties.depends` 指向真实 model 节点、`webAPI.url`/`scriptFunction`/`shortUrls` 作属性 | autocrm 上计数与预期一致 |
| L5 | 摄入时汇总「见过但未映射」的 JSON 键为 `Diagnostic` | 夹具上故意放未知键，图里能查到 |

遗留（PR #9 已声明）：解析正确但目标文件不存在的页面引用（autocrm 上 7 个）、17 张缺省 `dbTableName` 键的数据流（语义待确认）。

## 3. 脚本线

| 步 | 内容 | 门禁 |
|----|------|------|
| S0 | **只读 spike，不进产品代码**：对 141 个 `.action.ts` 跑 oxc 解析成功率、调用参数里字符串字面量对计算值的比例、oxc 加入后的二进制体积 | 报告落共享文件夹；体积按 AGENTS.md 在远端 `cargo build --release` 记录 |
| S1 | `Script` 节点（`X.action.ts` 为源，`X.action` 编译副本不单列）+ 确定性链接：`.tbl` 数据流 `Script` 节点按裸 hash id → 脚本；`.spg` `webAPI.url` → 脚本路径；脚本 `import` → 脚本；`.spg` `script` 动作按（页面键、函数名）→ `custom.ts` 函数 | autocrm 上每种链接形态的命中数与盘点报告一致；解析不了的链接记诊断，不造空节点 |
| S2 | oxc 层 1：数据库读写（表、字段、操作类型）、外部调用、脚本间调用，`origin=parser`，带文件与行区间 | 抽样 20 个脚本人工对照；不存脚本原文 |
| S3 | 层 2：LLM 补残余，走 PR #5 §6 的校验提交命令，`origin=llm`，必须带引用片段并由校验器核对 | 层 1 能推出的不接受 LLM 提交 |

S1 起依赖 PR #5（新节点/边类型要登记）；S0 不依赖，可立刻开始。

## 4. 并行的冲突点与规则

- 热点文件：`graph.rs`（`NodeType`/`EdgeType`）、`graph_schema.rs`（PR #5 之后）、`scanner/spg.rs`、`scanner/tbl.rs`、`tests/` 里的快照。
- 规则：**同一时刻只有一个 PR 改枚举与 schema 登记**，另一线的 PR 等它合入后再基于 main 更新；其余文件按线分开。
- 每个 PR 单独说明 `Baseline impact`，涉及依赖的（S0/S2 的 oxc）记录二进制体积变化。
- 扫描语义变化时递增 `SCANNER_SEMANTICS_VERSION`（PR #9 引入），保证旧图重扫。

## 5. 待决项

1. 是否放行 PR #5（上文 §1）。
2. 本计划与 spec 的批准（spec 仍是 draft）。
3. spec §7 里仍未问的：stdio 工具表（Q3）、M59-PATH（Q5）、单文件模式（Q6）、评测通过线（Q7）。它们属于阶段 2 之后，不阻塞本计划。
