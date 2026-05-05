# M9 Corpus 目录说明

## 目录结构

```
tests/fixtures/corpus/
├── README.md                    # 本文件
├── manifest.json                # M9-A：完整语料索引（仅元信息）
├── selection.json               # M9-B：回归测试专用精选语料清单
├── samples/                     # M9-B 从真实项目复制的样本文件
│   ├── app__销售.app__销售__客户.spg
│   ├── app__售后.app__待开发__验证URL传参.spg
│   ├── app__关联员工企微.app__SuperPage.spg
│   ├── app__售后.app__维修保养预约单__预约__预约成功.spg
│   └── app__销售.app__标签__标签库创建.spg
└── snapshots/                   # M9-C：规范化输出契约 snapshot
    ├── README.md
    ├── snapshot_cases.json
    ├── query_page_logic_actions_test.json
    ├── explain_button1_actions_test.json
    ├── context_button1_depth1.json
    ├── query_dataflow_output.json
    └── explain_model_dataflow_output.json
```

## M9-A vs M9-B 区别

| 维度 | M9-A manifest | M9-B selection |
|------|---------------|----------------|
| 目的 | 完整索引，供抽样和审计 | 稳定回归，供 `cargo test` 直接执行 |
| 体积 | 仅元信息，~100KB | 精选样本 + 引用 fixture，~8KB |
| 数量 | 1290 条记录 | 12 条（5 copied + 7 referenced） |
| 更新频率 | 按需重新扫描 | 里程碑变更时审核更新 |
| 包含大文件 | 是（索引） | 否（单文件 <5KB，总 <20KB） |

## 样本选择原则

1. **代表性优先**：每个样本覆盖至少一个 M9-B 要求的风险点
2. **体积受控**：复制的真实项目样本单文件 <5KB，优先选择更小的替代
3. **覆盖风险点**：
   - 复杂页面（conditionExp + submit_write + visibility）
   - 只读页面（readonly_page）
   - 表单写入页（submit_write）
   - 弹窗/dialog 页（dialog）
   - 嵌套 visibility 页（visibility）
   - DataFlow .tbl（DataFlow）
   - 链式物理表/DataFlow（df_a → physical_x → df_b）
   - 刷新类 action（refreshData/refreshModels）
   - conditionExp（条件表达式）
   - waitPrev（等待前置动作）
   - link_param（参数传递）
   - duplicate_action_id（重复 action ID）
4. **两类样本**：
   - `copied`：从真实项目复制到 `samples/`，确保测试不依赖外部路径
   - `referenced`：引用仓库内已有 `tests/fixtures/test_project/` fixture，避免重复

## M9-C Snapshot 输出契约

M9-C 在 M9-B 基础上建立**规范化 snapshot**，确保输出 contract 变化可被 diff 捕获。

| 维度 | M9-B selection | M9-C snapshots |
|------|--------------|----------------|
| 目的 | 验证文件可解析、图可构建 | 验证输出契约稳定、语义一致 |
| 内容 | 文件元信息 + 解析测试 | 规范化后的 AiOutput 摘要 |
| 体积 | ~8KB | ~5 个 snapshot 文件，总计 ~10KB |
| 触发方式 | `cargo test --test corpus_tests` | `cargo test --test corpus_snapshot_tests` |
| 更新方式 | 手动编辑 selection.json | `UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests` |

### Snapshot 格式

Snapshot 是**结构化摘要**，不是完整 JSON dump：
- 保留：schema_version、kind、query_target、summary 关键字段、diagnostics code/severity、evidence claim/edge_type/confidence、details 数组计数
- 去除：绝对路径、大段 raw JSON、diagnostics message、summmary 长描述

### Snapshot case 覆盖

| case_id | command | 覆盖 contract |
|---------|---------|---------------|
| query_page_logic_actions_test | --query-page-logic | PageLogic、action_flows、visibility |
| explain_button1_actions_test | --explain comp:... | Explain、reads/writes/triggered_by |
| context_button1_depth1 | --context --budget normal | Context、related_actions、budget |
| query_dataflow_output | --query-dataflow | DataFlowQuery、inputs/outputs |
| explain_model_dataflow_output | --explain model:... | Explain(DataFlow)、lineage |

## 如何更新 selection

1. 修改 `tests/fixtures/corpus/selection.json`
2. 如新增 `copied` 样本，先复制文件到 `samples/`
3. 更新本 README 的目录结构列表
4. 同步更新 `tests/corpus_tests.rs` 中的测试断言
5. 运行 `cargo test --test corpus_tests` 验证

## 外部依赖

- `manifest.json` 扫描的真实项目路径：`/Users/wuhaocheng/Documents/repos/succ-definitions/projects/xiaoshouyi`
- 复制样本时**不得**引用 `/Users/wuhaocheng/Downloads/bi`（源码目录，非稳定项目）
