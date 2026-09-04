# 本地图查询平面与后端深化研究计划

> 状态：**approved**（2026-09-04 用户确认方向：解决分析时的查询缺口，不把问题简化为 jq 读取 redb）
> 范围：为模型提供可控的本地图查询入口，同时覆盖图关系和未进入图的原始 SPG/TBL 元数据；先研究与原型，后决定是否替换 redb。
> 关联：[M58.3 spec](../specs/2026-08-24-m58-3-command-surface-gap-fixes-design.md)、[M58.3 plan](2026-08-24-m58-3-command-surface-gap-fixes-plan.md)、[性能基线](../milestones/performance/performance-baseline.md)。

## 1. 决策边界

本计划解决的是“工具现有命令面无法表达某些元数据查询，分析者被迫回到原始文件使用 jq”的问题。

目标不是单纯把 redb 改成 JSONL，也不是先选定 DuckDB 或某个 Cypher 引擎。持久化后端必须服从查询平面、原始文档覆盖、增量一致性和跨平台约束。

现有高层命令（`find`、`explain`、`relations`、`query-*`）继续保留；新增低层只读查询能力，用于取证、探索和补充高层语义输出。

## 2. 现状与约束

- 当前 `GraphReadStore` 只提供按 ID 取节点、邻边、遍历节点和计数，不能表达任意过滤、投影、聚合或路径查询。
- 当前节点/边只保存被 scanner 选择的 `meta`；原始 SPG/TBL 中未抽取的字段不一定存在于图中。
- redb/petgraph 当前没有 Cypher 执行层；“Cypher 透传”只有在底层引擎支持 Cypher 时才是真透传，否则属于受限 Cypher 编译器。
- M58.3 页面局部模型 ID、schema version、表达式和事实输出仍在收尾，新的查询平面不能固化旧的全局 `model:<name>` / `field:<model>.<field>` 身份。
- xiaoshouyi 当前性能基线记录约为 89,094 节点、199,575 条边、514 MiB graphdb，stdio 启动加载约 1,008 秒；release binary 约 3.6 MiB。这是性能基线快照，不等同于下述 2026-08-30 session 工作态；阶段 0 必须重新钉住可复现的源目录、提交/内容 hash 和清单。新方案必须测量是否能避免全量 hydrate，而不只比较文件大小。
- Rust 2024、四平台构建、release binary 10MB 上限、只读安全和现有 `AiOutput`/`--non-human` 契约继续有效。

### 2.1 既有惠通陆华 session 证据

本计划回顾了以下本地 session；它们是需求证据，不是当前 `metadata-checker` 仓库的提交记录。

- `01a05232-a228-7e82-acfc-da3aab3517dd`：2026-08-30 启动后即中断，只包含工作区指令和未完成的路径输入，没有可复用的业务结论。
- `01a05233-5b52-7da2-acf0-eae776264098`：完成销售与合同二阶段深化。以 2026-08-30 工作态为准建立了 1,387 个 SPG/TBL 文件、92,862 个节点、209,526 条关系的项目图；同时记录 314 个未识别容器键、1,331 个重复组件 ID。871 个 TBL 是已索引文件子集，不能直接等同于“未覆盖”；session 明确指出 146 个服务动作和 155 个 TypeScript action 源码不在当前图查询的完整覆盖范围内。该 session 还指出 compact 输出会截断/抽样、模糊 target 可能命中多个节点，关键结论必须回到原始 SPG 和 action 源码。
- `01a056da-5e7c-7fa0-8f62-ff24bf05075e`：完成 `026`—`028` 岗位手册和税务口径复核。审查把“解约流程创建、客户签署完成、CRM 文件回写、ERP 状态完成”拆成不同状态，并确认当前没有可证明的通用重放队列、幂等键或补偿工作台；同时区分静态按钮/读取条件、生产 ACL 和外部平台回执。该 session 的主 reviewer 因容量不可用，按规则由 Luna/max fallback 通过，不能等同于主 reviewer 通过。

这些证据对查询平面的直接影响是：

1. 查询对象不能只有 nodes/edges，还必须能返回原始文档、action 源码引用和 JSON 路径；
2. 结果必须保留“业务规范 / 静态实现 / 交付快照 / 运行事实 / 外部回执”的证据层级，不能把页面存在或接口调用成功写成业务完成；
3. 查询接口需要显式处理多命中、抽样、截断和未覆盖范围，不能把空结果当成“不存在”；
4. 真实 session 中的 jq 命令应成为阶段 0 的固定回归集，尤其是那些当前 `find`/`explain`/`relations` 无法表达的联合查询；但必须先按查询意图分类，区分原始元数据查询、工具响应裁剪和展示格式化，并脱敏、规范化结果。

## 3. 阶段与交付物

### 阶段 0：真实查询缺口冻结

从惠通陆华分析仓库和上述 session 产物中收集实际使用过的 jq 命令，不用抽象猜测替代。记录命令对应的工具版本、源目录、git commit 或内容 hash；只保存脱敏后的命令和证据引用。

交付物：查询缺口清单，至少 20–30 条样例，覆盖：

- 图关系、邻接和有限路径；
- 原始 JSON 任意子树或字段；
- 图关系与原始文档联合查询。

每条样例记录原始命令（脱敏）、查询意图分类、规范化期望结果、当前工具缺口、证据路径、快照 hash 和可接受的输出大小；把响应裁剪/格式化样例单列，不把包装差异当作查询能力缺口。

**门槛**：没有真实样例，不进入后端选型结论。

### 阶段 1：事实覆盖与身份盘点

沿 `SPG/TBL -> parser -> scanner -> GraphDB -> query/output` 链路核对：

- 哪些节点、边、表达式和 `meta` 已入图；
- 哪些细节只能从原始 SPG/TBL 取得；
- `source_file`、`json_path`、文件 hash 与图事实如何关联；
- M58.3 页面局部模型迁移后需要保留的节点、边和目标语法。

交付物：事实覆盖矩阵和最终候选查询 schema，至少包含：

`nodes`、`edges`、`documents`、`file_states`、`scanner_diagnostics`、`graph_meta`。

每条可查询事实还必须有 `evidence_layer`（业务规范/静态实现/交付快照/运行事实/外部回执）、`source_kind`（graph/raw/action/runtime/external）、`snapshot_id`、`coverage_status`（complete/partial/sampled/truncated/unavailable）和稳定的 `source_file`/`json_path`；空结果必须能区分“不存在”和“未覆盖/外部不可用”。

### 阶段 2：候选后端深度研究

候选至少包括：

1. DuckDB + SQL/JSON；
2. 可嵌入且支持 Cypher 的图数据库；
3. 基于现有 `GraphReadStore` 的受限 Cypher；
4. SQLite 或其他轻量 SQL 控制组。

研究维度固定为：维护状态、许可证、Rust API、四平台、二进制体积、JSON 能力、只读隔离、参数绑定、超时/取消、事务、增量更新、冷启动、内存、查询延迟、原始文档证据定位。

**证据规则**：优先官方文档和官方仓库；记录版本日期；区分事实、推断和未验证项；不得把 DuckDB 当作原生 Cypher 引擎描述。

### 阶段 3：查询协议与安全模型

先定义后端无关的只读查询协议，再选择具体实现。

建议命令形态：

```json
{
  "command": "graph_query",
  "language": "sql-or-cypher-subset",
  "query": "...",
  "params": {},
  "limit": 100,
  "timeout_ms": 5000
}
```

协议必须规定：

- 只读、单语句、参数绑定；
- 最大行数、字节数、跳数和执行时间；
- 结构化错误、截断标记和诊断；
- 每条事实的节点/边 ID、`source_file`、`json_path`、`evidence_layer`、`source_kind`、`snapshot_id`、`coverage_status`；
- `--non-human` 单 JSON 输出；
- 禁止写操作、任意文件读取、外部网络和无界遍历；
- 原始文档中的凭据、token、cookie 等敏感字段的脱敏或拒绝策略。

如果最终使用受限 Cypher，第一版只支持 `MATCH`、有限跳数关系、`WHERE`、`RETURN`、`ORDER BY`、`LIMIT`，不宣称完整 openCypher 兼容。

### 阶段 4：非侵入式原型

暂不删除 redb。由 `SPG/TBL -> projection` 的独立生成器从固定源快照生成查询投影或临时 DuckDB 数据库，同时保留当前运行时路径；查询-only 进程必须验证不打开、不 hydrate redb，不能从已 hydrate 的 redb 再生成投影。

原型需要验证：

- 真实 jq 样例是否可以逐条重现；
- 原始 JSON 和图关系能否联合查询；
- 查询是否能返回足够的证据定位；
- 是否可以绕过启动时完整 hydrate redb；
- 高层命令输出不发生变化。

补充负向验收：拒绝写语句、任意文件读取、路径穿越、外部网络访问和无界遍历；验证嵌套敏感字段脱敏/拒绝、源库 hash 不变、超时取消和结构化错误。

受限 Cypher方案应与 DuckDB SQL方案分别做最小 spike，不直接开始完整 parser 或全量迁移。

### 阶段 5：基准与决策门

使用同一份惠通陆华不可变 pin 语料，明确记录源目录、git commit/内容 hash、文件清单和脱敏规则；阶段 0 与阶段 5 必须使用同一 `snapshot_id`，禁止混用性能基线与 session 工作态。记录：

- 全量构建、增量提交和重启恢复；
- 数据库体积与文件数量；
- 冷启动、首条查询和连续查询；
- 内存峰值与输出体积；
- 真实 jq 样例的正确率和 P50/P95；
- release binary 大小与四平台构建结果；
- 并发/锁、损坏数据和异常查询行为。

决策不能只看数据库大小。关键问题是：模型能否以受控成本查询图和原始元数据，以及是否避免全量 hydrate。

### 阶段 6：正式实现与迁移

研究和 spike 通过后，另行提交后端无关的 approved spec/plan，明确：

- query backend interface；
- canonical 数据与 raw document schema；
- schema/generation/checkpoint；
- `IndexCommit` 原子语义；
- LongLived deferred persistence；
- 增量更新、崩溃恢复和旧库迁移；
- redb 保留、双写、投影或完全替换的边界。

正式迁移须在 M58.3 页面局部 schema 和最终重建验收后进行，避免把旧身份问题复制到新后端。

## 4. 推荐的初始技术路线

完成阶段 0–3 并进入阶段 4 后，优先做“DuckDB 查询投影 + 原始 documents 表”的 spike，同时保留现有 redb 运行时；并行评估“受限 Cypher 编译到 GraphReadStore/SQL”的模型体验。

理由：

- 原始 JSON 细节是 jq 回退的主要来源，SQL/JSON 对这类查询更直接；
- 图关系可以用 `nodes`/`edges` 表和视图表达；
- 可以先验证按需查询是否解决冷启动和低层取证问题；
- 不需要在第一轮就承担完整 Cypher 引擎或大规模持久化迁移风险。

这只是原型顺序，不是最终后端结论。

## 5. 最终验收

计划完成必须同时满足：

1. 真实 jq 缺口有明确替代查询；
2. 图事实和原始文档事实均可查询；
3. 结果带可复核的文件/JSON 路径；
4. 现有高层命令和 M58.3 验收不退化；
5. 查询接口只读、限额、可取消、错误可见；
6. 增量、崩溃恢复、schema 迁移和锁行为有测试；
7. 四平台和二进制大小符合项目约束；
8. 独立复核确认后，才决定是否移除或降级 redb。
