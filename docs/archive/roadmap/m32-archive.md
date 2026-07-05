# M32：裸字段值来源继承容器数据上下文

| milestone | M32 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M32：裸字段值来源继承容器数据上下文

### 背景

真实问题“`会员已注册.spg` 中 `text41` 如果显示，显示的数据来源于哪张物理表？”暴露出新缺口：

- `text41.value = ${CUSTOMAUTOMYAUTOLIST}` 只有字段名，没有模型名前缀。
- 字段所属数据集不在 `text41` 自身，而在祖先容器 `sliderpanel2.source = data` / `sliderpanel2.dataSet = model11`。
- AI 若只看 `text41.value`，会误把 `CUSTOMAUTOMYAUTOLIST` 当成全局模型或物理表。

### 目标

让 `explain_condition` 对组件裸字段值输出稳定链路：

```text
目标组件 value = ${FIELD}
→ 最近带 dataSet 的祖先数据容器
→ 页面 source 中的 dataSet 模型
→ 表路径 / DataFlow 加工表
→ 字段级原始输入表（能证明时）
```

### 非目标

- 不把所有裸字段都强行解析为页面 source；只有存在最近数据容器时才输出确定结论。
- 不用业务名称猜物理表；字段级来源必须来自 DataFlow 元数据或图边证据。
- 不替代 M31 显示条件链，M32 只补“显示后值从哪里来”。

### 细化任务清单

- [x] 扫描组件数据上下文：
  - 从原始 `.spg` JSON 递归记录组件 `json_path`。
  - 记录组件 `source`、`dataSet`、`parent_id`。
  - 将组件 `properties`、`json_path`、`source`、`dataSet` 写入 Component 节点 meta。
- [x] 保留页面 source 模型节点：
  - 对 `dwtable` source 显式创建局部 `model:<source.id>` 节点。
  - 保留 `sourcePath`，建立局部 model 与表路径的关系。
- [x] 识别裸字段值：
  - 只识别 `${FIELD}` 且 `FIELD` 不含 `.` 的表达式。
  - 输出 `bare_symbol`，避免把字段名误当模型名。
- [x] 最近数据容器解析：
  - 用 `json_path` 前缀查找最近祖先组件。
  - 只接受带 `source` 或 `dataSet` 的祖先容器。
  - 输出 `nearest_data_context`。
- [x] 表路径解析：
  - 用容器 `dataSet` 找页面 source model。
  - 输出 `field_path = dataSet.FIELD` 与 `table_source_path`。
- [x] DataFlow 字段来源解析：
  - 在 `.tbl` DataFlow meta 中保留 `nodeTablePaths`。
  - 通过 `nodeFields.dbfield/name` 匹配裸字段。
  - 通过 `originalNode` / `aliasMap` / `nodeTablePaths` 输出 `module_table_path`。
- [x] Fixture 回归测试：
  - `slider_data_context.dataSet = model1`。
  - 子组件 `text_bare_field_child.value = ${name}`。
  - 断言解析为 `model1.name` 和 `data/table1.tbl`。
- [x] 真实项目回归测试：
  - `text41.value = ${CUSTOMAUTOMYAUTOLIST}`。
  - 断言最近数据容器是 `sliderpanel2`。
  - 断言 dataSet 是 `model11`。
  - 断言表路径包含 `$DATA:/加工表/小程序/绑车.tbl`。
  - 能证明字段级来源时断言 `fact_autoCustomerAutoRel.tbl`。
- [x] 文档与 skill：
  - `docs/schema.md` 记录 `value_source_context` 契约。
  - `SKILL.md` 要求“值来源”问题先读 `value_source_context`，不要把裸字段当表。

### 验收目标

- AI 问“组件显示的数据来源于哪张表”时，不再停在 `${FIELD}`。
- 对裸字段必须先找最近数据容器，再解释 `dataSet.field`。
- DataFlow 字段级来源可证明时输出原始输入表；不可证明时只输出输入表候选。
