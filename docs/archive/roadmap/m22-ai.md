# M22：条件类 AI 评测与协议收敛

| milestone | M22 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M22：条件类 AI 评测与协议收敛

### 目标

把页面条件分析能力固定成长期回归，验证空上下文小模型不读 raw `.spg` 也能回答条件类问题。

### 问题清单

- 当前真实项目评测主要覆盖页面用途、按钮行为、DataFlow、目标定位和 `.tbl` 单文件。
- “页面为什么没数据 / 按钮为什么灰 / 面板为什么不显示”尚未形成固定评测。
- `SKILL.md` 没有明确条件类问题的命令路线。
- M18 残留的注意力漂移风险尚未进入评测：模型可能把 `--context` 中的同名局部模型、旁路 DataFlow、其他页面组件当成主链路证据。

### 工作清单

- 增加真实项目 eval case：`合同协议.spg 正常显示数据的条件是什么？`
- 增加真实项目 eval case：`合同协议.spg 的 input3 来源是什么？`，要求答案优先输出 `input3 -> model22.phoneNumber -> fact_qwSidebar.phoneNumber <- 潜客信息跟进.button1.action1/action4`，并禁止把旁路 context 当作必要条件。
- 增加 fixture case：简单显示条件、链式条件、按钮禁用、数据源为空、系统变量依赖。
- 更新 `SKILL.md`，增加“页面没数据 / 按钮灰 / 面板不显示”的命令路线。
- 空上下文 `5.4-mini` 验证时禁止读取 raw `.spg`，只能使用工具输出回答。
- 评测断言必须覆盖 `param`、`$user`、隐藏字段、`model.filter`、`totalRowCount__`、`visibleCondition/disableCondition`。
- 评测断言必须包含注意力漂移负例：同名局部模型的其他页面引用、旁路 DataFlow、低相关组件不得被写成主因或必要条件。

### 验收目标

- 小模型能回答条件类问题，且不需要直接读取 5MB 原始元数据。
- 回答包含证据链、截断说明和保守口径。
- 禁止误判：不能把条件门控说成渲染故障，不能把未参与初始展示的参数说成必需条件。
- 禁止漂移：当工具同时输出主链路和相关上下文时，小模型必须优先引用主链路，并显式区分“相关但非必要”的旁路关系。
