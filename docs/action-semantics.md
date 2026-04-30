# Action 语义表

> 版本：1.0  
> 目标：让 AI 不需要猜 action 名称，也能知道动作属于数据写入、导航、参数设置、UI 控制、校验还是刷新。

## action_category 枚举

| 分类 | 说明 | 典型 actionType |
|------|------|----------------|
| `data_write` | 数据写入动作 | submitData, insertData, updateData, deleteData |
| `data_read` | 数据读取/查询动作 | loadData |
| `navigation` | 页面跳转/导航 | link, showDialog |
| `param_mutation` | 页面参数设置 | setParamValue |
| `ui_control` | UI 组件控制 | showComponent, hideComponent, switchPanel, closeDialog |
| `validation` | 数据校验 | validateData |
| `data_refresh` | 数据刷新/重置 | refreshModels, resetData, newData |
| `unknown` | 未识别的动作类型 | 任意不在下表中的类型 |

## actionType → action_category 映射

| actionType | action_category | 语义说明 |
|-----------|-----------------|----------|
| submitData | data_write | 提交组件数据到模型字段 |
| insertData | data_write | 向模型插入新记录 |
| updateData | data_write | 更新模型现有记录 |
| deleteData | data_write | 删除模型记录（逻辑删除或物理删除） |
| setParamValue | param_mutation | 设置页面参数值 |
| link | navigation | 跳转到目标页面，可附带参数传递 |
| showDialog | navigation | 打开对话框页面 |
| closeDialog | ui_control | 关闭当前对话框 |
| switchPanel | ui_control | 切换面板/标签页显示 |
| showComponent | ui_control | 显示指定组件 |
| hideComponent | ui_control | 隐藏指定组件 |
| newData | data_refresh | 新建数据（清空表单或初始化新记录） |
| loadData | data_read | 加载模型数据到页面 |
| resetData | data_refresh | 重置模型/组件数据到初始状态 |
| refreshModels | data_refresh | 刷新多个模型数据 |
| validateData | validation | 校验数据合法性 |

## 图边语义分类

| EdgeType | 说明 | 对应 action_category |
|----------|------|---------------------|
| ActionReads | action 读取模型/字段 | data_write, data_read, validation |
| ActionWrites | action 写入模型/字段 | data_write |
| ActionNavigates | action 导航到页面 | navigation |
| ActionSetsParam | action 设置页面参数 | param_mutation |
| ActionControlsComponent | action 控制组件显隐 | ui_control |
| ActionValidates | action 校验数据 | validation |
| ActionLoadsData | action 加载数据 | data_read, data_refresh |

## 结构化字段定义

### action_flows[] 字段

```json
{
  "action_id": "action1",
  "action_type": "submitData",
  "action_category": "data_write",
  "semantic_summary": "点击 button1 后提交 input1 到 model1.name",
  "component_id": "comp:app/page.spg|button1",
  "trigger_type": "click",
  "wait_prev": "button11.action1",
  "blocks_on": {
    "component_id": "button11",
    "action_id": "action1",
    "raw": "button11.action1",
    "resolved": true
  },
  "condition": {
    "raw_expr": "input1.value=='test'",
    "refs": ["input1.value"],
    "resolved_refs": [{"type": "ComponentValue", "id": "input1", "field": "value"}],
    "confidence": "medium"
  },
  "reads": [...],
  "writes": [...],
  "navigation": [...],
  "sets_params": [...],
  "passes_params": [...]
}
```

### blocks_on（waitPrev 结构化）

| 字段 | 类型 | 说明 |
|------|------|------|
| `raw` | string | 原始 waitPrev 值，如 "button11.action1" |
| `component_id` | string | 提取的组件 ID，如 "button11" |
| `action_id` | string | 提取的 action ID，如 "action1" |
| `resolved` | boolean | 是否成功解析为结构化字段 |

### condition（条件执行结构化）

| 字段 | 类型 | 说明 |
|------|------|------|
| `raw_expr` | string | 原始 condition/conditionExp 表达式 |
| `refs` | array[string] | 粗粒度引用的组件/模型 ID 列表（正则提取） |
| `resolved_refs` | array[object] | 结构化引用（类型 + ID + 字段），M7 后完整实现 |
| `confidence` | string | high / medium / low，取决于解析完整度 |

## 未知动作类型处理

当 actionType 不在上表中时：
- `action_category`: "unknown"
- `semantic_summary`: "执行未知类型动作 {actionType}"
- 生成 diagnostic: `UNKNOWN_ACTION_TYPE`

## AI 使用指南

1. 优先查看 `action_category`，判断动作属于哪类语义。
2. 查看 `semantic_summary` 获取人类可读的动作意图。
3. 需要细节时查看 `reads` / `writes` / `navigation` / `sets_params` / `passes_params`。
4. 注意 `blocks_on`：如果存在，说明该动作需要等待前置动作完成。
5. 注意 `condition`：如果存在，说明动作有条件执行，不应默认一定会触发。
