# 知识库验收问题集（约 20 题）

> 用途：知识库构建完成后，由**空上下文 agent** 逐题查询并回答，人工核验答案。
> 判定标准：能找到正确源码位置、能识别限制、无证据时**停止推断**（答「未知/未在知识库中」）即为通过。
> 反误导题（标记 ⚠）答错即为**不通过**，必须修知识条目后重测。

## 使用方法

1. 知识库构建完成后，用空上下文 agent（无本仓库文件访问权限，只能检索知识库）逐题提问。
2. 每题记录：召回的 top chunk（来自哪个文件）、agent 回答、是否正确、是否需要修条目。
3. 「答错」分两类：**检索失败**（召回不到应有条目）与**条目误导**（条目本身把计划写成实现）。
4. 结果回填本文档下表，作为是否扩展到更多主题的依据。

## A. 调用链类（主题一）

| # | 问题 | 期望答案要点 | 结果 |
|---|------|--------------|------|
| A1 | `--relations page:app/a.spg` 从 CLI 参数到图查询经过哪些函数？ | `main` → `run_query_commands` → `run_surface` →（裸前缀/裸名处理）→ `route::route` → `run_cli_runtime_tool` → `GraphRuntime::query` | |
| A2 | stdio server 里 `diff_refresh` 命令在哪处理？走到 runtime 会怎样？ | `handle_diff_refresh`（`src/stdio_server.rs:378`）拦截；未绑定 context 时 `DIFF_REFRESH_CONTEXT_REQUIRED`；若到 `GraphRuntime::query` 一律报错（`src/runtime.rs:1099`） | |
| A3 | `--find` / `--explain` / `--relations` 三个动词最终汇合到哪个函数？ | CLI 与 stdio 都汇合到 `GraphRuntime::query`（`src/runtime.rs:971`）；差异只在 adapter 与输出封装 | |
| A4 | stdio 模式下哪个命令不经过 `GraphRuntime::query`？ | `Status`（`src/stdio_server.rs:385`）与 `ReloadGraph`（`:394`）；`DiffRefresh`（`:378`） | |
| A5 | `check_reload: true` 时 reload 失败会中断本次查询吗？ | 不会。只记 `GRAPH_RELOAD_FAILED` diagnostic（`src/stdio_server.rs:425-436`） | |
| A6 | graphdb 打不开时 CLI 与 stdio 的行为差异？ | CLI：输出 `GraphDB::check_graph_db` 诊断 JSON 后退出（`src/main.rs:2002`）；stdio：启动即失败 | |

## B. 扫描/持久化类（主题二）

| # | 问题 | 期望答案要点 | 结果 |
|---|------|--------------|------|
| B1 | `--build-graph` 的入口和六个阶段是什么？ | `scanner::scan_project_with_report`（`src/scanner/mod.rs:34`）→ `ProjectIndexer::scan_with_diagnostics`（`src/scanner/indexer.rs:607`）；发现/差分/解析/应用/持久化 | |
| B2 | 增量判断依据是什么？mtime 还是内容 hash？ | **内容 hash（xxhash64）**，`diff_file_states`（`src/scanner/indexer.rs:351`）；`FileState` 也存 mtime/size 但不用于脏判断 | |
| B3 | 坏 TBL 与坏 SPG 的失败行为有何不同？ | TBL → `ParseFailure` + `SCANNER_FILE_PARSE_FAILED`，旧图保留、下轮重试；SPG → 整轮索引失败（响亮失败） | |
| B4 | 首次构建与后续增量的持久化路径差别？ | `prev_states.is_empty()` 时 `delta=None` 走 full rebuild 并置 v2 `Current`；否则 delta 路径，v2 置 `Stale`（`src/graph_redb.rs:1160-1197`） | |
| B5 | v2 shadow 什么时候从 Stale 回到 Current？ | 累计受影响节点数 ≥ 1024（`V2_STALE_REBUILD_THRESHOLD_NODES`，`src/graph_redb.rs:87`）时同事务重建 | |
| B6 | scanner 诊断存在哪里、如何保证与图一致？ | per-file 计数序列化后随 `IndexCommit.scanner_entries` 与图**同事务**落库；报告出口按全库口径合并 | |
| B7 | `prepare` 与 `scan` 的区别？诊断谁负责落库？ | `prepare` 不调 `persist_index`；其 `commit.scanner_entries` 被置空，诊断经 `PreparedIndexUpdate` 透传，**调用方**负责挂载后落库 | |

## C. 反误导类（⚠ 答错即不通过）

| # | 问题 | 期望答案要点 | 结果 |
|---|------|--------------|------|
| ⚠C1 | Grafeo 已经上线了吗？ | **没有。** 当前默认后端仍是 redb + v2 shadow；Grafeo 只有已批准设计与计划，**无实现代码** | |
| ⚠C2 | B5 差分测试通过是否代表增量索引正确？ | **不代表。** `tests/m59_b5_full_vs_incremental_diff_tests.rs` 的两条缺陷测试是如实钉住当前分歧（丢跨文件边、占位节点残留），A3 修复后会变红 | |
| ⚠C3 | M59 真实语料测量（501 SPG / 828 TBL / 3.72 GiB）能否作为 Grafeo 验收？ | **不能。** 那是**迁移前**测量（pin `c3c0528`），不是 Grafeo 验收 | |
| ⚠C4 | 增量更新会不会丢边？ | **会。** `cross_file_edge_is_lost_by_incremental_update` 已钉住跨文件边丢失 | |
| ⚠C5 | 占位页节点在引用者删除后会被清理吗？ | **不会，会永久泄漏。** `src/scanner/spg.rs:1069-1076` 只 `add_node` 不 `node_ids.insert` | |
| ⚠C6 | `--query-page-logic --human` 与 non-human 走同一条代码路径吗？ | **不是。** human 模式仍是旧路径（`src/main.rs:1200` 注释），待统一 human 渲染器 | |
| ⚠C7 | M28 stdio 契约收敛做好了吗？ | **没有。** INDEX 中仍为 `planned` | |

## D. 边界/停止推断类

| # | 问题 | 期望答案要点 | 结果 |
|---|------|--------------|------|
| D1 | browser / WASM 的持久化怎么实现？ | 知识库中**无**此条目（`src/persistence/indexeddb.rs` 不在两篇主题范围内）；正确回答是「未知」 | |
| D2 | 查询内部的 21 类边投影有哪些？ | 知识库中**无**此条目；正确回答是「未知，需读源码」 | |
| D3 | 本地 `/Users/...` 路径下的真实项目可以直接用吗？ | 知识库中**无**授权资料清单；正确回答是「未知/未在知识库中」，不得假设路径可用 | |

## 结果记录表

| 轮次 | 日期 | 构建 SHA | A 类 | B 类 | ⚠C 类 | D 类 | 结论 | 需修条目 |
|------|------|----------|------|------|-------|------|------|----------|
| 1（计划） | — | — | — | — | — | — | 待执行 | — |

## 扩展判定

- ⚠C 类**全部答对**且 D 类能正确停止推断 → 可考虑扩展主题（查询内部、diff-refresh 编排、browser/WASM）。
- ⚠C 类任一答错 → 先修知识条目，重测；**不得**通过增加主题数量掩盖。
- D 类被编造答案 → 说明条目边界声明不足，需在条目「边界与限制」中显式补「不在范围」清单。
