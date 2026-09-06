# facts_review — xiaoshouyi_fact_qwsidebar_model_relationships

## expected_facts（全部 ✅）
- summary.read_by_count=13 ✅
- summary.written_by_count=6 ✅
- summary.consumed_by_dataflow_count=10 ✅
- readers 包含 comp:app/销售.app/销售/合同协议.spg|input3 ✅（13 个 reader 全量列出，normal 预算无截断）
- writers 包含 潜客信息跟进.spg button1 action1/action4 ✅（action1 写 phone_time 与 phoneNumber 两条边；action4 写 phoneNumber）
- writers 包含 保客信息.spg button3 action1/action4 ✅（同样覆盖 phoneNumber/phone_time）
- 附带核对：dataflow_role=DataFlowParticipant ✅

## forbidden_claims
- 没有跨页 writer —— 未出现；6 条 writer 全在合同协议页之外。✅
- 只看 compact 前 5 条后断言完整 —— 未出现；本命令用 normal 预算，confidence.level=full。✅
- 忽略 OUTPUT_TRUNCATED 风险 —— 未出现；summary.confidence.reasons 为空。✅
- 把 fact_qwSidebar 只当 DataFlow 输出 —— 未出现；readers/writers 均列出。✅

结论：与 standard_answer 完全一致，无异常。
