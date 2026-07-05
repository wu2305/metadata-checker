# M11：图数据库路径、只读与并发可用性

| milestone | M11 |
| status | unknown |
| archived_from | docs/real-project-optimization-roadmap.md |

---

## M11：图数据库路径、只读与并发可用性

### 目标

让项目级查询在真实只读项目、沙箱、多 Agent 并行场景下可稳定使用。

### 问题清单

- 项目级查询依赖项目根目录 `.metadata-checker.graphdb`，不存在时直接失败。
- `--build-graph` 默认写真实项目根目录，沙箱或只读目录下会失败。
- 图数据库创建成功后，普通查询仍可能因沙箱权限无法读取。
- 多个项目级查询并发执行时会出现 `Database already open. Cannot acquire lock`。
- `SKILL.md` 没有把 graphdb 缺失、权限失败、锁冲突定义为明确 fallback 流程。
- 没有临时图数据库路径，空模型只能在真实项目中写入副产物。
- 没有“只读检查 graphdb 是否可用”的轻量命令，AI 需要尝试真实查询才知道是否阻塞。

### 验收目标（M15 已收敛）

- CLI 增加 `--graph-db-path <PATH>`，支持将 graphdb 放在 `/tmp` 或工作区内。
- CLI 增加只读检查能力，例如 `--check-graph`，输出图数据库是否存在、可读、可写、是否需要 rebuild。
- 项目级查询在只读图数据库上支持并发读，或至少返回明确可恢复诊断与重试建议。
- `SKILL.md` 明确 graphdb 缺失时的 fallback：先检查、再构建到可写路径、最后退回单文件模式。
- 错误输出进入统一 JSON diagnostic，而不是只输出裸 `Error:`。
- 真实项目上可连续运行页面逻辑、模型查询、组件 explain，不因锁冲突失败。
