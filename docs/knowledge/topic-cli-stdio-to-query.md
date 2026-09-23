# 主题一：CLI / stdio 到查询的调用链

> 状态分层：**当前实现**（以下调用链均已存在于源码）
> 分析 SHA：`31d020417d1ecbea440b0c7e92dffa2bd4b42d4b`（PR33 head；squash-merge 进 `main` 后内容一致）
> 行号核验：本文所有 `src/xxx.rs:NNN` 均按上述 SHA 逐条核对（PR33 期间 `graph_redb.rs` 新增 2 行注释，
> 旧版 `55fdaa2` 的行号整体偏移 2 行，已全部刷新）
> 适用版本：`main` @ 上述 SHA；M59-1 起身份/schema 改造会改动本链路的 target 解析，但入口不变
> 维护责任：改 `cli.rs` / `main.rs` / `stdio_server.rs` / `runtime.rs` / `route.rs` 后必须更新本文件

## 0. 检索速查（自包含，放文首以保证落在首个分块内）

> **问：`--query-page-logic --human` 与 non-human 走同一条代码路径吗？**
> **答：不是，走两条不同的代码路径。** `--query-page-logic` 在 `run_query_commands`
> （`src/main.rs:1054`）内按 `args.is_human()` 分叉：human 走旧渲染函数
> `query::query_page_logic`（`src/main.rs:1200` 注释：待统一 human 渲染器后再迁到 runtime），
> non-human 走 `run_cli_runtime_tool` → `CliAdapter::parse_input` → `GraphRuntime::query`。
> 因此同一命令的 human / non-human **输出契约不统一**；该缺陷已确认未修。
> 修复条件是「统一 human 渲染器」落地，与 M28 stdio 契约收敛的归属关系**知识库未记录**。

> **问：`--find` / `--explain` / `--relations` 从 CLI 参数到图查询经过哪些函数？**
> **答：`main` → `run_query_commands`（`src/main.rs:1054`）→ `run_surface`（`src/main.rs:78`）
> → `execute_resolved_target`（`src/main.rs:263`）→ `route::route`（`src/route.rs:96`）
> → `run_cli_runtime_tool`（`src/main.rs:19`）→ `CliAdapter::parse_input`
> → `GraphRuntime::query`（`src/runtime.rs:971`）。**
> 上表之外还有一个反例：`--query-page-logic --human` 不进 runtime（见本节第一问）。

## 1. 当前实现：调用链

### 1.0 完整链路速查（自包含，供检索直接命中）

**问：`--find` / `--explain` / `--relations` 从 CLI 参数到图查询经过哪些函数？**
**答：`main` → `run_query_commands`（`src/main.rs:1054`）→ `run_surface`（`src/main.rs:78`）
→ `execute_resolved_target`（`src/main.rs:263`）→ `route::route`（`src/route.rs:96`）
→ `run_cli_runtime_tool`（`src/main.rs:19`）→ `CliAdapter::parse_input`
→ `GraphRuntime::query`（`src/runtime.rs:971`）。**

- **统一终点是 `GraphRuntime::query`（`src/runtime.rs:971`）**：`--find` / `--explain` /
  `--relations` 三个动词与 stdio 请求**都在此汇合**；差异只在 adapter 与输出封装。
- `run_query_commands`（`src/main.rs:1054`）在前：按固定顺序逐条 `if` 判断参数，命中即返回。
- `run_surface`（`src/main.rs:78`）居中：把三动词展开成多条 `RoutedCall`，再逐条调用 runtime；
  路由执行由 `execute_resolved_target` 完成，再经 `run_cli_runtime_tool` / `CliAdapter`
  落到 `GraphRuntime::query`。旧动词也可直接调用 `run_cli_runtime_tool`，它不是旧动词专用入口。
- **stdio handler 的三个例外**：`Status`（`src/stdio_server.rs:385`）、`ReloadGraph`（`:398`）、
  `DiffRefresh`（`:378`）分别在 `handle_request` 的 `if` 分支直接返回，不调用 `GraphRuntime::query`。
  这是 stdio 层的分派；直接调用 runtime 时的命令处理见 1.4。
- **`GraphRuntime::query` 之下还有旧 target 解析层**（M59-2）：`QueryModel`/`Explain`
  在 `build_query_model_output` / `build_explain_output` 内先经
  `resolve_legacy_model_target` 解析裸 `model:`/`field:` 再精确查表，见 1.8。

### 1.1 CLI one-shot 路径

```
main()                                  src/main.rs:1464
 ├─ Cli::parse()                        src/cli.rs:32      clap 派生，参数定义全在这里
 ├─ set_graph_lock_timeout_ms(...)      src/main.rs:1467   → src/graph_redb.rs:27
 ├─ telemetry::init(...)                src/main.rs:1467   --trace off|json|otlp
 ├─ （session 命令先短路 return）        src/main.rs:1484-1909
 ├─ tool_contract::validate_budget      src/main.rs:1843   compact|normal|full
 ├─ --serve-stdio → 走 1.2（不返回）     src/main.rs:1848
 ├─ graphdb-only 生命周期命令           src/main.rs:1911   status/reload/check-reload
 ├─ --project-dir 分支                   src/main.rs:1958
 │   ├─ --check-graph   → GraphDB::check_graph_db   src/graph_redb.rs:390
 │   ├─ --build-graph   → scanner::scan_project_with_report  src/scanner/mod.rs:34
 │   └─ 否则 load runtime → run_query_commands      src/main.rs:1054
 └─ 单文件 <FILE> 分支                   src/main.rs:2033   parser::parse_file + output
```

`run_query_commands`（`src/main.rs:1054`）逐条 `if` 判断参数，**命中即 `return Ok(true)`**：
`status` → `reload_graph` → `check_reload` → `query_model` → `query_page` → `query_cross`
→ `query_dataflow` → `query_page_logic` → `find` → `explain` → `relations`
→ `explain_condition` → `context` → …。顺序即优先级，同时传多个只有第一个生效。

### 1.2 两类 runtime 工具入口

| 入口 | 函数 | 说明 |
|------|------|------|
| 直接工具 | `run_cli_runtime_tool` `src/main.rs:19` | `CliAdapter` 解析 → `runtime.query(req)`，取 `.result` |
| 三动词表面 | `run_surface` `src/main.rs:78` | `--find` / `--explain` / `--relations` |

`run_surface` 与 `execute_resolved_target` 的展开顺序（`src/main.rs:78` / `:263`）：

1. 裸前缀（`--relations page:`）→ `enumerate_prefix_targets`，列为探查结果，不报 `TARGET_NOT_FOUND`（`src/main.rs:94-102`）。
2. 无类型前缀的裸名 → 先发一次 `Find`，用 `route::resolve_bare_target` 解析（`src/main.rs:104`，`src/route.rs:227`）。
3. `run_surface` 将已归一 target 交给 `execute_resolved_target`，其中调用 `route::route(surface, target, depth)`（`src/route.rs:96`）→ `RoutePlan`：`Explain` 展开为多条 `RoutedCall`（`src/route.rs:109`），`Relations` 展开为读写与关系（`src/route.rs:110`），`Find` 只有主调用（`src/route.rs:108`）。
4. 每条 `RoutedCall` 经 `run_cli_runtime_tool` → `CliAdapter::parse_input` →
   `GraphRuntime::query` 执行，再合并：主输出保留 `summary`/`details`/`evidence`/`diagnostics`，补充调用的 `details` 折进 `details.<merge_key>`（`src/main.rs:78` 顶部注释）。
5. `print_surface_result`（`src/main.rs:419`）决定 human / JSON 输出。

### 1.3 stdio server 路径

`handle_request`（`src/stdio_server.rs:361`）依次用 `if` 判断 `DiffRefresh`、`Status`、
`ReloadGraph`，分别交给 `handle_diff_refresh`、`runtime.status()`、`runtime.reload()` 后返回。
这三个 stdio 短路分支不进入 `GraphRuntime::query`，也不属于 runtime 内部的命令分派。
普通查询携带 `check_reload: true` 时，stdio 先调用 `reload_if_changed()`，失败只记诊断，
仍继续 `runtime.query(req)`；这个布尔选项不是短路命令。
构造 `req` 时保留 `command: invocation.command`，并设 `check_reload: false`
（`src/stdio_server.rs:451-459`）。因此普通查询仍执行原命令，不会因布尔选项变成
`ToolCommand::CheckReload`；runtime 的 CheckReload 分支要求命令本身就是 CheckReload（`src/runtime.rs:1028`）。

```
run_stdio_server                        src/stdio_server.rs:185
 ├─ GraphRuntime::load_with_project_dir_and_mode(..., LongLived)   src/runtime.rs:395
 │   产品路径强制 LongLived：否则不建 DenseGraph / Availability Facts / PageDependencyIndex
 ├─ 有 diff_refresh_context → RuntimeBinding::Bound(orchestrator)，否则 Plain
 └─ 逐行 stdin JSONL：
     空行跳过 / 读失败 → QUERY_FAILED / 反序列化失败 → INVALID_JSON（src/stdio_server.rs:233-274）
     handle_request                     src/stdio_server.rs:361
      ├─ StdioAdapter::parse_input      src/stdio_server.rs:102
      ├─ ToolRegistry::find_by_command
      ├─ DiffRefresh  → handle_diff_refresh（未绑定 context 时 DIFF_REFRESH_CONTEXT_REQUIRED）src/stdio_server.rs:378
      ├─ Status       → runtime.status() 直返，不经 query          src/stdio_server.rs:385
      ├─ ReloadGraph  → runtime.reload()                            src/stdio_server.rs:398
      ├─ check_reload → runtime.reload_if_changed() 前置检查        src/stdio_server.rs:425
      ├─ human 且 spec 不支持 → HUMAN_MODE_NOT_SUPPORTED 降级为 false  src/stdio_server.rs:437
      └─ runtime.query(req)             src/stdio_server.rs:462
```

### 1.4 统一汇合点：`GraphRuntime::query`

`src/runtime.rs:971`。CLI 与 stdio **在此汇合**，差异只在 adapter 与输出封装。

- **runtime 层**：请求已经进入 `GraphRuntime::query` 后，`ReloadGraph` / `CheckReload`
  在该函数主 `match` 之前处理（`src/runtime.rs:991`、`src/runtime.rs:1028`）；
  源码的“需要可变借用 self”注释只解释 runtime 内部这两个分支，不解释 stdio 的三个短路分支。
- `DiffRefresh` 到达此处**一律报错** `DIFF_REFRESH_CONTEXT_REQUIRED`（`src/runtime.rs:1099`）——它只能由 stdio handler 或 CLI one-shot 先行拦截。
- 其余命令分发到 `query` / `explain` / `answer_contract` 等 builder，结果经 `ResponseProcessor::runtime_response` 统一封装 timing 与 diagnostics。

### 1.5 读写什么状态

| 状态 | 位置 | 谁写 |
|------|------|------|
| 内存图 + redb 句柄 | `GraphRuntime.graph` `src/runtime.rs:158` | `load` / `reload` / `install_replacement` |
| 派生读模型（DenseGraph 等） | `GraphRuntime.read_model` | 仅 `LongLived` 模式构建 |
| 指纹（mtime/size/前 4096 字节 hash） | `GraphFingerprint` `src/runtime.rs:226` | `load`；`is_graph_changed` 据此判断（`src/runtime.rs:1361`） |
| `load_diagnostics` | 含 hydrate 诊断 + `SCANNER_*` 全库合并诊断 | `load_with_project_dir_internal` `src/runtime.rs:403` |
| 进程级锁超时 | `graph_lock_timeout_ms`（CLI `--graph-lock-timeout-ms`，默认 10000） | `src/cli.rs:91`、`src/graph_redb.rs:27` |

### 1.6 失败后怎样

| 场景 | 行为 |
|------|------|
| budget 非法 | `INVALID_BUDGET`，`src/tool_contract.rs:434` |
| target 缺/前缀错 | `MISSING_TARGET` / `INVALID_TARGET`，adapter 层返回 |
| 未知命令 | CLI：`UNKNOWN_COMMAND`；stdio 附 `Supported: <列表>` |
| `diff_refresh` 走 runtime | `DIFF_REFRESH_CONTEXT_REQUIRED`（协议错误） |
| reload 失败 | `GRAPH_RELOAD_FAILED`；stdio 保留旧图继续可用（见 `tests/stdio_server_tests.rs:902`） |
| graphdb 打不开 | CLI：`GraphDB::check_graph_db` 输出诊断 JSON 后退出（`src/main.rs:2002`）；stdio：启动即失败 |
| 拿不到锁 | `GRAPH_DB_LOCK_TIMEOUT`（`src/graph_store.rs:53`），超时由 `--graph-lock-timeout-ms` 控制 |
| human 不被支持 | `HUMAN_MODE_NOT_SUPPORTED`，降级为 non-human 而非报错（`src/stdio_server.rs:437`） |
| 裸 `model:` / `field:` target 匹配多个节点 | `AMBIGUOUS_TARGET` + `candidate_targets` + 可直接执行的 `next_queries`；不静默挑一个（见 1.8） |
| `check_reload` 失败 | 只记 diagnostic，不中断本次查询 |

### 1.7 改动时跑哪些测试

```bash
cargo test --features cli-local --test stdio_server_tests        # stdio 协议与错误码
cargo test --features cli-local --test m58_3_command_surface_tests  # 三动词表面（子进程调用真实二进制）
cargo test --features cli-local --test runtime_tests
cargo test --features cli-local --test core_feature_tests
```

`m58_3_command_surface_tests` 用 `env!("CARGO_BIN_EXE_metadata-checker")` 拉起真实二进制；
不能写死 `target/debug/...`，远端 CI 用自定义 `CARGO_TARGET_DIR`（`tests/m58_3_command_surface_tests.rs:14-20`）。

### 1.8 旧 target 解析（M59-2 A1 接线，2026-09-22 起已接线）

**问：页面局部身份启用后，裸 `model:orders` 这类旧 target 怎么解析？**
**答：先经 `resolve_legacy_model_target`（`src/query/model.rs`）显式解析，再交给精确
`get_node`。唯一局部命中改用该局部节点的真实 id 并如实回报 `query_target`；多个候选
（跨页同名、或局部与物理同名）交回全部候选 + `AMBIGUOUS_TARGET` 诊断 + 可执行的
`next_queries`，**不静默挑一个**；无候选维持 `TARGET_NOT_FOUND`。**

- 带 `|` 的 scoped id（`model:<PAGE>|<local>`）仍走精确查表，不参与裸名解析；
- kind 隔离由解析器保证：裸 `field:` 只匹配 Field 节点，裸 `model:` 只匹配 Model
  节点，跨 kind 同名返回 Missing；`cond`/`comp`/`action`/`param` 文法恒为 scoped，
  裸名解析对它们未定义；
- **六条入口共用同一套解析**（2026-09-22 复验补全后）：
  - `build_query_model_output`（runtime / runtime 工具）
  - `query_model`（human 分支，纵深防御——CLI human 模式实际落 `HUMAN_MODE_NOT_SUPPORTED`）
  - `build_explain_output`（`field:` 的生产入口）
  - `build_explain_condition_output_with_intent_and_retrieval`（`src/explain.rs`）
  - `build_context_output`（`src/context.rs`）
  - `build_query_dataflow_output`（`src/query/dataflow.rs`，`--relations model:X` 的补充调用）

  stdio 经 `GraphRuntime::query` 汇合，因此与 CLI 一致；**新增裸名查询入口必须调用同一
  resolver，不要直接 `get_node`**——否则会退回「静默命中物理模型」的旧行为。

  后三条是复验补上的：`--explain` 在 `route.rs:120-133` 展开成
  `Explain`（主）+ `ExplainCondition`（补充 `condition_facts`）+（显式 `--depth` 时）
  `Context`（补充 `neighbor_context`）；`--relations model:X` 展开成
  `QueryModel`（主）+ `QueryDataflow`（补充 `dataflow_subgraph`）。
  只接线主调用时，补充调用仍做精确 `get_node`：裸 `field:`/`model:` 会报
  `TARGET_NOT_FOUND`，而 supplement 的 `required=false` 让整条命令照样「成功」——
  **主调用有答案、补充块静默缺失，且无任何可见失败信号**。

  `QueryDataflow` 这条尤其要注意：它未接线时会**静默命中物理模型**并返回它的子图，
  于是同一个 target 上主调用报歧义、补充调用给出一个看起来正常的答案——比单纯缺块
  更糟，因为它把「静默挑一个」包装成了有效结果。改动 `route_explain` /
  `route_relations` 的展开列表时，必须同步检查每条展开调用是否都已接线。
- `execute_resolved_target` 在补充调用前会跳过「主调用已报定位失败」的调用，但
  **只跳过 `shares_legacy_target_resolution` 白名单内的命令**（`src/main.rs`）。
  新增接线 resolver 的命令要同步加入白名单；不在白名单里的命令照常执行，宁可多跑
  一次也不静默吞掉一个可能给出不同答案的补充块。
- CLI 路由层（`run_surface` → `normalize_target_against_graph`）先于 resolver 生效：
  精确 id 存在时判 `Exact` 直通（物理同名的歧义由 resolver 报出）；唯一局部命中多在
  路由层就 `Resolved`；≤3 候选的歧义由 `answer_ambiguous_target` 逐候选作答
  （`AMBIGUOUS_TARGET_ANSWERED`）。两种 code 均已在 `answer_effect` 登记。
- `AMBIGUOUS_TARGET` 的 severity 在 `severity_for`（`src/diagnostics.rs`）显式登记为
  `Error`，与同属寻址失败的 `TARGET_NOT_FOUND` 一致；未登记时会落默认 `Warning` 档，
  导致同一类失败在 diagnostics 里显得一轻一重。
- `TARGET_NOT_FOUND` 的补救建议由 **target 前缀**决定，不由发起查询的 `OutputKind`
  决定（`find_command_for_target`）：`model:`/`dataflow:` → `--find-model`，
  `page:` → `--find-page`，`comp:`/`action:` → `--find-component`，`field:` 退到
  所属模型名（字段没有独立 find 动词）。**关键词必须去掉前缀**：`find_nodes` 拿
  关键词匹配 `id`/`name`/`path` 的子串，而 id 形如 `model:app/a.spg|ordersView`，
  带前缀的串从不是任何 id 的子串。此前按 `OutputKind` 分支，`--context model:X`
  会给 `--find-page model:X`——类型错且关键词也搜不到。

## 2. 已批准计划（尚未实现）

- **M59-4**（plan `docs/plans/2026-09-06-m59-grafeo-implementation-plan.md`）：查询与投影改造，
  要求 `query` 不绑定 redb、21 类链路边各有精确投影测试、修复 M59-PATH（same_page 与物理字段识别）。
  本链路的**入口不变**，target 解析与投影输出会变。
- **M59-1**：页面局部身份、原始引用 token、统一路径。会改动 `route.rs` 的 target 归一化与 `normalize_prefixed_target`（`src/route.rs:356`）。M59-2 起旧 target 的裸名解析已在 `src/query/model.rs` + `src/explain.rs` 生产接线（见 1.8）。
- **M28 / M29 / M30**（INDEX 中仍为 `planned`）：stdio 请求/响应契约收敛、FC 工具层优化、性能与容量治理。

## 3. 已知缺陷（已确认，未修）

- **M59-PATH**：整条路径的 `same_page` 与真实物理字段来源判定不正确，交 M59-4 修复。影响 `explain` / `relations` 的输出正确性。
- **`--query-page-logic` 的 human 模式仍是旧路径**（`src/main.rs:1200` 注释：待统一 human 渲染器后再迁到 runtime）。
  **问：`--query-page-logic --human` 与 non-human 走同一条代码路径吗？答：不是，走两条不同代码路径。**
  原因：human 模式尚未迁移到 runtime，仍在旧渲染路径上；因此同一命令的 human / non-human **输出契约不统一**。
  该缺陷已确认未修，修复条件是「统一 human 渲染器」落地。
  **边界**：此处只记录该 CLI 命令的分叉事实；**本文不覆盖扫描/索引层的失败语义**
  （坏 TBL / 坏 SPG 的 ParseFailure 与整轮失败），那部分见
  [topic-scan-incremental-persist.md](topic-scan-incremental-persist.md) 的 1.2 / 1.6，
  两篇文档描述的是不同层，不存在冲突，也不要互相外推。
- **单文件分支与图查询分支不共用输出**（`src/main.rs:2033` 起）：给 `<FILE>` 时 `--explain` 走 `explain::explain_component_spg`，给 `--project-dir` 时走 runtime tool，两者行为不同。

## 4. 历史状态

- M24 引入 stdio server；M27 扩展查询命令面；M38 统一到 `GraphRuntime::query` 单一入口。
- M38 之前各命令有独立 handler，现仅 `Status` / `ReloadGraph` / `DiffRefresh` 在 `handle_request` 里保留短路分支（`src/stdio_server.rs:385/398/378`）。
- M28 契约收敛仍为 `planned`，**未做**；stdio 请求/响应契约以 `docs/reference/stdio-server.md` 现状为准。
- **问：M28 stdio 契约收敛做好了吗？答：没有，未做。** INDEX 中仍为 `planned`，
  这条是「历史状态 / 已批准计划」，不是已实现能力。
- **M28 与 `--query-page-logic` human 旧路径缺陷的关系：知识库中无该信息。**
  M28 的记载只有「stdio 请求/响应契约收敛」一句，没有列出它包含哪些具体缺陷，
  因此**不得**断言 human / non-human 双路径缺陷属于或不属于 M28，也不得断言它的修复条件
  与 M28 无关。判断这类归属须读 `docs/milestones/INDEX.md` 与对应 plan，不能从本文推断。

## 5. 边界与限制

- 本文只覆盖 **CLI / stdio 到 `GraphRuntime::query`**。查询内部的图遍历（21 类边、DataFlow 子图、条件抽取）**不在本文范围**，暂无知识条目。
- **不得跨层外推**：本文的「失败后怎样」（1.6）只列 CLI / stdio 与 runtime 的错误码，
  **不包含**扫描/索引层对坏 TBL / 坏 SPG 的处理，也**不声称**该层与 CLI/stdio 层对同类
  输入的行为一致或不一致。涉及扫描失败语义的问题应检索主题二文档；
  检索结果只有本文时，正确回答是「未知」，不是比较两篇文档是否冲突。
- **不得推断里程碑归属**：本文记载缺陷时只给修复条件；
  某缺陷是否属于 M28 / M59 等里程碑、与哪个计划相关，本文未记录时须答「未知」。
- 持久化与增量索引见 [topic-scan-incremental-persist.md](topic-scan-incremental-persist.md)。
- 本文不覆盖 browser / WASM 路径（`browser/`、`src/browser_wasm_bindgen.rs`）。
