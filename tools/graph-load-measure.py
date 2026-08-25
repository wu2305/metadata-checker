#!/usr/bin/env python3
"""graphdb 加载耗时独立实测（M58.3 PR2 性能基线补测，复核 P1 闭环）。

为什么需要它：`tools/m50-real-project-perf.mjs` 只测 stdio 进程总 wall，而 stdio
模式下启动期加载不归入任何请求（响应里的 `graph_load_ms` 全为 0），无法从 runner
JSONL 拆出加载耗时。本脚本从进程 spawn 计时到 stderr 出现
`[stdio-server] Graph loaded` 标记，即 stdio 模式的启动 + 图加载耗时
（release-fast 二进制启动本身为毫秒级，可忽略）。连续测 N 轮观察 page-cache
暖度差异；每轮独立进程，加载完成后发一次 `status` 验证服务可用并取节点/边数。

用法：
    python3 tools/graph-load-measure.py --bin <release-fast 二进制> \
        --graph-db <graphdb 路径> --project-dir <语料目录> [--runs 2]
"""

import argparse
import json
import subprocess
import sys
import time

MARKER = b"[stdio-server] Graph loaded"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin", required=True, help="metadata-checker 二进制路径")
    parser.add_argument("--graph-db", required=True, help="已建好的 graphdb 文件路径")
    parser.add_argument("--project-dir", required=True, help="语料项目目录")
    parser.add_argument("--runs", type=int, default=2, help="测量轮数（默认 2）")
    args = parser.parse_args()

    results = []
    for run in range(1, args.runs + 1):
        started = time.monotonic()
        proc = subprocess.Popen(
            [args.bin, "--serve-stdio", "--graph-db-path", args.graph_db,
             "--project-dir", args.project_dir],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        # 逐行读 stderr 直到加载完成标记
        load_ms = None
        marker_line = b""
        while True:
            line = proc.stderr.readline()
            if not line:
                break
            if MARKER in line:
                load_ms = round((time.monotonic() - started) * 1000)
                marker_line = line.strip()
                break
        # 加载完成后发一个 status，确认服务可用并取节点/边数
        status_ok = None
        result = {}
        if load_ms is not None:
            proc.stdin.write(b'{"request_id":"s1","command":"status"}\n')
            proc.stdin.flush()
            resp = proc.stdout.readline()
            try:
                payload = json.loads(resp)
                status_ok = payload.get("ok")
                result = payload.get("result") or {}
            except json.JSONDecodeError:
                status_ok = False
        proc.kill()
        proc.wait()
        total_ms = round((time.monotonic() - started) * 1000)
        results.append({
            "run": run,
            "spawn_to_graph_loaded_ms": load_ms,
            "status_ok": status_ok,
            "node_count": result.get("node_count"),
            "edge_count": result.get("edge_count"),
            "total_wall_ms_incl_query": total_ms,
        })
        print(json.dumps(results[-1], ensure_ascii=False), flush=True)
        if marker_line:
            print(marker_line.decode("utf-8", "replace"), file=sys.stderr, flush=True)

    print(json.dumps({"runs": results}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
