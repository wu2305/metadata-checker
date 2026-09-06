# xiaoshouyi_testdrive_input6_contract_no_generation 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1(cmd6.json) | `details.reads` | "IF(model7.totalRowCount__=1,model7.name" | " IF(model7.totalRowCount__=1,model7.name, CONCAT('SJ',input15,RIGHT(TOSTR(TODAY(),'yyyymmdd'),6),TOSTR(model6.totalRowCount__+1,'000')))" | PASS | input6 取值表达式含条件复用分支 |
| cmd1(cmd6.json) | `details.reads` | "TOSTR(model6.totalRowCount__+1,'000')" | " IF(model7.totalRowCount__=1,model7.name, CONCAT('SJ',input15,RIGHT(TOSTR(TODAY(),'yyyymmdd'),6),TOSTR(model6.totalRowCount__+1,'000')))" | PASS | 新编号含 model6 序号补零 |
| cmd2(cmd4.json) | `details.answer_facts.availability_facts.gates` | "model7.signed IS NULL" | "model7.signed IS NULL" | PASS | model7 限定未签署记录 |
| cmd2(cmd4.json) | `details.answer_facts.availability_facts.gates` | "model7.create_time.date=TODAY()" | "model7.create_time.date=TODAY()" | PASS | model7 限定当天记录 |
