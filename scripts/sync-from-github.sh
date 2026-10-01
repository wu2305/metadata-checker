#!/usr/bin/env bash
#
# sync-from-github.sh —— 把 GitHub 全量同步回当前仓库（CNB）。
#
# 触发方式（两条入口共用 .cnb.yml 的 .github_sync_from_ci 流水线）：
#   1. web_trigger_github_sync_from —— 页面按钮（.cnb/web_trigger.yml，人手动触发）
#   2. api_trigger_github_sync_from —— OPENAPI / cnb build start-build（LLM 与脚本调用）
#
# 风险消弭策略（分支管理）：
#   所有 GitHub 分支被推送到 CNB(origin) 的 `github-sync/<branch>` 命名空间，
#   所有 GitHub 标签被推送到 `github-sync-tags/<tag>` 命名空间。
#   绝不直写 CNB 既有分支（如 main）或既有标签，避免覆盖与冲突；
#   人工 review 后，再按正常 PR 流程把 github-sync/* 合并进目标分支。
#
# 密钥来源：CNB 密钥仓库 metadata-checker-keys 的 github-mirror.yml，
# 经 .cnb.yml imports 注入为环境变量（不在此处硬编码、不打印）：
#   GITHUB_MIRROR_URL    源 GitHub 仓库地址；未配置时默认 https://github.com/wu2305/metadata-checker.git
#   GITHUB_MIRROR_TOKEN  具备 repo 读权限的 GitHub PAT
set -euo pipefail

REMOTE_NAME="github"

# GITHUB_MIRROR_URL 未配置时使用仓库约定的默认 GitHub 地址。
if [ -z "${GITHUB_MIRROR_URL:-}" ]; then
  GITHUB_MIRROR_URL="https://github.com/wu2305/metadata-checker.git"
fi

# token 缺失则跳过。
if [ -z "${GITHUB_MIRROR_TOKEN:-}" ]; then
  echo "GITHUB_SYNC_SKIP: 未配置 GITHUB_MIRROR_TOKEN，跳过 GitHub 反向同步。"
  exit 0
fi

# 复用或新增 GitHub remote。
if git remote | grep -qx "$REMOTE_NAME"; then
  git remote set-url "$REMOTE_NAME" "$GITHUB_MIRROR_URL"
else
  git remote add "$REMOTE_NAME" "$GITHUB_MIRROR_URL"
fi

# 以 credential store 注入 token，避免 token 出现在日志或 URL 中。
git config --global credential.helper store
printf 'https://x-access-token:%s@github.com\n' "$GITHUB_MIRROR_TOKEN" >> "$HOME/.git-credentials"
chmod 0600 "$HOME/.git-credentials"

# 拉取 GitHub 全量分支与标签到独立命名空间，不污染本地分支/标签。
echo "==> git fetch $REMOTE_NAME (全量分支 + 标签)"
git fetch "$REMOTE_NAME" \
  '+refs/heads/*:refs/remotes/github/*' \
  '+refs/tags/*:refs/github-sync-tags/*'

# 把每个 GitHub 分支推到 CNB(origin) 的 github-sync/<branch> 命名空间。
pushed_branches=0
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  branch="${ref#refs/remotes/github/}"
  git push origin "+refs/remotes/github/$branch:refs/heads/github-sync/$branch"
  echo "pushed branch -> github-sync/$branch"
  pushed_branches=$((pushed_branches + 1))
done < <(git for-each-ref --format='%(refname)' refs/remotes/github)

# 把 GitHub 标签推到 CNB 的 github-sync-tags/<tag> 命名空间（标签不可前缀，单独命名空间避免冲突）。
pushed_tags=0
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  tag="${ref#refs/github-sync-tags/}"
  git push origin "+refs/github-sync-tags/$tag:refs/tags/github-sync-tags/$tag"
  echo "pushed tag -> github-sync-tags/$tag"
  pushed_tags=$((pushed_tags + 1))
done < <(git for-each-ref --format='%(refname)' refs/github-sync-tags)

echo "=== GitHub -> CNB 反向同步摘要 ==="
echo "分支推送数: $pushed_branches (命名空间 github-sync/*)"
echo "标签推送数: $pushed_tags (命名空间 github-sync-tags/*)"
echo ""
echo "人工 review 示例："
echo "  git fetch origin"
echo "  git log origin/github-sync/main"
echo "合并示例（按正常 PR 流程更稳妥）："
echo "  git checkout main && git merge origin/github-sync/main"
echo "GITHUB_SYNC_DONE"
