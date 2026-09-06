# facts_review — xiaoshouyi_model11_availability_dataflow_filters

## expected_facts（availability_facts 逐条核对，全部 ✅）
- dataflow_availability=dataflow_filters_present ✅
- dataflow_table=$DATA:/加工表/小程序/绑车.tbl ✅
- gates 同时包含页面 filter（`model11.ISSHOW == 1`，scope=direct）与 4 条 DataFlow 内部 filter（`[是否展示] == 1`、`[关系类型] == 车主关系`、`[使用人粉丝ID] is not null`、`[粉丝ID]=$user.WECHAT_UNIONID`）+ 1 条关联输出 filter（`[粉丝ID]=$user.WECHAT_UNIONID OR (...)`）✅
- referenced_vars 包含 $user.WECHAT_UNIONID ✅（仅此一项）
- join_rules 包含 LeftJoin ✅（2 条 join 均 LeftJoin，row_semantic=left_rows_preserved_right_fields_nullable）
- union_rules row_semantic=any_branch_can_output ✅
- 附带核对：physical_inputs 恰为 standard_answer 列出的 4 张物理表（fact_autoCustomerAutoRel / fact_cusAutoRelExt / fact_customAutoMyAutoListStore / fact_Unbound_Vehicle）✅

## forbidden_claims
- 只读取 current page filter 后停止 —— 未出现；DataFlow 内部 filter 全部列出。✅
- 忽略 DataFlow tbl —— 未出现。✅
- 混入其他页面的 model11 条件 —— 未出现；gates 的 source_file 仅当前页 spg 与 绑车.tbl。✅
- 把 LeftJoin 说成 InnerJoin —— 未出现。✅

结论：与 standard_answer 完全一致，无异常。
