# M9-C Snapshot 输出契约

## 目录结构

```
tests/fixtures/corpus/snapshots/
├── README.md                    # 本文件
├── snapshot_cases.json          # snapshot case 定义清单
├── query_page_logic_actions_test.json
├── explain_button1_actions_test.json
├── context_button1_depth1.json
├── query_dataflow_output.json
└── explain_model_dataflow_output.json
```

## Snapshot 格式说明

每个 snapshot 是**规范化后的结构化摘要**，不是完整原始 JSON 输出。只保留稳定关键信息：

| 字段 | 说明 |
|------|------|
| `schema_version` | 输出契约版本 |
| `kind` | 输出类型（PageLogic/Explain/Context/DataFlowQuery/...） |
| `query_target` | 查询目标 ID |
| `summary` | 过滤后的摘要对象，保留：计数字段、枚举字段（page_role/importance/action_category）、短文本（≤100字符）、小数组（≤5项） |
| `diagnostics` | 诊断列表，每项仅保留 `code` 和 `severity` |
| `evidence` | 证据列表，每项仅保留 `claim`、`edge_type`、`confidence` |
| `details_counts` | details 中各数组的计数，例如 `action_flows_count`、`entrypoints_count`、`write_targets_count`、`lineage_count`、`internal_topology_nodes_count` |

**不保留的内容**（避免误报）：
- 绝对路径
- 大段 raw JSON
- evidence 的 source_file/json_path/raw_expr
- diagnostics 的 message/suggestion（文案变化不影响 snapshot）
- summary 中长描述文本

## 如何更新 snapshot

```bash
# 重新生成所有 snapshot
UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests

# 或仅运行 snapshot 测试（比较模式，不更新）
cargo test --test corpus_snapshot_tests
```

**允许更新 snapshot 的场景**：
- 输出契约的预期变化（例如新增 summary 字段、调整诊断码）
- 修复真实 bug 后输出变化
- 新增 case

**不允许更新 snapshot 的场景**：
- 无意义的排序差异（应通过规范化逻辑消除）
- 路径差异（应通过规范化逻辑消除）
- 未审查的输出漂移

## 如何新增 case

1. 在 `snapshot_cases.json` 的 `cases` 数组中新增 entry
2. 指定 `case_id`、`command_kind`、`target`、`snapshot_path`
3. 运行 `UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests`
4. 审查生成的 snapshot 文件
5. 更新本 README 的目录结构列表
