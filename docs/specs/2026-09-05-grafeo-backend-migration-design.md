# Grafeo 后端迁移设计

> 状态：**approved**（2026-09-05 用户决策：「别查了，直接换」）
> 里程碑：[M59](../milestones/performance/m59-grafeo-backend-migration.md)（active；redb 退役）
> 实施：[M59 实施与交接计划](../plans/2026-09-06-m59-grafeo-implementation-plan.md)（approved，2026-09-06）
> 上游：[后端研究计划](../plans/2026-09-04-local-query-plane-and-backend-research-plan.md)、
> [计划复核](../plans/2026-09-05-graph-backend-migration-plan-review.md)、
> [Grafeo 实测记录](../ai-eval-runs/2026-09-05-grafeo-spike-measurements.md)
> 输入：2026-09-05 codex 只读源码审查（6 条 P1/P2 + 2 条附注），本文 §2 已逐条复核

## 0. 已决事项（不再讨论）

| 决策 | 说明 |
|---|---|
| **换 Grafeo** | 不再先查 load path。复核 §0 提的「1,008 s 是我们自己的 bug，值得先查」已提出并被否决，记录在案，不作为迁移前置 |
| **体积闸门解除** | 实测 grafeo 最小特性集 8.86 MiB、项目原上限 10 MB。按用户判断：相对最终库的能力，工具体积可接受。§2.3 的淘汰规则对本次迁移不适用 |

保留一条**非阻塞**记录：Grafeo 用 0.87 s 完成我们要 1,008 s 的同一件事（都是全量进
内存）。迁移完成后这个差值会自动消失，但**同样的病理代码可能还在 scanner/序列化
侧**，建议 M59 收尾时回看一眼，不在关键路径上。

## 1. 迁移的核心风险：schema 是一次性冻结的

`GrafeoDB` 一旦落库，**节点身份文法与边的属性集就固化了**。codex 审查出的 6 条里，
**有 4 条是身份 / 溯源 / 契约层面的**——它们不是「迁移前顺手修的 bug」，而是
**迁移 schema 本身要回答的问题**。带着它们迁移，等于把缺陷刻进新库。

这与计划阶段 6 的表述一致（「不能因为选择了 Grafeo 就把这些遗留项视为已解决」），
但优先级要提得更高：**它们是 schema 设计输入，不是并行工作项。**

## 2. codex 审查的逐条复核

全部 6 条**均已在源码中确认属实**。其中 2 条比原文更严重，已标注。

### 2.1 P1｜增量更新丢失未变更文件贡献的跨文件边 —— 确认，且更严重

`indexer.rs:349` `apply_incremental_changes` 先 `remove_nodes_by_ids(merged_removed)`
（含 dirty 文件的 `previous_node_ids`），再**只重跑 dirty 文件**。
`graph_redb.rs:896` 删节点连带删边。

A 页嵌入 B 页 → 只改 B 的标题 → 删 B 节点时 A→B 的 `EmbedsPage` 边一并消失 →
重建 B 不会重跑 A → **边永久丢失**，且没有任何诊断。

> **比原文更严重的一点**：diff-refresh 侧的 `invalidated_pages`
> （`orchestrator.rs:61`）**不是**补建引用方的机制，它只驱动**缓存失效与
> re-warm**（`runtime.rs:799-818`）。也就是说 re-warm 会在**已经丢了边的图**上重建
> 缓存，得到一份「与错误的图完全一致」的缓存。这比没有缓存更难发现——
> 全仓无 referrer / reverse-dependency 重扫逻辑（`grep referrer|reverse_dep|dependents`
> 在 `src/scanner/` 与 `src/diff_refresh/` 下无命中）。

**对迁移的含义**：这是 schema 问题。边必须带 **`origin_file`（产生这条边的源文件）**，
删除时按 origin 而非按端点删；未变更引用方的边不该被端点删除牵连。

> **已由 B5 转成可执行判据（commit `0266fcb` + `cad7335` + `34f5a5d`）**：
> `tests/m59_b5_full_vs_incremental_diff_tests.rs`，4 条。本条此前只有源码论证，
> 没有任何测试能证明它存在、也没有任何测试能在 A3 落地后证明它消失了。
>
> 做法是**逐属性差分**而非计数比对：同一最终文件集跑两条索引路径（全量 / 增量），
> 各自导出规范化快照（节点连同 `node_type` / `path` / `name` / `meta`；边连同
> `from` / `to` / `edge_type` / `field_path` / `meta`），排序后逐行比对。
> 计数比对挡不住「丢一条边、同时多一条占位边」这类**总数不变**的错误——
> 而本条缺陷恰好就发生在边的层面。
>
> 机制已逐行核对到源码：A 扫描时为内嵌目标 `add_node(page:app/b.spg)`，
> 却**不**把它写进 A 的 `node_ids`（`spg.rs:1067-1075`；对比 `:584` / `:757`
> 普通节点都显式 `node_ids.insert`）；B 扫描时把同一个 id 写进 **B** 的
> `node_ids`。于是 B 变脏 → `previous_node_ids` 含该节点 → 删节点连带删边 →
> A→B 的 `EmbedsPage` 边消失 → 只重跑 B，边不再重建。
>
> **B5 新发现的另一半（本文新增，codex 未报，原文未涵盖）**：
> 同一处 `add_node` 不登记 `node_ids`，还导致**占位页节点在引用方被删除后永久泄漏**。
> A 内嵌一个**不存在**的 `ghost.spg` → 占位节点 `page:app/ghost.spg` 只由 A 产生，
> 却不在 A 的 `previous_node_ids` 里 → 删掉 A，没有任何文件会声明它，
> `remove_nodes_by_ids` 永远碰不到（全仓无 orphan / prune 清理路径）。
> 已实测复现：全量路径下该节点不存在，增量路径下残留 1 个。
>
> 这两条是**同一个根因的两面——节点归属没有记账**：
> 原文那条是「别人的边被我的删除牵连」，新发现这条是「我的节点没人来删」。
> **对 A3 的直接修正**：`origin_file` 不能只加在边上，**节点也要带**，
> 否则修完仍剩泄漏这一半。B5 的第四条用例即节点归属半边的验收判据。
>
> 四条用例的定位分工：无跨文件边时两侧必须**逐字节一致**（同时证明差分器不空转，
> 而不是拿两个空集互比）；被改动文件自身的节点必须重建得一模一样（确认分歧
> **只**出在跨文件边，不是增量路径整体失灵）；然后才是两条缺陷各自的钉子。
> 两条钉子都写成**「钉住当前行为，且当前行为是错的」**——A3 落地后它们会立刻
> 变红，届时把期望改成 `full == inc` 即可，注释里已写明**不要放宽断言**。
>
> 远程验证（`34f5a5d`）：`FMT_EXIT=0`，4 passed; 0 failed。
>
> **codex 复审返修（`2db8af9` 之后 → `a681499`）：两条钉子没把差异约束到那一处。**
> 它们原先只断言**过滤出来的那一类**行——`embeds(&inc).is_empty()`、
> `ghost(&inc).len() == 1`。codex 实测注入两种故障：把增量侧的边**全部**清空、
> 把 B 的节点连同所有边一起删掉——**四条用例照样全绿**。
>
> 原因是这类断言只说明「差异**至少包含**这处缺陷」，而退化探针需要的是
> 「差异**只有**这处缺陷」。A3 落地前这两条用例的全部价值就在后者：任何**别的**
> 退化混进来时它们必须变红，否则一次真实的图崩塌会被当成「已知问题依然存在」。
>
> 现改为断言**完整差异**（`complete_diff` 返回 `(全量有而增量没有, 增量有而全量没有)`）：
> 跨文件边那条钉住 `missing.len() == 1` 且 `missing[0]` 逐属性等于全量侧那条
> `EmbedsPage` 边、`extra` 为空；占位节点那条钉住 `extra_nodes` 恰好是那个 ghost 节点、
> `missing_nodes` 为空、且**边集两侧完全一致**（泄漏的是悬空节点，不是悬空边——
> a.spg 自身节点被删时它的 `EmbedsPage` 边随之消失）。两条都补了
> 「增量侧不是空图」的前置健全性断言，避免「只少一条」其实是拿空集在比。

### 2.2 P1｜损坏 TBL 被当成功的空结果提交 —— 确认

`tbl.rs:14-22`：`content.is_empty()` 与 `serde_json::from_str` 的 `Err(_)` 都返回
`Ok(空集)`。配合 §2.1 的先删后建：一个原本正常的 `.tbl` 写入半截内容 →
旧模型图被删 → 新解析失败返回空 → **file hash 照常记录** → 下次内容不变直接跳过。

「读取失败」被静默转成「模型不存在」，无 `PARSE_FAILED` 类诊断。

**对迁移的含义**：提交语义问题。新库要么在解析验证通过后才动候选图，要么保留上一份
有效图并标记陈旧。**不能在迁移时沿用「失败即空集」**。

> **已落地（B3，commit `f8091c5` + `eaeae4b`）**：两条要求**都**做到了。
> `tbl.rs` 的空内容与非法 JSON 改为**响亮失败**（`bail!`）；校验前移到
> `indexer::parse_dirty_files_with_failures`，解析失败的文件**整个跳过**：
>
> - 不进 `updates` ⇒ `previous_node_ids` 不进 `merged_removed` ⇒ **旧图原样保留**；
> - 不写 `new_states` ⇒ **旧 file hash 保留** ⇒ 文件下一轮仍是脏的，会自动重试
>   （这是原缺陷「损坏被永久固化」的正解——不靠人工介入）；
> - 写一条 `SCANNER_FILE_PARSE_FAILED` 诊断（Warning / `IMPACT_PARTIAL`）⇒
>   图里这部分是**陈旧**而非缺失，查询方能区分这两件事。
>
> 借用既有的 per-file `scanner_entries` overwrite-by-key 机制，
> 因此文件修好后该标记**自行退役**（TBL 分支写零计数 entry 覆盖旧值），
> 不需要额外的清理路径。
>
> 刻意保留的不对称：**SPG 解析失败仍然整轮抛错**，不降级成 ParseFailure。
> 它本来就是响亮的失败，从没有「静默转成空结果」的问题；B3 要修的是 TBL
> 那条静默路径，不是把 SPG 也改成部分成功。
>
> 回归测试 `tests/m59_b3_tbl_parse_failure_tests.rs`（3 条）覆盖四轮时序：
> 有效 → 损坏（旧图与边数不变 + 诊断出现）→ 重扫（`dirty > 0`，证明 hash 未被记录）
> → 修复（诊断消失、边数增长）。远程 `FMT_EXIT=0`、`B3_EXIT=0`，
> 相邻 7 个套件共 55 条测试全绿。
>
> **codex 复审补漏（`985c27e` / `91f2dac`，B3 落地后新增 2 条用例）**：
> 上面那套淘汰机制只挂在「文件**这轮被解析**了 ⇒ 按 logical_path 覆盖 entry」上，
> 因此有两个够不着的角落，`SCANNER_FILE_PARSE_FAILED` 会**无限期挂在一个
> 已经不存在问题的路径上**：
>
> 1. **内容被改回上一次解析成功时的字节**。hash 与保留下来的 `FileState` 相同
>    ⇒ 文件不脏 ⇒ 不解析 ⇒ 覆盖不触发。原用例修复时写的是**另一份**合法内容
>    （多一个维度，用来证明确实重新解析了），恰好绕开了这个角落。
> 2. **首轮就解析失败的文件被删除**。它只写了诊断、**没写 `FileState`**
>    （失败文件整个跳过 apply），删除后既不在 discovered 里、也进不了
>    `plan.deleted`（后者由 `prev_states` 推出），无人清理。
>
> 两者是同一件事的两面——**诊断的生命周期从未与「当前文件集」对账过**，
> 只跟着「这轮解析了谁」走。故合并为一个 `stale_scanner_diagnostic_paths`：
> 未被发现的路径 ⇒ 孤儿；已发现但不脏且 entry 记着 `parse_failed` ⇒ 陈旧。
> 两类都并入 `scanner_deleted_paths`，与图同事务落库；`scan` 的提交分支条件
> 相应放宽，否则「只有诊断要清」时永远不会产生提交。
>
> 第 2 类**直接删 entry 而不强制重新解析**，依据是本模块自持的不变量：
> `FileState.file_hash` 只从 `updates` 写入，而 `updates` 只含解析成功的文件，
> 所以「hash 与已存 `FileState` 相同」等价于「这份字节解析得通」。
> 反过来强制重解析会把内容未变的文件推进 apply 路径、删除并重建其节点——
> 在 §2.1 尚未修复前，那等于为清一条警告去触发一次真实的丢边。
>
> 边界：**解码不出来的 entry 一律不碰**，哪怕路径已不存在。损坏有自己的响亮
> 信号（`SCANNER_DIAGNOSTICS_REFRESH_FAILED`），删掉等于毁掉证据。
> 首版没画这条线，把 `m58_3_pr2` 用例故意注入的损坏 entry 当孤儿清了，
> 该用例当场转红——那条用例现在就是这个边界的回归网。
>
> **codex 复审返修（`2db8af9` 之后 → `a681499`）：条数相等不等于事实未变。**
> 三条端到端用例原先用「`model:orders` 的邻边条数与基线相同」证明「旧图逐边保留」，
> 用「`edge_count > baseline`」证明「修好后 customer 字段进图了」。两者都不成立：
> 把一条字段边**换成另一条**、改坏边上的 `field_path` / `meta`、或者把边**掉个方向**，
> 条数纹丝不动；`edge_count` 变大也只证明「多了点什么」，不证明多的正是那个字段。
>
> 现改为整图**逐属性快照 + 完整差异**（`SnapshotDiff`）。每一处「保留」断言都写成
> `diff_from(&baseline) == SnapshotDiff::default()`——空差异 ⇔ 两张图逐属性一致。
> 修复轮则写成精确差异：`added_nodes == [field:orders.客户]`、
> `changed_nodes == [model:orders]`、
> `added_edges == [model:orders -Contains-> field:orders.客户]`、
> `removed_*` 均为空。
>
> `SnapshotDiff` 之所以把 `changed_nodes` 与 `added_nodes` 分开，是因为
> `tbl.rs` 把 `dimensions` 存在**模型节点自己的 `meta` 里**：加一个维度**既**新增
> 一个字段节点、**又**改变模型节点本身。平铺成一个行集合会把这两件事糊在一起，
> 分开之后修复轮才能说清「谁新增、谁变了、没有谁消失」。

### 2.3 P1｜值追溯的切片越界 panic 与中文乱码 —— 确认，可复现

`dependency.rs:392-416` `replace_with_boundary`：

```rust
while i <= s_bytes.len().saturating_sub(pat_bytes.len()) {
    if &s_bytes[i..i + pat_bytes.len()] == pat_bytes {
```

`pat_bytes.len() > s_bytes.len()` 时 `saturating_sub` 归零，循环条件 `0 <= 0` 成立，
`s_bytes[0..pat_len]` 在短切片上**直接 panic**。调用点 `:365-369` 构造的 pattern 是
`format!("{}.value", dep_id)`（比 `dep_id` 长 6 字节），只要 `expanded` 短于它就触发。

同一函数 `result.push(s_bytes[i] as char)` 把 UTF-8 **逐字节**转 `char`，中文表达式
必然乱码。入口是 `output/component.rs:127`，属用户可达路径。

违反 AGENTS.md「禁止使用 `panic!` 处理可恢复错误」。**与后端无关**，任何时候都该修。

> **已落地（B4，commit `3f84a96` + `c99a14d`）**：改用 `str::match_indices` +
> 字符边界切片拼接。匹配只落在合法字符边界上，pattern 长于 s 时不产生匹配，
> 越界不再可能；未匹配区间按 `&s[last..start]` 原样搬运。词边界判定行为不变
> （多字节字符的字节 `>= 0x80`，`is_word_char` 返回 false）。
> 回归测试 `tests/m59_b4_value_trace_utf8_tests.rs` 已在**修复前的 `src/dependency.rs`**
> 上验证会失败，两条缺陷均精确复现：
> `range end index 7 out of range for slice of length 1`（dependency.rs:399）、
> `合同金额：` → `ååéé¢ï¼`。
>
> **修复过程中暴露的新缺陷（本文新增，codex 未报）**：panic 一直掩盖着
> `RefType::ComponentValue` 的**替换 pattern 与来源文法不匹配**。
> `expr_ast.rs:862-868` 把 `b.value`、`b.step` 和裸 `b` **一律**归一为
> `ComponentValue("b")`，丢掉了后缀；而 `dependency.rs:365-369` 只按
> `format!("{}.value", dep_id)` 一种形态去替换。后果：
> - 裸 `${b}`：pattern 不匹配（旧代码在此 panic），值追溯**静默不展开**，
>   实测 `a.value` 追溯结果是 `=b` 而非展开到 `param1`；
> - `b.step`：同样不匹配，同样静默不展开。
>
> 这不是替换函数的 bug，是**引用文法在归一时丢了信息**——修在替换侧只能靠猜
> 后缀（先试 `.value` 再试裸 id，会把 `b.step` 错误改写成 `(展开).step`）。
> 正确的修法是让 `RefType::ComponentValue` 保留被引用的原始 token，属**阶段 A1
> 身份文法**的范围，故不在 B4 内顺手改。已在 A1 增列一条。
>
> **A1b 的值追溯半边已落地**（`f5d35b9` / `20921ad`）。拆成两半的理由：
> 改 `RefType::ComponentValue` 的元数（让它带原始 token）要动 `src/`、`tests/`、
> `benches/` 共 **103 处**，且与 A1 的身份文法重写重叠——那半边仍留在 A1 内。
> 但「裸引用静默不展开」和「`.step` 可能被错误替换成值」是**当下就在产出错误
> 结果**的缺陷，不该等 A1。故新增 `dependency::replace_component_value_ref`，
> 在替换点按后缀显式判定：`id.value` 与裸 `id` 替换，`id.<其它后缀>` 跳过，
> 词边界挡住 `bx` / `id.values` 这类同前缀更长标识符；替换文本不被二次扫描，
> 且剥掉前导 `=`，避免拼出 `CONCAT(a, =(param1))` 这种嵌套等号。
> 断言写在**行为层**（`tests/m59_a1b_component_value_ref_tests.rs`，4 条），
> 因此 A1 落地后重构替换实现时这些断言应当继续成立，是 A1 的回归网。
> 同时把 `m59_b4_value_trace_utf8_tests.rs` 里那条**把缺陷当期望**的断言
> （`assert_eq!(trace.expanded_expr, "=b")`）改掉——它原本会把 A1b 的修复判成回归。
>
> **codex 复审补漏（`985c27e`）：替换会改写字符串字面量。** 上面这个替换点用
> `match_indices` 扫整串，引号内的同名文本照样命中——**词边界判定挡不住它**，
> 引号本身就不是词字符，前后检查都通过。于是 `=CONCAT("b", b.value)` 被展开成
> `=CONCAT("(param1)", (param1))`：**表达式语义被静默改掉**，无任何诊断。
> 这是 A1b 引入裸 id 替换时带进来的——只认 `id.value` 的旧代码碰不到这一格。
>
> 修法是新增 `dependency::string_literal_spans`，词法与
> `expr_ast::Tokenizer::read_string_literal` 同口径（单/双引号各自成对、
> 反斜杠转义下一个字符、未闭合吃到串尾），两个替换点整段跳过字面量区间。
> `replace_with_boundary`（`Param` / `ModelField` 侧）是同一个坑，一并修：
> `=CONCAT("param1", param1)` 原本前一个也会被替换。
>
> 用例加在行为层（`m59_a1b_component_value_ref_tests.rs` 增至 6 条），
> 共用 fixture 新增 `lit` / `plit` 两个组件。A1 重写替换实现后这两条应继续成立。
>
> **断言强度返修（`99ec146`，codex 指出 B4 的子串断言偏弱）**：
> `m59_b4_value_trace_utf8_tests.rs` 两条用例原先只断言
> `!expanded_expr.is_empty()` 与几个 `contains` / `!contains`，已换成完整比对
> （`=(param1)`、`=CONCAT("合同金额：", (param1), "元")`）。codex 同时指出
> A1b 已对同一批 fixture 做了精确断言，因此这里是**刻意重复**：
> panic 类缺陷的回归网必须自带完整期望值，不能依赖另一个文件恰好也在断言同一件事。
> 保留 `!contains('\u{fffd}')` 那条——它写的是**意图**（不得出现替换字符），
> 与「等于某个字面量」是两种不同的约束。

### 2.4 P1｜模型身份跨页/跨目录碰撞 —— 确认（本文认定为迁移的头号前置）

- `spg.rs:559`：内嵌模型命名为全局 `model:<source.id>`；
- `tbl.rs:23-29`：`Path::file_stem()` 构造 `model:<文件名>`，**不含目录**。

两个页面各有 `model1`、两个目录各有 `orders.tbl` → 同一个节点，来源与属性互相覆盖，
关系混在一起；删一个页面还可能删掉另一页仍在用的同名节点。

这正是 M58.3 的 F4 / PR4b（页面局部身份），spec 里已有完整迁移矩阵
（`2026-08-24-m58-3-command-surface-gap-fixes-design.md:191-230`），**尚未落地**。

**对迁移的含义**：身份文法就是 schema。**必须在落 Grafeo 之前定稿**，否则迁移要做
两遍（复核 §2.2 已论证）。

### 2.5 P2｜相对资源路径未归一 —— 确认

`utils.rs:60-62`：`current_dir.join(ref_path)` 后只做 `\` → `/` 替换，不归一
`.` / `..`。`app/a.spg` 引用 `../b.spg` 得到 `app/../b.spg`，而真实扫描产出 `b.spg`，
导航边指向一个占位节点。

**对迁移的含义**：页面身份的一部分，与 §2.4 同批定稿。

### 2.6 P2｜内存与 redb 存储行为不一致 —— 确认，且是迁移的硬前置

`memory_graph_store.rs:149-168`：

- `upsert_node` 无条件 `insert`；
- `add_edge` 无条件 `push`，**不去重**；
- 邻接表存的是**节点副本**（`to_node` / `from_node` 在建边时 clone）——
  之后 `upsert_node` 更新节点，`get_node` 与邻边返回的节点内容会**不一致**。

redb 侧则去重边、对占位模型保留既有 metadata。同一组写入，两个实现产出不同查询结果。

> **codex 没连上的一点**：迁移会引入**第三个** `GraphStore` 实现。现在两个实现就已
> 经不一致，再加一个而没有共享契约测试，等于保证三方分叉——**而且我们将无法判断
> Grafeo 实现是对的还是错的**，因为没有基准。因此共享存储契约测试套件是
> **迁移的硬前置，必须先于 Grafeo 实现存在**，不能等迁移完再补。

**已落地（B1，commit `4962412` + `e4b1106`，断言强度返修 `a681499`）**：
`tests/m59_b1_graph_store_contract_tests.rs`，13 个用例 × 2 个实现 = 26 条测试，
由 `contract_suite!` 宏展开。用例只用 trait 方法、不碰实现细节，
**C1 落地时加一个 `impl` 分支即可接入，用例一行不动——用例通过即为 C1 的验收判据。**

覆盖 §3 B1 点名的五项（重复边 / 节点更新 / 占位节点升级 / 删除 / 邻接一致性），
另加三项：计数自洽（`iter_nodes` ↔ `node_count`、逐节点出边之和 ↔ `edge_count`）、
入出边互为镜像、以及「端点删后重建同一条边必须能重新写入」——
去重键若不随删除一起清，图里会**静默永久缺一条边**，无任何报错。

分歧一律以 **redb 为准**（生产查询路径实际跑的实现）。规则本身提到
`graph_store.rs` 由两实现共用**同一份实现**，避免规则分头演化——
这正是本条缺陷的成因：

- `merge_upsert_meta` 由 `graph_redb.rs` 的私有函数提升为
  `graph_store::merge_upsert_meta`（`graph_redb` 是 `cli-local` gated 而
  `memory_graph_store` 不是，只有 `graph_store` 两边都可见）；
- 新增 `graph_store::edge_dedup_key`，把「`field_path` 参与去重键」写成明文——
  同一对端点上不同字段产生的引用是**不同的事实**，不能合并。

`MemoryGraphStore` 三处偏离逐条纠正：`add_edge` 不去重 → 按四元组去重且删节点时
连带清去重键；邻接表存节点副本导致 upsert 后邻边读到旧值 → 只存 `Edge`、读时按 id
现查（**结构上**消除不一致的可能，而不是靠记得同步两份）；`upsert_node` 无条件覆盖
meta → 走共享规则。

> 顺带修掉一个**无效基准**：`add_test_edge` 原先用两个 `if let` 各判一端，
> 只有一端存在时会留下**半悬挂的单向边**。这种状态 redb 侧根本构造不出来，
> 拿它搭出来的测试期望验证不了任何生产行为。现改为直接走 `add_edge`。

远程验证（`4962412` + `e4b1106`）：契约套件 24/24 通过，
**`cargo test --features cli-local` 全量套件 `FULL_EXIT=0`**（`MemoryGraphStore`
是 57 处调用点的共用替身，改它必须全量回归），`FMT_EXIT=0`。

> **codex 复审返修（`2db8af9` 之后 → `a681499`）：这套契约测试能被一个明显错误的
> 实现骗过。** codex 用变异实现实测：把 `MemoryGraphStore` 改成①边类型一律返回
> `Contains`、②读边时丢掉 `field_path`、③邻接视图回填错误的节点 `path`、
> ④删除任一**存在**的节点就清空所有边——**十二条用例全绿**。
>
> 这不是「测试不够多」，是**断言约束错了东西**：它们看的是**条数**、单个**字段名**、
> 或者「两个输出彼此一致」。后者尤其危险——两侧可以一起错还互相印证。具体地：
>
> - 全套用例只用 `DependsOn` 一种 `EdgeType`，且从不断言读回来的类型 ⇒ ①逃逸；
> - 断言只比 `edge_count` 与 `name` ⇒ ②③逃逸；
> - 两条删除用例里**每一条边都挨着被删的节点**，于是「删除即清空」与「只删该删的」
>   产生完全相同的观测；`removing_unknown_node_is_a_noop` 删的又是个**不存在**的
>   id，「只在节点存在时清空」这一变异同样滑过去 ⇒ ④逃逸。
>
> **这条的严重性由 §2.6 本身决定**：本套件是 C1（Grafeo 实现）**唯一**的验收判据。
> 一套能被行为明显错误的实现骗过的契约测试，给出的是**虚假的验收信号**——比没有更糟，
> 因为它会让我们相信第三个实现是对的。
>
> 返修后本文件的断言遵守三条硬规则（已写进模块文档）：
> 1. 比**完整**内容（`edge_repr` / `node_repr` 展开全部属性），不比条数、不比单个字段；
> 2. 与**写死的期望值**比，不只比「两个输出彼此一致」；
> 3. 破坏性操作必须留一个**不该被波及的幸存者**（`removing_a_node_removes_its_incident_edges`
>    现在种一条与被删节点无关的 `x→y` 边，整体塌方当场暴露）。
>
> 另新增第 13 条用例 `edge_type_is_stored_verbatim_and_participates_in_dedup`：
> `EdgeType` 原样存取，且**参与去重键**（同一对端点上不同类型是两条不同的边）。
> 边的类型就是查询语义的全部——把 `EmbedsPage` 读成 `Contains`，「谁内嵌了谁」直接错答。

### 2.7 两条附注 —— 确认

- `graph_redb.rs:888`：每删一个节点对整个 `seen_edges` 做一次 `retain`，
  实际是 O(删除节点数 × 边总数)，与注释声称的邻接规模复杂度不符。迁移后此段代码
  随 redb 一起退役，**不单独修**。
- `core_feature_tests.rs:832`：来源类型断言接受 `Param|UserInput|Constant`、
  `Computed|Unknown`，另一处只要求原始表达式含 `model1`。这类断言**挡不住迁移引入的
  分类退化**——迁移期正是最需要它们的时候。收紧到唯一期望值。

  > **已落地（B2，commit `276d61f`）**：四条断言全部收紧到唯一 `SourceType`，
  > 并把 `expanded_expr` 与 `source_chain`（逐节点 id + 分类）一并钉住——
  > 分类只是最终标签，展开式与链条才是它由之而来的事实，只钉标签仍会漏掉中间层退化。
  > 远程验证 `FMT_EXIT=0`，`core_feature_tests` 41 passed；
  > `m59_a1b_component_value_ref_tests` 4、`m59_b4_value_trace_utf8_tests` 2 同步复跑通过。
  >
  > **收紧过程暴露两条既有缺陷**（本文新增，codex 未报）。B2 只做探针，两条都
  > **如实钉住当前行为**并在测试注释里写明它是错的，不在 B2 内修：
  >
  > 1. **`=123` 判为 `Unknown` 而非 `Constant`。** `determine_source_type`
  >    判常量的条件是「不以 `=` 开头且不以 `${` 开头」（`dependency.rs:498`），
  >    于是**公式形态的字面量**落不进 `Constant`，一路掉到兜底的 `Unknown`。
  >    名为 `test_source_type_constant` 的测试从来没验到常量分类——旧断言把
  >    `Constant|Computed|Unknown` 三个都收下，正好盖住这件事。
  >    修法属分类器本身（`=` 开头但不含任何引用的纯字面量应判 `Constant`）。
  > 2. **`RefType::ModelField` 引用不进 `source_chain`。**
  >    `expand_expression`（`dependency.rs:371-378`）对 `ModelField` 只做字符串
  >    替换、**不 push `SourceNode`**，所以 `model1` 从不出现在链条里。旧断言那道
  >    `has_model_node || raw_expr.contains("model1")` 或门正是靠**后**半边通过的，
  >    等于把缺陷藏在或门里。这条与 §3 的 C3（链路查询动词，每类边自己的投影规则）
  >    相关：模型字段是链路上的真实一跳，链条里没有它，跨模型的值溯源就断了。
  >    现改为显式断言链条**不含** `ModelAuto` 节点，行为一旦修好这条立刻变红。

### 2.8 本轮返修的验收方式：变异测试

上面 §2.1 / §2.2 / §2.3 / §2.6 四处的返修有一个共同点——**它们修的不是产品缺陷，
而是测试的断言强度**。这类修复有个天然陷阱：改完之后「测试依然全绿」什么也证明不了，
因为原来的弱断言本来也是绿的。

所以本轮的验收判据不是「测试通过」，而是**把 codex 用来揭示问题的那些故障重新注入，
看测试是否会红**。八处故障逐个注入、逐个还原：

| 注入的故障 | 位置 | 结果 |
|---|---|---|
| 边类型一律返回 `Contains` | `memory_graph_store.rs`（真实库） | 17 处断言失败 |
| 读边时丢掉 `field_path` | 同上 | 11 处断言失败 |
| 邻接视图回填错误的节点 `path` | 同上 | 5 处断言失败 |
| 删除任一存在的节点即清空所有边 | 同上 | 3 处断言失败 |
| 增量侧的边全部清空 | B5 观测边界 | 3 处断言失败 |
| 删掉 B 的节点与所有边 | B5 观测边界 | 3 处断言失败 |
| 条数不变但换掉一条字段边 | B3 观测边界 | 3 处断言失败 |
| 修复轮多出的不是那条字段边 | B3 观测边界 | 3 处断言失败 |

**8/8 被断言 panic 捕获，0 处编译失败**（编译失败会伪装成「被捕获」，因此校验脚本
先查 `error[E`、再查断言失败，并打印命中的 `panicked at` 行确认它落在新断言上）。
B1 的四处注入在**真实库**里做，不是在测试里模拟——那正是 codex 的原始复现方式。

全量回归（`99ec146`）：`cargo test --features cli-local` `FULL_EXIT=0`，
97 个测试二进制、**1189 passed / 0 failed**（基线 1187，+2 为 B1 新增用例 × 2 个实现），
`FMT_EXIT=0`。

### 2026-09-06 补充：完整快照与 metadata 断言

`c9b1611` / `df394f2` 进一步收紧 B1/B3/B5：非空边 metadata、完整端点更新、
修复后完整节点内容，以及节点/边比较的重复次数。远端 39 项目标测试通过；丢弃 metadata、
污染修复内容、B3 重复边、B5 重复边四类故障均被断言拒绝。
指定主 style/smell reviewer 对该三文件批次 PASS，未覆盖整个 PR14。
B5 仍记录的跨文件边丢失与占位节点残留必须在 A3 修复。

## 3. 迁移顺序

```
阶段 A  身份与事实定稿（schema 输入，必须先于落库）
        A1 F4/PR4b 身份文法：项目 + 源文件 + 局部 id，旧 target 经显式解析 + 歧义诊断兼容
        A1b ComponentValue 保留原始引用 token（.value / .step / 裸 id），
            使值追溯的替换 pattern 与来源文法一致——见 §2.3 的新增缺陷
            —— 值追溯半边**已完成**（f5d35b9 / 20921ad）；枚举元数半边（103 处
            调用点）并入 A1 一起改
        A2 路径归一化统一函数（. / .. / 分隔符 / 越界），扫描与引用解析共用
        A3 边带 origin_file；删除按 origin 而非按端点牵连
           —— **范围经 B5 修正**：节点同样要带 origin_file，否则占位节点仍会泄漏（§2.1）
        A4 PR4a：schema 版本键 + fail-closed + 强制全量重建（迁移的开关本身）

阶段 B  契约与正确性（先于 Grafeo 实现）
        B1 共享 GraphStore 契约测试套件：同一组用例跑 memory / redb 两实现
           覆盖重复边、节点更新、占位节点升级、删除、邻接一致性
           —— **已完成**（`4962412` / `e4b1106`；断言强度返修 `a681499`），见 §2.6；
              C1 加一个 impl 分支即接入
        B2 收紧 core_feature_tests 的宽松断言（迁移期的退化探针）
           —— **已完成**（`276d61f`），见 §2.7；顺带钉住两条既有分类缺陷
        B3 TBL 解析失败不再返回 Ok(空)：验证通过才动候选图，否则保留旧图 + 陈旧标记
           —— **已完成**（`f8091c5` / `eaeae4b`；断言强度返修 `a681499`），见 §2.2
        B4 值追溯 panic 与中文乱码：改 token/span 定位替换，保留原始字符串片段
           —— **已完成**（`3f84a96` / `c99a14d`；断言强度返修 `99ec146`），见 §2.3
        B5 全量 vs 增量差分测试：同一最终文件集，逐节点/逐边/逐属性比对（非计数比对）
           —— **已完成**（`0266fcb` / `cad7335` / `34f5a5d`；断言强度返修 `a681499`），
              见 §2.1；顺带发现占位节点归属泄漏，**A3 需同时给节点加 origin_file**

阶段 C  Grafeo 落地
        C1 GrafeoGraphStore 实现 GraphStore/GraphWriteStore，跑通 B1 契约套件
        C2 直写导入路径（create_node_with_props / create_edge，自持 NodeId）
           —— 实测比逐条 Cypher 快 60 倍；建 property index
        C3 链路查询动词：覆盖 22 个 EdgeType 里的 21 个，每类边自己的投影规则
        C4 表投影层（节点层）：只读视图 + WHERE + 投影 + LIMIT + TSV/JSON
        C5 wasm32 实测：cargo build --target wasm32-unknown-unknown --features browser-wasm

阶段 D  验收与退役
        D1 真实语料复测（语料已在 dev workspace 可达，见 §4.1）：
           冷启动→首条链路查询、内存驻留、产物正确性
        D2 redb 退役：v2 shadow 族、坏行 hydrate 计数、graph_redb.rs:888 一并移除
```

**A → B → C 的顺序不可交换**：A 定 schema，B 给出「Grafeo 实现是否正确」的判据，
C 才有意义。

## 4. 已知阻塞项

1. ~~**真实语料在 dev workspace 不可达。**~~ —— **已解除**（2026-09-06）。

   最终诊断：dev 环境注入的 `CNB_TOKEN` 是用户态身份（`GET /user` 返回 `wu2305`），
   但授权范围只到本仓——实测 `GET /wu2305/succbi_project_container` 返回 **403**
   （不是 404，仓库存在且归属正确），`GET /wu2305/metadata-checker` 返回 200。
   跨仓凭据只能来自 `imports` 的 `metadata-checker-keys/real-fixture.yml`，
   而打开这一行要连过该密钥文件自身的**两道**声明：

   | 门 | 报错 | 处置 |
   |---|---|---|
   | `allow_events` | `event: vscode does not conform to allow_events of …/real-fixture.yml`（sn=`cnb-ao8-1k1ns62sh`） | 仓库所有者把 `vscode` 加进 `allow_events` |
   | `allow_branches` | `branch: codex/m58-slm-eval-foundation does not conform to allow_branches of …/real-fixture.yml`（sn=`cnb-j08-1k1p16s8b`） | 仓库所有者放开该分支。注意 `codex*` **匹配不到** `codex/m58-…`——`*` 不跨 `/` |

   任一道门关着，Prepare 直接失败、8 个 stage 全部 skipped，`build-runner-download-log`
   返回 404（未产出日志）。定位方法见
   [CNB 远程环境 runbook](../runbooks/cnb-remote-dev-env.md) §「Prepare 阶段失败怎么定位」。

   `.cloud_native_dev_env` 的 `fetch real project corpus` stage 随后经 7 轮返修
   （`4dc96bd`..`3c30fbd`）收紧为：语料仓、提交 pin、项目路径全部由 `real-fixture.yml`
   提供（不再硬编码 `xiaoshouyi-corpus` 分支与 `projects/xiaoshouyi`）；pin 必须是
   40 位 SHA，路径必须是仓库内安全相对目录；缓存复用前校验 HEAD/origin/worktree
   干净度，并把工作树 `.spg`/`.tbl` 计数与 pinned commit 的 tracked blob 计数逐一对齐
   （中文文件名须 `core.quotePath=false`，否则真实 metadata 会被漏计为 0）；凭证只走
   `http.extraHeader`，不落盘、不进 remote URL，并拒绝复用任何持久化了 extraheader
   或 userinfo URL 的缓存仓。全程 fail-soft：拉不到就打印 `CORPUS_UNAVAILABLE` 后
   继续开环境。

   实测通过（sn=`cnb-ubl-1k1qnvicp`，2026-09-06）：

   ```
   success,32916,Prepare,prepare
   success,2668,fetch real project corpus,stage-5
   [corpus] READY spg=501 tbl=828 sha=c3c0528fdd28e2600e0b0235040fb349b3c2d446 path=xiaoshouyi
   157M    target/real-project/succbi_project_container/xiaoshouyi
   ```

   取舍已明确记账：该部署 token 会注入到**可交互 SSH 的开发环境**，而不只是短生命
   周期流水线。这是决定，不是疏漏。收益是 D1 与本节第 2 条不再需要专门的
   `api_trigger_*` 事件，可在 dev 环境交互式迭代。

   未采用 submodule：语料 156 MB，submodule 会进每一次 clone 与 CI checkout，
   而现有流水线是刻意用 `--depth 1 --filter=blob:none --sparse` 规避这个开销的。
2. ~~**内存驻留在真实语料上未知。**~~ —— **已解除**（2026-09-06）。实测记录见
   [M59 真实语料实测](../ai-eval-runs/2026-09-06-m59-real-corpus-measurements.md) §4。

   读数（真实语料、stdio 加载路径、`VmRSS`）：**3,805.7 MiB = 3.72 GiB** 驻留，
   `VmHWM` 3.78 GiB，加载耗时 1,148.7 s，节点 89,178 / 边 200,028。

   原先按合成图 16 倍膨胀外推的 **8 GB 偏高约 2.1 倍**：真实语料是
   514.0 MiB 落盘 → 3,805.7 MiB 驻留，**7.4 倍**。但结论方向不变，
   单进程常驻近 4 GB 不能忽略。

   **对 A4 的直接输入**：`VmHWM − VmRSS` 仅 60.7 MiB，即峰值几乎全部是常驻结构、
   没有大块临时缓冲。因此**瘦身节点 meta 的收益一比一落在常驻内存上**——
   「要不要瘦身 meta」从此是可以算账的，不是猜。摊薄下来约 13.5 KiB /
   每节点或边，远超单个组件节点的真实语义内容，差额在 meta 与索引结构里。

   注意勿代用另外两个数：建图路径峰值 1,693 MiB 是**写入侧**；
   `--check-graph` 的 6 MiB 只读文件头、根本没加载图。

   顺带确认：`SCANNER_UNRECOGNIZED_CONTAINER_KEY` 312 与
   `SCANNER_DUPLICATE_COMPONENT_ID` 1,105 两条陈旧诊断基线**实测仍准确**。

3. **`with_read_store` 未评估。** `GrafeoDB::with_read_store(Arc<dyn GraphStoreSearch>, Config)`
   允许只借 Cypher 引擎、不迁数据。既然已决定换库，此路仅作为 C1 受阻时的退路记录。
