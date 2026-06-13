#!/usr/bin/env bash
# 在本地终端运行此脚本，拉起带 metadata-checker 扩展的 Chrome。
# 不要依赖 Cursor agent 会话内 spawn——会话结束可能清理子进程树。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BROWSER_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(cd "${BROWSER_ROOT}/.." && pwd)"

EXT_DIR="${EXT_DIR:-${BROWSER_ROOT}/artifacts/metadata-checker-extension-chromium}"
CHROME_APP="${CHROME_APP:-/Users/wuhaocheng/Library/Caches/ms-playwright/chromium-1223/chrome-mac-arm64/Google Chrome for Testing.app}"
CHROME_BIN="${CHROME_BIN:-${CHROME_APP}/Contents/MacOS/Google Chrome for Testing}"
PROFILE="${PROFILE:-/private/tmp/metadata-checker-m47-pixi-chrome-profile}"
DEBUG_PORT="${DEBUG_PORT:-9222}"
PAGE_URL="${PAGE_URL:-https://autocrm-test.xiaoshouyi.com/xiaoshouyi/app/%E4%BB%B7%E5%AE%A1.app?:edit=true&:file=%E9%94%80%E5%94%AE%E8%AE%A2%E5%8D%95%E4%BB%B7%E6%A0%BC%E5%AE%A1%E6%89%B9-%E4%BF%A1%E6%81%AF%E8%A1%A5%E5%85%85.spg}"
LOG="${LOG:-/private/tmp/metadata-checker-chrome.log}"
PID_FILE="${PID_FILE:-/private/tmp/metadata-checker-chrome.pid}"
USE_OPEN="${USE_OPEN:-1}"

if [[ ! -d "${EXT_DIR}" ]]; then
  echo "extension dir not found: ${EXT_DIR}" >&2
  echo "run prepare-extension-package first" >&2
  exit 1
fi

mkdir -p "${PROFILE}"

ARGS=(
  "--remote-debugging-port=${DEBUG_PORT}"
  "--user-data-dir=${PROFILE}"
  "--disable-extensions-except=${EXT_DIR}"
  "--load-extension=${EXT_DIR}"
  "--no-first-run"
  "--no-default-browser-check"
  "--disable-popup-blocking"
  "--window-size=1440,1100"
  "${PAGE_URL}"
)

if [[ -f "${PID_FILE}" ]]; then
  OLD_PID="$(cat "${PID_FILE}" 2>/dev/null || true)"
  if [[ -n "${OLD_PID}" ]] && kill -0 "${OLD_PID}" 2>/dev/null; then
    echo "Chrome already running (pid=${OLD_PID})"
    echo "CDP: http://127.0.0.1:${DEBUG_PORT}"
    exit 0
  fi
fi

if [[ "${USE_OPEN}" == "1" && -d "${CHROME_APP}" ]]; then
  # 通过 LaunchServices 启动，与当前 shell/agent 会话解耦
  open -na "${CHROME_APP}" --args "${ARGS[@]}"
  echo "launched via open(1): ${CHROME_APP}"
else
  if [[ ! -x "${CHROME_BIN}" ]]; then
    echo "chrome binary not found: ${CHROME_BIN}" >&2
    exit 1
  fi
  nohup "${CHROME_BIN}" "${ARGS[@]}" >>"${LOG}" 2>&1 &
  echo $! > "${PID_FILE}"
  echo "launched via nohup pid=$(cat "${PID_FILE}")"
fi

echo "extension: ${EXT_DIR}"
echo "profile:   ${PROFILE}"
echo "CDP:       http://127.0.0.1:${DEBUG_PORT}"
echo "log:       ${LOG}"
