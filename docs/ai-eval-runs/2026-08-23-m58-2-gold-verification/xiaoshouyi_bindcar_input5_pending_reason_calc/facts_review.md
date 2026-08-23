# facts_review — xiaoshouyi_bindcar_input5_pending_reason_calc

结论：**not_viable**（核心事实无法经 CLI 建立，且 CLI 输出与源文件矛盾）

## 推断逐条核对
- 「input5 calcEnabled + 5 分支 CASE WHEN」—— ❌ **CLI 不暴露**：cmd1（intent auto）、cmd2（intent value-source）均返回 `value_source_context not available`；cmd3（--explain）reads/lineage 均为空。calc 表达式仅以 `cond:...|input5#exp#2` 出现在 evidence_refs，无法断言内容。「什么情况下算出『该车架号不存在』」无法回答。
- 「model6/model7/model8 同指 fact_customAutoMyAutoList.tbl」—— ❌ **CLI 与源文件矛盾**：cmd4_* availability 把 model6/7/8 分别解析到 `$DATA:/通用/szsys_4_users.tbl`、`$DATA:/销售/fact_saleContract.tbl`、`$DATA:.../fact_warrantyperiod.tbl`；源文件证实三者均为 `$DATA:/主数据/fact_customAutoMyAutoList.tbl`。cmd5 证实全局 `model:model6` 即 szsys_4_users——页面限定目标被降级到全局同名模型（fixture execution_notes 第 15 条已记载的同类问题波及 availability）。
- model6/7/8 的本页 filter 子句 —— ✅ 部分可见：gates 里确有 `model6.name=param1`、`model6.carOwerPhone=input2.value OR ...`、`model8.name=IF(param1 IS NULL,input1.value,param1)` 等（与源文件一致），但同时混入其他页面/模型的外来 gate（`model1.autoCusAutoRel == 车主关系`、`[维修类型] == 整车精品` 等），输出不可作为 gold 依据。
- 「input5 visible:false 静态隐藏」—— 未经 CLI 验证（display intent 返回 no_display_condition_found，静态 visible 属性不在 facts 输出中），未纳入。

## 处置
不出 gold_draft。本目录仅保留证据（cmd1-cmd5）备查。若后续修复页面级模型解析与 calc 表达式暴露，可重启本候选。
