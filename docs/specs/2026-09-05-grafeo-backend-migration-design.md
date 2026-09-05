# Grafeo 后端迁移设计

> 状态：**approved**（2026-09-05 用户决策：「别查了，直接换」）
> 里程碑：M59（新开；redb 退役）
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

### 2.2 P1｜损坏 TBL 被当成功的空结果提交 —— 确认

`tbl.rs:14-22`：`content.is_empty()` 与 `serde_json::from_str` 的 `Err(_)` 都返回
`Ok(空集)`。配合 §2.1 的先删后建：一个原本正常的 `.tbl` 写入半截内容 →
旧模型图被删 → 新解析失败返回空 → **file hash 照常记录** → 下次内容不变直接跳过。

「读取失败」被静默转成「模型不存在」，无 `PARSE_FAILED` 类诊断。

**对迁移的含义**：提交语义问题。新库要么在解析验证通过后才动候选图，要么保留上一份
有效图并标记陈旧。**不能在迁移时沿用「失败即空集」**。

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

### 2.7 两条附注 —— 确认

- `graph_redb.rs:888`：每删一个节点对整个 `seen_edges` 做一次 `retain`，
  实际是 O(删除节点数 × 边总数)，与注释声称的邻接规模复杂度不符。迁移后此段代码
  随 redb 一起退役，**不单独修**。
- `core_feature_tests.rs:832`：来源类型断言接受 `Param|UserInput|Constant`、
  `Computed|Unknown`，另一处只要求原始表达式含 `model1`。这类断言**挡不住迁移引入的
  分类退化**——迁移期正是最需要它们的时候。收紧到唯一期望值。

## 3. 迁移顺序

```
阶段 A  身份与事实定稿（schema 输入，必须先于落库）
        A1 F4/PR4b 身份文法：项目 + 源文件 + 局部 id，旧 target 经显式解析 + 歧义诊断兼容
        A2 路径归一化统一函数（. / .. / 分隔符 / 越界），扫描与引用解析共用
        A3 边带 origin_file；删除按 origin 而非按端点牵连
        A4 PR4a：schema 版本键 + fail-closed + 强制全量重建（迁移的开关本身）

阶段 B  契约与正确性（先于 Grafeo 实现）
        B1 共享 GraphStore 契约测试套件：同一组用例跑 memory / redb 两实现
           覆盖重复边、节点更新、占位节点升级、删除、邻接一致性
        B2 收紧 core_feature_tests 的宽松断言（迁移期的退化探针）
        B3 TBL 解析失败不再返回 Ok(空)：验证通过才动候选图，否则保留旧图 + 陈旧标记
        B4 值追溯 panic 与中文乱码：改 token/span 定位替换，保留原始字符串片段
        B5 全量 vs 增量差分测试：同一最终文件集，逐节点/逐边/逐属性比对（非计数比对）

阶段 C  Grafeo 落地
        C1 GrafeoGraphStore 实现 GraphStore/GraphWriteStore，跑通 B1 契约套件
        C2 直写导入路径（create_node_with_props / create_edge，自持 NodeId）
           —— 实测比逐条 Cypher 快 60 倍；建 property index
        C3 链路查询动词：覆盖 22 个 EdgeType 里的 21 个，每类边自己的投影规则
        C4 表投影层（节点层）：只读视图 + WHERE + 投影 + LIMIT + TSV/JSON
        C5 wasm32 实测：cargo build --target wasm32-unknown-unknown --features browser-wasm

阶段 D  验收与退役
        D1 真实语料复测（需新增 api_trigger_* 流水线事件，见 §4）：
           冷启动→首条链路查询、内存驻留、产物正确性
        D2 redb 退役：v2 shadow 族、坏行 hydrate 计数、graph_redb.rs:888 一并移除
```

**A → B → C 的顺序不可交换**：A 定 schema，B 给出「Grafeo 实现是否正确」的判据，
C 才有意义。

## 4. 已知阻塞项

1. **真实语料在 dev workspace 不可达——机制已就位，等一个授权决定。**

   诊断更正：dev 环境注入的 `CNB_TOKEN` **是用户态身份**——`GET /user` 返回
   `wu2305`——但**授权范围只到本仓**：实测 `GET /wu2305/succbi_project_container`
   返回 **403**（不是 404，仓库存在且归属正确），`GET /wu2305/metadata-checker`
   返回 200。此前记为「仓库级 token」方向对但机制说错了。

   与 imports 也无关：`.kimi_harness_smoke_ci` 同样没有 imports，它用的就是
   `$CNB_TOKEN`——差别在于**流水线的 CNB_TOKEN 有跨仓授权，dev 环境的没有**。

   已做：`.cloud_native_dev_env` 新增 `fetch real project corpus` stage，用
   `REAL_PROJECT_FIXTURE_DEPLOY_TOKEN` 做 sparse clone；fail-soft、已存在则跳过、
   凭证只走 `http.extraHeader` 不落盘、末尾断言未持久化进 `.git/config`。
   实测 stage 跑通（sn=`cnb-vo8-1k1nsqnju`，`success,474,fetch real project corpus`，
   走 fail-soft 分支，环境正常启动）。

   **仍缺一步（需人决策）**：`imports: real-fixture.yml` 现在**打不开**——该密钥
   文件自身声明了 `allow_events`，`vscode` 不在其中，加上会让 Prepare 阶段直接失败、
   整个开发环境起不来（实测 sn=`cnb-ao8-1k1ns62sh`）：

   ```
   Pipeline init error: event: vscode does not conform to allow_events of
   https://cnb.cool/wu2305/metadata-checker-keys/-/blob/main/real-fixture.yml
   ```

   两条出路，**都要人拍板**：

   - **A**：在 `metadata-checker-keys` 的 `real-fixture.yml` 里把 `vscode` 加进
     `allow_events`，再取消 `.cnb.yml` 里那两行注释。代价是该部署 token 会被注入到
     **可交互 SSH 的开发环境**，而不只是短生命周期流水线。收益是 M59 可以在真实语料
     上交互式迭代。
   - **B**：不动 `allow_events`，把真实语料实测做成 `api_trigger_m59_grafeo` 流水线
     事件（该类事件已在 `allow_events` 内）。安全面不变，但只能批处理跑，无法交互
     调试。

   未采用 submodule：语料 156 MB，submodule 会进每一次 clone 与 CI checkout，
   而现有流水线是刻意用 `--depth 1 --filter=blob:none --sparse` 规避这个开销的。
2. **内存驻留在真实语料上未知。** 合成图 16.5 MB 落盘 → 262 MB RSS（16 倍）。
   真实语料 514 MiB 若同比例约 **8 GB**。这是 D1 必测项，也可能反过来影响 schema
   （是否要瘦身 meta）。
3. **`with_read_store` 未评估。** `GrafeoDB::with_read_store(Arc<dyn GraphStoreSearch>, Config)`
   允许只借 Cypher 引擎、不迁数据。既然已决定换库，此路仅作为 C1 受阻时的退路记录。
