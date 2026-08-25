#!/usr/bin/env python3
"""graphdb 加载耗时独立实测（M58.3 PR2 性能基线补测，复核 P1 闭环）。

为什么需要它：`tools/m50-real-project-perf.mjs` 只测 stdio 进程总 wall，而 stdio
模式下启动期加载不归入任何请求（响应里的 `graph_load_ms` 全为 0），无法从 runner
JSONL 拆出加载耗时。本脚本从进程 spawn 计时到 stderr 出现
`[stdio-server] Graph loaded` 标记，即 stdio 模式的启动 + 图加载耗时
（release-fast 二进制启动本身为毫秒级，可忽略）。连续测 N 轮观察 page-cache
暖度差异；每轮独立进程，加载完成后发一次 `status` 验证服务可用并取节点/边数。

fail-closed：`--runs` 必须 >= 1；等待标记/响应均有超时（`--timeout-secs`）；
任何一轮未等到加载标记、进程提前退出或 status 响应异常，脚本最终以非零码退出。

用法：
    python3 tools/graph-load-measure.py --bin <release-fast 二进制> \
        --graph-db <graphdb 路径> --project-dir <语料目录> [--runs 2] \
        [--timeout-secs 1800]
"""

import argparse
import json
import select
import subprocess
import sys
import time

MARKER = b"[stdio-server] Graph loaded"


def readline_with_timeout(stream, timeout_secs, deadline):
    """在剩余超时预算内逐行读；超时或 EOF 返回 None。"""
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return None
    ready, _, _ = select.select([stream], [], [], remaining)
    if not ready:
        return None
    line = stream.readline()
    return line if line else None


def measure_run(run, args):
    """单轮测量；返回 (result_dict, marker_line, ok)。"""
    started = time.monotonic()
    deadline = started + args.timeout_secs
    proc = subprocess.Popen(
        [args.bin, "--serve-stdio", "--graph-db-path", args.graph_db,
         "--project-dir", args.project_dir],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    load_ms = None
    marker_line = b""
    status_ok = None
    result = {}
    ok = False
    try:
        # 逐行读 stderr 直到加载完成标记（带超时，EOF 视为失败）
        while True:
            line = readline_with_timeout(proc.stderr, args.timeout_secs, deadline)
            if line is None:
                break
            if MARKER in line:
                load_ms = round((time.monotonic() - started) * 1000)
                marker_line = line.strip()
                break
        # 加载完成后发一个 status，确认服务可用并取节点/边数
        if load_ms is not None:
            proc.stdin.write(b'{"request_id":"s1","command":"status"}\n')
            proc.stdin.flush()
            resp = readline_with_timeout(proc.stdout, args.timeout_secs, deadline)
            if resp is not None:
                try:
                    payload = json.loads(resp)
                    status_ok = payload.get("ok")
                    result = payload.get("result") or {}
                except json.JSONDecodeError:
                    status_ok = False
            ok = status_ok is True
    finally:
        proc.kill()
        proc.wait()
    total_ms = round((time.monotonic() - started) * 1000)
    return {
        "run": run,
        "spawn_to_graph_loaded_ms": load_ms,
        "status_ok": status_ok,
        "node_count": result.get("node_count"),
        "edge_count": result.get("edge_count"),
        "total_wall_ms_incl_query": total_ms,
    }, marker_line, ok


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, help="metadata-checker 二进制路径")
    parser.add_argument("--graph-db", required=True, help="已建好的 graphdb 文件路径")
    parser.add_argument("--project-dir", required=True, help="语料项目目录")
    parser.add_argument("--runs", type=int, default=2, help="测量轮数（默认 2，必须 >= 1）")
    parser.add_argument("--timeout-secs", type=int, default=1800,
                        help="单轮等待加载标记/status 响应的超时秒数（默认 1800）")
    args = parser.parse_args()

    if args.runs < 1:
        parser.error("--runs 必须 >= 1")
    if args.timeout_secs < 1:
        parser.error("--timeout-secs 必须 >= 1")

    results = []
    all_ok = True
    for run in range(1, args.runs + 1):
        entry, marker_line, ok = measure_run(run, args)
        results.append(entry)
        all_ok = all_ok and ok
        print(json.dumps(entry, ensure_ascii=False), flush=True)
        if marker_line:
            print(marker_line.decode("utf-8", "replace"), file=sys.stderr, flush=True)

    print(json.dumps({"runs": results}, ensure_ascii=False, indent=2))
    if not all_ok:
        print("ERROR: 存在未等到加载标记或 status 异常的轮次", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
