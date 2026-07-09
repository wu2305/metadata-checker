# M53 warm cache 可扩展性设计（page diff 地基）

> 状态：approved（用户确认 2026-07-08）
> 范围：修复 M53 任务 A（扩展 warm cache 覆盖 prerequisites/paths）引入的两个可扩展性代价（O(P²) clone、内存不随 budget 收缩），并为后续"runtime page diff 触发选择性 re-warm"预留地基（反向索引 + 失效 API）。**不含**修改数量阈值判断、自动触发 reload 的业务逻辑——那部分留给后续任务。

## 背景

M53 任务 A 把 `PageLogicAvailabilityCache` 从只缓存 `key_model_availability` 扩展到同时缓存 `prerequisites` 和 `path_summary`，真实项目 compact 场景 `warmed_query_dispatch` p50 从 639ms 降到 81ms（详见 `docs/milestones/performance/m53-performance-continuation.md` §3）。

这次修复引入两个可扩展性代价（已用代码 + profile 数据验证，非猜测）：

1. **O(P²) clone**：`GraphRuntime::warm_page_logic_availability`（`src/runtime.rs:342`）每次 warm 一个页面都要 `read_model.page_logic_availability.clone()` 深拷贝已经 warm 过的所有页面缓存，再插入新条目。缓存条目现在携带完整 `PagePrerequisites` + `PageLogicPaths`（数千条 `serde_json::Value`），使这个模式的代价从"小"变成"显著"。
2. **内存不随 budget 收缩**：`build_page_logic_paths`（`src/query/page_logic/path_summary.rs`）不接收 `budget` 参数，永远计算全量 `candidate_paths`/`related_context` 等字段；真正的裁剪（`take(3)`/`take(10)`）发生在输出组装阶段，warm cache 里常驻的是裁剪前的全量数据。实测：会员已注册页 compact 输出仅 110KB，但 cache 里常驻 `candidate_paths`(6706) + `related_context`(5858) ≈ 12,500 条 JSON 值，跟 full 预算量级相当。

用户确认下一步会新增"监测远程元数据修改数量，达到阈值后触发相关页面 reload"的逻辑（**runtime page diff 触发**），且预期场景是 many-pages（几十到上百个页面级别）warm，而不是当前测试用的 1-3 个热点页面。在这个量级下，O(P²) clone 是真实瓶颈，不是理论问题。

## 目标

1. **T1**：消除 warm 单页面时的 O(P²) clone 代价。
2. **T2**：warm 阶段按 `budget` 裁剪 `prerequisites`/`path_summary` 的物化范围，让 compact 预算的内存占用显著小于 full/normal。
3. **T3**：新增 `node_id -> page_id` 反向索引 + `invalidate_pages_for_dirty_nodes` API，供未来"page diff 触发"逻辑消费；本次只交付索引构建和失效接口，不交付触发判断本身。

## 非目标（本次不做）

- 修改数量阈值判断逻辑。
- 自动触发 `reload`/`ReloadGraph` 的业务编排。
- 远程元数据修改数量监测（浏览器侧/RemoteMetadataProvider 相关）。
- 改变任何默认查询输出的公开 JSON 契约。
- 批量并发 warm（多线程同时 warm 多个页面）——本次只解决单线程顺序 warm 的 O(P²) 问题。

## 架构

### T1 — 消除 O(P²) clone

现状（`src/runtime.rs:342-351`）：

```rust
let mut page_logic_availability = read_model.page_logic_availability.clone();
page_logic_availability.insert(key, cache);
self.read_model = Some(Arc::new(RuntimeReadModel { .. , page_logic_availability }));
```

**方案**：先验证 `self.read_model` 在 warm 调用期间是否有其他 `Arc` 持有者存活（检查所有 `self.read_model.clone()` / `Arc::clone(&self.read_model)` 调用点，确认查询路径是否会跨越 warm 调用长期持有旧快照）。

- 若确认无并发持有者（预期情况，`GraphRuntime` 是单线程顺序调用）：改用 `Arc::make_mut(&mut self.read_model)` 原地修改 `page_logic_availability` 字段，`Arc` 引用计数为 1 时零拷贝退化。
- 若发现有历史/未来场景需要持有旧快照做只读比较（比如未来 diff 逻辑本身要拿"旧 read model" 和"新 read model" 对比），改用 `Arc<RwLock<HashMap<String, PageLogicAvailabilityCache>>>` 包裹 `page_logic_availability` 字段本身（而不是整个 `RuntimeReadModel`），只在这一层做细粒度可变，其余字段（`dense_graph`、`availability_facts`）保持只读共享不变。

同时新增批量 API：

```rust
pub fn warm_page_logic_batch(&mut self, targets: &[(String, String)]) -> Result<BatchWarmReport>
```

一次调用内构建所有目标缓存后再统一写入（无论选哪种底层方案，都应保证批量调用只有 O(1) 次结构性写入，不是 O(P) 次）。`BatchWarmReport` 至少包含每个 page 的 warm 耗时和成功/失败状态（复用现有 `Result` 错误处理风格，单个页面失败不应导致整批失败——记录到报告里，继续处理剩余页面）。

### T2 — budget-aware 物化裁剪

- `build_page_logic_paths` 新增 `budget: &str` 参数；`collect_page_prerequisites` 同理。
- 裁剪规则**复用现有输出阶段已经在用的常量**（`key_model_availability_limit = if is_compact { Some(3) } else { None }` 之外，还要找到 `take(3)`/`take(10)` 对应的裁剪意图并前移），不引入新的裁剪阈值语义。
- **安全性论证**：cache key 已经是 `page_id + budget`，同一页面不同 budget 是独立缓存条目，某个 budget 裁剪掉的数据不会被其他 budget 的查询误读——因为它们各自建各自的缓存。
- **等价性要求**：裁剪后，compact 预算走 warm cache 命中路径的输出，必须与不走 cache、走当前非 warm 路径（现有裁剪在输出阶段生效）的输出**字节级一致**。这是本次最容易出错的地方，需要专门的等价测试覆盖 compact + normal + full 三种 budget。

### T3 — Dirty-node 反向索引 + 失效 API

新增模块（建议 `src/query/page_logic/page_diff.rs`，或复用 `dependency.rs` 现有反向追溯能力，实现前需确认是否已有可复用的反向遍历函数，避免重复实现）：

```rust
/// 从 node_id 反查其归属/引用它的所有 page_id。
/// 具体覆盖哪些 NodeType / EdgeType 组合，必须对齐
/// `graph_collect::collect_page_logic_nodes` 的实际遍历口径（见下方"未决问题"），
/// 不要独立臆造一套遍历规则。
pub struct PageDependencyIndex {
    node_to_pages: HashMap<String, HashSet<String>>,
}

impl PageDependencyIndex {
    pub fn build(graph: &dyn GraphReadStore) -> Result<Self>;
    pub fn affected_pages(&self, dirty_node_ids: &[String]) -> HashSet<String>;
}
```

`GraphRuntime` 新增：

```rust
/// 根据 dirty node 列表，从 warm cache 中摘除受影响页面的缓存条目，
/// 返回被摘除（需要重新 warm）的 page_id 列表。不自动重新 warm，不判断是否应该触发。
pub fn invalidate_pages_for_dirty_nodes(&mut self, dirty_node_ids: &[String]) -> Result<Vec<String>>
```

- 索引构建时机：跟 `MaterializedAvailabilityFactsIndex`/`DenseGraphSnapshot` 一样，在 `RuntimeReadModel` 构建时生成一次（LongLived 模式），存进 `RuntimeReadModel`。
- 索引口径必须与 `graph_collect::collect_page_logic_nodes` 使用的遍历规则**保持一致**——即"page logic 输出会引用到的节点"和"反向索引覆盖的节点"必须是同一个集合，否则会出现"节点变了但索引没覆盖，导致 stale cache 未被摘除"的正确性问题。这是本次唯一涉及正确性（不只是性能）的部分，需要专项测试。

## 验收标准

| 项 | 标准 |
|----|------|
| T1 | 新增测试证明连续 warm N（N≥20，模拟 many-pages）个页面时，总 clone/写入次数为 O(N) 而非 O(N²)（可以用 counter 断言，或者用页面数递增的 profile 耗时曲线证明接近线性而非二次） |
| T1 | `warm_page_logic_batch` 与循环调用单页 `warm_page_logic_availability` 产出的缓存内容等价 |
| T2 | compact/normal/full 三种 budget 的等价测试：warm-cache-hit 输出 == 非 warm 路径输出（字节级） |
| T2 | 真实项目 profile 证明 compact 预算下 warm cache 内存足迹（可用缓存内 `candidate_paths`/`related_context`/prerequisites 条目数作为代理指标）显著小于 full/normal |
| T3 | 反向索引覆盖率测试：对比索引口径与 `collect_page_logic_nodes` 遍历口径一致 |
| T3 | `invalidate_pages_for_dirty_nodes` 单元测试：dirty 一个被多个页面引用的公共节点（如共享 Model），验证所有相关页面都被正确摘除，无关页面不受影响 |
| 全局 | 不改变任何默认查询输出的公开 JSON 契约；`cargo test --features cli-local` 影响面测试全过；`cargo check` 无新增警告 |

## 依赖

- M53 任务 A 已完成（`docs/milestones/performance/m53-performance-continuation.md` §3，commit `00e8d10`）。
- `src/graph_redb.rs` 的 `dirty_nodes: HashSet<String>` 概念可作为 T3 的输入形态参考（不要求直接复用该类型，只要求语义对齐：都是"文件级 mtime+size+hash 增量检测出的变更节点集合"）。

## 未决问题（需要在实现前进一步确认，如有必要在 spec 审阅时一并确认）

1. T1 的 `Arc::make_mut` 方案是否可行，取决于当前是否存在跨 warm 调用持有旧 `Arc<RuntimeReadModel>` 快照的场景——需要在实现开始时先用一个只读排查步骤确认，若发现有隐藏的并发持有者，直接切到 `RwLock` 方案，不要在 `make_mut` 上反复试错。
2. T3 的反向索引口径对齐 `collect_page_logic_nodes`——若该函数逻辑复杂（涉及递归/多跳遍历），需要复用其内部逻辑而不是重新实现一遍相似但不完全一致的遍历，避免两套口径长期漂移。
