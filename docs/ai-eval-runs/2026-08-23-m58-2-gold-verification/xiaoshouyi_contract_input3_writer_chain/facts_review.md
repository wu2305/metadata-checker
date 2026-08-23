# facts_review — xiaoshouyi_contract_input3_writer_chain

## expected_facts（writer_facts 逐条核对，全部 ✅）
- writer_facts.result=writer_paths_found ✅
- 路径包含 field:model22.phoneNumber ✅（3 条路径均含）
- 路径包含 field:fact_qwSidebar.phoneNumber ✅（3 条路径均含）
- 路径包含 action:...潜客信息跟进.spg|button1|action1 ✅
- 路径包含 action:...潜客信息跟进.spg|button1|action4 ✅
- 路径包含 action:...保客信息.spg|button3|action4 ✅

## 观察（非阻塞）
- writer paths 共 3 条，未包含 保客信息.spg button3 action1（standard_answer 中仅为 should_mention，不是 expected_fact；case 5 的 --relations 输出确认 button3 action1 确实写 fact_qwSidebar.phoneNumber/phone_time，属于 writer intent compact 预算下未展开，不算违背）。
- 3 条路径均以 `comp:...合同协议.spg|input3>` 开头，与断言的子串匹配一致。

## forbidden_claims
- 只查询合同协议页面后断言没有写入 —— 未出现。✅
- 把 Reads 当成生成动作 —— 未出现；路径终点均为 action 节点。✅
- 跳过 fact_qwSidebar 物理字段 —— 未出现。✅
- 忽略 潜客信息跟进.spg —— 未出现。✅

结论：expected_facts 全部命中，无异常。
