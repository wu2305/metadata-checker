# xiaoshouyi_contract_button13_display_condition 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1(cmd2.json) | `details.answer_facts.display_facts.has_direct_condition` | true | true | PASS | panel13 自身有 direct visibleCondition |
| cmd1(cmd2.json) | `details.answer_facts.display_facts.direct_conditions` | "model10.totalRowCount__>0" | "model10.totalRowCount__>0" | PASS | direct 条件为 model10 行数门控 |
| cmd1(cmd2.json) | `details.answer_facts.display_facts.inherited_conditions` | "model4.totalRowCount__>0" | "model4.totalRowCount__>0" | PASS | 继承祖先 panel2 的 model4 行数门控 |
| cmd1(cmd2.json) | `details.answer_facts.display_facts.expanded_data_gates` | "model10.QWCONTACTID=text41" | "model10.QWCONTACTID=text41" | PASS | model10 行数门控展开为本页 filter，引用本页 text41 |
