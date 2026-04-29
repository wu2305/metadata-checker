# Action 语义表

> 定义低代码平台常见 action 的标准语义分类，帮助 AI 无需猜测 action 名称即可理解行为类别。

## Action 分类体系

| 分类 | Action 名称 | 语义说明 | 读写特征 |
|------|-------------|----------|----------|
| **数据写入** | `submitData` | 提交表单数据到指定模型 | Writes |
| | `insertData` | 向模型插入新记录 | Writes |
| | `updateData` | 更新模型中的记录 | Writes |
| | `deleteData` | 删除模型中的记录 | Writes |
| | `newData` | 创建新数据实例 | Writes |
| **数据加载** | `loadData` | 加载模型数据到页面 | Reads |
| | `resetData` | 重置模型数据状态 | Reads + Writes |
| | `refreshModels` | 刷新多个模型的数据 | Reads |
| **导航** | `link` | 跳转到指定页面/URL | Navigates |
| | `showDialog` | 显示对话框页面 | Navigates |
| | `switchPanel` | 切换面板显示 | Navigates |
| **参数设置** | `setParamValue` | 设置页面参数值 | SetsParam |
| **UI 控制** | `showComponent` | 显示指定组件 | ControlsComponent |
| | `hideComponent` | 隐藏指定组件 | ControlsComponent |
| **校验** | `validateData` | 校验数据有效性 | Validates |

## 图边分类映射

每个 action 在图数据库中应产生对应类型的边：

```
ActionReads          → action 读取模型字段（loadData, refreshModels）
ActionWrites         → action 写入模型字段（submitData, insertData, updateData, deleteData）
ActionNavigates      → action 导航到页面（link, showDialog, switchPanel）
ActionSetsParam      → action 设置参数（setParamValue）
ActionControlsComponent → action 控制组件显示/隐藏（showComponent, hideComponent）
ActionValidates     → action 校验数据（validateData）
ActionLoadsData      → action 加载数据（loadData, resetData, refreshModels）
```

## Action 语义字段

每个 action 输出应包含：

```json
{
  "action_type": "submitData",
  "action_category": "数据写入",
  "semantic_summary": "将表单数据提交到 model1",
  "reads_from": [ /* 读取的组件/参数 */ ],
  "writes_to": [ /* 写入的模型/字段 */ ],
  "navigates_to": null,
  "sets_params": null,
  "controls_components": null,
  "validates": null,
  "trigger_type": "click",
  "condition": null,
  "wait_prev": false
}
```

## 使用指南

AI 在分析 action 时，不应依赖 action 名称的字面含义，而应查询此语义表确定行为类别。

例如：
- 看到 `submitData` → 数据写入类，关注 `writes_to`
- 看到 `link` → 导航类，关注 `navigates_to`
- 看到 `setParamValue` → 参数设置类，关注 `sets_params`
