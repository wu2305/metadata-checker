# 主题一：CLI / stdio 到查询的调用链

> 状态分层：**当前实现**（以下调用链均已存在于源码）
> 分析 SHA：`55fdaa2b11486f69224bfd7fa2944f2759a08940`（PR14 合入后的 main）
> 适用版本：`main` @ 上述 SHA；M59-1 起身份/schema 改造会改动本链路的 target 解析，但入口不变
> 维护责任：改 `cli.rs` / `main.rs` / `stdio_server.rs` / `runtime.rs` / `route.rs` 后必须更新本文件

## 1. 当前实现：调用链

### 1.1 CLI one-shot 路径

```
main()                                  src/main.rs:1463
 ├─ Cli::parse()                        src/cli.rs:31      clap 派生，参数定义全在这里
 ├─ set_graph_lock_timeout_ms(...)      src/main.rs:1465   → src/graph_redb.rs:27
 ├─ telemetry::init(...)                src/main.rs:1466   --trace off|json|otlp
 ├─ （session 命令先短路 return）        src/main.rs:1483-1841
 ├─ tool_contract::validate_budget      src/main.rs:1842   compact|normal|full
 ├─ --serve-stdio → 走 1.2（不返回）     src/main.rs:1847
 ├─ graphdb-only 生命周期命令           src/main.rs:1930   status/reload/check-reload
 ├─ --project-dir 分支                   src/main.rs:1957
 │   ├─ --check-graph   → GraphDB::check_graph_db   src/graph_redb.rs:388
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

`run_surface` 的展开顺序（`src/main.rs:78-260`）：

1. 裸前缀（`--relations page:`）→ `enumerate_prefix_targets`，列为探查结果，不报 `TARGET_NOT_FOUND`（`src/main.rs:94-102`）。
2. 无类型前缀的裸名 → 先发一次 `Find`，用 `route::resolve_bare_target` 解析（`src/main.rs:104`，`src/route.rs:227`）。
3. `route::route(surface, target, depth)`（`src/route.rs:96`）→ `RoutePlan`：`Explain` 展开为多条 `RoutedCall`（`src/route.rs:109`），`Relations` 展开为读写与关系（`src/route.rs:110`），`Find` 只有主调用（`src/route.rs:108`）。
4. 逐条执行后合并：主输出保留 `summary`/`details`/`evidence`/`diagnostics`，补充调用的 `details` 折进 `details.<merge_key>`（`src/main.rs:78` 顶部注释）。
5. `print_surface_result`（`src/main.rs:419`）决定 human / JSON 输出。

### 1.3 stdio server 路径

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
      ├─ ReloadGraph  → runtime.reload()                            src/stdio_server.rs:394
      ├─ check_reload → runtime.reload_if_changed() 前置检查        src/stdio_server.rs:425
      ├─ human 且 spec 不支持 → HUMAN_MODE_NOT_SUPPORTED 降级为 false  src/stdio_server.rs:437
      └─ runtime.query(req)             src/stdio_server.rs:462
```

### 1.4 统一汇合点：`GraphRuntime::query`

`src/runtime.rs:971`。CLI 与 stdio **在此汇合**，差异只在 adapter 与输出封装。

- `ReloadGraph` / `CheckReload` 在 `match` 之前单独处理（`src/runtime.rs:993`、`src/runtime.rs:1025`），因为它们需要 `&mut self`。
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

## 2. 已批准计划（尚未实现）

- **M59-4**（plan `docs/plans/2026-09-06-m59-grafeo-implementation-plan.md`）：查询与投影改造，
  要求 `query` 不绑定 redb、21 类链路边各有精确投影测试、修复 M59-PATH（same_page 与物理字段识别）。
  本链路的**入口不变**，target 解析与投影输出会变。
- **M59-1**：页面局部身份、原始引用 token、统一路径。会改动 `route.rs` 的 target 归一化与 `normalize_prefixed_target`（`src/route.rs:356`）。
- **M28 / M29 / M30**（INDEX 中仍为 `planned`）：stdio 请求/响应契约收敛、FC 工具层优化、性能与容量治理。

## 3. 已知缺陷（已确认，未修）

- **M59-PATH**：整条路径的 `same_page` 与真实物理字段来源判定不正确，交 M59-4 修复。影响 `explain` / `relations` 的输出正确性。
- **`--query-page-logic` 的 human 模式仍是旧路径**（`src/main.rs:1200` 注释：待统一 human 渲染器后再迁到 runtime）。同一命令 human / non-human 走两条不同代码路径，输出契约不统一。
- **单文件分支与图查询分支不共用输出**（`src/main.rs:2033` 起）：给 `<FILE>` 时 `--explain` 走 `explain::explain_component_spg`，给 `--project-dir` 时走 runtime tool，两者行为不同。

## 4. 历史状态

- M24 引入 stdio server；M27 扩展查询命令面；M38 统一到 `GraphRuntime::query` 单一入口。
- M38 之前各命令有独立 handler，现仅 `Status` / `ReloadGraph` / `DiffRefresh` 在 `handle_request` 里保留短路分支（`src/stdio_server.rs:385/394/378`）。
- M28 契约收敛仍为 `planned`，**未做**；stdio 请求/响应契约以 `docs/reference/stdio-server.md` 现状为准。

## 5. 边界与限制

- 本文只覆盖 **CLI / stdio 到 `GraphRuntime::query`**。查询内部的图遍历（21 类边、DataFlow 子图、条件抽取）**不在本文范围**，暂无知识条目。
- 持久化与增量索引见 [topic-scan-incremental-persist.md](topic-scan-incremental-persist.md)。
- 本文不覆盖 browser / WASM 路径（`browser/`、`src/browser_wasm_bindgen.rs`）。
