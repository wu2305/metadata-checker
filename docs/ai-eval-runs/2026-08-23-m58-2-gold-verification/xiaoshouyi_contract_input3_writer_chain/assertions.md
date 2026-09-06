# xiaoshouyi_contract_input3_writer_chain 断言核对

通过 2/2

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `details.answer_facts.writer_facts.paths` | "field:model22.phoneNumber>field:fact_qwSidebar.phoneNumber>action:app/销售.app/销售/潜客信息跟进.spg|button1|action1" | "comp:app/销售.app/销售/合同协议.spg|input3>field:model22.phoneNumber>field:fact_qwSidebar.phoneNumber>action:app/销售.app/销售/潜客信息跟进.spg|button1|action1" | PASS | 必须有 action1 跨页写入路径 |
| cmd1 | `details.answer_facts.writer_facts.paths` | "field:model22.phoneNumber>field:fact_qwSidebar.phoneNumber>action:app/销售.app/销售/潜客信息跟进.spg|button1|action4" | "comp:app/销售.app/销售/合同协议.spg|input3>field:model22.phoneNumber>field:fact_qwSidebar.phoneNumber>action:app/销售.app/销售/潜客信息跟进.spg|button1|action4" | PASS | 必须有 action4 跨页写入路径 |
