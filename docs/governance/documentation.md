# 文档类型与归档

## 四类文档

| 类型 | 目录 | 何时写 | 生命周期 |
|------|------|--------|----------|
| **Reference** | `docs/reference/` | 协议、schema、对外契约随代码变 | 持续维护 |
| **Milestone journal** | `docs/milestones/<area>/` | 每个 M 一条工作线 | `active` → `done` → `archive/` |
| **Spec** | `docs/specs/` | 编码前 | `approved` 后实施 |
| **Plan** | `docs/plans/` | spec 批准后 | `done` 后归档 |
| **Runbook** | `docs/runbooks/` | 需人工环境步骤 | 随环境更新 |

## 命名

- Journal：`mNN-<kebab-slug>.md`（如 `m53-performance-continuation.md`）
- Spec / Plan：`YYYY-MM-DD-<topic>-{design|plan}.md`
- Archive roadmap：`archive/roadmap/mNN-<slug>.md`

## 迁移期 stub

PR-C2/C3 搬迁后，旧路径保留 ≤1 个 release 周期的 stub：

```markdown
# 已迁移
见 docs/<new-path>
```

## 禁止

- 向 `real-project-optimization-roadmap.md` **追加**新里程碑（已冻结，PR-C3 拆档）
- 在 `docs/` 根目录新建无 INDEX 登记的 `mXX-*.md`
- 静态站点生成（后续单独里程碑）

## 入口

所有 Agent 与人类读者从 [docs/README.md](../README.md) 进入。
