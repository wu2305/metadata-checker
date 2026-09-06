# facts_review — xiaoshouyi_testdrive_input6_contract_no_generation

结论：**viable_with_changes**（表达式与 model7 filter 坐实；物理表链路砍掉；命令改用 --explain + availability 组合）

## 推断逐条核对
- 「input6 exp = IF(model7.totalRowCount__=1, model7.name, CONCAT('SJ',input15,RIGHT(TOSTR(TODAY(),'yyyymmdd'),6),TOSTR(model6.totalRowCount__+1,'000')))」—— ✅（cmd6 reads[0].raw_expr 精确匹配，含前导空格；what_is_it=「input input6，读取 model7」）。
- 「model7 带 6 条 filter」—— ✅ 全中（cmd4 gates：lgDelete IS NULL OR lgDelete = 0、TestDrivingMethod='1'、signed IS NULL、create_time.date=TODAY()、qwContactId=input14、externalUserId=input11）。
- 「model6 同表仅 filter create_time.date=TODAY()」—— ⚠️ 部分：cmd5 gates 含 `model6.create_time.date=TODAY()` ✅，但 model6 的 dataflow_table 被错位解析到 szsys_4_users.tbl，且 gates 混入外来表达式，「同表」不可证。
- 「model7 → fact_testDrive.tbl」—— ❌ **CLI 无法证明**：cmd4 dataflow_table=$DATA:/销售/fact_saleContract.tbl（同名模型降级解析，同候选 2 的错位模式）。gold 不含物理表断言，并在 must_not_mention 明确禁止引用该错位结果。
- 插曲：--explain-condition --intent value-source/auto 对 input6 均返回 value_source_context not available（cmd1/cmd2）——calc 表达式不走 value-source intent，只能用 --explain 的 reads 取（与存量 case 7 同风格）。

## gold 要点
两条 required_commands（explain compact + model7 availability compact）；4 条断言全部取自 cmd6/cmd4 实际输出。
