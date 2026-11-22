# 主题：`.grafeo` 图库的只读 GQL 直查（`--gql`）

> 分析 SHA：`c800341`（M59-3 C4）。SHA 与当前 HEAD 差距大时，以源码为准并回源码确认。

## 文首速查

- **问：能不能用 GQL 直接查 `.grafeo` 图库？** 答：能，`--graph-db-path x.grafeo --gql "<查询>"`；只对 `.grafeo` 图库有效，redb（`.graphdb`）图库会返回 `GQL_BACKEND_UNSUPPORTED`。
- **问：GQL 能写图吗？** 答：不能。写入、DDL、会话命令一律被拒（`GQL_READ_ONLY`），拒绝之后图原样不变。
- **问：GQL 能读本地文件吗？** 答：不能。`LOAD CSV` / `LOAD DATA` 被形态闸拒绝；引擎自己的只读会话**会放行**它，所以形态闸不可去掉。
- **问：要开 `lpg` 吗？** 答：不要。GQL 来自 `edge` 特性集已带上的 `gql`；`lpg` 只多出 Cypher / Gremlin / SQL-PGQ 解析器与完整 regex，C4 用不到。
- **图结构**：节点只有一个标签 `Node`，属性 `id / node_type / path / name / meta / origin_file`；边类型名就是 `EdgeType` 变体名（如 `Reads`、`Contains`、`Triggers`），属性 `field_path / meta / origin_file`。`meta` 是 JSON **文本**，不是 map。
- **问：LLM 写 GQL 前要知道什么？** 答：运行 `--graph-schema`（JSON；`--human` 为 Markdown）。它由 `src/graph_schema.rs` 的契约表生成，列出 id 格式、各 `node_type` 的 `meta` 键、每种边的端点组合与 `field_path` 含义、「边缺席何时不代表关系不存在」、解释规则与方言差异（多边类型写 `[:A|B]`，无 `=~`、无 `CONTAINS()` 函数形式、无 `NOT (n)--()` 模式谓词）；`--gql` 的 long help 是它的紧凑版。`tests/graph_schema_tests.rs` 在真实夹具图上逐元素核对契约，`gql_long_help_claims_hold_against_a_real_graph` 验方言说法。
- **问：哪些边类型在枚举里但扫描器从不写？** 答：`OpensPage`、`SetsParam`、`DataflowInternal`（契约表 `emitted = false`）；页面跳转实际是 `ActionNavigates`，参数赋值是 `ActionSetsParam`，参数传递是 `PassesParam`。

## 1. 当前实现

**入口与输出**（状态：已实现，`--gql` 是 CLI 一等参数）

- CLI：`--gql <QUERY>`，行数上限 `--gql-max-rows <N>`（默认 200）。`src/cli.rs:196`、`src/cli.rs:204`；分发在 `src/main.rs:1287`，`--gql` 也计入 `has_session_query_request`（`src/main.rs:71`）使其可只凭 `--graph-db-path` 运行。
- 成功输出 JSON：`{columns, rows, row_count, total_rows, truncated}`；`--human` 时输出 TSV（首行列名，截断时末尾追加一行 `# 已截断…`）。结果类型 `GqlRows` 在 `src/graph_store.rs:152`。
- 失败输出 `{"ok":false,"error":{"code","message"}}`（退出码 0，与 session 命令同信封）。错误码：`GQL_READ_ONLY`（语句被禁止）、`GQL_QUERY_INVALID`（语法/语义有误，消息带位置）、`GQL_QUERY_TIMEOUT`（引擎默认 30 秒超时）、`GQL_QUERY_FAILED`（其他）、`GQL_BACKEND_UNSUPPORTED`（非 `.grafeo` 图库）。`GqlError` 在 `src/graph_store.rs:121`。
- runtime 委托：`RuntimeGraphBackend::query_gql_read_only`，`src/runtime.rs:235`；redb 变体显式失败。
- 实现：`GrafeoGraphStore::query_gql_read_only`，`src/graph_grafeo.rs:301`。
- 结果超过 `max_rows` 时截断并置 `truncated: true`，`total_rows` 仍是截断前的真实行数——调用方应收窄查询，不要把截断表当全集。

**只读由两道闸共同保证，缺一不可**（状态：已实现，`tests/m59_c4_grafeo_gql_tests.rs` 覆盖）

1. **引擎闸**：`session_with_role(Role::ReadOnly)`（`src/graph_grafeo.rs:303`）。引擎在执行前检查计划，带写算子（INSERT / SET / REMOVE / DELETE / DETACH DELETE / MERGE / CREATE）就拒绝；测试对每种写入形态都断言了拒绝**且图指纹不变**。DDL（建索引、建图、DROP）以 `CREATE` / `DROP` 开头，在形态闸就被挡在引擎之前，没有单独的引擎闸测试。必须落在会话上，因为 runtime 用读写 `open()` 打开 `.grafeo`（要回放 WAL），文件级只读做不到。
2. **形态闸**：`check_gql_read_only_shape`，`src/graph_grafeo.rs:111`。起始关键字白名单 `MATCH / OPTIONAL / UNWIND / FOR / RETURN`（`src/graph_grafeo.rs:94`）；查询里出现 `LOAD` 一词（大小写不敏感，含反引号与字符串字面量内）就整条拒绝；文本上限 16 KiB（`src/graph_grafeo.rs:97`）。

不要把形态闸当成可选的纵深防御：引擎的只读会话把 `LOAD CSV FROM '<本地路径>' AS row RETURN row[0]` 当作读放行，实测能返回任意本地文本文件的内容。

**可查的图模式**（状态：与写入路径一致，`src/graph_grafeo.rs:41`–`src/graph_grafeo.rs:56`）

- 项目节点：`(:Node {id, node_type, path, name, meta, origin_file})`。`node_type` 是 `NodeType` 变体名（`Page / Component / Model / Field / Action / Condition`）。
- 索引侧状态：标签 `IndexState`（file state / scanner 诊断 / checkpoint）与项目节点同库。查项目图请始终写 `(n:Node)`，否则会扫到这些内部记录。
- 边：`(a:Node)-[r:Reads]->(b:Node)`，边类型名即 `EdgeType` 变体名，共 22 种。

示例：`MATCH (a:Node)-[r:Reads]->(b:Node) RETURN a.id, b.id, r.field_path`。

## 2. 已批准计划

- C4 原设计是手写「只读视图 + WHERE + 投影 + LIMIT + TSV/JSON」迷你语言；2026-10-01 用户决策改为直接开放引擎 GQL，不再自造查询语言（`docs/specs/2026-09-05-grafeo-backend-migration-design.md` 阶段 C）。
- 问：C4 还会做手写迷你语言吗？答：不会，已被取代。
- 问：`--gql` 已经进 stdio / MCP 工具契约了吗？答：没有。本轮只做 CLI；stdio 的工具表面被 M58 评测固定，加入新动词会改变模型可见的工具列表，需单独决定。

## 3. 已知缺陷

- **浮点字面量在聚合之后的投影里会被返回成 `Int64`（IEEE 位模式）**：grafeo 0.5.43 引擎缺陷，例如 `RETURN count(n) AS total, 1.5 AS ratio` 得到 `4609434218613702656`。本层不修，无法可靠识别。图属性全是字符串，浮点只会来自 LLM 自己写的字面量或 `avg()` 之类计算；单独 `RETURN 1.5` 正常。
- **重查询只靠引擎 30 秒超时兜底**：行数上限只在查询返回之后截断，不限制引擎内部物化。实测 178 节点语料上四路笛卡尔积（约 1e9 行）在 6 GB 地址空间上限下 30.8 秒后返回 `GQL_QUERY_TIMEOUT`，未 OOM；这不是对更大语料的保证。
- **形态闸保守误拒**：任何含 `load` 一词的查询（变量名、属性名、字符串字面量，如 `WHERE n.name = 'load'`）都会被拒；`download`、`load_time` 不受影响。这是有意取舍：判定边界与引擎词法不一致时宁可误拒，也不能漏放。
- `WITH` / `SELECT` 不能作为语句开头（引擎 GQL 语法本身不支持），`CALL` 刻意不放行。
- `--gql` 对 redb 图库不可用，且 `.grafeo` 图库的 `reload()` 仍显式拒绝（见 C3 小节）。

## 4. 历史状态

- C1 起 `Cargo.toml` 的 grafeo 特性集就是 `["edge", "storage"]`；`edge` 本身带 `gql`，所以 GQL 解析器一直在依赖里，只是此前没有任何代码调用它。「`lpg` 才有查询语言」的说法不成立：`lpg` 额外带来的是 Cypher / Gremlin / SQL-PGQ 与完整 regex。
- 体积变化与三组实测数据见 `docs/milestones/performance/m59-grafeo-backend-migration.md` 的 C4 小节。
