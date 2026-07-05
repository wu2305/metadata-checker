# M39：GraphStore 与 Indexer 分层

| milestone | M39 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

#### M39：GraphStore 与 Indexer 分层

目标：把图查询、图写入、索引状态提交拆开，让 query 不再绑定 redb，让 indexer 的阶段可测试，同时保持现有 CLI / stdio / schema 行为不变。M39 不做并发优化，不实现 MCP、远程、session 或 WASM 正式后端。

设计边界：

- `GraphReadStore` 只表达低语义图读取能力，不包含 reader/writer/dataflow 这类业务 helper。
- `GraphReadStore` 必须提供节点遍历原语，避免 `query::find_nodes` 继续泄漏 `GraphDB` 内部索引。
- `find_candidates` 属于 query/diagnostic 层策略，不进入 `GraphReadStore`。
- `GraphWriteStore` 与 `GraphReadStore` 真正分离，写接口接收完整 `Node` / `Edge`，方便后续自定义新增节点和边。
- 需要同时读写的索引流程使用组合边界（例如 `GraphIndexStore` 或函数泛型 `GraphReadStore + GraphWriteStore + IndexStateStore`），不要通过 `GraphWriteStore: GraphReadStore` 伪装分离。
- `IndexStateStore` 负责 file states 与一次索引结果提交，`persist_index` 放在 index 层，并明确提交当前 dirty graph 与 file states 的一致性边界。
- `DocumentProvider` 只负责读取文件内容与 metadata，不负责目录遍历；文件发现由 M39 独立的本地 `ProjectFileDiscoverer` / `discover_files` 完成。
- `Arc` / `Mutex` 不进入 trait 定义；未来 runtime/session 可在外层用 `Arc<dyn GraphReadStore + Send + Sync>` 或 `Arc<Mutex<dyn GraphWriteStore + Send>>` 包装。
- `MemoryGraphStore` 在 M39 仅作为 test-only 替身和调试雏形，不暴露 CLI / stdio / runtime tool。
- `GraphStoreError` 提供固定枚举和稳定 `code()`，保持错误信息来源统一；具体映射到 `ToolError` / diagnostics 由 runtime/adapter 统一处理。

任务清单：

- [x] M39.1：新增 `src/graph_store.rs`
  - 定义：
    - `GraphStoreResult<T> = Result<T, GraphStoreError>`
    - `GraphStoreError`
    - `GraphEdgeView`
    - `GraphNeighbors`
  - `GraphStoreError` 至少覆盖：
    - `NotFound`
    - `InvalidArgument`
    - `OpenFailed`
    - `ReadFailed`
    - `WriteFailed`
    - `SerializeFailed`
    - `DeserializeFailed`
    - `LockTimeout`
    - `PermissionDenied`
    - `Corrupted`
    - `UnsupportedOperation`
  - 要求：
    - `GraphStoreError` 实现 `Display`。
    - `GraphStoreError::code()` 是内部图存储错误的稳定 code 来源。
    - `GraphEdgeView` 使用 owned `Node` / `Edge`，避免 trait 生命周期复杂化。
    - 不直接复用 `ToolErrorCode`，但预留统一映射函数。

- [x] M39.2：定义读写分离 trait（冷脸验收修正）
  - `GraphReadStore`：
    - `get_node(&self, node_id: &str) -> GraphStoreResult<Option<Node>>`
    - `get_node_edges(&self, node_id: &str) -> GraphStoreResult<Option<GraphNeighbors>>`
    - `iter_nodes(&self) -> GraphStoreResult<Box<dyn Iterator<Item = Node> + '_>>`
    - `node_count(&self) -> GraphStoreResult<usize>`
    - `edge_count(&self) -> GraphStoreResult<usize>`
  - `GraphWriteStore`：
    - `upsert_node(&mut self, node: Node) -> GraphStoreResult<()>`
    - `add_edge(&mut self, edge: Edge) -> GraphStoreResult<()>`
    - `remove_nodes_by_ids(&mut self, node_ids: &[String]) -> GraphStoreResult<()>`
  - `IndexStateStore`：
    - `load_file_states(&self) -> GraphStoreResult<HashMap<String, FileState>>`
    - `persist_index(&mut self, commit: IndexCommit) -> GraphStoreResult<IndexReport>`
  - `IndexCommit`：
    - `file_states: HashMap<String, FileState>`
    - `summary` / `report` 所需的 dirty / deleted / unchanged 计数
    - 语义：提交调用时 store 内存中的 dirty graph 与 commit 中的 file states 必须一起落盘或一起失败。
  - 要求：
    - trait 不包含 `Arc` / `Mutex`。
    - read trait 不包含写入、事务、持久化、file state。
    - write trait 接收完整 `Node` / `Edge`，不要固化当前 `GraphDB::add_node` 的长参数形态。
    - `find_candidates` 不在 trait 内；改由 query/diagnostic helper 基于 `iter_nodes` 实现。
    - `GraphWriteStore` 不继承 `GraphReadStore`；需要组合能力时在调用处显式写出组合约束。

- [x] M39.3：让现有 `GraphDB` 实现 trait（冷脸验收修正：按 M39.2 新边界复核）
  - 实现：
    - `GraphReadStore for GraphDB`
    - `GraphWriteStore for GraphDB`
    - `IndexStateStore for GraphDB`
  - 要求：
    - 不改 redb 文件格式。
    - 不改 petgraph 内部结构。
    - 不删除现有 `GraphDB` public 方法。
    - trait 方法内部可调用现有方法。
    - `GraphReadStore::iter_nodes` 返回 owned `Node` 迭代，不能暴露 `node_indices` / `petgraph::NodeIndex`。
    - `IndexStateStore::persist_index` 调用现有 `GraphDB::persist` 或等价逻辑，但必须明确当前 dirty graph 与 file states 的提交一致性。

- [x] M39.4：迁移 query 层到 `GraphReadStore`（已完成：query/model/dataflow/context/page_logic/path/model_scope/explain-condition/full explain 只读入口均可接收 `&dyn GraphReadStore`）
  - 必须迁移：
    - `query::build_query_page_output`
    - `query::query_page`
    - `query::build_query_cross_output`
    - `query::query_cross`
    - `query::find_nodes`
    - `context::build_context_output`
    - `explain::build_explain_output`
  - 继续迁移：
    - `query::model::build_query_model_output`
    - `query::model::query_model`
    - `query::dataflow::build_query_dataflow_output`
    - `query::dataflow::query_dataflow`
    - `model_scope` 中只读图访问函数
    - `path` 中只读图访问函数
  - 最终迁移：
    - `query_page_logic`
    - `explain_condition`
    - `answer_facts`
    - `condition_facts`
    - `page_logic` 子模块
  - candidates 迁移：
    - 新增 query/diagnostic 层 `find_candidates(graph: &dyn GraphReadStore, target_id, limit)`。
    - 所有原 `graph.find_candidates(...)` 调用改为 query/helper 调用。
    - helper 只能依赖 `iter_nodes`，不得访问 `GraphDB` 内部索引。
  - 要求：
    - 新增查询入口不得继续要求 `&GraphDB`。
    - 迁移后使用 `&dyn GraphReadStore` 或泛型 `G: GraphReadStore`。
    - 不改变 AI 输出 schema。

- [x] M39.5：业务 helper 从 GraphDB 下沉到 query/domain 层（find_cross_relations、find_readers 等已提取为模块级函数，candidates 已外移）
  - 从 `GraphDB` 调用路径迁出：
    - `find_readers`
    - `find_writers`
    - `find_cross_relations`
    - `find_dataflow_inputs`
    - `find_dataflow_outputs`
    - `find_produced_by`
    - `find_consumed_by_dataflows`
    - `find_upstream_dependencies`
    - `find_downstream_outputs`
    - `find_candidates`
  - 要求：
    - 新 helper 接收 `&dyn GraphReadStore`。
    - `GraphDB` 旧方法可暂时保留兼容，但新 query 代码不再依赖旧方法。
    - reader/writer/dataflow/candidate 语义集中在 query/domain 层，不下沉到 storage trait。

- [x] M39.6：新增 test-only `MemoryGraphStore`
  - 位置建议：
    - `tests/common/memory_graph_store.rs`
    - 或 `src/graph_store.rs` 内 `#[cfg(test)]`
  - 能力：
    - 手工构造节点和边。
    - 实现 `GraphReadStore`。
    - 可选实现 `GraphWriteStore`，方便测试写入。
  - 用途：
    - 验证 query 代码依赖 `GraphReadStore`，不是 `GraphDB` / redb。
    - 测试 `query_page`、`query_cross`、`context`、candidates、缺节点、孤立边、循环边界。
  - 禁止：
    - 不新增 `--memory-graph`。
    - 不作为正式 runtime backend。
    - 不暴露给 stdio / function calling。

- [x] M39.7：拆 `ProjectIndexer`（冷脸验收修正后完成）
  - 冷脸验收修复：`parse_dirty_files` 只解析并返回 `ParsedGraphUpdate`，`apply_graph_updates` 只接收 `GraphWriteStore` 并更新内存图，`persist_index` 保持唯一 file states 提交边界。
  - 新增位置建议：
    - `src/indexer.rs`
    - 或 `src/scanner/indexer.rs`
  - 阶段：
    - `discover_files`
    - `diff_file_states`
    - `parse_dirty_files`
    - `apply_graph_updates`
    - `persist_index`
  - 建议结构：
    - `ProjectIndexer<P: DocumentProvider>`
    - `DiscoveredFile`
    - `DirtyFile`
    - `IndexPlan`
    - `IndexReport`
    - `ProjectFileDiscoverer` 或等价本地文件发现函数
  - 要求：
    - `scan_project(project_dir, db_path)` 继续作为对外入口。
    - `scan_project` 内部改为调用 `ProjectIndexer`。
    - `ProjectIndexer` 不应要求 `DocumentProvider` 负责目录遍历。
    - `discover_files` 只做本地项目目录发现；远程/session 文件发现留给后续里程碑。
    - `parse_dirty_files` 使用 `DocumentProvider` 读取已发现文件。
    - `apply_graph_updates` 只修改内存中的 `GraphWriteStore`，不落盘。
    - `persist_index` 是唯一落盘提交点，提交 dirty graph 与 file states。
    - 不改变现有增量行为。
    - 不引入远程 provider。
    - 不引入 session。

- [x] M39.8：错误转换统一（冷脸验收修正：补 code 映射反向测试）
  - 实现：
    - `GraphStoreError::code()`
    - `GraphStoreError` 到 `ToolError` / diagnostics 的统一映射函数。
  - 要求：
    - runtime / stdio / CLI 不再各自猜测 graph store 错误字符串。
    - `ToolErrorCode` 保持不动。
    - redb lock、permission、corruption、serde 错误应尽量映射到稳定 `GraphStoreError`。
    - 必须有反向测试覆盖 lock、permission、corruption、serialize、deserialize、read、write 的 code 映射。

- [x] M39.9：补测试覆盖（冷脸验收修正后完成）
  - 必须新增：
    - `test_graphdb_implements_graph_read_store`
    - `test_graphdb_implements_graph_write_store`
    - `test_graphdb_index_state_store_load_persist`
    - `test_graph_read_store_iter_nodes_supports_find_nodes`
    - `test_memory_graph_store_query_page`
    - `test_memory_graph_store_query_cross`
    - `test_memory_graph_store_candidates`
    - `test_query_functions_do_not_require_graphdb`
    - `test_find_candidates_lives_outside_graph_store`
    - `test_project_indexer_discovers_spg_tbl`
    - `test_project_indexer_diff_detects_dirty_deleted_unchanged`
    - `test_project_indexer_does_not_require_document_provider_for_discovery`
    - `test_persist_index_commits_graph_and_file_states_together`
    - `test_scan_project_behavior_unchanged`
    - `test_query_model_graph_store_schema_parity`
    - `test_query_dataflow_graph_store_schema_parity`
    - `test_query_page_logic_graph_store_schema_parity`
    - `test_explain_condition_graph_store_schema_parity`
    - `test_explain_graph_store_schema_parity`
    - `test_context_graph_store_schema_parity`
    - `test_graph_store_error_code_lock_timeout`
    - `test_graph_store_error_code_permission_denied`
    - `test_graph_store_error_code_corrupted`
    - `test_graph_store_error_code_serde_and_read_write`

- [x] M39.10：文档同步
  - 更新：
    - `docs/real-project-optimization-roadmap.md`
    - `docs/schema.md` 中 M36/M39 边界说明
  - 要求：
    - 不改 function-calling / stdio 工具协议。
    - 不改 AI 输出 schema。
    - 明确 M39 不做 MCP、远程、session、WASM 正式实现。

不做：

- 不处理并发优化。
- 不把 `Arc` / `Mutex` 写进 trait。
- 不实现 MCP server。
- 不实现远程拉取。
- 不实现 session 目录。
- 不实现 IndexedDB / WASM 正式 backend。
- 不改 graphdb 文件格式。
- 不改查询输出 schema。
- 不新增用户可见 runtime tool。

#### 冷脸验收遗留项（已修复）

- 阻塞项 1：M39.7 未形成完整分层闭环。
  - 修复结果：`ProjectIndexer` 已形成 `discover_files -> diff_file_states -> parse_dirty_files -> apply_graph_updates -> persist_index` 阶段链路；`apply_graph_updates` 已降为 `GraphWriteStore`，并新增 write-only store 测试证明该阶段不要求读接口或 IndexState。

- 阻塞项 2：M39.9 的 schema parity 测试未按 roadmap 明确项完整落地。
  - 修复结果：已新增 `m39_graph_store_schema_parity_tests`，覆盖 `query_model`、`query_dataflow`、`query_page_logic`、`context`、`explain_condition`、`explain` 的 GraphReadStore 顶层输出 contract。

验收标准：

- `GraphReadStore` / `GraphWriteStore` / `IndexStateStore` 分离明确。
- 查询层不再直接绑定 redb。
- 写入和 index persist 不混入 read trait。
- `MemoryGraphStore` 证明查询可脱离 redb。
- `scan_project` 外部行为不变。
- 业务 helper 不进入 storage trait。
- 固定错误枚举成为 graph store 错误 code 来源。
- `cargo test` 全量通过。
- 真实项目 build-graph / query 回归通过或明确记录未跑原因。
