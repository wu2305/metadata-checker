# xiaoshouyi_text41_value_source_dataflow_origin 断言核对

通过 3/3

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `details.answer_facts.value_source_facts.nearest_data_context` | "sliderpanel2" | "sliderpanel2" | PASS | 裸字段必须通过最近数据容器解析 |
| cmd1 | `details.answer_facts.value_source_facts.field_path` | "model11.CUSTOMAUTOMYAUTOLIST" | "model11.CUSTOMAUTOMYAUTOLIST" | PASS | 裸字段应被归一为 model11.CUSTOMAUTOMYAUTOLIST |
| cmd1 | `details.answer_facts.value_source_facts.proven_physical_input` | "$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN" | "$DATA:/主数据/fact_autoCustomerAutoRel.tbl.车辆VIN" | PASS | 必须输出已证明的字段级物理来源 |
