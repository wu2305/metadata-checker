# M58.3 快照断言变迁台账

> 依据 spec `2026-08-24-m58-3-command-surface-gap-fixes-design.md` 验收第 5 条：
> 任何断言值变化必须记录 `case_id / 断言 / 旧值 / 新值 / 源证据命令 / 复核人决定`，
> 由独立验收者复核，不允许「数值变好」自动通过。
> 本台账为补录（2026-08-26）：以下变化已随 PR #14 提交，待独立验收者复核确认。

## b3d67d3 — PR2 落地后 corpus 快照重建

- **case_id**：`tests/corpus_snapshot_tests.rs` 全量快照（fixture corpus）
- **断言**：组件/边计数类字段（各快照的 counts、related 集合规模）
- **旧值**：白名单递归口径（组件数 26,007，见 spec 附录 A）
- **新值**：形态感知递归口径（组件数 34,317，+32%），新增 comp→comp Contains 与
  继承链数据上下文边
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --features cli-local --test corpus_snapshot_tests`；
  交叉证据 `python3 tools/corpus-shape-audit.py --corpus <xiaoshouyi>`（
  `docs/ai-eval-runs/2026-08-25-m58-3-corpus-shape-audit-post-pr2.json`，
  `acceptance_gap_missed_minus_exclusion == 0`）
- **复核人决定**：PR2（spec F1/F2）设计内的口径扩展，非回归

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
- **复核人决定**：主线程逐字段 diff 复核，新增边均为真实引用，排名变化为修复预期后果；
  待独立验收者复验

## 312c938 — severity 映射表修复（dbfe4b8）后的快照重建

- **case_id**：`query_page_logic_actions_test`
- **断言**：`diagnostics[EVIDENCE_SAMPLED].severity`
  - 旧值：`Warning`；新值：`Info`
  - 原因：`severity_for` 原实现忽略入参恒返回 Warning，`EVIDENCE_SAMPLED`（按设计采样）
    被静默升档；显式映射表恢复其意图 severity。`UNKNOWN_ACTION_TYPE` 保持 Warning 不变
- **源证据命令**：`UPDATE_CORPUS_SNAPSHOTS=1 cargo test --features cli-local --test corpus_snapshot_tests`
- **复核人决定**：severity 回归设计意图，非内容变化；待独立验收者复验
