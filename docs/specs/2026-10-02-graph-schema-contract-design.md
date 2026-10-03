# 图 Schema 契约与图内事实设计

> 状态：**approved**（2026-10-02 用户合入 PR #4；S1 先行，S2–S5 分包推进）
> 里程碑：[M59](../milestones/performance/m59-grafeo-backend-migration.md)（active）；承接 plan 账本中 deferred 的 M59-F3 / M59-FACTS
> 上游：[Grafeo 后端迁移设计](2026-09-05-grafeo-backend-migration-design.md)（schema 一次性冻结的风险）、
> [语义完整性修复](2026-09-21-semantic-integrity-design.md)（事实键与证据契约）、
> [`--gql` 现状](../knowledge/topic-grafeo-gql.md)
> 方向来源：2026-10-02 用户——「需要一份 schema 定义，既指导小模型写查询，也约束工具实现」

## 1. 目标与证据

终极目标：LLM 只查 `.grafeo` 图库就能回答问题，不必读源文件。

2026-10-01 的 30 题评测（Haiku 4.5 代理、单次、夹具语料、自评；
报告 `/mnt/project-files/c4-gql-eval/report.md`，不入库）给出的结论是：

| 现象 | 性质 |
|---|---|
| 单跳 / 遍历类问题只靠 GQL 就答对 | 现有图够用 |
| 字段级血缘（`field:df_b.id` 只有来自 model 的 `Contains` 边）查不到 | **图缺事实** |
| `UNKNOWN_ACTION_TYPE` 这类诊断按文本查不到 | **图缺事实**（见 §3.2：该诊断是查询期计算的，不在图里） |
| 过滤字段名（`externalUserId`/`staffId`）两组都没报出 | 待查：事实是否已在图里（§7 开放问题） |
| 把隐藏组件说成渲染故障、编造「参数未提供」「表为空」等运行时事实 | **缺解释规则**，不是缺数据 |
| 方言误写（`[:A\|:B]`、`CONTAINS()`）可自行恢复 | 提示即可，PR #3 已补 help |

所以问题有两半：图里缺的事实，以及没有任何东西规定「图里应有什么、各自什么意思、哪些结论不能下」。
本 spec 用**一份机器可读的 schema 定义**同时解决后者，并以它为约束落实前者。

## 2. 核心决定：一份 schema 定义，三处消费

新增 `src/graph_schema.rs`（Rust 常量表，不引入新依赖），描述图的全部合法形状。它是**唯一真源**：

| 消费方 | 做什么 | 防止什么 |
|---|---|---|
| **LLM 侧** `--graph-schema`（JSON；`--human` 为文本）与 `--gql` 的 long help | 从同一张表生成：节点类型、属性、边类型（含起止节点类型）、`meta` 键、id 文法、示例查询、方言注意事项、**解释规则** | 手写 help 与实现漂移 |
| **实现侧**写入校验 | 导入 / 增量写图时，每个节点和边都对照表校验：未知边类型、不允许的端点类型组合、缺必需 `meta` 键、id 不合文法 ⇒ 返回 `anyhow` 错误（带 `.with_context()`，不 `panic!`，不静默丢弃） | 实现悄悄长出表里没有的形状 |
| **测试侧**一致性测试 | 扫描 `tests/fixtures/test_project`（有真实语料时再加语料）后遍历全图，断言全部元素合规；同一张表生成 `docs/reference/graph-schema.md`，CI 比对漂移 | 文档、表、图三者不一致 |

### 2.1 表的形状（本 spec 固定形状，行内容由实现从现有 `scanner/` 写入路径逐行核出）

- **节点类型行**：`node_type`、一句话语义、id 文法、必需 / 可选 `meta` 键（名 + 类型 + 含义）。
- **边类型行**：边类型名、`from`/`to` 允许的节点类型、`field_path` 语义、`meta` 键、一句话语义、
  产生该边的规则（引用 `scanner/` 的哪条路径），以及**不产生**的情形。
- **解释规则行**（新）：告诉读图者「由这些事实能下什么结论、不能下什么」。首批来自评测暴露的错误：
  - 组件的 `visibleCondition` 为假（或被隐藏）是配置意图，不是渲染故障；
  - 图只含静态元数据，运行时值（参数是否传入、表是否为空）不可从图推断，必须说「图中无法确定」；
  - 缺失的出边不等于「没有该关系」，见各边类型的「不产生」条件。
- **方言行**：`[:A|B]` 多类型写法、无 `=~`、无 `CONTAINS()` 函数形式、无 `NOT (n)--()` 模式谓词（均已由 PR #3 的测试验证）。

现有 22 种 `EdgeType`（`src/graph.rs`）与 6 种 `NodeType` 是表的初始行集；变更任何一种都必须同改表。

## 3. 图内新增事实（schema v2）

仅列评测证明缺失的两类；先不扩张。

### 3.1 字段级血缘边（承接 M59-F3 / M59-FACTS 中的血缘部分）

- 现状：`--explain` / `query_dataflow` 在**查询期**沿 `FieldAlias`、`DataflowInternal` 等边与表达式计算字段血缘（`src/query.rs`、`src/dependency.rs`），结果不在图里。
- 方案：导入期把已确定的字段→字段血缘物化为 `Field → Field` 边（边类型名在实现阶段与既有 22 种对照后定，候选 `DerivesFrom`），`meta` 带来源边类型与证据表达式。
- 约束：只物化**静态可确定**的血缘；不确定的保留为诊断（§3.2），不得猜测补边。
- 验收：评测 `dataflow_chain_trace` 用例仅用 GQL 能列出完整链路；与 `--explain` 的结果逐项相等（差分测试，不降低断言）。

### 3.2 诊断入图

- 现状（已核对源码）：`UNKNOWN_ACTION_TYPE` 由 `src/query/page_logic/diagnostics.rs` 在查询期从 action flows 聚合产生，**不存储**；扫描期诊断（`SCANNER_*`）以不透明字节块存在 `IndexState`，GQL 看不到内容。
- 方案：新增 `node_type = Diagnostic` 与边 `HasDiagnostic`（被诊断对象 → Diagnostic 节点）；属性 `code`、`severity/impact`、`message`、`json_path`，沿用 `src/diagnostics.rs` 现有 code 与 impact 分级，**不另造 code**。
- 范围：仅物化**导入期可由图与元数据确定**的诊断（含 `UNKNOWN_ACTION_TYPE` 的判定条件是否可在导入期复算，实现阶段先核实；不可复算者在表的解释规则里标注「仅查询期产生」，不入图）。
- 与 `IndexState` 的关系：`IndexState` 仍是索引内部状态；`Diagnostic` 节点属于项目图，随源文件 origin 撤销（沿用 M59-2 归属契约）。

### 3.3 版本与兼容

- schema 版本写入图库元信息（沿用既有 `fact_schema_version` 一类版本键机制，`src/graph_redb.rs:42`；`.grafeo` 侧对应落点实现阶段对齐）。
- v1 库（无新增事实）打开 v2 工具：拒载并提示重建，沿用「事实键升级不与旧库混写」原则；从原始元数据重建，不从旧图推断。

## 4. 不做什么

- 不改 GQL 引擎、不开 `lpg`（评测与体积实测均无必要）。
- 不把 `--gql` 加入 stdio / MCP 工具契约（M58 评测固定了工具表面；等本 spec 落地并复评后单独决定）。
- 不压平 `meta`（评测未证明它是失败原因；schema 固定 `meta` 键集合，日后压平有据可依）。
- 不为「小模型会犯错」重造查询语言；靠 schema 与解释规则，不靠新语法。
- **不放宽 `--gql` 的只读边界**：LLM 衍生的关系不经 GQL 写入（引擎只读会话与形态闸两道闸原样保留），只经专用的、由契约表校验的提交命令进入（§6.4）。
- 工具本身不调用 LLM、不联网：脚本分析发生在工具之外（§6），工具只负责校验与存储提交上来的关系。

## 5. 实施分包与验收（待批后落为 plan）

| 包 | 内容 | 验收 |
|---|---|---|
| S1 | `graph_schema.rs` 表（现状 v1 行集）+ `--graph-schema` + help 由表生成 + 一致性测试 + `docs/reference/graph-schema.md` 生成与漂移检查 | 现有 22 边 / 6 节点全部有行；夹具图全元素合规；删表一行测试变红 |
| S2 | 写入校验（导入与增量路径） | 构造违规元素被拒并带上下文；增量与全量产物仍相等 |
| S3 | 血缘边（§3.1）+ schema v2 版本键与拒载 | 差分测试与 `--explain` 相等；v1 库被拒并提示重建 |
| S4 | 诊断入图（§3.2） | `MATCH (d:Node {node_type:'Diagnostic'})` 可查；与查询期同 code 的诊断一致 |
| S5 | 复跑同一 30 题评测（夹具 + 可得时真实语料），记录与 79% / 86% 基线的差异 | 报告入 `docs/ai-eval-runs/`，如实记录未改善的题 |
| S6 | 脚本衍生关系层（§6）：`Script` 节点、扫描期锚点边、`--annotate` 提交与校验、sidecar 持久化与重建重放 | 违规提案被拒并逐条给原因；脚本改动后旧标注变 stale；全量重建后标注按 hash 重放；扫描结果与是否有标注无关 |

S1 无图结构变化，可先行；S3 / S4 触发 schema 版本变化，须在 S2 之后。每包单独 PR。
体积：S1/S2 预计可忽略，S3/S4 在远端 `cargo build --release` 后按 AGENTS.md 记录实测体积。
`Baseline impact`：S3/S4 增加节点和边数量，影响建图与加载 bench，实施时说明。

## 6. 脚本衍生关系：LLM 标注层（2026-10-02 用户补充）

### 6.1 场景与已核实的边界

用户指出两类静态扫描看不见的关系：前端脚本（操作页面参数、在 action 之间自定义逻辑）与后端 Nashorn 脚本（接收第三方系统查询、写库、在 dataflow 内跑特殊逻辑）。设想由 LLM 读脚本后，把关系写进图。

已在仓库核实的事实：

- 前端（用户 2026-10-02 补充）：前端脚本是后缀为 `.ts` 的独立文件（与后端的 `action.ts` 以后缀区分）；除了操作页面参数、在 action 之间串联逻辑，**脚本还能修改 SuperPage 的内容**（即运行时改组件属性，让页面与静态元数据不一致）。仓库现状：`actionType: "script"` 的 action 已被识别（`src/action_semantics.rs`，分类 `script_execution`），语义摘要写着「具体副作用需人工确认」；真实语料里这类 action 只带 `scriptFunction`（如 `setMsg`，一个页面里有 11 个），**页面 JSON 里看不到指向 `.ts` 文件的路径**。扫描器不为脚本 action 建任何边，也不收录 `.ts` 文件类型。
- 后端（用户 2026-10-02 答复）：Nashorn 脚本是独立的元数据文件，后缀 `action.ts`；工作流（workflow）节点通过路径链接到它，第三方系统的调用也有路径可作标识。仓库现状：全库没有 Nashorn 样例；`src/parser.rs` 只对「workflow 风格 `nodes`」做浅层组件抽取，扫描器**不扫描**工作流文件，也**不收录** `action.ts` 文件类型。也就是说，两种文件在图里目前都是盲区，S6 要新增两种文件类型的发现与解析（`parser.rs` 注册）。

### 6.2 决定：两类事实，物理上分开

| | 扫描事实（scan） | 脚本衍生事实（inferred） |
|---|---|---|
| 来源 | 元数据的确定性解析 | LLM 对脚本源码的判断 |
| 可信度 | 可重建、可差分验证 | 建议性，必须带证据与置信度 |
| 重建 | 随时从原始元数据重新生成 | **不能**由扫描重新生成；见 §6.5 |
| 类型 | 现有 22 种边及 S3/S4 新增 | 专用的 `Script*` 边族，**与扫描边类型不相交** |

**不复用 `Reads` / `ActionWrites` 等扫描边类型。** 否则 `MATCH ()-[r:Reads]->()` 会悄悄混入 LLM 的猜测，读图者无法分辨，且违背「扫描事实可差分验证」。边类型本身就是隔离，不依赖读图者记得加 `WHERE`。

### 6.3 图形状（契约表新增的行，随 S6 落地）

- 新节点类型 `Script`：脚本本身。id `script:<源文件>|<脚本引用>`；`meta` 含 `kind`（`frontend`：普通 `.ts`；`backend`：`action.ts`，Nashorn）、`content_hash`（脚本文本的 hash）、`entry_names`（已知函数名，如 `setMsg`）。
- 新**扫描**边（确定性，属于扫描事实）：`ExecutesScript`：`Action → Script`（前端 `script` action，按 `scriptFunction` 在页面绑定的 `.ts` 文件里定位函数；页面与 `.ts` 的绑定规则见 §7 问题 9）；后端：`workflow 节点 → Script` 的链接边（暂名 `LinksScript`）由扫描从工作流元数据里的 `action.ts` 路径确定性地产生；路径解析不到文件时落占位 `Script` 节点并报诊断，不静默丢弃。扫描只负责「这里有一段脚本」，不解读脚本。
- 新**衍生**边（仅起点为 `Script` 节点，端点必须是图里已存在的节点）：

| 边类型 | 端点 | 含义 |
|---|---|---|
| `ScriptReads` | Script → Model / Field | 脚本读取该模型或字段 |
| `ScriptWrites` | Script → Model / Field | 脚本写入 / 插入 |
| `ScriptSetsParam` | Script → Field（`param:`） | 脚本设置页面参数 |
| `ScriptTriggersAction` | Script → Action | 脚本在 action 之间调用 / 触发其他 action |
| `ScriptNavigates` | Script → Page | 脚本打开页面 |
| `ScriptModifiesComponent` | Script → Component | 脚本在运行时修改页面内容（组件的属性、显隐、数据绑定等）；属性名放 `meta.property` |
| `ScriptHandlesRequest` | Script → `Endpoint` | 后端脚本接收第三方请求；`Endpoint` 是以调用路径为 id 的新节点类型（`endpoint:<路径>`），路径来源见 §7 问题 5 |

清单是起点，不是穷举；每增一种都要在契约表加行（`layer = inferred`）并说明「缺席何时不代表没有」。

- 脚本改页面内容使「静态元数据 = 运行时页面」不再成立。契约表解释规则新增 `script-may-change-page`：被 `ScriptModifiesComponent` 指向的组件，其静态属性（显隐、取值、绑定）只是初始状态；没有该边不代表没有脚本改它（脚本分析未做或判断为空），回答要说明这一点。
- 每条衍生边的**必填属性**：`analyzer`（模型标识 + 提示词版本）、`script_hash`（分析时脚本的 hash）、`confidence`（`high` / `medium` / `low`）、`evidence`（脚本里的原文片段）、`analyzed_at`、`status`（`current` / `stale`）。因为要被 GQL 过滤（`r.status = 'current'`），这些必须是**扁平边属性**，不放进 JSON 文本 `meta`；这也是需要 schema v2（§3.3）的原因之一。

### 6.4 提交与校验：不经 GQL 写

- `--gql` 保持只读（引擎只读会话加形态闸，两道闸不动）。写入走专用命令（暂名 `--annotate <file.json>`），载荷是提案列表：`{edge_type, from, to, script_hash, evidence, confidence, analyzer}`。
- 校验全部由契约表驱动，任何一条不过即**拒收该提案并说明原因**（不静默丢弃、不 `panic!`）：
  1. `edge_type` 必须是表里 `layer = inferred` 的类型；
  2. 起止节点必须已存在，且类型符合该边的端点组合；起点必须是 `Script` 节点；
  3. 工具自己重读脚本源码：当前 `content_hash` 必须等于提案声明的 `script_hash`，且 `evidence` 必须是脚本文本的逐字子串（防止 LLM 编造依据）；
  4. `confidence` 属于枚举；`analyzer` 非空；
  5. 同一 `(from, to, edge_type, script_hash)` 幂等。
- 校验只保证「形状合法、有真实依据」，**不保证 LLM 的判断正确**。解释规则新增一条：衍生边是建议性的，回答中要标明「由脚本分析推断，置信度 X」，不得当作确定事实陈述。

### 6.5 持久化与重建

- 与现有原则一致——「重建从原始元数据重新生成，不从旧图推断」（语义完整性 spec 契约 5）：**标注提交是原始输入，不是图的派生物**。因此推荐把已通过校验的标注写入项目旁的 sidecar 文件（如 `.metadata-checker/annotations/*.json`，可审阅、可入版本库），图是「元数据 + 标注」的派生。
- 重建 / 全量扫描：先按元数据重建扫描层，再重放 sidecar：脚本 `content_hash` 与标注一致则恢复为 `current`；不一致则标 `stale` 保留（默认视图排除，可查看以便重新分析），脚本节点消失则对应标注移除。
- 增量扫描：脚本文件变化只影响该 `Script` 的标注（置 `stale`），不触碰其他文件的标注；沿用归属 / 撤销契约（按来源文件记账）。
- 验收要求：同一份元数据加同一份 sidecar，无论扫描顺序，重建出的图相等；删除全部 sidecar 后扫描层与现在逐边相等（标注层对扫描结果零影响）。

### 6.6 与 S1–S5 的关系

不改变 S1 / S2。S3 引入的 schema v2（扁平边属性、拒载旧库）是 S6 的前提；契约表的 `layer`（scan / inferred）列在 S6 加入，S1 的表不需要预留。S6 单独出 plan，待 §7 问题 5–9 有答案后再定。

## 7. 待决问题（未答复时按推荐处理；问题 5–9 来自 2026-10-02 脚本场景）

1. **过滤字段名**：评测里 `externalUserId`/`staffId` 没被报出，是图里没有、还是模型没查到？实现前先在夹具上核实；若图里没有，作为 S3 的第三类事实加入。
2. **解释规则放哪**：推荐放在 schema 表里随 `--graph-schema` 一起输出（单一真源）；备选是只放 SKILL 文档。
3. **写入校验的严格度**：推荐导入期违规即报错（fail-visible）；备选是仅在测试中校验、运行期只告警。
4. **`Diagnostic` 节点是否包含查询期才能算出的诊断**：推荐否（保持图 = 静态事实，查询期诊断仍由查询层给）。
5. **（已答复一半）** 脚本是独立的 `action.ts` 文件，工作流节点按路径链接；第三方调用「有路径可标识」。还需要：工作流元数据文件的后缀与结构（节点里哪个字段放 `action.ts` 路径？第三方调用路径是写在工作流元数据里、脚本里，还是别处？），以及一个 `action.ts` 样例。有这两个样例才能定 `Script`、`Endpoint` 的 id 文法与扫描规则。
6. **`action.ts` 的路径基准**：节点里的路径是相对项目根、相对工作流文件，还是带 `$DATA:` 之类前缀（现有 `.tbl` 引用有这种形式）？决定 `Script` 节点 id 的归一化方式。
7. **LLM 分析由谁跑**：推荐在工具之外（agent / skill）读脚本，经 `--annotate` 提交；工具不内置模型调用。是否同意？
8. **标注持久化**：推荐 sidecar 文件为准（§6.5）；备选是只存在图库内。后者在全量重建时会丢标注。
9. **页面与前端 `.ts` 怎么绑定**：真实语料的页面 JSON 里只有 `scriptFunction`，没有 `.ts` 路径。是约定同名（`合同协议.spg` 对 `合同协议.ts`），还是页面里有别处声明（哪个字段）？需要一个页面加对应 `.ts` 的样例。
