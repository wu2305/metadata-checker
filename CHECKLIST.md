# SuperPage 元数据解析器测试用例 Checklist

## 一、元数据结构解析测试 (Metadata Structure Parsing)

### 1.1 基础字段解析
- [x] 版本号解析 (version)
- [x] 主题解析 (theme)
- [x] 页面参数解析 (params: id, name, desc, value)
- [x] 数据源解析 (sources: id, modelType, path)
- [x] 缺失基础字段的容错处理（缺 version / 缺 canvas / 缺 id 均已覆盖）
- [x] 空 params/sources 数组处理

### 1.2 组件树解析
- [x] 单层组件解析
- [x] 嵌套组件解析 (canvas → panel → child)
- [x] 深层嵌套组件解析 (≥4 层)
- [x] 缺失 id/type 字段的组件容错
- [x] 空 components 数组处理
- [x] 组件类型覆盖: input, text, panel, button, list, steps, combobox, fieldsFilter
- [x] 特殊容器组件: panelbook, floatdialog, dialog, fieldsFilter
- [x] 列表组件: list + columns (list1.column1.value)
- [x] fieldsFilter.comp 子组件提取

### 1.3 表达式字段提取
- [x] value 字段提取
- [x] defaultValue 字段提取
- [x] visible 字段提取
- [x] exp 字段提取
- [x] enable 字段提取
- [x] text 字段提取
- [x] formula 字段提取
- [x] html 字段提取
- [x] 多字段同时有表达式的情况

## 二、表达式解析测试 (Expression Parsing)

### 2.1 引用类型识别
- [x] 组件值引用: input1.value
- [x] 组件属性引用: input1.checked.value
- [x] 模型字段引用: model1.A
- [x] 页面参数引用: param1
- [x] 用户属性引用: $user.dept_id
- [x] 系统变量引用: $project, $user (无属性)
- [x] 列表列引用: list1.column1.value
- [x] steps 组件引用: steps1.step
- [x] 带空格的组件ID引用

### 2.2 表达式语法覆盖
- [x] 简单赋值: =input1.value
- [x] 算术运算: +, -, *, /
- [x] 条件表达式: IF(...)
- [x] 宏表达式: ${model2.C}
- [x] 嵌套 IF: IF(IF(...), ...)
- [x] 逻辑运算符: AND, OR, NOT
- [x] 比较运算符: =, !=, >, <, >=, <=
- [x] 字符串拼接: CONCAT(...)
- [x] 函数调用: TOSTR(), TODAY(), ROUND()
- [x] 多参数函数: IF(a, b, c)
- [x] 空值判断: IS NULL, != NULL
- [x] 复杂嵌套: CONCAT('前缀', input2.value, '中缀', input3.value, '后缀')

### 2.3 边界情况
- [x] 字符串字面量跳过: 'abc', ""
- [x] 数字常量跳过: 0, 100
- [x] 空表达式: ""
- [x] 纯常量表达式: =123
- [x] 无引用的表达式: =TODAY()
- [x] 超长表达式处理
- [ ] 包含换行符的表达式
- [x] 包含中文的表达式（当前正则不支持中文变量名，但不应panic）

## 三、依赖关系测试 (Dependency Analysis)

### 3.1 依赖图构建
- [x] 简单依赖: input3 → input2, input1
- [x] 链式依赖: input1 → input2 → input3
- [x] 多源依赖: compA → (compB, compC, compD)
- [x] 分支依赖: compA → compB, compA → compC
- [x] 交叉依赖: compA → compB, compB → compC, compA → compC
- [x] 无依赖组件（独立组件）
- [x] 自引用组件

### 3.2 拓扑排序
- [x] 无环场景拓扑排序
- [x] 多独立组件的排序
- [ ] 同一层级组件的顺序稳定性
- [ ] 大量组件（>100）的排序性能

### 3.3 循环依赖检测
- [x] 无环场景
- [x] 简单两组件循环: A → B → A
- [x] 三组件循环（隐含在简单循环中）
- [ ] 复杂循环: A → B, B → C, C → D, D → B
- [x] 自引用循环: A → A

## 四、值来源追溯测试 (Value Source Tracing)

> 当前实现已从早期组件内 `value_trace` 扩展为 `ComponentQuery`、`--explain`、`--context` 与 DataFlow 字段级 lineage。旧版 `UserInput/ModelAuto/Computed/Constant` 分类未作为稳定输出枚举，后续若需要应另起里程碑重新定义。

### 4.1 追溯链展开
- [x] 单组件查询输出 `value_trace`
- [x] 字段 inputField 映射 lineage（DataFlow inputField）
- [x] 字段表达式计算 lineage（DataFlow exp）
- [x] 页面 action 写入字段 lineage（submitData/updateData 等）
- [x] 链式 DataFlow 字段 lineage（df_a → physical_x → df_b）
- [x] `--context` 中输出字段周边 lineage
- [ ] 组件值到页面参数的单级追溯 fixture（旧目标: input1 → param1）
- [ ] 组件值多级追溯 fixture（旧目标: input3 → input2 → param1-model1.A）
- [ ] 多分支组件值追溯 fixture（旧目标: input5 → input1/input2）

### 4.2 来源类型识别
- [x] 表达式引用类型识别：Param
- [x] 表达式引用类型识别：SystemVar
- [x] 表达式引用类型识别：ModelField
- [x] 表达式引用类型识别：ComponentValue / ComponentProperty
- [x] 常量/纯函数表达式不产生引用
- [ ] 输出层稳定来源分类枚举（UserInput / ModelAuto / Computed / Constant）未定义，需后续重新设计

## 五、错误处理测试 (Error Handling)

### 5.1 文件级错误
- [x] 文件不存在
- [x] 非 JSON 格式文件（缺少逗号）
- [x] 损坏的 JSON 文件
- [x] 空文件

### 5.2 结构级错误
- [x] 缺少 canvas 字段
- [x] 缺少 version 字段
- [x] 组件缺少 id 字段（跳过处理）
- [x] 组件缺少 type 字段（跳过处理）
- [x] params 元素缺少 id 字段
- [x] sources 元素缺少 id 字段

### 5.3 表达式级错误
- [x] 引用的组件不存在
- [x] 不完整的表达式: =input1.（仍被提取，refs为空）
- [x] 无效的系统变量: $unknown（被提取为 SystemVar）

## 六、真实场景测试 (Real-world Scenarios)

### 6.1 实际 .spg 文件解析
- [x] 新增活动（含 panelbook/panels）
- [x] 购车贷款（含 exp 过滤条件）
- [x] 代付款协议（含 dataflow 数据源、复杂 filter）
- [x] 销售合同（含 fieldsFilter、USER_INGROUP）
- [x] 解析含 actions 的文件
- [x] 解析含 dialog 的文件（showDialog/closeDialog 边已覆盖）

### 6.2 性能测试
- [ ] 大型文件解析（>100KB）
- [ ] 大量组件解析（>100个组件）
- [ ] 大量表达式解析（>200个表达式）

## 七、M9-A 真实语料索引与样本清单

- [x] 生成 `tests/fixtures/corpus/manifest.json`（仅元信息，不复制真实大文件）
- [x] 样本记录包含 `stable_id/source_kind/path/file_type/size_bytes/coverage_tags`
- [x] 覆盖标签包含 `actions/DataFlow/dialog/conditionExp/visibility/waitPrev/link_param/refresh_action/submit_write/embedded_page/duplicate_action_id/readonly_page`
- [x] 明确标注 `detection_method`（启发式文本扫描）
- [x] 增加 manifest 约束测试（存在性、schema 基础字段、来源白名单、关键 tag 覆盖）
- [x] 文档化维护方式（`docs/corpus-manifest.md`）

## 八、M9-B 精选回归语料

- [x] 生成 `tests/fixtures/corpus/selection.json`
- [x] 精选 12 条代表样本（5 copied + 7 referenced）
- [x] copied 样本文件存在且可解析
- [x] referenced fixture 文件存在且可解析
- [x] 总体积受控
- [x] 来源路径不得引用 `/Users/wuhaocheng/Downloads/bi`
- [x] 覆盖复杂页面、只读页面、表单写入、弹窗、visibility、DataFlow、链式物理表、刷新类 action、conditionExp、waitPrev、link_param、duplicate_action_id
- [x] 对 `tests/fixtures/test_project` 执行 build graph 回归测试

## 九、M9-C Snapshot 输出契约

- [x] 建立 `tests/fixtures/corpus/snapshots/snapshot_cases.json`
- [x] 覆盖 `--query-page-logic`
- [x] 覆盖 `--explain`
- [x] 覆盖 `--context`
- [x] 覆盖 `--query-dataflow`
- [x] snapshot 只保存规范化结构摘要，避免 raw JSON 噪声
- [x] snapshot 测试自建 graphdb，不依赖被 git ignore 的本地数据库
- [x] 支持 `UPDATE_CORPUS_SNAPSHOTS=1 cargo test --test corpus_snapshot_tests` 更新

## 十、M9-D AI 问答评测集

- [x] 建立 `tests/fixtures/corpus/ai_eval/ai_eval_cases.json`
- [x] 至少 10 个 AI eval case
- [x] 每个 case 包含 question、expected_facts、forbidden_claims、evidence_requirements、answer_rubric、uncertainty_policy
- [x] eval case 的 CLI 命令真实可执行
- [x] 空上下文 `5.4-mini` 抽测通过
- [x] 同步仓库根 `SKILL.md` 的 AI 回答协议

## 十一、M9-E AI Eval 自动判分与执行隔离

- [x] 引入 `expected_output_assertions` 结构化断言
- [x] 支持 `path/op/value/field` 断言
- [x] 支持多命令 case 的 `command_index`
- [x] 使用隔离临时目录构建 graphdb，避免 redb 锁冲突
- [x] 串行执行 eval CLI
- [x] DataFlow 链路断言精确校验 `field:df_b.id` 追溯到 `field:df_a.id`
- [x] `cargo test --test ai_eval_tests` 覆盖结构化断言执行

## 十二、M9-F AI Eval 独立运行器与回答判分

- [x] `minimal_command_plan` 升级为结构化对象（command_kind / target / args / requires_project_dir / budget）
- [x] 测试校验结构化 plan 可展开为 `required_commands`
- [x] 增加 `answer_assertions`
- [x] 增加 `risk_tags` 并覆盖 page_logic / explain / context / dataflow / lineage / condition / diagnostic
- [x] 增加 `max_command_count`
- [x] 增加 `allowed_output_sections`
- [x] 增加 `case_status`
- [x] 增加 `difficulty`
- [x] 增加 `answer_style_policy`
- [x] 增加 `docs/ai-eval-run-template.md`
- [x] `readonly_page_check` 使用 `--query-page-logic` 并验证 `readonly_dashboard`
- [x] 空上下文 `5.4-mini` 抽测通过
- [x] `cargo test --test ai_eval_tests` 当前 23 项测试通过

## 十三、剩余待办

- [ ] 包含换行符的表达式
- [ ] 同一层级组件的顺序稳定性
- [ ] 大量组件（>100）的排序性能
- [ ] 复杂循环: A → B, B → C, C → D, D → B
- [ ] 组件值到页面参数的单级追溯 fixture
- [ ] 组件值多级追溯 fixture
- [ ] 多分支组件值追溯 fixture
- [ ] 输出层稳定来源分类枚举（UserInput / ModelAuto / Computed / Constant）定义与测试
- [ ] 大型文件解析性能（>100KB）
- [ ] 大量表达式解析性能（>200个表达式）
