# facts_review — xiaoshouyi_member_registered_page_overview

## expected_facts（全部 ✅）
- summary.entrypoint_count=25 ✅
- summary.data_source_count=105 ✅
- summary.write_target_count=32 ✅
- summary.page_jump_count=7 ✅
- summary.page_role=mixed_interaction_page ✅
- 附带核对：navigation_count=19 ✅（与 standard_answer 一致）；key_model_count=25 与 entrypoint_count 同值但字段分开，summary.conclusion 正确引用 entrypoint 语义。

## 观察（非阻塞）
- summary.confidence.level=reduced，原因 PRIMARY_PATHS_TRUNCATED / ACTION_FLOW_INCOMPLETE / UNKNOWN_ACTION_TYPE —— 属于 compact 预算下 details 截断提示，不影响 summary 计数断言；standard_answer 只要求计数。

## forbidden_claims
- 不调用工具直接猜数字 —— 不适用（本次为工具实测）。✅
- 把 key_model_count=25 当成入口数 —— 输出中两者字段独立，conclusion 未混淆。✅

结论：与 standard_answer 完全一致，无异常。
