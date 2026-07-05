# M12：Explain 语义摘要与证据质量收敛

| milestone | M12 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M12：Explain 语义摘要与证据质量收敛

### 目标

让 `--explain` 对组件、动作、模型、字段输出真正可回答业务问题的语义摘要，并避免伪证据误导模型。

### 问题清单

- 单文件 `--explain button1` 仍可能输出 `button component button1`，业务语义不足。
- 项目级 `--explain` 的 `type_detail` 可能出现组件 ID（如 `button1`），而不是真实组件类型。
- 文本统计类组件可能被误判为 `entrypoint`，空模型已指出 `text11` 这类组件容易把 AI 带向错误方向。
- evidence 中存在 `node_id="?"`、`json_path="<graph-edge-derived>"`、`raw_expr="n/a"` 或 `raw_expr="?"`，看似证据但无法核验。
- `LINEAGE_SOURCE_MISSING` 对没有字段血缘需求的按钮/普通组件也频繁出现，形成噪声。
- `triggered_by` 与 `affects` 的关系命名对 AI 不够直观，容易把“页面包含组件”理解成业务触发。
- Action 的 `semantic_summary` 在 `PageLogic` 中较好，但 `Explain` 的 component/action 摘要没有完全复用。

### 验收目标（M15 已收敛）

- `--explain comp:...|button1` 输出 `semantic_summary`，明确“点击后打开哪个对话框/跳转/写入/刷新”。
- `--explain` 的 `type_detail` 使用真实类型，不使用 ID。
- `importance` 区分 `entrypoint`、`calculated_display`、`container`、`static_display`，文本统计不能默认算入口。
- evidence 若无法定位 JSON 路径，必须降级为 `confidence=low` 或移动到 `diagnostics`。
- 删除或替换 `node_id="?"`，不能定位时输出明确诊断 `EVIDENCE_LOCATION_MISSING`。
- 仅在用户询问字段来源或目标确实有字段 lineage 时输出 `LINEAGE_SOURCE_MISSING`。
- 增加真实项目回归：`首页.spg` 的 `button1`、`text11`、`button2`，以及一个表单输入组件。
