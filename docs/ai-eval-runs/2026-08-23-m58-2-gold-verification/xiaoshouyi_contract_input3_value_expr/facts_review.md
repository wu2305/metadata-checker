# facts_review — xiaoshouyi_contract_input3_value_expr

## expected_facts（全部 ✅）
- summary.what_is_it 包含「读取 model22」 ✅（原文：`input input3，读取 model22`）
- details.reads 包含 raw_expr=model22.phoneNumber ✅（4 条 reads 的 raw_expr 均为 model22.phoneNumber）
- details.reads 包含 field_path=fact_qwSidebar.phoneNumber ✅（经 FieldAlias 的 model/field 两条 reads 均给出 fact_qwSidebar.phoneNumber）

## forbidden_claims
- 不调用工具直接猜字段名 —— 不适用。✅
- 把 writer 链问题混进取值表达式回答 —— 未出现；输出仅 reads/what_is_it。✅

结论：与 standard_answer 完全一致，无异常。
