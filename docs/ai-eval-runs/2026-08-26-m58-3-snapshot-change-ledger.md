# M58.3 快照断言变迁台账

> 依据 spec `2026-08-24-m58-3-command-surface-gap-fixes-design.md` 验收第 5 条：
> 任何断言值变化必须记录 `case_id / 断言 / 旧值 / 新值 / 源证据命令 / 复核人决定`，
> 由独立验收者复核，不允许「数值变好」自动通过。
> 本台账为补录（2026-08-26）；2026-09-06 已补独立源码复核，范围与保留缺口见文末。

## b3d67d3 — PR2 落地后 corpus 快照重建

> 2026-09-05 hostile review 补登：原条目把「旧值/新值」登记为审计脚本候选数
> （26,007→34,317），不是任何快照断言字段，且漏登记断言 5 的 severity 翻转。
> 以下按 spec 验收第 5 条格式逐断言重登。

- **case_id**：`tests/corpus_snapshot_tests.rs` 全量快照（fixture corpus）
- **断言 1**：`context_button1_depth1.json` 的 `related_components_count`
  - 旧值：0；新值：1
  - 原因：PR2 comp→comp Contains 边落地，button1 的父容器进入 related_components
- **断言 2**：`context_button1_depth1.json` 的 `related_nodes_count`
  - 旧值：2；新值：3
  - 原因：同上，Contains 边带入一个新增关联节点
- **断言 3**：`query_page_logic_actions_test.json` 的 `primary_paths_count`
  - 旧值：35；新值：36
  - 原因：新 Contains 路径进入路径发现
- **断言 4**：`query_page_logic_actions_test.json` 的 `related_context_count`
  - 旧值：86；新值：81
  - 原因：F2 祖先链继承口径变化
- **断言 5（补登，原条目漏登记）**：`query_page_logic_actions_test.json` 的
  `diagnostics[EVIDENCE_SAMPLED].severity`
  - 旧值：`Info`；新值：`Warning`
  - 原因：**把 bug 输出锁成了期望值**——PR1 初版 `severity_for` 忽略入参恒返回
    Warning，`EVIDENCE_SAMPLED`（按设计采样，意图 Info）被静默升档，本提交重建
    快照时把错误的 Warning 固化为期望值。后由 `dbfe4b8`（severity_for 显式映射表）
    修复、`312c938` 重建快照回归 Info（见下方条目）。溯源链在此补齐
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --features cli-local --test corpus_snapshot_tests`；
  交叉证据 `python3 tools/corpus-shape-audit.py --corpus <xiaoshouyi>`（
  `docs/ai-eval-runs/2026-08-25-m58-3-corpus-shape-audit-post-pr2.json`，
  `acceptance_gap_missed_minus_exclusion == 0`；34,317 为审计脚本候选数，非快照断言）
- **复核人决定**：断言 1–4 为 PR2（spec F1/F2）设计内的口径扩展，非回归；断言 5 为
  缺陷显影，已按上述链路修复回归（2026-09-05 独立验收复核确认补登）

## 41c011a — 前向引用修复（feb5ad8）后的快照重建

- **case_id**：`query_page_logic_actions_test`（fixture `app/actions_test.spg`）
- **断言 1**：`summary.related_context_count`
  - 旧值：81；新值：88
  - 原因：`feb5ad8` 两遍建图后，fixture 中 7 条前向引用 DependsOn/Contains 边不再被
    静默丢弃（此前目标组件节点未注册，`add_edge` 跳过）
- **断言 2**：`summary.key_primary_paths[1]` 与 `[2]` 的路径身份
  - 旧值：`input1 -> field:model1.name -> field:table1/app_table.name`
    （path_length 2，same_page false）
  - 新值：`buttonValidate -> actionValidate -> field:model1.name -> ...`
    （path_length 3，same_page true，含 Triggers 段）
  - 原因：补齐的边改变了路径发现/排名输入；新路径含真实 entrypoint（组件→动作→字段）
    链路，信息更完整。注：`same_page` 字段的跨文件误标是更早起因的既有语义问题
    （见复核记录 P2-8），不在本台账变化范围内
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --features cli-local --test corpus_snapshot_tests`；
  重建后全量 `cargo test --features cli-local` 88 目标 1109 通过
- **复核人决定（2026-09-06 更新）**：独立复核确认 buttonValidate→actionValidate 的
  Triggers 身份与 fixture 一致，前向引用补齐解释了路径发现输入变化。**不判整条路径排名
  语义 PASS**：same_page 与 contains_physical_field 仍有已知缺口，见文末 M59-PATH 交接。

## 312c938 — severity 映射表修复（dbfe4b8）后的快照重建

- **case_id**：`query_page_logic_actions_test`
- **断言**：`diagnostics[EVIDENCE_SAMPLED].severity`
  - 旧值：`Warning`；新值：`Info`
  - 原因：`severity_for` 原实现忽略入参恒返回 Warning，`EVIDENCE_SAMPLED`（按设计采样）
    被静默升档；显式映射表恢复其意图 severity。`UNKNOWN_ACTION_TYPE` 保持 Warning 不变。
    注：旧值 Warning 是 `b3d67d3` 在 bug 存续期重建快照时锁入的（该条目已于
    2026-09-05 补登此翻转，见上方断言 5）
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --features cli-local --test corpus_snapshot_tests`
- **复核人决定（2026-09-06）**：独立源码复核 PASS；severity 恢复为 Info，
  UNKNOWN_ACTION_TYPE 仍为 Warning。构造点与权威映射见文末。

## 3f5960f 后的快照重建（条件符号经页面上下文归一 / 空字段名不再造尾点节点）

- **case_id**：`query_page_logic_actions_test`（fixture `app/actions_test.spg`）
- **断言 1**：`summary.top_data_prerequisites[2].depends_on[0]`
  - 旧值：`model:name.`；新值：`model:name`
  - 原因：`3f5960f` 的 `ensure_model_field` 在字段名为空时不再建字段节点，
    `model_field_path()` 也不再拼出尾点。旧值 `model:name.` 是尾点垃圾节点被
    锁进期望值的结果
- **断言 2**：`details_counts.data_sources_count` 与 `summary.data_source_count`
  - 旧值：28；新值：27
  - 原因：尾点节点 `model:name.` 与真实节点 `model:name` 原本各占一个数据源，
    消除尾点后二者合一
- **断言 3**：`summary.related_context_count`
  - 旧值：88；新值：87
  - 原因：同上，少一个垃圾节点进入关联上下文
- **断言 4**：`evidence[16].claim` 文案
  - 旧值：`Page page:app/actions_test.spg reads from 28 data sources`
  - 新值：`... reads from 27 data sources`
  - 原因：断言 2 的计数在证据文案中的投影，非独立变化
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests`
  （CNB 远端 workspace `cnb-abg-1k1ofgjjt`）。重建后全量回归
  `cargo test --features cli-local --no-fail-fast`：**93 目标 1,148 通过 / 0 失败 /
  24 忽略，退出码 0**；`cargo fmt --all -- --check` 干净
- **复核人决定**：四条断言同源于一个缺陷修复，方向均为**消除垃圾节点**，
  新值为正确值，非回归。
- **流程缺陷（自查登记）**：本次不一致是 `3f5960f` 落地时**当场就该暴露**的，
  但当时的影响面测试集（30 个 suite）**未包含 `corpus_snapshot_tests`**，
  直到 M59-B4 跑 `cargo test --tests` 全量才发现。教训：涉及节点身份/符号文法的
  改动，影响面集合必须显式包含快照套件，不能靠挑选相关 suite。


## 2026-09-06 独立复核与范围收口

复核者：独立只读 agent `snapshot_closeout_review`（gpt-5.6-luna / max），基点
`df394f2153b16f26b03126185638fcbed699fb03`；主线程随后核对所列源码、fixture 和提交 diff。
本轮未修改 snapshot 或运行时源码。运行证据为该基点 CNB `cnb-gn8-1k1rhce0o` 的
Rust test/coverage 全绿；以下语义判定来自源码与 fixture，不由快照重建成功推导。

| 条目 | 源证据与复核决定 | 保留边界 |
|------|----------------|----------|
| 41c011a：前向引用后的路径身份 | `git show 41c011a -- tests/fixtures/corpus/snapshots/query_page_logic_actions_test.json`；`tests/fixtures/test_project/app/actions_test.spg:210-221` 的 buttonValidate/actionValidate；`git show feb5ad8 -- src/scanner/spg.rs` 的两遍建图。确认新增 Triggers 身份与发现输入变化 | 路径特征的整体语义不通过；本条按已证实路径身份收口，不将全部 rank_features 作为正确答案 |
| 312c938：Info 恢复 | `src/diagnostics.rs:92-113` 的 severity_for + `src/query/page_logic/diagnostics.rs:385-403` 的 envelope_diagnostic 调用；当前 snapshot 中 EVIDENCE_SAMPLED=Info、UNKNOWN_ACTION_TYPE=Warning。PASS | 只覆盖该 severity 变迁 |
| 3f5960f：尾点消除 | `src/scanner/spg.rs:348-381` 的 ensure_model_field 空字段分支不造字段节点；`src/conditions.rs:163-166` 生成 model:name；snapshot 从 model:name. 改为 model:name、数据源 28→27。窄范围 PASS | 通用表达式路径仍可能留下模型级 model:name；不把此修复当页面局部身份已解决 |
| 51907af：PR2 性能基线 | `git show --stat 51907af` 仅修改 performance-baseline 与 spec；其记录可作为历史测量证据 | 它不是路径排名快照提交，不可用来证明 41c011a 的语义 |

**M59-PATH：路径标记缺口，deferred 到 M59-4。** `src/path.rs:934-935` 只需任一段两端
同页就设 same_page=true；当前 snapshot 中到 data/table1.tbl 的跨文件路径仍标
same_page=true（source_file_count=2）。`src/path.rs:938-942` 以 field_path 字符串特征
识别物理字段，导致 field:model1.name→field:table1.name 路径 contains_physical_field=false。
41c011a 的 diff 显示后一个 false 原已存在；same_page 的错误判定算法也未由该快照提交修复。

接手者为 [M59-4 查询执行包](../plans/2026-09-06-m59-grafeo-implementation-plan.md)。
关闭条件：按整条路径的源范围判定 same_page，按真实节点/字段来源识别物理字段；同页、
跨页、跨 SPG/TBL 与普通 name 字段均有精确回归，再独立复核快照变化。此项保留为明确的
查询语义缺口；本次台账收口不宣称其已修复，也不代表整个累计 PR14 的语义验收。
