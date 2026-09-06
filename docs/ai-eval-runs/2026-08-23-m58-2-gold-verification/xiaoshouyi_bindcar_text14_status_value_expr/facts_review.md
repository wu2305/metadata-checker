# facts_review — xiaoshouyi_bindcar_text14_status_value_expr

结论：**viable_with_changes**（表达式坐实；「对应哪张表」砍掉）

## 推断逐条核对
- 「text14 value = ${IF(model6.totalRowCount__ >0,'绑定成功','已提交审核，车辆待认证中')}」—— ✅（cmd1 reads[0].raw_expr 精确匹配）。
- 「读取 model6」—— ⚠️ 部分：表达式确实引用 model6.totalRowCount__，但 CLI 对表达式内模型引用解析粗糙，model id 被截成 `IF(model6`（cmd1 reads[0].id/name），what_is_it 也显示「读取 IF(model6」。gold 用 raw_expr 子串断言，不断言 model 节点 id。
- 「model6 → fact_customAutoMyAutoList.tbl」—— ❌ CLI 不可证（本页 model6 降级解析到 szsys_4_users，见候选 2 cmd4_model6/cmd5）。问题与 gold 均不含表名。

## gold 要点
单命令 `--explain ... --budget compact`；1 条断言取自 cmd1 实际输出；must_not_mention 含「给 model6 安 CLI 未证明的物理表名」。
