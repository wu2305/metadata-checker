# xiaoshouyi_model11_availability_dataflow_filters 断言核对

通过 3/3

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `details.answer_facts.availability_facts.dataflow_table` | "$DATA:/加工表/小程序/绑车.tbl" | "$DATA:/加工表/小程序/绑车.tbl" | PASS | page-scoped model11 必须解析到 DataFlow 表 |
| cmd1 | `details.answer_facts.availability_facts.gates` | "[是否展示] == 1" | "[是否展示] == 1" | PASS | 必须包含 DataFlow 源过滤 |
| cmd1 | `details.answer_facts.availability_facts.gates` | "$user.WECHAT_UNIONID" | "[粉丝ID]=$user.WECHAT_UNIONID" | PASS | 必须包含用户变量过滤 |
