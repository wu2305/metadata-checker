# 开发 Checklist

## P0：表达式与依赖正确性

- [x] 换行表达式 fixture：`tests/fixtures/newline_expressions.spg`
- [x] 换行表达式测试：`test_newline_expression_parsing`
- [x] 复杂循环 fixture：`tests/fixtures/complex_cycle.spg` (A→B→C→D→B)
- [x] 循环检测测试：`test_complex_cycle_detection`（A 不在环内，B/C/D 在环内）
- [x] 同层组件顺序稳定性 fixture：`tests/fixtures/stable_order.spg`
- [x] 稳定性测试：`test_topological_sort_stability`（多次运行结果一致）
- [x] 稳定排序规则实现：按元数据出现顺序处理入度为0节点

## P1：组件值追溯 fixture

- [x] 单级追溯 fixture：`tests/fixtures/value_trace_single.spg`
- [x] 多级追溯 fixture：`tests/fixtures/value_trace_multi_level.spg`
- [x] 多分支追溯 fixture：`tests/fixtures/value_trace_multi_branch.spg`
- [x] 跨组件类型追溯测试：`test_value_trace_cross_component_types`
- [x] JSON 输出断言：summary/details/evidence 已覆盖

## P2：输出层来源分类定义

- [x] SourceType 枚举已存在：Param、UserInput、ModelAuto、System、Computed、Constant、Unknown
- [x] 分类规则已存在：`src/dependency.rs:determine_source_type`
- [x] 测试覆盖：`test_source_type_param`、`test_source_type_computed`、`test_source_type_model_auto`、`test_source_type_constant`

## P3：性能与规模测试

- [x] 大型文件 fixture：`tests/fixtures/large_page.spg`（150 组件）
- [x] 大量组件测试：`test_large_page_parsing`
- [x] 性能阈值测试：`test_large_page_topological_sort_performance`（debug < 2s）
- [x] 表达式引用数量测试：`test_large_page_expression_refs_count`

## P4：文档收敛

- [x] 更新 docs/schema.md（新增 SourceType 字段说明）
- [x] 更新 README.md（新增 P0-P3 测试说明）
- [x] 更新 SKILL.md（新增来源分类 AI 使用指南）
- [x] 提交变更

## 下一阶段：真实项目 AI 使用效果优化

- [x] M10：表与 DataFlow 单文件输出可理解，详见 `docs/real-project-optimization-roadmap.md`
- [x] M11：图数据库路径、只读与并发可用性，详见 `docs/real-project-optimization-roadmap.md`
- [x] M12：Explain 语义摘要与证据质量收敛，详见 `docs/real-project-optimization-roadmap.md`
- [x] M13：面向 AI 的低噪声 brief 输出模式，详见 `docs/real-project-optimization-roadmap.md`
- [x] M14：目标定位与命令规范防错，详见 `docs/real-project-optimization-roadmap.md`
- [ ] M15：真实项目 AI 评测集扩展，详见 `docs/real-project-optimization-roadmap.md`
- [ ] M16：SKILL.md 真实使用协议收敛，详见 `docs/real-project-optimization-roadmap.md`
