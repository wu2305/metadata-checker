# xiaoshouyi_testdrive_name_cross_page_writer 断言核对

通过 4/4

| cmd | path | expected | actual | result | 说明 |
|---|---|---|---|---|---|
| cmd1(cmd2.json) | `summary.written_by_count` | 248 | 248 | PASS | fact_testDrive 应有 248 个写入节点 |
| cmd1(cmd2.json) | `summary.read_by_count` | 84 | 84 | PASS | fact_testDrive 应有 84 个读取节点 |
| cmd1(cmd2.json) | `details.writers` | "action:app/销售.app/完善合同信息/上传信息.spg|button2|action3" | "action:app/销售.app/完善合同信息/上传信息.spg|button2|action3" | PASS | 必须查到上传信息 insertData 写入 |
| cmd1(cmd2.json) | `details.writers` | "action:app/销售.app/完善合同信息/试乘试驾协议.spg|button5|action2" | "action:app/销售.app/完善合同信息/试乘试驾协议.spg|button5|action2" | PASS | 必须查到试乘试驾协议 submitData 写入 |
