#!/usr/bin/env python3
"""跑一条命令并记录 wall time 与峰值 RSS，输出 JSON。

为什么不用 `/usr/bin/time -v`：CNB 的 browser-wasm-ci 镜像里没装 GNU time
（`/usr/bin/time: No such file or directory`，只有 shell 内建的 `time`，
而内建 time 不给 RSS）。这里用 `resource.getrusage(RUSAGE_CHILDREN)`，
Linux 上 `ru_maxrss` 单位是 KiB，取的是已回收子进程的峰值上界。

只跑一个子进程，所以 RUSAGE_CHILDREN 的最大值就是该进程的峰值，不会被兄弟
进程污染；若将来在同一个解释器里连跑多条命令，读数会变成「至今为止所有子
进程的最大值」——那时必须每条命令各起一次本脚本。

fail-closed：子进程非零退出时本脚本同码退出，JSON 仍写出（含 exit_code），
便于失败也留下读数。

用法：
    python3 tools/peak-rss-run.py --json out.json -- <命令> [参数...]
"""

import argparse
import json
import resource
import subprocess
import sys
import time


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--json", required=True, help="读数写到这个文件")
    parser.add_argument("--label", default="", help="记进 JSON 的自由标签")
    parser.add_argument(
        "command",
        nargs=argparse.REMAINDER,
        help="`--` 之后的整条命令",
    )
    args = parser.parse_args()

    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        parser.error("缺少要执行的命令（用 `--` 分隔）")

    before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    start = time.monotonic()
    completed = subprocess.run(command)
    wall_s = time.monotonic() - start
    after = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss

    # ru_maxrss 是「至今所有子进程的最大值」，取差值没有意义；直接取 after，
    # 并断言它确实被本次运行抬高过（before 为上一次的上界）。
    peak_kib = after
    reading = {
        "label": args.label,
        "command": command,
        "wall_s": round(wall_s, 3),
        "peak_rss_kib": peak_kib,
        "peak_rss_mib": round(peak_kib / 1024, 1),
        "peak_rss_was_raised_by_this_run": after > before,
        "exit_code": completed.returncode,
    }
    with open(args.json, "w", encoding="utf-8") as handle:
        json.dump(reading, handle, ensure_ascii=False, indent=2)
        handle.write("\n")
    print(json.dumps(reading, ensure_ascii=False))
    return completed.returncode


if __name__ == "__main__":
    sys.exit(main())
