# M9-A 真实语料 Manifest

## 状态

- M9-A 已完成：`tests/fixtures/corpus/manifest.json`
- 当前仅建立样本索引与覆盖矩阵，不复制真实项目大文件，不修改解析逻辑

## 文件说明

- `tests/fixtures/corpus/manifest.json`：机器可读样本清单
- `tests/fixtures/corpus/README.md`：语料目录约束与注意事项

Manifest 顶层字段：

- `schema_version`：manifest 结构版本
- `generated_at`：生成时间（UTC）
- `sample_count`：样本总数
- `samples`：逐文件记录
- `coverage_matrix`：按 tag 汇总的样本覆盖
- `coverage_gaps`：未命中的 tag 列表（若为空表示当前矩阵无缺口）

每个 `samples[]` 记录至少包含：

- `stable_id`
- `source_kind`（`real_project` 或 `fixture`）
- `path`
- `file_type`（`spg` 或 `tbl`）
- `size_bytes`
- `coverage_tags`（含 `tag` + `detection_method`）

## 来源范围（白名单）

- 真实项目：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- fixture：`tests/fixtures/real_world_*.spg`、`tests/fixtures/test_project/**`

禁止来源：

- `/Users/wuhaocheng/Downloads/bi`

## 覆盖标签与识别方式

`coverage_tags` 由文本启发式扫描得到，不等同于语义解析结论。关键标签包括：

- `actions`
- `DataFlow`
- `dialog`
- `conditionExp`
- `visibility`
- `waitPrev`
- `link_param`
- `refresh_action`
- `submit_write`
- `embedded_page`
- `duplicate_action_id`
- `readonly_page`

另外补充 `showDialog` 与 `refreshData` 两个动作级 tag，便于直接回答样本覆盖问题。

`readonly_page` 当前采用保守启发式：

- 仅对 `.spg` 生效
- 不含写入类动作：`submitData/updateData/insertData/deleteData/newData/copyDataTo/importData/exportData/executeFlow/batchStartFlow/webAPI/script`
- 且满足以下其一：
  - 页面 action 全部落在只读/导航白名单（如 `link/showDialog/closeDialog/switchPanel/showComponent/hideComponent/refresh/load/validate/maximize/goBack/setParamValue/scroll` 等）
  - 页面存在结构信号（`canvas/components/sources/visibility/referenceResources`）且无 action
- 该规则为文本启发式，可能存在误报/漏报

`readonly_page` 样本示例：

- `fixture:tests/fixtures/real_world_4.spg`
- `fixture:tests/fixtures/test_project/app/page_relations.spg`
- `fixture:tests/fixtures/test_project/app/visibility_contract.spg`

## 维护方式

1. 扫描白名单来源的 `.spg`/`.tbl` 文件并更新 `manifest.json`
2. 变更后运行 `cargo test`（含 `tests/corpus_manifest_tests.rs`）
3. 如标签规则调整，需同步更新：
   - `manifest.json` 内 `coverage_tags[].detection_method`
   - 本文档中的“覆盖标签与识别方式”

## 快速查询示例

查看 `showDialog` 覆盖样本：

```bash
jq '.coverage_matrix.showDialog.sample_ids' tests/fixtures/corpus/manifest.json
```

查看 `DataFlow`/`waitPrev`/`visibility`/`refreshData`/`conditionExp` 覆盖计数：

```bash
jq '.coverage_matrix | {DataFlow, waitPrev, visibility, refreshData, conditionExp}' tests/fixtures/corpus/manifest.json
```
