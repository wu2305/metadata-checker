# M14：目标定位与命令规范防错

| milestone | M14 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M14：目标定位与命令规范防错

### 目标

减少 AI 在真实项目中找不到目标、写错 target、shell 命令被管道拆开的概率。

### 问题清单

- `next_queries` 中 `comp:...|button1` 未加引号，shell 会把 `|` 当管道。
- M10 遗留观察：单文件 `.tbl` 输出的 `next_queries` 也需要纳入统一命令规范，不能只修页面/组件 target。
- `SKILL.md` 虽有 ID 格式说明，但没有强制规定所有含 `|`、空格、中文路径的 target 必须加单引号。
- 真实项目存在大量同名或局部名模型，如 `model1/model5/model74`，全项目查询容易命中错误节点。
- 缺少 `find`/`search` 类命令，AI 要先依赖 `rg --files` 或猜路径。
- 缺少“从页面内局部模型 ID 解析到真实表/DataFlow”的命令。
- 查询错误时缺少候选目标建议，例如“你可能想查 page:... 或 comp:...|...”。
- `--query-model` 接收裸模型名，但 `--explain` 接收 `model:` 前缀，规范差异容易让模型混用。

### 验收目标（M15 已收敛）

- 所有 `next_queries` 对含 `|`、空格、中文路径的 target 自动加单引号。
- 所有 `next_queries` 都应输出可直接复制执行的命令片段；无法确定 `<DIR>` 或 `<MODEL>` 时必须明确标注占位符来源和解析方式。
- `SKILL.md` 增加命令引用硬规则：target 一律单引号包裹。
- CLI 增加 `--find-page <KEYWORD>`、`--find-model <KEYWORD>`、`--find-component <KEYWORD>`。
- CLI 增加页面作用域解析能力，例如 `--resolve-model page:... model5`。
- 查询目标不存在时，返回候选列表与推荐下一条命令。
- 统一说明 `--query-model` 与 `--explain model:` 的参数差异，或在 CLI 内兼容两种写法。
