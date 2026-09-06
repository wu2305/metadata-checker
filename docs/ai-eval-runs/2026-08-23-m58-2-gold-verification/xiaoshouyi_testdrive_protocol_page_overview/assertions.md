# xiaoshouyi_testdrive_protocol_page_overview 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1(cmd1.json) | `summary.entrypoint_count` | 46 | 46 | PASS | 页面应有 46 个用户入口 |
| cmd1(cmd1.json) | `summary.data_source_count` | 208 | 208 | PASS | 页面应读取 208 个数据源 |
| cmd1(cmd1.json) | `summary.write_target_count` | 106 | 106 | PASS | 页面应写入 106 个目标 |
| cmd1(cmd1.json) | `summary.page_jump_count` | 2 | 2 | PASS | 页面应有 2 个真实页面跳转 |
