# xiaoshouyi_fact_qwsidebar_model_relationships 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `summary.read_by_count` | 13 | 13 | PASS | fact_qwSidebar 应有 13 个读取节点 |
| cmd1 | `summary.written_by_count` | 6 | 6 | PASS | fact_qwSidebar 应有 6 个写入节点 |
| cmd1 | `details.readers` | "comp:app/销售.app/销售/合同协议.spg|input3" | "comp:app/销售.app/销售/合同协议.spg|input3" | PASS | 必须能查到合同协议 input3 的读取 |
| cmd1 | `details.writers` | "action:app/销售.app/销售/潜客信息跟进.spg|button1|action1" | "action:app/销售.app/销售/潜客信息跟进.spg|button1|action1" | PASS | 必须能查到潜客信息跟进 action1 写入 |
