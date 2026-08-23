# facts_review — xiaoshouyi_text41_display_conditions

## expected_facts
- has_direct_condition=false —— ✅ `details.answer_facts.display_facts.has_direct_condition == false`，`direct_conditions` 为空数组。
- inherited_conditions 包含 panel35 —— ✅ `inherited_conditions[0].condition_id = cond:...|panel35#visibleCondition#8`。
- inherited condition raw_expr 为 model11.totalRowCount__ > 0 —— ✅ 精确匹配。
- expanded_data_gates 包含 model11.ISSHOW == 1 —— ✅ `expanded_data_gates[0].raw_expr = "model11.ISSHOW == 1"`，`condition_scope=expanded_from_total_row_count`。
- value_source_context 不应作为显示条件证据 —— ✅ display intent 输出中无 value_source 字段；evidence_refs 仅指向 panel35.visibleCondition 与 model11 filter。

## forbidden_claims
- 把 value/source 当成显示条件 —— 未出现，输出仅含 display 证据。✅
- 只回答 text41.value —— 未出现。✅
- 忽略祖先容器 panel35 —— panel35 是主证据。✅
- 忽略 totalRowCount__ 的 filter 展开 —— expanded_data_gates 已展开。✅

结论：与 standard_answer 完全一致，无异常。
