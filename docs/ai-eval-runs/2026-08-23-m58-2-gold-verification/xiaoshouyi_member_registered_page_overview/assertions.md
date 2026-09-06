# xiaoshouyi_member_registered_page_overview 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1 | `summary.entrypoint_count` | 25 | 25 | PASS | 页面应有 25 个用户入口 |
| cmd1 | `summary.data_source_count` | 105 | 105 | PASS | 页面应读取 105 个数据源 |
| cmd1 | `summary.write_target_count` | 32 | 32 | PASS | 页面应写入 32 个目标 |
| cmd1 | `summary.page_jump_count` | 7 | 7 | PASS | 页面应有 7 个真实页面跳转（page_jump_count 只统计 OpensPage / ActionNavigates 边） |
