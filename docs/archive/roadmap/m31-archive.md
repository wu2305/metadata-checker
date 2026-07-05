# M31：条件因果方向与继承去重

| milestone | M31 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M31：条件因果方向与继承去重

### 背景

真实问题“`会员已注册.spg` 中 `text41` 哪些情况会显示？”暴露出两类缺口：

- 工具能找到祖先容器 `panel35.visibleCondition = model11.totalRowCount__ > 0`，但 AI 没有继续展开 `model11` 的当前页面 filter。
- 组件自身与父组件可能声明相同限制条件，若不去重会导致回答重复或误判为多个独立必要条件。

### 目标

让 `explain_condition` 对显示/隐藏问题输出面向因果解释的条件链：

```text
目标组件自身条件
→ 祖先容器继承条件
→ totalRowCount__ 数据命中门控
→ 当前页面模型 filter 条件
```

### 非目标

- 不重写 GraphDB 存储边方向。
- 不把所有 related_context 自动升为必要条件。
- 不做跨页面同名 model 的业务推断。

### 功能点

- [x] 条件作用域标注：
  - `condition_scope = direct`
  - `condition_scope = inherited`
  - `condition_scope = expanded_from_total_row_count`
  - 输出 `owner_node_id`、`inherited_from`、`ancestor_distance`、`expanded_from_condition`。
- [x] 祖先容器条件收集：
  - 目标为组件时，沿 `Contains` 入边向上查找父组件。
  - 收集祖先 `visibleCondition` / `disableCondition` / action condition。
  - 祖先条件进入主 `blocking_conditions`，不放入 `related_context`。
- [x] `totalRowCount__` 门控展开：
  - 从 blocking condition 的 `referenced_symbols` / `raw_expr` 中识别 `modelX.totalRowCount__`。
  - 在当前页面作用域下查找 `modelX` 的 `SourceFilterExp` / `SourceFilterClause`。
  - 展开结果进入 `data_empty_gates`，并标注来源条件。
- [x] 条件去重：
  - 按 `source_file + condition_type + effect_type + normalized_expr/raw_expr + referenced_symbols` 生成 `condition_key`。
  - 自身与祖先条件等价时只保留一条。
  - 去重后保留 `deduped_condition_ids` / `deduped_scopes` / `deduped_owner_node_ids`。
- [ ] 真实项目验收：
  - `/app/售后.app/绑定车辆/会员已注册.spg|text41`
  - 必须回答：自身无直接 visible，继承祖先容器门控，`model11.totalRowCount__ > 0` 继续展开为当前页面 `model11` filter。
- [ ] SKILL 决策树更新：
  - 显示问题不能只查目标组件。
  - 遇到 `modelX.totalRowCount__` 必须继续展开当前页面 `modelX` filter。
  - 明确区分 direct / inherited / expanded / related_context。

### 细化任务清单

- [x] Fixture 补齐：
  - 新增父容器 `panel_total_gate.visibleCondition = model1.totalRowCount__ > 0`。
  - 新增子组件 `text_total_child`，自身无 visible 条件。
  - 新增子组件 `text_total_child_duplicate`，自身与父容器声明相同 visible 条件。
- [x] `explain_condition` 数据结构：
  - 为条件对象补充 `condition_scope`。
  - 为条件对象补充 owner/继承/展开来源字段。
  - 不删除既有字段，保持兼容。
- [x] 继承条件遍历：
  - 实现组件祖先链遍历。
  - 避免跨页面条件进入主条件。
- [x] 数据门控展开：
  - 实现 `totalRowCount__` 模型识别。
  - 从当前页面模型条件中抽取 filter。
- [x] 去重实现：
  - 实现条件 key。
  - 合并重复条件并保留证据来源。
- [x] Fixture 回归测试：
  - 子组件继承父容器 totalRowCount 门控。
  - totalRowCount 门控展开为当前页面 filter。
  - 子/父重复 visibleCondition 去重。
- [ ] 真实项目回归测试：
  - ignored 测试覆盖 `text41`。
  - 断言 `panel35#visibleCondition#8` 与当前页面 `model11` filter 同时出现。
- [ ] 文档与 skill：
  - 更新 `SKILL.md` / `docs/schema.md`。
  - 增加弱指向显示问题回答规则。

### 验收目标

- AI 问“组件哪些情况显示”时，不停在目标组件直接条件。
- 祖先容器门控被标为继承必要条件，而不是 related_context。
- `modelX.totalRowCount__` 不再作为终点，必须展开到当前页面数据源 filter。
- 相同条件不会重复污染主答案，但证据来源仍可追溯。
