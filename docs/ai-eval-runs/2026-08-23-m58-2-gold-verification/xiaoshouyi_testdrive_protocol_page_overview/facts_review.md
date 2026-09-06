# facts_review — xiaoshouyi_testdrive_protocol_page_overview

结论：**viable**（计数型 case，值全部取自 CLI 实跑）

## 实跑结果（cmd1，--relations page 预算 compact）
- entrypoint_count=46、data_source_count=208、write_target_count=106、page_jump_count=2、page_role=mixed_interaction_page、navigation_count=9、key_model_count=21。
- 跳转目标：合同协议.spg（summary.conclusion）。
- confidence.level=reduced（PRIMARY_PATHS_TRUNCATED / ACTION_FLOW_INCOMPLETE / UNKNOWN_ACTION_TYPE），与存量 page overview case 的 compact 表现一致，不影响 summary 计数。

## 核对性质说明
计数无法从源文件人工推断（候选描述已声明），本 case 的 gold 值直接锚定 CLI 实跑输出；断言 4 条全部取自 cmd1。风险：未来图构建逻辑变化会导致计数漂移，复审时需整 case 重跑。
