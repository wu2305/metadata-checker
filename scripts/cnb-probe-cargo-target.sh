#!/bin/sh
# 检查 cargo target 及父目录可写性；仅删除本次创建的探测文件。
# EACCES 不足以证明缓存损坏，失败时保留目录内容与权限，交由维护者排查。
# 用法：CARGO_TARGET_DIR=target/cnb/coverage sh scripts/cnb-probe-cargo-target.sh
set -u

TARGET_DIR="${CARGO_TARGET_DIR:-target/cnb/coverage}"
# 相对路径加 ./，避免以 - 开头的自定义路径被命令解释为选项。
case "$TARGET_DIR" in /*) ;; *) TARGET_DIR="./$TARGET_DIR" ;; esac
TARGET_PARENT="$(dirname "$TARGET_DIR")"
PROBE_PATH=""

cleanup() {
  if [ -n "$PROBE_PATH" ]; then
    rm -f "$PROBE_PATH"
  fi
}
trap cleanup 0
trap 'exit 1' HUP INT TERM

probe_directory() {
  PROBE_PATH="$(mktemp "$1/.cnb-write-probe.XXXXXXXXXX")" || return 1
  # 实际写入字节，避免只创建空文件而漏掉部分空间不足错误。
  printf 'probe\n' > "$PROBE_PATH" || return 1
  rm -f "$PROBE_PATH" || return 1
  PROBE_PATH=""
}

if mkdir -p "$TARGET_DIR" &&
  probe_directory "$TARGET_DIR" && probe_directory "$TARGET_PARENT"; then
  echo "[cargo-target] $TARGET_DIR 与其父目录可写"
  exit 0
fi

echo "[cargo-target] 写入探测失败；未清空缓存或更改目录权限" >&2
echo "[cargo-target] 检查权限、属主、挂载状态、剩余空间及探测命令错误" >&2
id >&2
ls -ld "$TARGET_PARENT" "$TARGET_DIR" >&2
exit 1
