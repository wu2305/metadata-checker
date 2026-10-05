# 图优先重设计：实施计划（脚本线 + 图事实线并行）

> 状态：**draft**（待用户批准；批准前不写实现代码）
> 上位 spec：[图优先重设计](../specs/2026-10-02-graph-first-redesign-design.md)（draft，PR #6）
> 方向来源：2026-10-03 用户在决策卡上选「Both, parallel」，并说明最关心**数据处理逻辑与数据库操作逻辑**
> 已完成的前置（均已合入 `main`）：PR #5（`graph_schema.rs` 契约表与 `--graph-schema`）、PR #8（扫描诊断逐次记录）、PR #9（阶段 1.0 的ⓐ、ⓑ：空 `dbTableName` 与跨页引用解析；ⓒ、ⓓ未完成，见 L0b）、PR #10（契约表补 `landed` 与 `IndexState` 记录形态）

## 0. 做什么、不做什么

两条线同时推进，各自拆成可独立评审的小 PR：

- **图事实线（spec 阶段 1）**：把「数据怎么流、动作做什么」沉进图——动作类型、取值模式、血缘边、诊断节点。
- **脚本线（spec 阶段 5）**：把 141 个后端 `.action.ts` 与每应用一个 `custom.ts` 里的**数据处理与数据库操作**沉进图。只存能成边或成事实的内容（读写哪张表、哪些字段、什么操作、调哪个外部地址或脚本、数据怎么加工），**不存脚本原文与无关字面量**。

不做：解析 `.fapp` / `.meta` / `.wfl`；删除任何旧动词（阶段 4 另批）；MCP；评测（阶段 2）；`.dash` / `.rpt`。

## 1. 共同前置：schema 注册表（已落地）

两条线都要新增节点或边类型。`graph_schema.rs`（PR #5，2026-10-03 合入）是这些类型的单一登记处（`--graph-schema`、`--gql` help、生成文档、一致性测试都从它出）。
新增节点或边类型时，`graph.rs` 的枚举与 `graph_schema.rs` 的表行必须同一个 PR 改（一致性测试会拦），见 §4 的冲突规则。

## 2. 图事实线

| 步 | 内容 | 门禁 |
|----|------|------|
| L0 | **写入校验**（PR #4 的 S2；spec 阶段 1 ① 要求它先于新图形状）：契约表现在只描述不拦截，新增的血缘边、诊断节点、脚本节点边若没有它，端点错误、缺必填元信息或未登记的形状都能落盘。让写入路径对照 `graph_schema.rs` 拒收违规项并说明原因（不静默丢弃） | 对每类违规（端点类型不符、缺必填 meta、未登记的边/节点类型）各有拒收测试；**L1、L2、L3、L4、S1 起的新增类型都以它为前提** |
| L0b | **幽灵 Page 收尾**（spec 阶段 1.0 ⓒ）：解析正确但目标文件不存在的页面引用（autocrm 上 7 个）不再留无文件的 Page 节点，改为 `SCANNER_UNRESOLVED_REFERENCE`（入图后按 L3 成为 `Diagnostic`）；判定用本次发现的文件集合 | autocrm 上每个 `Page` 节点都对应磁盘文件；夹具快照不变；增量/全量逐属性相等 |
| V2 | **schema v2：扁平边属性**（契约设计 §3.3 的版本拒载，加 §4 修订后的**扁平属性白名单**：`source_field`、`confidence`、`analyzer`、`script_hash`，其余仍在 `meta`；该白名单修订随本 PR 一起待批准，批准前 V2 不开工）。现在 Grafeo 边只落 `field_path`、JSON 文本 `meta`、`origin_file`（`graph_grafeo.rs` 的 `PROP_*`），L2 要按字段过滤的 `source_field`、S3 要的 `confidence` / `analyzer` / `script_hash` 都进不了可过滤的位置。内容：边模型加扁平属性、存储版本键与旧版拒绝（重建即迁移，但不得悄悄读旧库）、序列化、GQL 与 `--graph-schema` 暴露 | 夹具上 `WHERE r.confidence = 'high'`、`WHERE r.source_field = ...` 可直接过滤；旧版存储被明确拒绝并提示重建；B5 逐属性相等 |
| L1 | `action_kind`（动作类型）、`eval_mode`（取值模式）、循环标记入图 | 夹具上 `--explain` 的对应 fact block 可由 GQL 复现；B5 全量/增量逐属性相等 |
| L2 | 血缘边 `DerivesFrom`，带 `confidence` / `transform` / `lineage_kind`（借用 OpenLineage 的 `DIRECT` / `INDIRECT`，值域实现时对照现有 `transform` 文本）**以及现有 `--explain` 血缘已有的 `via_node` 与证据 `json_path`**（少了这两项，GQL 使用者就找不到中间节点和证据位置，比现在更不可信）。**依赖 V2**：值来源闭包要按字段追，`source_field` 必须是可过滤的边属性 | 同上，且对等门禁逐项比对 `via_node` / `json_path`；真实语料上边数与抽样核对 |
| L3 | 诊断入图：`Diagnostic` 节点带 `path` / `json_path` 与 `detail`（含未解析引用的原始标识），对象存在时用 `HasDiagnostic` 边挂到所属节点；**文件级失败（整文件解析失败，没有 Page / Model 可挂）只有节点、按 `path` 查**。来源不止扫描记录：①`SCANNER_*`（PR #8 的逐次记录）；②`GRAPH_OWNERSHIP_CONFLICT`（`scanner/indexer.rs` 写入，`runtime.rs` 重扫时清除，生命周期与扫描记录不同，要单列）；③`UNKNOWN_ACTION_TYPE`（现在查询期算，入图需导入期计算，可复算性实现时核实）（spec §2.3） | 每个 `SCANNER_*` 记录对应一个节点；**`GRAPH_OWNERSHIP_CONFLICT` 与 `UNKNOWN_ACTION_TYPE` 各有夹具与旧输出对照**；解析失败的文件也能按路径查到；增量重扫后随文件退场 |
| L4 | 低成本边：`dimensionPath`（字段→表；先对账 26,994 与 32,231 的差）、`properties.depends` 指向真实 model 节点、`webAPI.url`/`scriptFunction`/`shortUrls` 作属性 | autocrm 上计数与预期一致 |
| L5 | 摄入时汇总「见过但未映射」的 JSON 键为 `Diagnostic` | 夹具上故意放未知键，图里能查到 |

遗留（PR #9 已声明）：17 张缺省 `dbTableName` 键的数据流（语义待确认，待用户答复）。

## 3. 脚本线

| 步 | 内容 | 门禁 |
|----|------|------|
| S0 | **只读 spike，不进产品代码**：对 141 个 `.action.ts` **和 6 个 `custom.ts`**（前端脚本的语法与调用形态可能不同，S2/S3 都要用）跑 oxc 解析成功率、调用参数里字符串字面量对计算值的比例、oxc 加入后的二进制体积；**顺带统计被当作调用参数抽取的字面量里带凭据形态的数量**（URL 用户信息、token 查询参数），好让是否需要规则由数据决定（spec §3.5） | 报告落共享文件夹；体积按 AGENTS.md 在远端 `cargo build --release` 记录 |
| S1a | **脚本文件进入摄入管线**（现在文件发现与 `parser.rs` 只认 `.spg` / `.tbl`，光改 `graph.rs` 与扫描热点，S1/S2 永远看不到脚本文件）。**本地与远端两条路径都要改**：本地——登记新的源文件类型、文件发现、解析结果（`ParsedGraphContent` 新变体）、来源账本、增量与删除生命周期（改一个脚本只重扫它，删一个脚本撤销它的节点与边）；远端会话——`MetadataContentType` 加脚本变体（`remote_metadata.rs`，现在只有 SuperPage / Table / Unknown）、`is_analyzable_content_path`（现在只认 `spg` / `tbl` / `json`，不含 `ts`）、`session::remote_sync::is_analyzable_entry`（现在只收 SuperPage / Table）、内容队列、同步与删除处理。`X.action` 编译副本不进管线 | 夹具上放一个 `.action.ts` 与一个 `custom.ts`：全量建图出现它们的文件状态记录，改动只重扫该文件，删除后随文件退场；**远端会话路径有对应测试（同步进来、改动重拉、删除撤销）**；B5 全量/增量逐属性相等 |
| S1 | `Script` 节点（`X.action.ts` 为源，`X.action` 编译副本不单列）+ 确定性链接：`.tbl` 数据流 `Script` 节点按裸 hash id → 脚本；`.spg` `webAPI.url` → 脚本路径；脚本 `import` → 脚本；`.spg` `script` 动作按（页面键、函数名）→ `custom.ts` 函数。**解析不了的链接不造占位 `Script` 节点**（与页面引用一致，PR #9），而是记一条 `Diagnostic`（`detail` 带原始脚本标识），用 `HasDiagnostic` 边挂在发出引用的 `Action` / `Model` / `Script`（`import` 找不到目标时）上，**L0 契约表里 `HasDiagnostic` 的合法起点要包含 `Script`**，图里仍能查到「它期望哪个脚本、没找到」（契约设计 §6.3 已同步修订）。依赖 L3 的 `Diagnostic` 节点；L3 落地前只记 `SCANNER_UNRESOLVED_REFERENCE` | autocrm 上每种链接形态的命中数与盘点报告一致；解析不了的链接各有一条可从发出引用的节点查到的 `Diagnostic`（含一个缺目标的 `import` 夹具，从 `Script` 出发查），且没有空 `Script` 节点 |
| S2 | oxc 层 1：数据库读写（表、字段、操作类型）、外部调用、脚本间调用，`origin=parser`，带文件与行区间。**登记为扫描层的边类型，与契约设计 §6 的 `Script*` 衍生边族不相交**（具体名字在登记它们的 PR 里定）：层由边类型决定，不由 `origin` 决定，否则 `MATCH ()-[r:ScriptReads]->()` 会把 LLM 推断混成解析事实。不存脚本原文 | **夹具驱动的自动化测试**（`tests/fixtures/` 下的脚本样例）：成功抽取的各形态，以及边界与不支持的形态（语法错误、计算出来的参数、别名/重新赋值、字符串里恰好像表名的假阳性）各有断言，改解析器不会悄悄退化；另抽 20 个真实脚本人工对照作补充，不替代自动化测试 |
| S3a | 层 2 的校验提交：按[契约设计 §6.4](../specs/2026-10-02-graph-schema-contract-design.md)（暂名 `--annotate`；提案载荷含 `script_hash`、`evidence`、按边类型登记的专属 `attrs`（如 `ScriptModifiesComponent` 必填 `property`）；校验器重读脚本、核对 hash 与逐字子串；同一 `(from, to, edge_type, script_hash)` 幂等），`origin=llm`，落 `Script*` 衍生边；**图只存 `evidence_span` 与 `evidence_sha256`，不存原文**。**依赖 V2**（`confidence` / `analyzer` / `script_hash` 要是扁平边属性）。**该协议目前只有设计、没有实现**；`cli.rs` 的命令定义、`main.rs` 分发、`--help` 与 README/SKILL 的说明都在本步范围内 | 层 1 能推出的不接受 LLM 提交；校验器拒收的各类提案都有测试（含缺必填 `attrs` 键与未登记键）；**端到端 CLI 测试**（真实调用 `--annotate`，成功与拒收各一，输出里有拒收原因）；`--help` 与文档同步 |
| S3b | 标注的持久化与生命周期（契约设计 §6.5）：已通过校验的标注写入 sidecar 文件（原始输入，可审阅、可入版本库），全量重建后按 `script_hash` 重放（一致 → `current`，不一致 → `stale` 保留），增量扫描里脚本改动使其标注置 `stale`，**并把对应衍生边从图里撤下**（过期标注只留在 sidecar，不物化成边，这样普通 GQL 看不到过期内容；由标注命令列出待重新分析的清单），同时给该 `Script` 物化一个 `Script → Diagnostic` 过期标记（暂名 `SCRIPT_ANALYSIS_STALE`，重新分析成功或 sidecar 删除后消失），让 GQL 读者分得清「分析是最新的、确实没有关系」与「分析已过期」，脚本删除时对应标注移除 | 全量重建后标注按 hash 重放且与重建前图相等；改脚本后旧标注变 `stale`，对应衍生边从图里消失（`MATCH ()-[r:ScriptReads]->()` 查不到），该 `Script` 带有过期 `Diagnostic`，标注命令仍能列出它，重新分析成功后标记消失；删脚本后标注消失；删除全部 sidecar 后扫描层与现在逐边相等 |

S1a、S1 起要在 `graph_schema.rs` 登记新节点与边类型（PR #5 已合入，不再被挡），并且以 L0（写入校验）为前提；S0 不依赖，可立刻开始。

## 4. 并行的冲突点与规则

- 热点文件：`graph.rs`（`NodeType`/`EdgeType`）、`graph_schema.rs`、`graph_grafeo.rs`（V2 的扁平边属性与版本键）、`parser.rs` 与 `scanner/indexer.rs`（S1a 的文件类型登记）、`remote_metadata.rs` 与 `session/remote_sync.rs`（S1a 的远端路径）、`cli.rs` 与 `main.rs`（S3a 的命令）、`scanner/spg.rs`、`scanner/tbl.rs`、`tests/` 里的快照。
- 规则：**同一时刻只有一个 PR 改枚举、schema 登记与存储属性模型**，另一线的 PR 等它合入后再基于 main 更新；其余文件按线分开。
- 每个 PR 单独说明 `Baseline impact`，涉及依赖的（S0/S2 的 oxc）记录二进制体积变化。
- 扫描语义变化时递增 `SCANNER_SEMANTICS_VERSION`（PR #9 引入），保证旧图重扫。

## 5. 待决项

1. 本计划与 spec 的批准（spec 仍是 draft）。
2. spec §7 里仍未问的：stdio 工具表（Q3）、M59-PATH（Q5）、单文件模式（Q6）、评测通过线（Q7）。它们属于阶段 2 之后，不阻塞本计划。
