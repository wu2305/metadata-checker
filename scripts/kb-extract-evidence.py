#!/usr/bin/env python3
"""从 CNB 完整 runner 日志响应提取问答证据，拒绝截断或损坏的归档。"""
import argparse
import base64
import gzip
import hashlib
import json
from pathlib import Path
import re


def extract(response):
    """只解析标记之间的归档，不输出可能含环境信息的 runner 日志。"""
    data = response['data']
    if response['status'] != 200 or data['type'] != 'base64':
        raise ValueError('expected successful base64 runner log response')
    log = base64.b64decode(data['data'], validate=True).decode('utf-8')
    lines = [re.sub(r'^\[[^\]]+\] ', '', line) for line in log.splitlines()]
    if lines.count('KB_EVIDENCE_BEGIN') != 1 or lines.count('KB_EVIDENCE_END') != 1:
        raise ValueError('missing or ambiguous evidence markers; download the full runner log')
    start, end = lines.index('KB_EVIDENCE_BEGIN'), lines.index('KB_EVIDENCE_END')
    if end <= start:
        raise ValueError('invalid marker order')
    archive = base64.b64decode(''.join(lines[start + 1:end]), validate=True)
    report = json.loads(gzip.decompress(archive))
    if report['schema_version'] != 1 or not isinstance(report['records'], list):
        raise ValueError('unsupported evidence schema')
    return archive


def main():
    """验证后独占写入输出路径，避免覆盖已有验收记录。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('log_json', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    archive = extract(json.loads(args.log_json.read_text()))
    with args.output.open('xb') as output:
        output.write(archive)
    print('sha256=' + hashlib.sha256(archive).hexdigest())


if __name__ == '__main__':
    main()
