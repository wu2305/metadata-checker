# xiaoshouyi_text41_display_conditions 断言核对

通过 3/3

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `details.answer_facts.display_facts.has_direct_condition` | false | false | PASS | text41 不应被判定为自身有 direct visibleCondition |
| cmd1 | `details.answer_facts.display_facts.inherited_conditions` | "model11.totalRowCount__ > 0" | "model11.totalRowCount__ > 0" | PASS | 继承祖先容器的 totalRowCount__ 显示门控 |
| cmd1 | `details.answer_facts.display_facts.expanded_data_gates` | "model11.ISSHOW == 1" | "model11.ISSHOW == 1" | PASS | totalRowCount__ 展开为当前页面 model11 filter |
