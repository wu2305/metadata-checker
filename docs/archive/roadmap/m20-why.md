# M20：Why 条件查询能力

| milestone | M20 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M20：Why 条件查询能力

### 目标

支持面向目标对象的条件解释，回答“为什么不显示、为什么按钮灰、为什么数据为空”。

### 问题清单

- 当前只能通过 PageLogic + raw `.spg` 人工推导某个面板或按钮的门控条件。
- `model*.totalRowCount__` 作为数据命中行数门控没有专门解释。
- target 不存在或写错时，条件查询需要沿用 M14 的候选建议和 shell-safe 命令规范。
- 通用 `--context` 适合探索，但不适合作为“为什么不显示 / 为什么没数据”的默认回答入口；它会把主因、旁路依赖和低相关邻居混在一起，增加注意力漂移风险。

### 工作清单

- 新增条件解释命令，命名可采用 `--explain-condition <TARGET>` 或拆分为 `--why-visible`、`--why-disabled`、`--why-data-empty`。
- 输出目标条件链、上游依赖、失败原因候选。
- 对 `model*.totalRowCount__` 特殊处理，解释为“数据集命中行数门控”。
- 支持 page、component、model、field 四类 target。
- target 不存在时返回候选目标和下一条查询命令。
- 输出目标化 `primary_reason` / `primary_path`，把与目标无直接因果关系的邻居放入低优先级 `related_context`，并标明其不是必要条件。

### 验收目标

- 能解释“这个面板为什么不显示”。
- 能解释“这个按钮为什么灰”。
- 能解释“这个数据源为什么可能为空”。
- 输出必须区分确定条件和推断条件。
- 对 `合同协议.spg|input3` 这类目标查询，默认输出聚焦主因链路；不需要 AI 再从数百个 context 节点中自行筛选。
