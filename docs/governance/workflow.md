# Git 与 PR 工作流

## 分支

- 命名：`codex/<topic>` 或 `feat/<topic>`
- 从 `main` 拉出；长期跟踪分支仅 `main`
- merge 后**删除远程来源分支**

## `main` 保护

- **`main` 只接受 PR merge**，禁止直接 `git push` 到 `main`
- CI 验证也通过 PR 触发（打开/更新 PR 或 merge 后 `main.push`）
- 历史例外：曾为验证 CI 修复做过直推，**自 2026-07-05 起废止**

## PR 要求

使用 [pr-template.md](pr-template.md)，至少包含：

1. **Summary** — 1–3 句
2. **Test plan** — 可复制的命令
3. **Docs** — 变更的 `docs/` 路径
4. **Baseline impact** — 是否影响 `performance-baseline.md` / Bencher 趋势

## 合并后

```bash
# 示例：删除已 merge 的远程分支
git push cnb --delete codex/<topic>
git branch -d codex/<topic>
```

### 陈旧分支（Phase C 记录）

若 `cnb/codex/m52-redb-v2-and-ci` 与 `cnb/codex/m52-performance-optimization` 差异已并入 `main`（`f1e7285` 及以后），merge Phase C PR 后删除上述远程分支。

## 提交信息

格式：`type: description`（与 AGENTS.md 一致）

常用 type：`feat` / `fix` / `perf` / `test` / `docs` / `refactor`
