#!/usr/bin/env python3
"""一次性固定输入诊断：重复检索与提示词对照，不改语料、不重建索引。"""
import copy
import datetime
import gzip
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[3]
ARCHIVE = ROOT / 'docs/knowledge-acceptance/runs/2026-09-08-state-paths.json.gz'
ARCHIVE_HASH = 'a0a4abebcd6d7f3d5d810fbe17a94e76260d2023f08fb5098d25081555b9dffc'
INDEX_SHA = 'f910fdc3eeaaf6439a1ff913a27857829d4264ba'
ADDITION = ('回答前逐条核对：每个主体、动作、条件和所属处理层必须由同一条证据或明确的调用关系支持；'
            '不能把相邻段落的函数、状态或行为移接给另一个主体。'
            '逐条列出结论及支持它的证据原句；没有直接证据的部分明确标为未知。')
OUTPUT = ROOT / 'target/knowledge-acceptance/fixed-controls.json.gz'


def main():
    """保留完整请求、SSE 和索引前后状态，遇到错误也归档已取得的记录。"""
    spec = importlib.util.spec_from_file_location('kb_evaluate', ROOT / 'scripts/kb-evaluate.py')
    evaluator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(evaluator)
    archive_bytes = ARCHIVE.read_bytes()
    if hashlib.sha256(archive_bytes).hexdigest() != ARCHIVE_HASH:
        raise ValueError('frozen archive hash mismatch')
    archive = json.loads(gzip.decompress(archive_bytes))
    original = {record['id']: record for record in archive['records']}
    queries = [
        ('H1-original', original['H1']['question']),
        ('H1-paraphrase-a', 'full-criterion CI 在缺少真实项目 fixture 凭证时如何处理？'),
        ('H1-paraphrase-b', '仓库的全量性能验收流水线对真实项目数据和访问凭证有什么要求？'),
    ]
    if OUTPUT.exists():
        raise ValueError('output exists; retain it and run in a fresh checkout')
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    report = {
        'schema_version': 1, 'experiment': 'fixed-input-controls',
        'started_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
        'source_sha': os.environ['CNB_COMMIT'], 'build_id': os.environ['CNB_BUILD_ID'],
        'runtime': sys.version, 'expected_indexed_sha': INDEX_SHA,
        'archive_sha256': ARCHIVE_HASH, 'archive_source_sha': archive['source_sha'],
        'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        'decoder_sha256': hashlib.sha256((ROOT / 'scripts/kb-evaluate.py').read_bytes()).hexdigest(),
        'system_addition': ADDITION, 'replicates': 3,
        'query_schedule': queries, 'status': 'running', 'records': [],
    }
    token = os.environ['CNB_TOKEN']
    base = 'https://api.cnb.cool/wu2305/metadata-checker/-/'

    def request(route, payload=None):
        """认证头不写入产物；只归档业务响应。"""
        req = urllib.request.Request(base + route,
            data=None if payload is None else json.dumps(payload).encode(), headers={
                'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json',
                'Accept': 'application/vnd.cnb.api+json' if payload is None else 'text/event-stream'})
        with urllib.request.urlopen(req, timeout=180) as response:
            return response.read().decode('utf-8')

    try:
        info = json.loads(request('knowledge/base'))
        report['index_before'] = info
        if info['last_commit_sha'] != INDEX_SHA:
            raise ValueError('index SHA mismatch')
        if set(info['include'].split(',')) != {'docs/knowledge/**/*.md', 'AGENTS.md', 'SKILL.md'}:
            raise ValueError('index whitelist mismatch')
        if 'docs/knowledge-acceptance/**' not in info['exclude'].split(',') or info.get('issue_sync_enabled') is not False:
            raise ValueError('index exclusions or issue sync mismatch')
        for replicate in range(1, 4):
            for query_id, query in queries:
                route = 'knowledge/base/query?' + urllib.parse.urlencode({'query': query, 'top_k': 5})
                record = {'kind': 'retrieval', 'id': query_id, 'replicate': replicate, 'route': route}
                report['records'].append(record)
                record['retrieval_raw'] = request(route)
                if not isinstance(json.loads(record['retrieval_raw']), list):
                    raise ValueError('malformed retrieval')
                evaluator.write_report(OUTPUT, report)
                print(f'{query_id}/{replicate}: captured', flush=True)
        for replicate in range(1, 4):
            # 交替先后顺序，减小总由同一处理先请求造成的时间偏差；不宣称随机试验。
            treatments = ['baseline', 'claim-check'] if replicate % 2 else ['claim-check', 'baseline']
            for question_id in ['E3', 'B6']:
                for treatment in treatments:
                    source = original[question_id]
                    payload = copy.deepcopy(source['model_request'])
                    if [message['role'] for message in payload['messages']] != ['system', 'user']:
                        raise ValueError('unexpected archived message roles')
                    user = json.loads(payload['messages'][1]['content'])
                    if set(user) != {'question', 'retrieved_chunks'} or user['retrieved_chunks'] != json.loads(source['retrieval_raw']):
                        raise ValueError('archived request/retrieval mismatch')
                    if treatment == 'claim-check':
                        payload['messages'][0]['content'] += ADDITION
                    record = {'kind': 'answer', 'id': question_id, 'replicate': replicate,
                              'treatment': treatment, 'model_request': payload,
                              'judgment': 'pending_manual_review'}
                    report['records'].append(record)
                    record['completion_sse'] = request('ai/chat/completions', payload)
                    record['answer'], record['models_returned'] = evaluator.decode_completion(record['completion_sse'])
                    evaluator.write_report(OUTPUT, report)
                    print(f'{question_id}/{treatment}/{replicate}: captured', flush=True)
        report['status'] = 'captured_pending_manual_review'
    except Exception as error:
        report['status'] = 'error'
        report['error_type'] = type(error).__name__
    finally:
        try:
            report['index_after'] = json.loads(request('knowledge/base'))
            if report['index_after'] != report.get('index_before'):
                report['status'] = 'error'
                report['index_error'] = 'index changed or initial capture failed'
        except Exception as error:
            report['status'] = 'error'
            report['index_error_type'] = type(error).__name__
        evaluator.write_report(OUTPUT, report)
    return 0 if report['status'] == 'captured_pending_manual_review' else 1


if __name__ == '__main__':
    sys.exit(main())
