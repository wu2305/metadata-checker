# facts_review — xiaoshouyi_fact_salecontract_read_write_counts

## expected_facts（全部 ✅）
- summary.read_by_count=112 ✅
- summary.written_by_count=15 ✅
- summary.consumed_by_dataflow_count=55 ✅
- 附带核对：compact 预算下 readers/writers 列表各截断为 5 条，confidence.level=partial 且带 OUTPUT_TRUNCATED 诊断，与 standard_answer「compact 截断不影响 summary 计数」一致。

## forbidden_claims
- 不调用工具直接猜数字 —— 不适用。✅
- 把 OUTPUT_TRUNCATED 后的 5 条列表当成完整计数 —— 未出现；summary 计数与截断列表分离且带诊断。✅

结论：与 standard_answer 完全一致，无异常。
