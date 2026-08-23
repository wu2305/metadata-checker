# facts_review — xiaoshouyi_testdrive_name_cross_page_writer

结论：**viable_with_changes**（按预案降级为 relations 型；conditionExp 确认不暴露；多条写入推断被修正）

## 关键验证点：conditionExp 是否暴露
❌ **不暴露**。cmd1（writer intent）paths 的 result 字符串无条件信息；cmd2（--relations）writers entry 的 meta 仅含 actor_id/operation/reason/source_expr/target_field/target_model/trigger，**无 conditionExp 字段**。insertData 的 model9.totalRowCount__=0 / updateData 的 =1 门控无法经 CLI 证实，gold 禁止编造。

## 推断逐条核对
- 「上传信息.spg button2 action3（insertData）写 fact_testDrive.name」—— ✅（cmd2 writers：field_path=fact_testDrive.name，operation=insertData）。
- 「上传信息.spg button2 action6（updateData）写 name」—— ❌ **被推翻**：action6 写 userid/limit/licenceLevel/issuingAuthority/driverLicenseNumber/driveeaddress/create_time/account/Close，**不含 name**。
- 「公里数.spg button3 action2 写该表」—— ✅ 写表但 ❌ 不写 name（写 kilometer1/kilometer2/departureTime/returnTime）。
- 「英力士/捷豹路虎登记表 button1 action5 写该表」—— ✅ 写表但 ❌ 不写 name（写 is_questionnaire）。
- 「试乘试驾登记表.spg」—— ❌ 该页面不存在（目录下只有品牌级 登记表.spg）。
- 新事实：字段级 name 的 action 写入者共 5 个——试乘试驾协议.spg button5 action2（submitData，推断未提及）、上传信息.spg button2 action3、不用/销售旧/上传信息_copy1.spg button2 action3、不用/销售旧/上传信息_copy.spg button2 action3/action5。
- 模型计数：read_by=84 / written_by=248 / dataflow 消费=19（cmd2 summary，全取自实跑）。
- 插曲：writer intent 对 input6 的 paths 终点是组件节点（input6 自身、上传信息_copy 的 input26/input11），不是 action——fields 级 Writes 边，不足以回答「哪个动作写入」，故必需 relations 型命令。

## gold 要点
单命令 `--relations 'model:fact_testDrive' --budget normal`；断言 4 条取自 cmd2 实际输出；standard_answer 明确声明 conditionExp 不可见。
