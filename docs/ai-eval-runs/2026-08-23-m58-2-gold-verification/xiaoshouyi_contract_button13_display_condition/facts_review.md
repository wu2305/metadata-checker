# facts_review — xiaoshouyi_contract_button13_display_condition

结论：**viable_with_changes**（目标从 button13 改为 panel13；model10 物理表推断被推翻；新增 panel2 继承事实）

## 推断逐条核对
- 「button13 自身无 visibleCondition」—— ✅ CLI 确认（cmd1：has_direct_condition=false）。
- 「button13 继承祖先 panel13 的 model10.totalRowCount__>0」—— ❌ **被推翻（CLI 行为）**：cmd1 对 button13 返回 `no_display_condition_found`，inherited_conditions 为空。根因（cmd3）：图中 button13 的 Contains 入边仅来自 `page:...合同协议.spg`，不含 panel13，祖先链断裂。但源文件证实 button13 确实嵌套在 panel13 内（canvas>…>panel2>panel3>panel13>button13），属工具图谱缺口，非事实错误。
- 「panel13 的 visibleCondition = model10.totalRowCount__>0」—— ✅（cmd2：direct_conditions[0].raw_expr 精确匹配）。
- 「model10 页面级 filter 三 clause」—— ✅（cmd2 expanded_data_gates：SIGNED IS NOT NULL AND SIGNED!='相关合同已经撤销'、EXTERNALUSERID=param5、QWCONTACTID=text41，三条全中）。
- 「model10 是内嵌 DataFlow，originalNode=FACT_TESTDRIVE，物理表 fact_testDrive.tbl」—— ❌ **被推翻**：cmd4 availability 解析 model10 → `$DATA:/销售/fact_gap.tbl`，物理输入 fact_customAutoMyAutoListStore.tbl + fact_repairWorkOrders.tbl，与 FACT_TESTDRIVE 无关。
- 推断遗漏（新事实）：panel13 还**继承 panel2 的 `model4.totalRowCount__>0`**（cmd2 inherited_conditions），gold 已纳入。

## gold 要点
断言 4 条全部取自 cmd2 实际输出路径与值；不含 model10 物理表断言；must_not_mention 含「断言 model10 物理表是 fact_testDrive.tbl」。
