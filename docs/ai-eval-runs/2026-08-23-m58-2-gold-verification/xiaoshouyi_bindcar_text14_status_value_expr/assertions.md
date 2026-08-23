# xiaoshouyi_bindcar_text14_status_value_expr 断言核对

通过 1/1

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1(cmd1.json) | `details.reads` | "IF(model6.totalRowCount__ >0,'绑定成功','已提交审核，车辆待认证中')" | "${IF(model6.totalRowCount__ >0,'绑定成功','已提交审核，车辆待认证中')}" | PASS | text14 取值表达式必须是 IF 行数判断双文案 |
