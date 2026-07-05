# M98：远期性能展望 - 查询速度数量级优化

| milestone | M98 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M98：远期性能展望 - 查询速度数量级优化

> 记录日期：2026-05-23。
> M98 是 planning-only 的远期占位，不纳入 M40 当前验收范围。后续实施时建议拆成 M50-M55 一组性能里程碑，先建立可复现性能画像，再从索引、查询规划、路径算法和物化事实层做数量级优化。

### 背景判断

当前性能问题不应只从 `clone`、release 参数或局部微优化入手。真实项目查询慢的核心风险来自查询计算模型：

- `find_candidates` / target 解析仍可能退化为全图扫描。
- 多跳路径发现容易先扩展大量路径再排序。
- 页面、组件、局部 model、物理表、DataFlow 之间的高频事实每次查询重复推理。
- compact 输出需要为小模型服务，但输出构造和 evidence 采样本身也可能成为耗时来源。
- 冷启动优化已经有 runtime / stdio 方向，但单次查询内部仍需要算法和数据结构层面的加速。

M98 的目标不是现在开始改性能，而是明确未来 M50-M55 的优化方向，避免后续只做零散热点修补。

### 总体目标

- 查询速度争取获得数量级优化，而不是 10%-30% 的微调。
- 从“全图遍历 + 路径排序”转向“索引命中 + 查询规划 + 有界路径模板 + 物化事实读取”。
- 性能优化不得牺牲语义正确性；路径打分只能排序，不能替代结构化因果链路判断。
- 所有性能结论必须有真实项目 benchmark 和阶段耗时 breakdown 支撑。

### 推荐拆分：M50-M55

- [ ] M50：性能基线与查询画像
  - 契约文档：`docs/m50-performance-bench-contract.md`
  - 建立真实项目 benchmark 集：
    - `query_page_logic`
    - `explain`
    - `explain_condition`
    - `query_model`
    - DataFlow 展开
    - 多跳来源追溯
  - 指标拆分：
    - 冷启动加载耗时
    - graph store 读取耗时
    - 候选节点查找耗时
    - path finding 耗时
    - evidence / JSON 输出构造耗时
  - 输出要求：
    - P50 / P95
    - 节点数、边数、候选数、扩展边数
    - 每阶段耗时 breakdown
    - benchmark 命令、commit、graphdb 路径、真实项目路径。

- [ ] M51：索引体系重构
  - 目标：消灭查询阶段的全图扫描。
  - 建议索引：
    - `node_id -> node`
    - `node_type -> node_ids`
    - `page_path -> component/model/action ids`
    - `local_model_id + page_path -> resolved model`
    - `physical_table -> local models`
    - `field_name -> field nodes`
    - `edge_type + from -> targets`
    - `edge_type + to -> sources`
    - `component_id/page_path -> conditions`
    - `dataflow output field -> original node/field`
  - 验收要求：
    - `find_candidates` 不再依赖 `iter_nodes()` 扫全图作为主路径。
    - 索引更新与增量 build graph 一致，不出现 stale index。
    - MemoryGraphStore 与 GraphDB 的索引行为等价。

- [ ] M52：查询规划器
  - 目标：不同问题走不同查询计划，不再统一盲 BFS。
  - 计划类型：
    - 组件显示条件：从 component / parent condition edges 开始。
    - 值来源：从 component field -> Reads / FieldAlias / DataflowOutput 反向追。
    - 写入来源：从 model/field -> incoming Writes / ActionWrites / FieldWrite。
    - 页面数据源优化：从 page scope models -> filters / conditions / writers / readers。
    - DataFlow：直接进入 dataflow subgraph，不从页面图盲搜。
  - 输出可增加调试字段：
    - `query_plan.intent`
    - `query_plan.entry_index`
    - `query_plan.expanded_nodes`
    - `query_plan.pruned_nodes`
  - 要求：query plan 只解释性能路径，不改变已有 answer contract 的事实语义。

- [ ] M53：多跳路径算法优化
  - 目标：从“找很多路径再排序”改成“有目标地找少量高价值路径”。
  - 可选算法：
    - bounded bidirectional search
    - edge-type priority queue
    - beam search 作为剪枝参数
    - path dominance pruning
    - typed path templates
  - typed path templates 示例：
    - `component -> local model -> physical table <- action`
    - `component -> field alias -> physical field <- writer`
    - `component -> dataflow output -> original node -> input table`
  - 要求：
    - visited state 从 `node_id` 升级为 `(node_id, relation_context)`。
    - 同目标、同语义、证据更弱的路径可以淘汰。
    - 打分只用于排序，不能决定路径是否具备语义因果关系。

- [ ] M54：物化事实层
  - 目标：把高频推理结果预计算，查询时直接读取。
  - 可物化组件事实：
    - `value_sources`
    - `visibility_conditions`
    - `availability_gates`
    - `read_models`
    - `write_targets`
  - 可物化 model/field 事实：
    - `writers`
    - `readers`
    - `aliases`
    - `dataflow_lineage`
  - 可物化页面事实：
    - `page_scoped_model_map`
    - `data_sources`
    - `entry_actions`
    - `cross_page_writers`
  - 要求：
    - 物化事实必须带 schema version 和 source evidence。
    - 增量更新必须能清理旧事实。
    - 查询输出仍能回溯原始 graph evidence。

- [ ] M55：性能回归门禁
  - 目标：防止后续功能把查询性能打回去。
  - 建立 benchmark fixture 和真实项目 ignored benchmark。
  - 性能预算示例：
    - 小 fixture：`< 50ms`
    - 中型页面：`< 300ms`
    - 大型真实页面：`< 1s` 或按 M50 基线设定。
  - 记录字段：
    - expanded nodes
    - loaded nodes
    - result size
    - elapsed per stage
  - 要求：
    - 性能退化超过阈值时本地 benchmark 给出明确报警。
    - CI 可先只跑 fixture benchmark，真实项目 benchmark 保持 ignored/manual。

### 明确不建议

- 不要先做局部 clone 清理就宣称性能优化完成。
- 不要用更激进的 path 打分替代路径语义建模。
- 不要为了快而丢掉 evidence、diagnostics 或 answer contract 字段。
- 不要把性能优化和 M40 browser glue 混在同一个提交里。
- 不要在没有 M50 基线的情况下重写 pathfinder。

### 预期收益来源

- M51 索引体系：减少全图扫描，通常是第一层数量级收益。
- M52 查询规划：减少错误入口和无关扩展，提升复杂问题稳定性。
- M53 路径算法：控制多跳爆炸风险。
- M54 物化事实：对 AI 高频问题提供最大收益，尤其是重复问组件、model、字段来源时。
- M55 门禁：保证性能收益不会被后续语义功能抵消。
