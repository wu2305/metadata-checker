# 知识库验收问题集（24 题）

> 本文件**不被知识库索引**（位于 `docs/knowledge-acceptance/`，见同目录 README）。
> 用途：知识库重建后，由**空上下文 agent** 逐题查询并回答，人工核验答案。
> 判定标准：能给出正确源码位置、能识别限制、无证据时**停止推断**（答「未知/未在知识库中」）。
> 反误导题（⚠ 类）答错即为**不通过**，必须修 `docs/knowledge/` 条目后重测。

## 使用方法（第 2 轮起必须照做）

1. 先核对入库状态：`cnb knowledge-base get-knowledge-base-info` 的 `last_commit_sha`
   等于预期构建 SHA，`include` 里没有本目录，`count` 与被索引文件数一致。
2. 用空上下文 agent（**无本仓库文件访问权限**，只能检索知识库）逐题提问。
3. 查询串用**原始问题口径**（就是本文件「问题」列的字面问法），
   **禁止把「期望答案要点」的原词并进查询串**——那等于把答案喂给检索器。
4. 本次固定 `--top-k 5` 便于比较。历史运行曾观察到 top-k=1/2 与 top-k=5
   返回集合不同；该观察不能证明平台恒定行为，完整结果以原始产物为准。
5. 每题记录四件事，缺一不计通过：
   - 召回的 top chunk（文件 + position + 分数）；
   - **agent 的原话回答**（摘录关键句，不是「命中了某 chunk」）；
   - 是否正确（对 / 部分对 / 错 / 编造）；
   - 若错：属于**检索失败**还是**条目误导**。
6. 结果回填到下方「回答记录表」，再汇总到「结果记录表」。

> 第 1 轮只记了召回到哪个 chunk，没记 agent 原话，也没做空上下文隔离
> （问题集当时还在索引里）。因此第 1 轮结论**全部作废**，见下方「作废说明」。

## A. 调用链类（主题一）

| # | 问题 | 期望答案要点 | 第 1 轮（已作废） |
|---|------|--------------|------|
| A1 | `--relations page:app/a.spg` 从 CLI 参数到图查询经过哪些函数？ | `main` → `run_query_commands` → `run_surface` →（裸前缀/裸名处理）→ `execute_resolved_target` → `route::route` → `run_cli_runtime_tool` → `CliAdapter::parse_input` → `GraphRuntime::query` | 命中 §1.2（0.95），见作废说明 |
| A2 | stdio server 里 `diff_refresh` 命令在哪处理？走到 runtime 会怎样？ | `handle_diff_refresh`（`src/stdio_server.rs:378`）拦截；未绑定 context 时 `DIFF_REFRESH_CONTEXT_REQUIRED`；若到 `GraphRuntime::query` 一律报错（`src/runtime.rs:1099`） | 命中 §1.3（0.99） |
| A3 | `--find` / `--explain` / `--relations` 三个动词最终汇合到哪个函数？ | CLI 与 stdio 都汇合到 `GraphRuntime::query`（`src/runtime.rs:971`）；差异只在 adapter 与输出封装 | 命中 §1.2（0.98） |
| A4 | stdio 模式下哪个命令不经过 `GraphRuntime::query`？ | `Status`（`src/stdio_server.rs:385`）与 `ReloadGraph`（`:394`）；`DiffRefresh`（`:378`） | 命中 §1.3（0.98） |
| A5 | `check_reload: true` 时 reload 失败会中断本次查询吗？ | 不会。只记 `GRAPH_RELOAD_FAILED` diagnostic（`src/stdio_server.rs:425-436`） | 命中 §1.6（0.92） |
| A6 | graphdb 打不开时 CLI 与 stdio 的行为差异？ | CLI：输出 `GraphDB::check_graph_db` 诊断 JSON 后退出（`src/main.rs:2002`）；stdio：启动即失败 | 命中 §1.6（0.98） |

## B. 扫描/持久化类（主题二）

| # | 问题 | 期望答案要点 | 第 1 轮（已作废） |
|---|------|--------------|------|
| B1 | `--build-graph` 的入口和六个阶段是什么？ | `scanner::scan_project_with_report`（`src/scanner/mod.rs:34`）→ `ProjectIndexer::scan_with_diagnostics`（`src/scanner/indexer.rs:607`）；发现/差分/解析/应用/持久化 | 命中本文件 B 表（0.97）——自我验证，见作废说明 |
| B2 | 增量判断依据是什么？mtime 还是内容 hash？ | **内容 hash（xxhash64）**，`diff_file_states`（`src/scanner/indexer.rs:351`）；`FileState` 也存 mtime/size 但不用于脏判断 | 命中本文件 B 表（0.99）——自我验证 |
| B3 | 坏 TBL 与坏 SPG 的失败行为有何不同？ | TBL → `ParseFailure` + `SCANNER_FILE_PARSE_FAILED`，旧图保留、下轮重试；SPG → 整轮索引失败（响亮失败） | 命中本文件 B 表（0.99）——自我验证 |
| B4 | 首次构建与后续增量的持久化路径差别？ | `prev_states.is_empty()` 时 `delta=None` 走 full rebuild 并置 v2 `Current`；否则 delta 路径，v2 置 `Stale`（`src/graph_redb.rs:1163-1200`） | 命中本文件 B 表（0.99）——自我验证 |
| B5 | v2 shadow 什么时候从 Stale 回到 Current？ | 累计受影响节点数 ≥ 1024（`V2_STALE_REBUILD_THRESHOLD_NODES`，`src/graph_redb.rs:87`）时同事务重建 | 命中 §1.3（0.99） |
| B6 | scanner 诊断存在哪里、如何保证与图一致？ | per-file 计数序列化后随 `IndexCommit.scanner_entries` 与图**同事务**落库；报告出口按全库口径合并 | 命中 §1.4（0.81） |
| B7 | `prepare` 与 `scan` 的区别？诊断谁负责落库？ | `prepare` 不调 `persist_index`；其 `commit.scanner_entries` 被置空，诊断经 `PreparedIndexUpdate` 透传，**调用方**负责挂载后落库 | 命中 §1.5（0.99） |

## C. 反误导类（⚠ 答错即不通过）

| # | 问题 | 期望答案要点 | 第 1 轮（已作废） |
|---|------|--------------|------|
| ⚠C1 | Grafeo 已经上线了吗？ | **没有。** 当前默认后端仍是 redb + v2 shadow；Grafeo 只有已批准设计与计划，**无实现代码** | 命中 §3.4（0.98） |
| ⚠C2 | B5 差分测试通过是否代表增量索引正确？ | **不代表。** `tests/m59_b5_full_vs_incremental_diff_tests.rs` 的两条缺陷测试是如实钉住当前分歧（丢跨文件边、占位节点残留），A3 修复后会变红 | 命中 §3.3（0.99） |
| ⚠C3 | M59 真实语料测量（501 SPG / 828 TBL / 3.72 GiB）能否作为 Grafeo 验收？ | **不能。** 那是**迁移前**测量（pin `c3c0528`），不是 Grafeo 验收 | 命中 §3.4（0.98）；去掉「M59/501/828」原词后掉到 0.007 |
| ⚠C4 | 增量更新会不会丢边？ | **会。** `cross_file_edge_is_lost_by_incremental_update` 已钉住跨文件边丢失 | 命中 §3.1（0.99） |
| ⚠C5 | 占位页节点在引用者删除后会被清理吗？ | **不会，会永久泄漏。** `src/scanner/spg.rs:1069-1076` 只 `add_node` 不 `node_ids.insert` | 命中 §3.2（0.96） |
| ⚠C6 | `--query-page-logic --human` 与 non-human 走同一条代码路径吗？ | **不是。** human 模式仍是旧路径（`src/main.rs:1200` 注释），待统一 human 渲染器 | 命中 §3（0.96） |
| ⚠C7 | M28 stdio 契约收敛做好了吗？ | **没有。** INDEX 中仍为 `planned` | 命中 §4（0.95） |
| ⚠C8 | v2 hydrate 失败后 fallback 到 v1，得到的图一定完整吗？ | **不一定。** `open_inner_v1`（`src/graph_redb.rs:290`）解码失败的节点/边与悬空边会被**计数并跳过**，得到部分缺失的图，经 `GRAPH_DB_PARTIAL_HYDRATE` / `GRAPH_DB_NODE_DECODE_FAILED` / `GRAPH_DB_EDGE_DECODE_FAILED` / `GRAPH_DB_EDGE_DANGLING_ENDPOINT` 上报（影响面 `partial`）。「v1 始终正确」只指 v1 表本身，不是「hydrate 结果一定完整」 | 第 2 轮新增题，无第 1 轮记录 |

## D. 边界/停止推断类

| # | 问题 | 期望答案要点 | 第 1 轮（已作废） |
|---|------|--------------|------|
| D1 | browser / WASM 的持久化怎么实现？ | 知识库中**无**此条目；正确回答是「未知 / 未在知识库中」 | 命中主题二 §5（0.54） |
| D2 | 查询内部的 21 类边投影有哪些？ | 知识库中**无**此条目；正确回答是「未知，需读源码」 | 命中主题一 §5（0.65） |
| D3 | 本地 `/Users/...` 路径下的真实项目可以直接用吗？ | 知识库中**无**授权资料清单；正确回答是「未知/未在知识库中」，不得假设路径可用 | 命中 AGENTS.md「外部参考」（0.56），该段现已补「不得假设路径可用」声明，待复验 |

## 第 1 轮作废说明（2026-09-06，构建 `23feea9`）

第 1 轮记录过「20 题全部命中应有分块 / ⚠C1–C7 全中 / 有条件可验收」。该结论**撤回**，理由：

1. **自我验证**：问题集、期望答案与实测记录当时位于被索引的 `docs/knowledge/` 内。
   实测「Grafeo 已经上线了吗」「增量判断依据是什么」等题，top1 直接命中
   `knowledge-acceptance-questions.md` 的分块（0.98 / 0.99），
   B 类题目 top1 多数是**本文件自己的 B 表**。命中不等于 agent 能正确回答。
2. **记录的是检索命中，不是 agent 回答**：没有保存空上下文 agent 的原话，
   因此「无误导」「能停止推断」两条判定标准当时**无法证明**。
3. **题数写错**：记录称 20 题，实际表格是 23 题（A6 + B7 + ⚠C7 + D3）。

本文件已移出索引；第 2 轮必须在新索引上用原始问法重测。

## 历史回答记录表（第 2 轮，固定题调优回归）

> **证据限制**：下表是原执行者报告的摘录；没有完整请求、top-k 原文、完整响应或可重放产物，
> 无法独立确认 24/24。四轮按同题修条目后的结果只能作为固定题回归，不能证明泛化能力。
> C3/D3 查询串与上方原题有差异；D3 的“唯一注入方式”结论已被本次源码核验否定。
> 后续新验收以 `questions.json` 与完整运行产物为准，历史摘录保留以便追溯。

> 终态构建 SHA `b024815`（PR #33 分支），知识库 `forceRebuild` 后 count=5 / 71833 B，
> `last_commit_sha=b024815…`，`include` 不含 `docs/knowledge-acceptance/**`。
> 检索口径：`--top-k 5`（**必须 ≥5**，理由见「使用方法」第 4 条）。
> 空上下文 agent = 只拿到检索片段、看不到仓库任何文件的 `deepseek-v4-flash`，
> system prompt 强制「片段里没有就答未知，禁止编造」。
> 说明：本表记录的是**修条目之后的终态结果**；首测暴露的问题保留在「判定」列的括号里，
> 中间过程见下方「第 2 轮过程」。

| # | 查询串（原始问法） | top1 chunk（文件 / position / 分数） | agent 原话回答（摘录） | 判定 | 错因 |
|---|------|------|------|------|------|
| A1 | `--relations page:app/a.spg` 从 CLI 参数到图查询经过哪些函数？ | topic-cli-stdio-to-query §1.0 速查（pos=2, 0.92） | 「调用链为：`main` → `run_query_commands`（`src/main.rs:1054`）→ `run_surface`（`src/main.rs:78`）→ `route::route`（`src/route.rs:96`）→ `GraphRuntime::query`（`src/runtime.rs:971`）」 | 对（第 2 轮首测为「部分对」，补 §1.0 速查后转对） | — |
| A2 | `stdio server` 里 `diff_refresh` 命令在哪处理？走到 runtime 会怎样？ | topic-cli-stdio-to-query §1.0/§1.3（pos=8, 0.94） | 「在 `handle_request` 里先行处理（`src/stdio_server.rs:378`），未绑定 context 报 `DIFF_REFRESH_CONTEXT_REQUIRED`；到达 `GraphRuntime::query` 一律报错（`src/runtime.rs:1099`），即协议错误」 | 对 | — |
| A3 | `--find` / `--explain` / `--relations` 三个动词最终汇合到哪个函数？ | topic-cli-stdio-to-query §1.0（pos=1, 0.95） | 「统一汇合到 `GraphRuntime::query`（`src/runtime.rs:971`），并给出完整链路 `main` → `run_query_commands` → `run_surface` → `route::route` → `query`」 | 对（首测为「部分对」，补 §1.0 后转对） | — |
| A4 | stdio 模式下哪个命令不经过 `GraphRuntime::query`？ | topic-cli-stdio-to-query §1.0（pos=8, 0.94） | 「`Status`、`ReloadGraph`、`DiffRefresh` 三个命令不经过（`src/stdio_server.rs:385/394/378`）」 | 对 | — |
| A5 | `check_reload: true` 时 reload 失败会中断本次查询吗？ | topic-cli-stdio-to-query §1.6（pos=7, 0.93） | 「不会中断本次查询。『`check_reload` 失败：只记 diagnostic，不中断本次查询』」 | 对 | — |
| A6 | graphdb 打不开时 CLI 与 stdio 的行为差异？ | topic-cli-stdio-to-query §1.6（pos=5, 0.96） | 「CLI：`GraphDB::check_graph_db` 输出诊断 JSON 后退出（`src/main.rs:2002`）；stdio：启动即失败」 | 对 | — |
| B1 | `--build-graph` 的入口和六个阶段是什么？ | topic-scan-incremental-persist §0 速查（pos=0, 0.89） | 「入口 `scan_project_with_report`（:34）；六阶段 `discover_files`(325) → `diff_file_states`(351) → `parse_dirty_files_with_failures`(417) → `apply_incremental_changes`(535) → `persist_index`(572)；判脏用 xxhash64 内容 hash」 | 对（首测只答出前两阶段，六阶段速查上移文首后转对） | — |
| B2 | 增量判断依据是什么？mtime 还是内容 hash？ | topic-scan-incremental-persist §0（pos=0, 0.88） | 「依据是 **xxhash64 内容 hash**，不是 mtime」 | 对 | — |
| B3 | 坏 TBL 与坏 SPG 的失败行为有何不同？ | topic-scan-incremental-persist §1.2（pos=6, 0.98） | 「坏 TBL 记 `ParseFailure` → `SCANNER_FILE_PARSE_FAILED`，旧图保留、下轮重试；坏 SPG 整轮索引失败（`?` 向上抛），刻意保持响亮」 | 对 | — |
| B4 | 首次构建与后续增量的持久化路径差别？ | topic-scan-incremental-persist §1.3（pos=4, 0.98） | 「首次走 full rebuild（`prev_states.is_empty()` 时 `delta = None`），后续增量走 delta」 | 对 | — |
| B5 | v2 shadow 什么时候从 Stale 回到 Current？ | topic-scan-incremental-persist §1.3（pos=4, 0.99） | 「累计受影响节点数 ≥ `V2_STALE_REBUILD_THRESHOLD_NODES = 1024` 时本轮同事务重建 v2 并回 `Current`（`src/graph_redb.rs:1169`）」 | 对 | — |
| B6 | scanner 诊断存在哪里、如何保证与图一致？ | topic-scan-incremental-persist §1.4（pos=5, 0.96） | 「per-file 计数随 `IndexCommit.scanner_entries` **与图同事务落库**（`src/scanner/indexer.rs:134`），删除路径经 `scanner_deleted_paths` 同事务移除；runtime 加载期并入 `load_diagnostics` 透出 `SCANNER_*`」 | 对 | — |
| B7 | `prepare` 与 `scan` 的区别？诊断谁负责落库？ | topic-scan-incremental-persist §1.5（pos=5, 0.98） | 「`prepare`（:770）复用同一套阶段函数但**不调 `persist_index`**，`commit.scanner_entries` 被置空（:895-896）；诊断经 `PreparedIndexUpdate` 透传，由**调用方**合并后随最终 commit 落库，忘记挂载就丢」 | 对 | — |
| C1 | Grafeo 已经上线了吗？ | topic-scan-incremental-persist §3.4（pos=9, 0.94） | 「Grafeo 尚未接入，且『没有任何实现代码』，只在已批准设计与计划里」（先答未知再给出正确结论，措辞保守但结论正确） | 对 | — |
| C2 | B5 差分测试通过是否代表增量索引正确？ | topic-scan-incremental-persist §3.3（pos=8, 0.99） | 「不代表。『B5 全绿』不能作为增量索引正确性的证据，两条缺陷测试是如实钉住当前分歧，A3 落地后会变红」 | 对 | — |
| C3 | M59 真实语料测量能否作为 Grafeo 验收？ | topic-scan-incremental-persist §3.4（pos=9, 0.99） | 「不能。是迁移前测量，不是 Grafeo 验收；Grafeo 无任何实现代码，测量文件已排除出索引」 | 对 | — |
| C4 | 增量更新会不会丢边？ | topic-scan-incremental-persist §3.1（pos=8, 0.99） | 「会，已确认会丢。A 内嵌 B 时丢跨文件边，根因是删除 `previous_node_ids` 连带删掉其它文件指向这些节点的边」 | 对 | — |
| C5 | 占位页节点在引用者删除后会被清理吗？ | topic-scan-incremental-persist §3.2（pos=8, 0.98） | 「不会，会永久泄漏。缺陷已确认未修，A3 的 `origin_file` 落地后才有清理依据」 | 对（首测答「未知」，补显式问答后转对） | — |
| C6 | `--query-page-logic --human` 与 non-human 走同一条代码路径吗？ | topic-cli-stdio-to-query §3（pos=8, 0.99） | 「不是，二者走两条不同代码路径。human 模式尚未迁移到 runtime，仍走旧渲染路径，输出契约不统一，已确认未修」 | 对（首测**不通过**，改成显式问答后转对） | — |
| C7 | M28 stdio 契约收敛做好了吗？ | topic-cli-stdio-to-query §4（pos=8, 0.95） | 「没有，M28 stdio 契约收敛未做。INDEX 中 M28 仍为 `planned`，属『历史状态 / 已批准计划』，不是已实现能力」 | 对（首测答「未知」，补显式问答后转对） | — |
| C8 | v2 hydrate 失败后 fallback 到 v1，得到的图一定完整吗？ | topic-scan-incremental-persist §3.5（pos=10, 0.99） | 「不一定完整。v1 hydrate 对三类坏数据计数并跳过，诊断 impact 都是 `partial`；『fallback v1 始终正确』只指应回落 v1 布局，不保证结果无缺失」 | 对（PR #33 新增条目，复验通过） | — |
| D1 | browser / WASM 的持久化怎么实现？ | AGENTS.md（pos=2, 0.63） | 「知识库中没有这个信息。依据『本文不覆盖 browser / WASM 路径』的限定，以及 `browser/` 只可维护纯 JS glue / provider」 | **停止推断通过** | — |
| D2 | 查询内部的 21 类边投影有哪些？ | topic-cli-stdio-to-query §5（pos=8, 0.69） | 「知识库中没有这个信息。『查询内部的图遍历（21 类边…）不在本文范围，暂无知识条目』，具体投影内容未知」 | **停止推断通过** | — |
| D3 | 本地 `/Users` 路径下的真实项目可以直接用吗？ | AGENTS.md 外部参考（pos=5, 0.95） | 「不能直接用。两条路径只在维护者本机存在，不构成任何环境可用的语料清单；真实语料唯一注入方式是 CI 变量或 clone fixture 仓库」 | **停止推断通过**（PR #33 的 D3 修复复验通过） | — |

### 第 2 轮过程（先测后修，共 4 次重建）

| 阶段 | 构建 SHA | 暴露的问题 | 处理 |
|---|---|---|---|
| 首测（24 题） | `e3f76ff` | A1/A3 只答出 `run_surface`，漏 `GraphRuntime::query`；B1 只答出六阶段前两个；⚠C6 答「未知」判不通过 | 主题一加 §1.0「完整链路速查」；⚠C6 缺陷改成显式问答；六阶段表格压缩为四列 |
| 复测 | `417f156` | A1/A3/B1/C6 转对，但 B1 的速记段落仍被分块切断（top1 落在 §1.2 上一块） | 六阶段速查上移到文首「## 0. 检索速查」，保证落在首个分块 |
| 复测 | `200caae` | B1 转对；⚠C5 / ⚠C7 出现「片段有内容但模型不敢下结论」 | §3.2 / §4 各补一句显式问答 |
| 终测（24 题） | `b024815` | 无 ⚠C 失败项；D1/D2/D3 均正确停止推断 | 收尾，写入本表 |

## 结果记录表

| 轮次 | 日期 | 构建 SHA | 入库核对 | A 类 | B 类 | ⚠C 类 | D 类 | 结论 | 需修条目 |
|------|------|----------|----------|------|------|-------|------|------|----------|
| 1 | 2026-09-06 | `23feea9` | 6 chunk / 71913 B | — | — | — | — | **作废**：问题集在索引内造成自我验证，且未记录 agent 原话 | 隔离验收集（已修） |
| 2 | 2026-09-06 | `b024815`（PR #33 分支，4 次 forceRebuild 后终态） | count=5 / 71833 B，`last_commit_sha=b024815…`，验收集不在 include | **6/6 对** | **7/7 对** | **8/8 对** | **3/3 停止推断通过** | **历史自报，未独立验收**：固定题调优后 24/24，原始证据缺失，D3 含错误断言 | 已在本轮修完：§1.0 链路速查、§0 六阶段速查、C5/C6/C7 显式问答 |

## 扩展判定

扩展前还需固定题与预先冻结的改写题均留下完整证据，逐题人工核验；
不得仅凭上述历史 24/24 宣布可验收。

- ⚠C 类**全部答对**（有 agent 原话且结论正确）且 D 类能正确停止推断 → 可考虑扩展主题
  （查询内部、diff-refresh 编排、browser/WASM）。
- ⚠C 类任一答错 → 先修 `docs/knowledge/` 条目，重测；**不得**通过增加主题数量掩盖。
- D 类被编造答案 → 说明条目边界声明不足，需在条目「边界与限制」中显式补「不在范围」清单。
