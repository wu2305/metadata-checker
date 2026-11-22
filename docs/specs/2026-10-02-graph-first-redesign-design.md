# 图优先重设计：LLM 直查图之后，这个工具该是什么

> 状态：**draft**（待用户批准；批准前不写实现代码，不删任何现有功能）
> 里程碑：[M59](../milestones/performance/m59-grafeo-backend-migration.md)（active）之后的方向；与其承接账本并行，不改 M59 范围
> 上游：[图 Schema 契约与图内事实设计](2026-10-02-graph-schema-contract-design.md)（PR #4，本文是它的上位框架；其 S1–S5 在 §6 被重新编号归位）、
> [Grafeo 后端迁移设计](2026-09-05-grafeo-backend-migration-design.md)、[`--gql` 现状](../knowledge/topic-grafeo-gql.md)
> 方向来源：2026-10-02 用户——「LLM 可以直接查 Grafeo，这个工具的功能可以大幅改变、重新判断」
> 范围：`.fapp`、`.meta`、`.wfl` 三类文件不在本文考虑内，也不解析（用户，2026-10-03）
> 不做什么：不删除任何现有命令；**本文档 PR 不引入依赖**（阶段 5 的 oxc 在各自的实现 PR 里引入，并按 AGENTS.md 记录二进制体积变化）
> 已落地的前置（2026-10-03，均已合入 `main`）：PR #5（S1，`graph_schema.rs` 契约表与 `--graph-schema`）、PR #8（扫描诊断逐次记录）、PR #9（阶段 1.0 的ⓐ、ⓑ，ⓒ、ⓓ未完成）、PR #10（契约表补 `landed` 与 `IndexState` 记录形态）

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
| 云端环境现已能读真实语料（`wu2305/autocrm` 的 `projects/`，2026-10-03 同步）：另有 141 个 `.action.ts`、6 个 `custom.ts`、3 个 `.wfl`、10 个 `.tpg`、8 个 `.mpg`；删掉会使建图崩溃的 38 张表后，1,291 个文件约 18 秒建图，87,127 节点 / 197,124 边 | 兄弟线程的类型盘点（共享文件夹 `autocrm-metadata-types/report.md`，不入库）+ 本线程抽查 | 本文原先「真实语料不在云端」的限制解除；同时暴露两个阻塞缺陷（见 §6 阶段 1.0） |
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

- 每个被解析失败、未识别、重复、冲突的对象，图里有一个 `Diagnostic` 节点（PR #4 §3.2）。`Diagnostic` 带 `path`（所属源文件）与 `json_path`，**自己就能按路径查到**；被诊断的对象在图里存在时，另有一条 `HasDiagnostic` 边把它挂上去。**文件级**问题（整个 `.spg` / `.tbl` 解析失败，没有 Page / Model 节点可挂；现状 `parse_failure_diagnostic_entry` 只记路径、`node_id` 为空）只有 `Diagnostic` 节点、没有 `HasDiagnostic` 边：检查一个对象的覆盖情况，要同时看它的 `HasDiagnostic` 边与 `path` 等于它所在文件的 `Diagnostic`。生命周期随源文件的 origin 撤销；
- 解释规则写明：**没有出边永远不等于没有关系**。`Diagnostic` 只能把「已知的未知」变成可见，**不能**反过来证明「没有诊断就是完整」——解析器还没覆盖的来源（脚本在阶段 5 前、未决定的 `.tpg` / `.mpg`、不支持的表达式形态）不会产生任何诊断。只有 schema 里某个边类型的 `absent_when` 明确写着对某类来源「完整」，并且该来源已经摄入时，才允许把缺失读成否定；
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
| 值来源追溯 `dependency.rs`、`value_source_facts` | **入图（直接边 + 字段身份）+ 配方（闭包）** | 直接 `DependsOn` 边已在图里；但 `trace_value_source` 是**按字段**追的：从一个 (组件, 字段) 出发，只沿该字段表达式引用的目标继续，而图里同一组件节点上挂着它所有表达式字段的依赖，`source_field` 只在边的 `meta` 文本里。所以裸 `DependsOn*` 变长路径会混入不相干字段的依赖，**不能**直接当作闭包的替代。闭包要在「每一跳保留字段身份」后才可交给 GQL：把 `source_field` 提成可过滤的边属性（随 schema v2 的扁平边属性，PR #4 §4），或把 (组件, 字段) 建成一等节点；在此之前该项不得列为「可由 GQL 复现」，对等门禁里单列。循环检测改为节点属性 + `Diagnostic`，不在查询期现算 |
| 可见性门禁链（祖先组件条件 + 择父规则，`component_ancestor_chain`，`explain/condition_facts/conditions.rs:311`） | **配方 + 解释规则** | `Contains` 与条件 owner 边已在图；择父排序（`ancestor_parent_rank`）是**判断规则**，必须写进规则，不能让小模型自己猜 |
| `availability_facts`（数据源为何可能为空）、writer `conditionExp`（F5/F6） | **入图** | 过滤条件已有 `SourceFilterExp` 条件节点；缺的是过滤字段名（评测中两组都没报出，PR #4 §6 第 1 问待核实）和 writer 条件 |
| Grafeo 存储、增量、`ownership`、`IndexState` | **保留** | M59 进行中，本文不改 |
| redb 后端（`graph_redb*.rs` ≈ 0.31 万行）、v2 shadow | **退役** | M59-5 已定 |
| 页面局部身份、源范围、项目绑定 | **保留** | M59-1/2 已落地，图的可信度地基 |

### 3.2 查询面

| 功能 | 判定 | 理由 |
|---|---|---|
| `--gql`、`--gql-max-rows` | **保留，升为唯一查询入口** | 只读两道闸已验证（`graph_grafeo.rs`）；LLM 自己组合，不需要动词 |
| `--graph-schema`（PR #4 S1，已由 PR #5 合入） | **入口 0（已落地）** | 结构 + 解释规则 + 方言 + 配方的单一真源 |
| `--find` | **配方**（保留 CLI 别名） | 中文路径、`\|` 分隔的 id 文法是小模型第一道坎；保留「名字→规范 id」这一步，但实现收缩为 GQL 配方 + 候选排序（`candidate.rs`），并**回显它执行的 GQL** 让模型学会 |
| `--explain` / `--relations`（`explain*` 0.72 万、`query*` 0.80 万行） | **冻结 → 退役** | **事实**入图（上表），**叙述**（`what_is_it`、`primary_reason`、`key_findings`）与 envelope 退役，**判断**搬进规则。保留到对等门禁通过，作为差分测试的基准 |
| 10 个已隐藏的旧动词（`--explain-condition`、`--context`、`--query-model/page/cross/dataflow/page-logic`、`--find-page/model/component`） | **退役** | 已从 `--help` 隐藏、SKILL 已写「新集成不要用」；删除前核对哪些快照测试还依赖它们 |
| `--advise-query`、`--question-kind`、`--intent`、`--budget`、`route.rs`（0.09 万）、`answer_contract.rs`（0.09 万） | **退役** | 纯动词协议产物（§2.2）；`--budget` 的唯一对应物是 `--gql-max-rows` |
| `path.rs`（路径候选与排序）、`dense_graph.rs`（CSR）、`graph_retrieval.rs`（PPR spike） | **冻结 → 退役** | 为叙述挑「主链路」而存在；引擎自带路径遍历。**连带**：账本里的 M59-PATH（`same_page` 与物理字段识别缺陷）随之作废，不再修——这是需要用户确认的取舍（§7 Q5）。**因此** `path.rs` 产出的这两类判定**不得当作对等门禁的基准**：阶段 1/2 的差分与对照里，这些字段不做「与旧输出逐项相等」，改为对照人工核对过的、以源文件为准的金标准，旧输出与新配方各自与金标准比 |
| `page_logic`（`query/page_logic*` 约 0.4 万行，页面动作流） | **入图 + 配方** | 动作流是 Page→Component→Action→Model 的多跳路径，GQL 可表达；沿用其 `PRIMARY_PATH_LIMIT` 的教训：配方必须限量，否则大页面撑爆结果。**但限量不能静默**：`--gql` 的 `truncated` 是按引擎已返回的行数算的（`graph_grafeo.rs` 的 `query_gql_read_only`），配方里写死的 `LIMIT n` 会让它恒为 `false`，而 `PRIMARY_PATHS_TRUNCATED` 又随动词退役。所以配方约定：查询写 `LIMIT n+1`，配方说明里写明「返回 n+1 行即表示有遗漏」，并在 `--graph-schema` 的配方条目里带出 `limit` 参数；需要总数时用配套的计数查询。没有这个标记的限量配方不得放行 |
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
（`/mnt/project-files/graph-extraction-research/report.md`）与[图 Schema 契约设计](2026-10-02-graph-schema-contract-design.md) §6（尤其 §6.4「提交与校验：不经 GQL 写」：提案载荷、校验步骤、`script_hash` 与逐字子串核对、幂等键），不重做：

- **层 1 确定性解析（Rust，候选 oxc）**：`origin=parser`，带文件、行区间、脚本 hash；**登记为扫描层的边类型，与层 2 的 `Script*` 衍生边族不相交**（层由边类型决定，`origin` 只是来源标注）。字面量参数（URL、表名、参数键、组件 id、字段名）可确定地成边；
- **层 2 LLM 补残余**：只做语法判不了的，走[契约设计 §6.4](2026-10-02-graph-schema-contract-design.md) 的校验提交命令（**不走 GQL 写入**；该协议在那份文档里定义，尚无实现，由阶段 5 的 S3 落地），要求 `origin=llm`、置信度、行区间 + 引用片段并由校验器核对；层 1 能推出的不接受 LLM 提交；
- **前置**：~~额外仓库同步到 GitHub~~（已完成，`wu2305/autocrm`）；oxc 体积实测（AGENTS.md 要求）；一次只计数的 spike——所有调用参数里字符串字面量对计算值的比例，这个数字决定层 2 要多大；**脚本层的关注点（用户，2026-10-03）**：用户关心的是**数据处理逻辑与数据库操作逻辑**，凭据不是设计驱动力——平台没有密钥库，约 30 个脚本里的硬编码凭据「几乎无法避免」。因此层 1/层 2 只抽取能成边或成事实的内容（读写哪张表、哪些字段、什么操作、数据怎么加工、调哪个外部地址/脚本），**不把脚本原文或无关的字符串字面量存进图**；脚本原文不入图；层 2 提交的 `evidence` 原文只用于校验，图与 sidecar 只存行区间与 sha256（契约设计 §6.3）。**剩余的已知缺口，已按用户决定接受、不设闸**：作为调用参数被抽取的字面量（外部地址、表名）如果本身带凭据（例如 URL 里的用户信息或 token 查询参数）会随事实入图；S0 spike 顺带统计这类字面量的数量，让是否需要规则由数据决定。见到过的硬编码凭据取值不得写进任何文档；
- **在此之前确定的事（2026-10-03 按真实语料更正）**：先前写的「workflow 节点按路径指向 `action.ts`」**不成立**：`.wfl` 文件（3 个）不含任何脚本链接，且用户说明 `.wfl` 无需解析。语料里实际存在的链接形态是：`.tbl` 数据流的 `Script` 节点按**裸 hash id**（无路径、无后缀）指向脚本；`.spg` 的 `webAPI.url` 按路径指向 `.action.ts`（221 个 `webAPI` 动作）；脚本之间用 `import` 路径；`.spg` 的 `script` 动作按（页面键、函数名）指向 `custom.ts` 的 `CustomActions` 函数（绑定写在 `custom.ts` 一侧：`CustomJS` 映射的键是裸文件名或绝对路径，裸键在应用内可能歧义，歧义记诊断不猜；规则待 S0 核实，见契约设计 §7 Q9）。这些都是确定性可解析的，所以**脚本节点与「节点→脚本」边**仍可先于任何解析落地。`X.action` 是 `X.action.ts` 的编译副本，应视为同一节点的构建产物，不单独建节点。


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
| **证据偏薄**：评测是 Haiku 代理、单次、自评、仅夹具。真实语料现已可从云端读取（`wu2305/autocrm`），但**尚未跑过任何评测** | 阶段 2 要求独立评分、多次运行、必须含真实语料一轮（语料已可用，不再有借口跳过） |
| **脚本含凭据**：约 30 个脚本有疑似凭据的字面量 | 用户确认凭据难以避免、不是关注点；缓解靠抽取范围：只存成边/成事实的字面量，不存脚本原文与无关字面量（§3.5） |
| **GQL 引擎缺陷**（聚合后浮点位模式；30 秒超时兜底；形态闸误拒含 `load` 的查询） | 已知并记录于知识库；配方避开，评测里单列 |
| **浏览器侧无 GQL** | 范围外，单列风险，不假装解决 |

## 6. 分阶段路线

每个阶段一个或多个独立 PR；PR #4 的 S1–S5 在此归位，不改其内容。

| 阶段 | 内容 | 对等/验收门禁 |
|---|---|---|
| **0 批准** | 批准本文 + PR #4；回答 §7 的问题 | 用户批准（draft → approved） |
| **1.0 语料阻塞缺陷**（先于 1 的其余条目；独立于 schema 决策） | ⓐ 空 `dbTableName`（语料中 38 / 828 张表，均为 dataFlow）使 `--build-graph` 整体中止（`node id has an empty kind/page/local segment: model`）。用户确认这是**未落表数据流**（即时取数的逻辑，没有物理输出表），不是错误：模型照常入图并带 `landed: false`，不建输出边，不报诊断；ⓑ 路径前缀解析：`$TAPP:`（当前应用根）、`$APP:`、`$ANA:`、绝对路径 `/xiaoshouyi/...`、`..` 归一，**绝不为非 `.spg` 目标创建 Page 节点**——现状 636 个 Page 节点中有 42 个磁盘上不存在（含 18 个路径里嵌着原样 `$APP:/`），其上还挂着字段；解析不了的引用记 `SCANNER_UNRESOLVED_REFERENCE`（入图为 `Diagnostic` 属阶段 1 ⑧）。ⓐ与ⓑ的前缀与路径部分**已由 PR #9 实现**。ⓒ 解析正确但目标文件不存在的页面引用（autocrm 上 7 个页面、75 处引用）**已由 L0b 实现**：判定用本次发现的文件集合，这类引用不建边、不造 Page 节点、不造挂在目标页名下的 link 参数节点，每处记一条 `SCANNER_UNRESOLVED_REFERENCE`；目标页面出现、消失或被改写时，未改动的引用者随之重解析；旧版本留下的无文件占位在升级后的第一次扫描里清掉。**1.0 还剩一项**：ⓓ 17 张数据流缺省 `dbTableName` 键，语义未确认（待用户答复） | 在 autocrm 上建图成功且 `Page` 节点无幽灵（每个 Page 对应磁盘文件）——ⓒ 已使此门禁通过（501 个 Page，对应 501 个 `.spg`）；夹具上现有快照不变。「这个引用能解析吗」目前图答不了，而这是终极目标直接要的 |
| **1 图补全** | ① S1 `graph_schema.rs`（**PR #5，已合入**）→ S2 写入校验；② S3 血缘边 + schema 版本键；③ S4 诊断入图；④ 新增 S3b：`eval_mode`、`action_kind`、循环标记；⑤ 并行：F3 嵌套表达式；⑥ 核实并补过滤字段名（PR #4 §6.1）；⑦ 血缘边在现有 `confidence`/`transform` 之外借用 OpenLineage 的 `DIRECT`/`INDIRECT` 作为 `lineage_kind` 取值（值域待实现时核对现有 `transform` 文本）；⑧ 摄入时汇总「见过但未映射」的 JSON 键（现有 `SCANNER_UNRECOGNIZED_CONTAINER_KEY` 只覆盖容器键），作为 `Diagnostic` 挂在所属文件/页面上，让图能说明自己漏了什么（调研报告 Part 2；Backstage 同样把未解析引用记为状态而非丢弃）；⑨ 前置改动（已核对源码）：扫描期诊断目前只存**计数 + 每类一个样例位置**（`scanner/spg.rs:175` 起的 `ScanDiagnostics`），要把诊断挂到具体节点，必须先改为**逐次出现一条记录**（已由 PR #8 合入，记录存在 `IndexState` 的 `scanner_entry:<path>` 里，契约表已描述其形态）；`UNKNOWN_ACTION_TYPE` 目前在查询期算出，入图意味着导入期计算（可复算性待实现时核实）；`explain/importance.rs` 的 `classify_importance` 看起来可在导入期算出并成为节点属性，**此为推断，未核实**；⑩ 语料暴露的廉价高价值边（均来自已解析的文件，见盘点 §7）：`dimensionPath` → 字段到表的边（盘点数 26,994，与本线程原始字符串匹配 32,231 不一致，**未对账**，实现前先对账）、`properties.depends` 指向真实 model 节点、`webAPI.url`（含 `?method=`）与 `scriptFunction` 作为 Action 节点属性、`shortUrls` 作为页面别名 | 夹具上，`--explain` 每个 fact block 都能由 GQL 得出相同事实（差分测试，不降低断言）；**例外两处，各有替代判据**：值来源闭包在「每一跳保留字段身份」落地前不计入可复现项（§3.1），`same_page` 与物理字段识别按金标准而非旧输出判（§3.2）。增量与全量产物仍逐属性相等（B5 套件） |
| **2 配方与评测** | 配方数据文件；`--graph-schema` 输出配方；评测加 GQL 臂；S5 复跑 | **同模型同题**三臂（旧动词 / GQL+help / GQL+schema+配方+规则）；独立评分；多次运行；含真实语料一轮。阈值由用户定（§7 Q7） |
| **3 表面切换** | stdio 工具表重塑；SKILL.md 重写；旧动词发 deprecated 诊断；（可选）MCP 薄适配 | 用户批准工具表面变化（M58 曾固定它） |
| **4 退役** | 冻结项分批删除；redb（M59-5）；单文件模式去留 | 每批：影响面测试 + 全量；PR 里记录行数与二进制体积的变化 |
| **5 脚本**（独立线，只依赖 schema） | 脚本文件进入摄入管线（新增源文件类型、文件发现、解析结果与账本、删除与增量，**本地与远端会话两条路径**，见计划 S1a）→ 脚本节点 + 节点→脚本边 → oxc 计数 spike → 层 1 → 层 2（经契约设计 §6.4 的校验提交；前置 schema v2 扁平边属性，之后是 sidecar 持久化，见计划 V2 / S3a / S3b） | 额外仓库同步（已完成）+ S1 登记表（已合入）；每步先在真实脚本上计数再设计 |

顺序理由：阶段 1 最便宜，且是后面所有阶段的前提；脚本数据已到位（`wu2305/autocrm`），阶段 5 单列只因它是独立的一条线、不阻塞 1–4，实际前置是 schema（已合入）、本文与计划的批准、以及 S0 的 oxc 体积与解析 spike。
阶段 1 的 ① S1 已合入（PR #5）；新增节点或边类型的 PR 仍然一次只放一个（计划 §4 冲突规则）。

## 7. 开放问题（括号内为推荐）

0. **已由真实语料解决（2026-10-03）**：不处理 `.fapp` / `.meta`（用户）；「workflow 按路径指向脚本」不成立，链接形态以 §3.5 为准；真实语料已可用，Q4 的 spike 与阶段 2 的真实语料一轮不再被数据阻塞；阶段 1 的起点改为 1.0（用户确认前仍为推荐）。
1. **终态**：旧动词在对等达标后删除，还是长期并存？——**已决（2026-10-02 用户在决策卡上选「Retire」）：对等达标后删除**。删除仍按 §6 阶段 4 分批、每批需用户确认；`--find` 按 §3.2 保留为 GQL 配方别名，不属于被删除的「动词」。
2. **派生事实放哪**：导入期入图，还是 `CALL` 式按需过程？（导入期，只物化直接事实；闭包交给 GQL）
3. **stdio 工具表**：是否同意加入 `gql` / `graph_schema` 并最终收缩到四个工具？MCP 是否先不做？（同意；MCP 等契约稳定）
4. **脚本**：层 1 用 oxc（待体积实测）+ 层 2 走 §6 校验提交，接受吗？（接受，先做字面量占比 spike；数据已不再阻塞）
5. **M59-PATH**：随 `path.rs` 退役而作废、不再修，同时 `same_page` 与物理字段识别从对等基准里剔除、改按金标准判（§3.2），接受吗？（接受；若你想先保留旧动词更久，则保持排队，并仍按金标准而不是旧输出做对照）
6. **单文件模式**：终态改为「临时建图 + GQL」，还是保留 JSON 输出给开发调试？（临时建图；JSON 输出冻结）
7. **成功判据**：阶段 2 的通过线用什么数？（GQL+schema+配方+规则臂在同模型同题上不低于旧动词臂，且 `forbidden_claims` 违规数为 0；具体百分比请你定）
8. **脚本里的凭据**：**已决（2026-10-03 用户）**：不是设计驱动力，关注点是数据处理与数据库操作逻辑；抽取范围按 §3.5 收窄，不另设脱敏规则。
