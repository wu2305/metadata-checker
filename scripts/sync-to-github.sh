#!/usr/bin/env bash
#
# sync-to-github.sh —— 把当前仓库（CNB）全量镜像到 GitHub。
#
# 触发方式：由 .cnb.yml 的 main.push 流水线阶段自动调用。
#
# 密钥来源：CNB 密钥仓库 metadata-checker-keys 的 github-mirror.yml，
# 经 .cnb.yml imports 注入为环境变量（不在此处硬编码、不打印）：
#   GITHUB_MIRROR_URL    目标 GitHub 仓库地址；未配置时默认 https://github.com/wu2305/metadata-checker.git
#   GITHUB_MIRROR_TOKEN  具备 repo 写权限的 GitHub PAT（脚本以 x-access-token 形式注入）
#
# 行为：先拉取 CNB(origin) 的全部分支与标签到本地，再用带 --prune 的显式
#       heads/tags refspec 镜像到 GitHub（远端多余分支/标签删除）。
#       不用 `git push --mirror`：--mirror 会尝试删除 GitHub 的 refs/pull/* 等
#       隐藏引用，一旦 GitHub 上开过 PR 就会报 "deny updating a hidden ref"
#       使整条推送失败；显式 refspec 只覆盖 refs/heads/* 与 refs/tags/*，
#       不触碰 refs/pull/*、refs/notes/*。
#       隔离命名空间 github-sync/* 与 github-sync-tags/* 只在 CNB 内部使用，
#       推送前先删除本地同名引用，避免回流到 GitHub。
#       token 未配置时优雅跳过（打印 GITHUB_SYNC_SKIP 并以 0 退出），
#       避免 token 尚未在 CNB 后台配置时把整条 main CI 判红。
set -euo pipefail

REMOTE_NAME="github-mirror"

# GITHUB_MIRROR_URL 未配置时使用仓库约定的默认 GitHub 地址。
if [ -z "${GITHUB_MIRROR_URL:-}" ]; then
  GITHUB_MIRROR_URL="https://github.com/wu2305/metadata-checker.git"
fi

# token 缺失则跳过，不阻断 CI。
if [ -z "${GITHUB_MIRROR_TOKEN:-}" ]; then
  echo "GITHUB_SYNC_SKIP: 未配置 GITHUB_MIRROR_TOKEN，跳过 GitHub 镜像推送。"
  exit 0
fi

# 复用或新增 GitHub remote。
if git remote | grep -qx "$REMOTE_NAME"; then
  git remote set-url "$REMOTE_NAME" "$GITHUB_MIRROR_URL"
else
  git remote add "$REMOTE_NAME" "$GITHUB_MIRROR_URL"
fi

# 以 credential store 注入 token，避免 token 出现在日志或 URL 中。
# 使用追加重定向，避免覆盖同一容器内其它阶段写入的凭证。
git config --global credential.helper store
printf 'https://x-access-token:%s@github.com\n' "$GITHUB_MIRROR_TOKEN" >> "$HOME/.git-credentials"
chmod 0600 "$HOME/.git-credentials"

# 先把 CNB(origin) 的全部分支与标签拉到本地，保证镜像内容完整。
git fetch origin '+refs/heads/*:refs/heads/*' '+refs/tags/*:refs/tags/*'

# 删除本地隔离命名空间引用（github-sync/* 分支、github-sync-tags/* 标签），
# 这些只存在于 CNB 侧，不应回流到 GitHub；--prune 同时清除 GitHub 上可能已有的同名引用。
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  git update-ref -d "$ref"
done < <(git for-each-ref --format='%(refname)' refs/heads/github-sync refs/tags/github-sync-tags)

# 全量镜像（分支 + 标签）：--prune 按 refspec 目标命名空间删除远端多余引用，
# 达到与 --mirror 相同的镜像效果，但不触碰 GitHub 的 refs/pull/* 等隐藏引用。
echo "==> git push --prune $REMOTE_NAME (heads + tags)"
git push --prune "$REMOTE_NAME" \
  '+refs/heads/*:refs/heads/*' \
  '+refs/tags/*:refs/tags/*'

echo "GITHUB_SYNC_DONE: 已全量镜像到 $GITHUB_MIRROR_URL"
