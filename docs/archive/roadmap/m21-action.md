# M21：未知 Action 语义归类

| milestone | M21 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M21：未知 Action 语义归类

### 目标

减少 `UNKNOWN_ACTION_TYPE` 对复杂页面理解的干扰，让 AI 能识别附件查看、接口发送、消息提示和脚本执行的基础语义。

### 问题清单

- `合同协议.spg` 中大量 `script`、`webAPI`、`showMessage`、`showFilesGallary` 被标记为 `UNKNOWN_ACTION_TYPE`。
- 未知 action 会淹没 `key_findings`，导致 AI 只能保守地说“工具无法理解部分动作”。
- 附件查看、接口发送、消息提示是低代码页面的常见关键行为。

### 工作清单

- 为 `script` 增加 `script_execution` 语义分类。
- 为 `webAPI` 增加 `api_call` 语义分类，并保留接口参数证据。
- 为 `showMessage` 增加 `message_prompt` 语义分类。
- 为 `showFilesGallary` 增加 `file_gallery` 语义分类，识别附件数据集和附件字段。
- 将这些 action 纳入 `action_category` 和 PageLogic action flow。
- 保留无法解析参数的 diagnostic，但不再输出高噪声 `UNKNOWN_ACTION_TYPE`。

### 验收目标

- 复杂页面摘要不再被同类 `UNKNOWN_ACTION_TYPE` 重复刷屏。
- AI 能识别附件查看、接口发送、消息提示、脚本执行。
- 未解析脚本内容时仍保持保守，不编造脚本内部语义。
