# 图优先重设计：LLM 直查图之后，这个工具该是什么

> 状态：**draft**（待用户批准；批准前不写实现代码，不删任何现有功能）
> 里程碑：[M59](../milestones/performance/m59-grafeo-backend-migration.md)（active）之后的方向；与其承接账本并行，不改 M59 范围
> 上游：[图 Schema 契约与图内事实设计](2026-10-02-graph-schema-contract-design.md)（PR #4，本文是它的上位框架；其 S1–S5 在 §6 被重新编号归位）、
> [Grafeo 后端迁移设计](2026-09-05-grafeo-backend-migration-design.md)、[`--gql` 现状](../knowledge/topic-grafeo-gql.md)
> 方向来源：2026-10-02 用户——「LLM 可以直接查 Grafeo，这个工具的功能可以大幅改变、重新判断」
> 不做什么：不修改 PR #5（S1 实现，on hold）；不删除任何现有命令；不引入新依赖

## 0. 结论（一页版）

1. **工具的产品从「回答问题」变成「把源元数据编译成一张可信、自描述的图」。** 回答问题的是 LLM 写的 GQL。
2. **今天最大的一块代码（约 2.4 万行「解释层」）是替小模型拼装答案的。** 其中的**事实**要沉进图里，其中的**叙述与路由**随动词一起退役，其中的**判断规则**搬进 schema 随图交付。
3. **图必须说清自己不知道什么。** 以前缺口随动词响应带出（`diagnostics`）；GQL 没有响应信封，所以「已知的未知」必须是图里的节点，否则 LLM 会把「没查到边」读成「没有这个关系」。
4. **路线按「便宜且证明有效」排序：** 先补图里缺的事实（血缘、诊断、静态可判定的优先级/动作类型），再做配方与评测，再切换工具表面，最后退役旧代码，脚本（`action.ts` / `.ts`）单列一条线。
5. **每一步都有「对等门禁」**：旧动词的结果必须能被 GQL 配方逐项复现并通过评测，才允许冻结，再允许删除。方向变化很大，所以删除排在最后且需用户逐批确认。

## 1. 为什么现在重新判断（证据）

| 证据 | 来源 | 说明了什么 |
|---|---|---|
| 终极目标：LLM 只查图，不读源文件 | 用户，2026-10-01 | 判断每个功能的唯一标尺：**只靠图，能不能回答这个问题** |
| 真实语料 501 个 `.spg` + 828 个 `.tbl`，158 MB，图 89,178 节点 / 200,028 边 | [真实语料实测](../ai-eval-runs/2026-09-06-m59-real-corpus-measurements.md) §2 | 源文件 LLM 读不完；图是唯一可能的上下文入口 |
| 旧 `--query-*` 动词表面（M58.3 三动词收敛**之前**）：13 case × 3 trial = 39 次，23 次命令被拒（`wrong_command`），路由成功率 16/39 = 41%，路由成功后理解通过率 9/16 = 56%，总通过率 0.2308（deepseek-v4-flash，2026-08-01） | [M58 journal](../milestones/ai-eval/m58-cheap-model-comprehension-eval.md) 第 163–169 行 | 当时命令表面本身是主要失败源。**三动词表面（`--find/--explain/--relations`）从未用同一套评测重测**（M59-BASELINE 仍 deferred），所以今天的动词表面表现未知 |
| GQL 直查：15 题、51 个适用事实，只给 help 文本 79%，加 schema 说明 86% | GQL 评测报告（项目共享文件夹 `c4-gql-eval/report.md`，不入库） | 单跳 / 遍历类问题不需要任何动词 |
| 同一评测的失败：血缘与诊断不在图里；把隐藏组件说成渲染故障；编造「参数未提供」「表为空」 | 同上 | 失败是**图缺事实**和**缺解释规则**，不是缺动词 |

**可比性警告**：41% 与 79%/86% 不能直接相减——模型不同（deepseek-v4-flash 对 Haiku 4.5 代理）、动词表面不同（旧 `--query-*` 对 GQL）、题集不同、
评分口径不同、后者单次且自评。它们只支持「方向值得做」，**不能**当作新方案更好的证明；也**没有**数据说明现行三动词表面不如 GQL。证明要靠 §6 阶段 2 的同模型同题三臂对照。

代码规模（`wc -l`，2026-10-02，`3e9306e`）：

| 层 | 文件集 | 行数 |
|---|---|---|
| 摄入（源→图） | `scanner/`、`superpage/`、`tbl_single`、`conditions`、`parser`、`graph_identity`、`source_id`、`action_semantics`、`dependency`、`priority`、`parsed_content` | ≈ 0.95 万 |
| 图存储 | `graph*`、`memory_graph_store`、`ownership`、`persistence/`、`dense_graph` 等 | ≈ 0.71 万（其中 redb ≈ 0.31 万） |
| **解释层（图→答案）** | `explain*`（0.72 万）、`query*`（0.80 万）、`path`/`dense_graph`/`graph_retrieval`（0.26 万）、`answer_contract`/`route`/`context`/`candidate`/`model_scope`（0.33 万）、`output*`/`diagnostics`/`response_processor`（0.31 万） | **≈ 2.4 万** |
| 运行时 / stdio / 契约 | `runtime`、`stdio_server`、`tool_contract` | ≈ 0.36 万 |
| 远端 session / diff-refresh | `remote_metadata*`、`session/`、`diff_refresh/` | ≈ 0.87 万 |
| 浏览器 / WASM / 可视化 | `browser*`、`visualization/` | ≈ 0.47 万 |

解释层是摄入层的 2.5 倍。图优先之后，它是被重新评估的对象，不是被默认保留的对象。

### 1.1 在夹具图上核实过的几件事

`tests/fixtures/test_project`（16 个文件，178 节点 / 343 边，`3e9306e` 本地 `cargo build --release` 默认特性，二进制 8,087,512 B）：

| 核实项 | 结果 | 对本文的影响 |
|---|---|---|
| GQL 变长路径 `[:Contains*1..3]` | 可用，3 跳内返回 191 个终点 | 「闭包交给 GQL」可行，不需要为传递关系物化边 |
| Action 节点 `meta` | 只有 `triggerType` / `condition` / `conditionExp` / `waitPrev`，**没有动作类型** | `action_kind` 是新增事实，不是把现有属性搬位置 |
| Component 节点 `meta` | 有 `component_type`、`json_path`、`parent_id`、`properties`；**没有** `eval_mode` | 同上 |
| `meta CONTAINS 'UNKNOWN'` 全图搜索 | 0 行；`IndexState` 记录 32 条但内容不透明 | 诊断入图前，GQL 看不到任何诊断 |
| `--explain 'field:df_b.id'` 的 `lineage` | 查询期算出，每条带 `confidence`（此例 `medium`）、`transform`、`via_node`、`evidence.json_path` | `DerivesFrom` 边必须带 `confidence` 与 `transform` 元信息，否则入图后比现在**更**不可信 |

## 2. 新定位：三层加一条规则

```
  源元数据 (.spg .tbl, 之后 .ts)            ┌──────────────────────────────┐
        │                                   │ LLM：schema → GQL → 回答      │
        ▼                                   └───────────────▲──────────────┘
 ┌─────────────┐   事实+出处+覆盖    ┌────────────┐   只读 GQL / 配方 / 规则
 │ ① 编译器    │ ───────────────▶   │ ② 图 (Grafeo)│ ────────────────┘
 │ (摄入, Rust)│                    └────────────┘
 └─────────────┘                          ▲
        └── ③ 查询面：--graph-schema（结构+解释规则+配方）、--gql（只读）
```

**① 编译器**：把元数据变成静态事实。对 LLM 不可见，但决定图的完整度上限。
**② 图**：事实、出处（`origin_file` + `json_path`）、覆盖（诊断）。
**③ 查询面**：`--graph-schema` 告诉 LLM 图长什么样、什么结论能下、什么不能下，附带可复制的配方；`--gql` 执行。
**规则**：解释规则随 schema 版本一起发布（PR #4 §2.1 的「解释规则行」），不放在动词的响应里。

### 2.1 入图准则（判断「这个事实该不该进图」）

一个派生事实入图，当且仅当**同时满足**：

1. **静态可确定**：只依赖元数据，不依赖运行时值（参数是否传入、表是否为空）；
2. **与提问无关**：不依赖 intent / budget / 提问者想看什么；
3. **小模型重算代价高或易错**：多跳、需要优先级规则、需要祖先链择父规则；
4. **可带出处**：能写出 `origin_file` 与 `json_path`（或证据表达式）。

只有 1–2 成立、3 不成立的，留给 GQL 现算（如单跳邻居）。1 或 2 不成立的，**不入图**，在解释规则里写明「图中无法确定」。

### 2.2 退役准则

一个功能退役，当它是**动词协议的产物**——存在的理由是替提问者在一组动词里挑一个、或把答案裁剪成预算大小。
动词表面没了，这些就没有被服务的对象：`intent`、`budget`、`answer_contract`、`forbidden_fact_paths`、
`required_followups`、`truncation_guard`、路由与 `advise-query`。

### 2.3 新原则：已知的未知必须在图里

LLM 不能打开源文件复核，所以「缺失」比「错误」更危险。要求：

- 每个被解析失败、未识别、重复、冲突的对象，图里有一个 `Diagnostic` 节点挂在被诊断对象上（PR #4 §3.2）；
- 解释规则写明：**没有出边 ≠ 没有关系**，除非被查对象上没有 `Diagnostic`；
- 查询期才能算出的诊断（依赖提问上下文的）不入图，规则里标「仅查询期」。

## 3. 功能逐项判定

判定标记：**保留** 留在核心；**入图** 事实沉进图，GQL 回答；**配方** 留为带名字的参数化 GQL（数据，不是 Rust 代码）；
**冻结** 不再新增功能，对等达标后删除；**退役** 目标态删除；**新增**。

### 3.1 摄入与图

| 功能 | 判定 | 理由 |
|---|---|---|
| `.spg` / `.tbl` 解析、组件树、表达式引用抽取 | **保留** | 图的完整度上限。F3（嵌套表达式、`CASE WHEN`，M59 账本 deferred）从「不阻塞迁移」升为**图完整度的首要缺口** |
| 条件抽取 `conditions.rs`（Condition 节点、`raw_expr`/`normalized_expr`/`referenced_symbols`） | **保留** | 已入图（`scanner/spg.rs:1836`）；`meta` 仍是 JSON 文本，压平另议（PR #4 §4） |
| 优先级判定 `priority.rs`（`defaultValue`/`exp`/`calcCondition`） | **入图** | 静态可确定、规则非平凡：Component 增 `eval_mode`（现有 5 种枚举值）。现在只有单文件模式读它 |
| 动作类型归类 `action_semantics.rs` | **入图** | Action 增 `action_kind`；未知类型 → `Diagnostic`（承接 `UNKNOWN_ACTION_TYPE`，PR #4 §3.2 已提） |
| 字段级血缘（`--explain` 的 `lineage`、`query_dataflow`） | **入图** | PR #4 §3.1 `DerivesFrom`；须携带现有的 `confidence` / `transform` / `via_node` / 证据 `json_path`（§1.1）。调研报告同判：已确定性计算、最便宜的首个收益 |
| 值来源追溯 `dependency.rs`、`value_source_facts` | **入图（直接边）+ 配方（闭包）** | 直接 `DependsOn` 边已在图里；传递闭包交给 GQL 变长路径。循环检测改为节点属性 + `Diagnostic`，不在查询期现算 |
| 可见性门禁链（祖先组件条件 + 择父规则，`component_ancestor_chain`，`explain/condition_facts/conditions.rs:311`） | **配方 + 解释规则** | `Contains` 与条件 owner 边已在图；择父排序（`ancestor_parent_rank`）是**判断规则**，必须写进规则，不能让小模型自己猜 |
| `availability_facts`（数据源为何可能为空）、writer `conditionExp`（F5/F6） | **入图** | 过滤条件已有 `SourceFilterExp` 条件节点；缺的是过滤字段名（评测中两组都没报出，PR #4 §6 第 1 问待核实）和 writer 条件 |
| Grafeo 存储、增量、`ownership`、`IndexState` | **保留** | M59 进行中，本文不改 |
| redb 后端（`graph_redb*.rs` ≈ 0.31 万行）、v2 shadow | **退役** | M59-5 已定 |
| 页面局部身份、源范围、项目绑定 | **保留** | M59-1/2 已落地，图的可信度地基 |

### 3.2 查询面

| 功能 | 判定 | 理由 |
|---|---|---|
| `--gql`、`--gql-max-rows` | **保留，升为唯一查询入口** | 只读两道闸已验证（`graph_grafeo.rs`）；LLM 自己组合，不需要动词 |
| `--graph-schema`（PR #4 S1，未合） | **新增，入口 0** | 结构 + 解释规则 + 方言 + 配方的单一真源 |
| `--find` | **配方**（保留 CLI 别名） | 中文路径、`\|` 分隔的 id 文法是小模型第一道坎；保留「名字→规范 id」这一步，但实现收缩为 GQL 配方 + 候选排序（`candidate.rs`），并**回显它执行的 GQL** 让模型学会 |
| `--explain` / `--relations`（`explain*` 0.72 万、`query*` 0.80 万行） | **冻结 → 退役** | **事实**入图（上表），**叙述**（`what_is_it`、`primary_reason`、`key_findings`）与 envelope 退役，**判断**搬进规则。保留到对等门禁通过，作为差分测试的基准 |
| 10 个已隐藏的旧动词（`--explain-condition`、`--context`、`--query-model/page/cross/dataflow/page-logic`、`--find-page/model/component`） | **退役** | 已从 `--help` 隐藏、SKILL 已写「新集成不要用」；删除前核对哪些快照测试还依赖它们 |
| `--advise-query`、`--question-kind`、`--intent`、`--budget`、`route.rs`（0.09 万）、`answer_contract.rs`（0.09 万） | **退役** | 纯动词协议产物（§2.2）；`--budget` 的唯一对应物是 `--gql-max-rows` |
| `path.rs`（路径候选与排序）、`dense_graph.rs`（CSR）、`graph_retrieval.rs`（PPR spike） | **冻结 → 退役** | 为叙述挑「主链路」而存在；引擎自带路径遍历。**连带**：账本里的 M59-PATH（`same_page` 判定缺陷）随之作废，不再修——这是需要用户确认的取舍（§7 Q5） |
| `page_logic`（`query/page_logic*` 约 0.4 万行，页面动作流） | **入图 + 配方** | 动作流是 Page→Component→Action→Model 的多跳路径，GQL 可表达；沿用其 `PRIMARY_PATH_LIMIT` 的教训：配方必须自带 `LIMIT`，否则大页面撑爆结果 |
| 单文件模式（`FILE`、`--query`、`--priority`、`--detail`、`--human` REPL、`--interactive`） | **冻结 → 终态改为「临时建图 + GQL」** | 它让 LLM 直接读 JSON 大对象，正是目标要避免的路径；保留给开发调试 |
| JSON 响应信封（`summary`/`details`/`evidence`/`diagnostics`） | **退役**（动词随之） | GQL 自有 `{columns, rows, truncated}`；`--human` 的 TSV 保留 |
| 评测 harness（M58 kimi / ai-eval，`tests/ai_eval_tests.rs`） | **保留并升格为门禁** | 阶段 2 的对等判据就是它；需新增「GQL 臂」 |

### 3.3 诊断（三类，去向不同）

| 类 | 例 | 去向 |
|---|---|---|
| **摄入期，描述图本身的覆盖** | `SCANNER_UNRECOGNIZED_CONTAINER_KEY`、`SCANNER_DUPLICATE_COMPONENT_ID`、`SCANNER_FILE_PARSE_FAILED`、`GRAPH_OWNERSHIP_CONFLICT`、`UNKNOWN_ACTION_TYPE`（若导入期可判定） | **入图**为 `Diagnostic` 节点，沿用现有 code 与 `answer_impact`，随源文件 origin 撤销 |
| **查询期，动词协议的** | `RESOLVED_TARGET`、`AMBIGUOUS_TARGET`、`TARGET_NOT_FOUND`、`OUTPUT_TRUNCATED`、`PRIMARY_PATHS_TRUNCATED`、`EXPR_*`（单文件） | 随动词**退役**；`--find` 配方保留其歧义语义 |
| **工具与运行时** | `GRAPH_DB_*`、`GRAPH_SCHEMA_STALE`、`RUNTIME_*`、`REMOTE_*`、`GQL_*` | **保留**，是 CLI/stdio 的错误码，不入图 |

### 3.4 运行时、远端、浏览器

| 功能 | 判定 | 理由 |
|---|---|---|
| `--build-graph`、diff-refresh（M54–M57）、`--check-graph`、`--status`、`--reload-graph`、`--check-reload` | **保留** | 「图如何出现、如何保鲜」，与终极目标正交且是前提 |
| 远端 session、BI 登录、`remote_metadata*`（≈ 0.87 万行） | **保留，不动** | 图的另一条来源；不在本次重判范围 |
| stdio server（`--serve-stdio`） | **保留传输，重塑工具表** | 目标工具表：`graph_schema`、`gql`、`status`、`diff_refresh`。M58 固定了工具表面，**改它需用户单独批准**（§7 Q3） |
| MCP adapter | **推迟，已在路线「待排期」** | 现在没有 MCP 代码；等 stdio 契约稳定后做薄适配 |
| 浏览器插件、WASM、`visualization/` | **冻结新功能，不在关键路径** | 数据源应改为图/GQL 结果而非 explain 信封。**风险**：`browser-wasm` 不链接 grafeo，浏览器里没有 GQL；图优先在浏览器侧是缺口，不在本文范围 |
| 性能 bench 体系 | **保留，换测量对象** | 从「explain 延迟」换成「建图 + GQL 查询延迟（含 30 秒超时、89k 节点规模）」 |

### 3.5 新增：脚本（`action.ts` 与 `.ts`）

终极目标下最大的缺口：后端 Nashorn 脚本做第三方请求、入库、数据流逻辑，前端 `.ts` 改页面参数与 superPage 内容，
这些**根本不在图里**，LLM 不读源文件就答不了。方向沿用调研报告
（`/mnt/project-files/graph-extraction-research/report.md`）与 PR #5 spec §6，不重做：

- **层 1 确定性解析（Rust，候选 oxc）**：`origin=parser`，带文件、行区间、脚本 hash。字面量参数（URL、表名、参数键、组件 id、字段名）可确定地成边；
- **层 2 LLM 补残余**：只做语法判不了的，走 §6 的校验提交命令（**不走 GQL 写入**），要求 `origin=llm`、置信度、行区间 + 引用片段并由校验器核对；层 1 能推出的不接受 LLM 提交；
- **前置**：额外仓库同步到 GitHub（用户计划中）；oxc 体积实测（AGENTS.md 要求）；一次只计数的 spike——所有调用参数里字符串字面量对计算值的比例，这个数字决定层 2 要多大；
- **在此之前确定的事**：workflow 节点按路径指向 `action.ts`（用户已确认路径必有），所以**脚本节点与「节点→脚本」边**可以先于任何解析落地。

PR #5 保持 on hold，本文不推进它。

## 4. 目标形态

命令表面（对 LLM 可见的部分）：

```
--build-graph / --session-diff-refresh / --status   让图存在并保鲜（运维）
--graph-schema                                       图长什么样 + 解释规则 + 配方（入口 0）
--gql "<query>"                                      唯一查询入口（只读）
--find <名字>                                        可选：名字 → 规范 id（GQL 配方）
```

LLM 工作流：`schema`（一次）→ `find`（定位）→ `gql`（可多次）→ 回答前查被引用对象上的 `Diagnostic`。
stdio / MCP 的工具表同构：`graph_schema`、`gql`、`status`、`diff_refresh`。

**配方**是数据文件（参数化 GQL + 一句话用途 + 适用的解释规则），随 `--graph-schema` 输出；
每个被退役的动词，其覆盖的问题类型必须对应至少一个配方或一个入图事实，否则不许退役。

## 5. 风险

| 风险 | 缓解 |
|---|---|
| **小模型会编造运行时事实**（评测里「参数未提供」「表为空」）。图再全也救不了 | 规则随 schema 发布并进评测；调研里同类工具自报 83% vs 文件探索 92%，期望要放在这个量级，而不是 100% |
| **退役不可逆，且旧动词是多轮返修的成果**（M58.3 的 answer_contract、大量快照测试） | 对等门禁：差分测试 + 同模型同题评测；冻结先于删除；删除按批由用户确认 |
| **物化事实让图变大**（真实语料起点 89k 节点 / 200k 边） | 只物化直接事实，闭包留给 GQL；每包按 AGENTS.md 在远端 `cargo build --release` 后记录体积与 bench 变化 |
| **证据偏薄**：评测是 Haiku 代理、单次、自评、仅夹具；真实语料在维护者本机，不在云端环境 | 阶段 2 要求独立评分、多次运行、必须含真实语料一轮；拿不到真实语料就如实标注未验证 |
| **GQL 引擎缺陷**（聚合后浮点位模式；30 秒超时兜底；形态闸误拒含 `load` 的查询） | 已知并记录于知识库；配方避开，评测里单列 |
| **浏览器侧无 GQL** | 范围外，单列风险，不假装解决 |

## 6. 分阶段路线

每个阶段一个或多个独立 PR；PR #4 的 S1–S5 在此归位，不改其内容。

| 阶段 | 内容 | 对等/验收门禁 |
|---|---|---|
| **0 批准** | 批准本文 + PR #4；回答 §7 的问题 | 用户批准（draft → approved） |
| **1 图补全** | ① S1 `graph_schema.rs`（**PR #5，on hold，等用户放行**）→ S2 写入校验；② S3 血缘边 + schema 版本键；③ S4 诊断入图；④ 新增 S3b：`eval_mode`、`action_kind`、循环标记；⑤ 并行：F3 嵌套表达式；⑥ 核实并补过滤字段名（PR #4 §6.1）；⑦ 血缘边在现有 `confidence`/`transform` 之外借用 OpenLineage 的 `DIRECT`/`INDIRECT` 作为 `lineage_kind` 取值（值域待实现时核对现有 `transform` 文本）；⑧ 摄入时汇总「见过但未映射」的 JSON 键（现有 `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 只覆盖容器键），作为 `Diagnostic` 挂在所属文件/页面上，让图能说明自己漏了什么（调研报告 Part 2；Backstage 同样把未解析引用记为状态而非丢弃） | 夹具上，`--explain` 每个 fact block 都能由 GQL 得出相同事实（差分测试，不降低断言）；增量与全量产物仍逐属性相等（B5 套件） |
| **2 配方与评测** | 配方数据文件；`--graph-schema` 输出配方；评测加 GQL 臂；S5 复跑 | **同模型同题**三臂（旧动词 / GQL+help / GQL+schema+配方+规则）；独立评分；多次运行；含真实语料一轮。阈值由用户定（§7 Q7） |
| **3 表面切换** | stdio 工具表重塑；SKILL.md 重写；旧动词发 deprecated 诊断；（可选）MCP 薄适配 | 用户批准工具表面变化（M58 曾固定它） |
| **4 退役** | 冻结项分批删除；redb（M59-5）；单文件模式去留 | 每批：影响面测试 + 全量；PR 里记录行数与二进制体积的变化 |
| **5 脚本**（独立线，只依赖 schema） | 脚本节点 + 节点→脚本边 → oxc 计数 spike → 层 1 → 层 2（经 §6 校验提交） | 额外仓库同步 + PR #5 放行；每步先在真实脚本上计数再设计 |

顺序理由：阶段 1 最便宜，且是后面所有阶段的前提；阶段 5 的数据尚未到位，所以单列、不阻塞 1–4。
阶段 1 的 ① 在 PR #5 放行前不动，**其余条目不依赖它**，可以先做。

## 7. 开放问题（括号内为推荐）

1. **终态**：旧动词在对等达标后删除，还是长期并存？——**已决（2026-10-02 用户在决策卡上选「Retire」）：对等达标后删除**。删除仍按 §6 阶段 4 分批、每批需用户确认；`--find` 按 §3.2 保留为 GQL 配方别名，不属于被删除的「动词」。
2. **派生事实放哪**：导入期入图，还是 `CALL` 式按需过程？（导入期，只物化直接事实；闭包交给 GQL）
3. **stdio 工具表**：是否同意加入 `gql` / `graph_schema` 并最终收缩到四个工具？MCP 是否先不做？（同意；MCP 等契约稳定）
4. **脚本**：层 1 用 oxc（待体积实测）+ 层 2 走 §6 校验提交，接受吗？（接受，先做字面量占比 spike）
5. **M59-PATH**：随 `path.rs` 退役而作废，不再修，接受吗？（接受；若你想先保留旧动词更久，则保持排队）
6. **单文件模式**：终态改为「临时建图 + GQL」，还是保留 JSON 输出给开发调试？（临时建图；JSON 输出冻结）
7. **成功判据**：阶段 2 的通过线用什么数？（GQL+schema+配方+规则臂在同模型同题上不低于旧动词臂，且 `forbidden_claims` 违规数为 0；具体百分比请你定）
