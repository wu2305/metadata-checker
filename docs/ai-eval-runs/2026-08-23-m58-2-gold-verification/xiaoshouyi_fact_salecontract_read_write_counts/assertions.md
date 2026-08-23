# xiaoshouyi_fact_salecontract_read_write_counts 断言核对

通过 2/2

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `summary.read_by_count` | 112 | 112 | PASS | fact_saleContract 应有 112 个读取节点 |
| cmd1 | `summary.written_by_count` | 15 | 15 | PASS | fact_saleContract 应有 15 个写入节点 |
