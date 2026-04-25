# SuperPage 元数据解析器测试用例 Checklist

## 一、元数据结构解析测试 (Metadata Structure Parsing)

### 1.1 基础字段解析
- [x] 版本号解析 (version)
- [x] 主题解析 (theme)
- [x] 页面参数解析 (params: id, name, desc, value)
- [x] 数据源解析 (sources: id, modelType, path)
- [ ] 缺失基础字段的容错处理
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
- [ ] text 字段提取（未在 fixture 中测试）
- [ ] formula 字段提取（未在 fixture 中测试）
- [ ] html 字段提取（未在 fixture 中测试）
- [x] 多字段同时有表达式的情况

## 二、表达式解析测试 (Expression Parsing)

### 2.1 引用类型识别
- [x] 组件值引用: input1.value
- [x] 组件属性引用: input1.checked.value
- [x] 模型字段引用: model1.A
- [x] 页面参数引用: param1
- [x] 用户属性引用: $user.dept_id
- [ ] 系统变量引用: $project, $user (无属性)
- [x] 列表列引用: list1.column1.value
- [x] steps 组件引用: steps1.step
- [ ] 带空格的组件ID引用

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
- [ ] 超长表达式处理
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

### 4.1 追溯链展开
- [ ] 单级追溯: input1 → param1
- [ ] 多级追溯: input3 → input2 → param1-model1.A
- [ ] 多分支追溯: input5 → (input1, input2)
- [ ] 跨组件类型追溯

### 4.2 来源类型识别
- [ ] 参数来源识别 (Param)
- [ ] 用户输入识别 (UserInput)
- [ ] 模型自动获取识别 (ModelAuto)
- [ ] 系统变量识别 (System)
- [ ] 组件计算识别 (Computed)
- [ ] 常量识别 (Constant)

## 五、错误处理测试 (Error Handling)

### 5.1 文件级错误
- [x] 文件不存在
- [x] 非 JSON 格式文件（缺少逗号）
- [x] 损坏的 JSON 文件
- [x] 空文件

### 5.2 结构级错误
- [x] 缺少 canvas 字段
- [ ] 缺少 version 字段
- [x] 组件缺少 id 字段（跳过处理）
- [x] 组件缺少 type 字段（跳过处理）
- [ ] params 元素缺少 id 字段
- [ ] sources 元素缺少 id 字段

### 5.3 表达式级错误
- [ ] 引用的组件不存在
- [x] 不完整的表达式: =input1.（仍被提取，refs为空）
- [x] 无效的系统变量: $unknown（被提取为 SystemVar）

## 六、真实场景测试 (Real-world Scenarios)

### 6.1 实际 .spg 文件解析
- [x] 新增活动（含 panelbook/panels）
- [x] 购车贷款（含 exp 过滤条件）
- [x] 代付款协议（含 dataflow 数据源、复杂 filter）
- [x] 销售合同（含 fieldsFilter、USER_INGROUP）
- [ ] 解析含 actions 的文件
- [ ] 解析含 dialog 的文件（已有 dialog 类型，但未测试 actions）

### 6.2 性能测试
- [ ] 大型文件解析（>100KB）
- [ ] 大量组件解析（>100个组件）
- [ ] 大量表达式解析（>200个表达式）
