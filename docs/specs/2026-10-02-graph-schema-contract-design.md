# 图 Schema 契约与图内事实设计

> 状态：**draft**（待用户批准；批准前不写实现代码）
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
| 过滤字段名（`externalUserId`/`staffId`）两组都没报出 | 待查：事实是否已在图里（§6 开放问题） |
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

## 5. 实施分包与验收（待批后落为 plan）

| 包 | 内容 | 验收 |
|---|---|---|
| S1 | `graph_schema.rs` 表（现状 v1 行集）+ `--graph-schema` + help 由表生成 + 一致性测试 + `docs/reference/graph-schema.md` 生成与漂移检查 | 现有 22 边 / 6 节点全部有行；夹具图全元素合规；删表一行测试变红 |
| S2 | 写入校验（导入与增量路径） | 构造违规元素被拒并带上下文；增量与全量产物仍相等 |
| S3 | 血缘边（§3.1）+ schema v2 版本键与拒载 | 差分测试与 `--explain` 相等；v1 库被拒并提示重建 |
| S4 | 诊断入图（§3.2） | `MATCH (d:Node {node_type:'Diagnostic'})` 可查；与查询期同 code 的诊断一致 |
| S5 | 复跑同一 30 题评测（夹具 + 可得时真实语料），记录与 79% / 86% 基线的差异 | 报告入 `docs/ai-eval-runs/`，如实记录未改善的题 |

S1 无图结构变化，可先行；S3 / S4 触发 schema 版本变化，须在 S2 之后。每包单独 PR。
体积：S1/S2 预计可忽略，S3/S4 在远端 `cargo build --release` 后按 AGENTS.md 记录实测体积。
`Baseline impact`：S3/S4 增加节点和边数量，影响建图与加载 bench，实施时说明。

## 6. 开放问题（批准前请决定或授权我按推荐处理）

1. **过滤字段名**：评测里 `externalUserId`/`staffId` 没被报出，是图里没有、还是模型没查到？实现前先在夹具上核实；若图里没有，作为 S3 的第三类事实加入。
2. **解释规则放哪**：推荐放在 schema 表里随 `--graph-schema` 一起输出（单一真源）；备选是只放 SKILL 文档。
3. **写入校验的严格度**：推荐导入期违规即报错（fail-visible）；备选是仅在测试中校验、运行期只告警。
4. **`Diagnostic` 节点是否包含查询期才能算出的诊断**：推荐否（保持图 = 静态事实，查询期诊断仍由查询层给）。
