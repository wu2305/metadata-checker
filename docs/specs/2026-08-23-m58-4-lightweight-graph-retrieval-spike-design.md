# M58.4 轻量图召回对比 Spike 设计

> 状态：**approved（用户确认 2026-08-23）**
> 类型：time-boxed retrieval spike
> 依赖：M58.2 agent harness、M58.3 三动词命令表面
> 实施环境：CNB 远端隔离 worktree；不在本地工作区实施

## 1. 问题

metadata-checker 已有确定性类型图、有限深度因果路径、上下文 BFS 和证据契约。当前缺口不是
“有没有图”，而是查询候选主要依赖词法命中、无权邻域或固定规则：

- 词法种子无法利用图上的远距离相关节点；
- 无权 N-hop 对 `Contains` 等高扇出结构边和因果边一视同仁；
- 固定路径分数不理解 writer、value-source、availability 等查询意图；
- 扩大 depth/budget 会同时扩大噪声和输出体积。

本 spike 验证：在不改图事实、不引入新持久化层的前提下，用“确定性种子分布 +
按意图赋权的 Personalized PageRank（PPR）”生成候选，是否能在相同 top-k 下改善
多跳关系召回。

## 2. 设计来源与取舍

采用两类设计思想，但不复制框架：

1. HippoRAG 的 query-time PPR：把检索命中转换成 restart distribution，再用图传播扩展候选；
2. KG²RAG / PathRAG 的证据组织边界：图排序只找候选，最终结论仍由关系路径支撑。

明确不采用 Microsoft GraphRAG 的社区摘要、全局报告、完整抽取流水线，也不把外部
Graph RAG 仓库作为运行时依赖。本项目已有更精确的结构化事实图，重新抽取实体、关系和
社区摘要会制造第二套事实源。

参考：

- HippoRAG: <https://github.com/OSU-NLP-Group/HippoRAG>
- HippoRAG 2: <https://arxiv.org/abs/2502.14802>
- KG²RAG: <https://github.com/nju-websoft/KG2RAG>
- MiniRAG: <https://github.com/HKUDS/MiniRAG>

实现保持本仓库原生、无新增 crate。

## 3. 核心约束

### 3.1 单一事实源

- `GraphReadStore` 是唯一图输入；不得重建第二份 persisted graph。
- seed score 和 PPR score 只代表候选优先级，不得生成节点、边或业务事实。
- 输出若进入问答链路，必须继续经过既有 path/evidence 层；分数不能替代证据。

### 3.2 公平对照

三种 variant 使用完全相同的 `seed_scores`、图快照和 `top_k`：

| variant | 定义 | 回答的问题 |
|---|---|---|
| `seed_only` | 只按 seed score 排序 | 图传播本身是否有增益 |
| `unweighted_hop` | 对 seed 做双向无权 N-hop 扩展 | 当前邻域式召回基线 |
| `typed_ppr` | 按 `TraversalIntent` 赋权后做 PPR | 意图化图传播是否减少噪声 |

首轮 fixture 直接提供确定性 seed score，不混入 embedding provider、LLM query rewrite 或
不同的 seed 模型，以隔离图算法贡献。

### 3.3 类型和方向

事实边方向与查询方向并不总相同。例如 writer 查询从 field 出发，但 `FieldWrite`
事实可能为 action → field，因此 transition 必须分别配置 forward/reverse 权重，不能把图
整体改成无向图。

初始分组：

- `Writer`：反向 `FieldWrite`、`ActionWrites`、`Writes` 高权；`FieldAlias` 双向；
- `ValueSource`：`Reads`、`FieldAlias`、DataFlow lineage 高权；
- `Availability`：`Reads`、`DependsOn`、DataFlow 输入/内部/输出链高权；
- `Display`：`DependsOn`、条件与组件控制关系高权；
- `Action`：`Triggers`、Action reads/writes/navigation/data-loading 高权；
- `Context` / `Auto`：语义边均衡，`Contains` 等结构边保持低权。

未知边类型不隐式获得高权。权重表集中定义，并以测试锁定。

## 4. 算法契约

输入为 `&dyn GraphReadStore`、`TraversalIntent`、非负 seeds 和
`restart_probability` / `tolerance` / `max_iterations` / `top_k`。

处理：

1. 对图中存在的正 seed 归一化；没有有效 seed 时返回带上下文的 `anyhow::Error`；
2. 以 node id 字典序建立稳定索引；
3. 按 intent 将事实边转换为 forward/reverse 加权 transition arc；
4. 每个源节点按出权重归一化；
5. 迭代 `next = restart * seed + (1 - restart) * transition(rank)`；
6. dangling mass 回灌 seed distribution；
7. L1 差值小于 tolerance 或达到 max_iterations 时结束；
8. 按 score 降序、node id 升序稳定输出。

输出包含 ranked node id、PPR score、原始 seed score、迭代轮数、收敛状态和有效 seed 数。
模块不打印日志、不解析 metadata、不依赖 redb 或 DOM。

## 5. Fixture 与指标

`tests/fixtures/graph_retrieval_spike_cases.json` 每个 case 包含 case/intent/description、
nodes/typed edges、seed scores、relevant node ids、gold terminal ids、top-k 和 hop depth。

首轮至少覆盖：

1. writer：field → alias → action，多条 `Contains` 结构噪声；
2. availability：component → model field → DataFlow lineage；
3. action/display：组件条件或 action 关系，存在高扇出页面节点；
4. dangling seed、并列分数和无有效 seed 边界。

每个 variant 记录 `relevant_recall_at_k`、`precision_at_k`、
`gold_terminal_recall_at_k`、候选数；typed PPR 另记迭代轮数与收敛状态。报告按 case id /
variant 固定顺序序列化，确保相同输入字节级稳定。

## 6. Go / No-Go

首轮 go 条件同时满足：

1. 所有 fixture 的 typed PPR 均召回 gold terminal；
2. typed PPR 的平均 relevant recall@k 不低于 unweighted hop；
3. typed PPR 的平均 precision@k 高于 unweighted hop，或同等 gold recall 下候选更少；
4. 确定性、边界和 WASM feature 编译检查通过；
5. 不改变公共 CLI 输出、graph schema 或持久化格式。

正确性条件失败即 no-go：保留结果，不通过继续调大 depth/top-k 掩盖问题。性能只作观察，
不在小 fixture 上设硬阈值。确定性层 go 不等价于产品效果成立；后续若另获批准，才在
M58.2 中做同模型、同 case、同 trial 的 paired variants。

## 7. 明确不做

- 不新增 embedding、向量数据库、LLM 或网络依赖；
- 不改冻结的 M58/M58.1 JSON-command runner；
- 不改 `.cnb.yml`，不触发 live LLM baseline；
- 不改现有 CLI / stdio / MCP 输出契约；
- 不做社区检测、社区摘要、实体重抽取；
- 不做 redb/IndexedDB schema 迁移；
- 不把 fixture 分数表述为真实项目或模型能力结论；
- 不在本地工作区编辑、编译或测试。

## 8. 兼容性与回滚

实现放在独立模块，经 `lib.rs` 暴露给测试和后续调用方；当前产品查询不接入该模块。
回滚只需删除模块、fixture 和文档登记，不影响数据库。模块使用现有 `GraphReadStore`
与 `TraversalIntent`，同时支持 native 和 browser-wasm feature 组合。
