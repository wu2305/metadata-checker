# facts_review — xiaoshouyi_text41_value_source_dataflow_origin

## expected_facts（value_source_facts 逐条核对，全部 ✅）
- raw_expr=${CUSTOMAUTOMYAUTOLIST} ✅
- bare_symbol=CUSTOMAUTOMYAUTOLIST ✅
- nearest_data_context=sliderpanel2 ✅
- data_set=model11 ✅
- dataflow_table=$DATA:/加工表/小程序/绑车.tbl ✅
- original_node=FACT_AUTOCUSTOMERAUTOREL ✅
- original_field=车辆VIN ✅
- proven_physical_input=$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN ✅（`via=originalNode/originalField`，`physical_source_fields` 仅此一项）

## forbidden_claims
- CUSTOMAUTOMYAUTOLIST 是表 —— 未出现；输出明确 bare_symbol 与 candidate_inputs 分离。✅
- 只根据 text41 自身判断表来源 —— 未出现；paths step2 经过 sliderpanel2 容器。✅
- 忽略 sliderpanel2 dataSet —— 未出现。✅
- 把候选输入表当成已证明来源 —— 未出现；candidate_inputs（4 个候选表）与 proven_physical_input 分开列出。✅

结论：与 standard_answer 完全一致，无异常。
