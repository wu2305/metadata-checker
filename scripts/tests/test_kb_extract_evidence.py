"""验证完整日志归档可恢复，缺失标记或截断 gzip 不能伪装成完成。"""
import base64
import gzip
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('kb_extract', Path(__file__).resolve().parents[1] / 'kb-extract-evidence.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class EvidenceExtractionTests(unittest.TestCase):
    """使用带 runner 时间前缀的日志覆盖正常与损坏产物。"""

    def response(self, log):
        return {'status': 200, 'data': {'type': 'base64', 'data': base64.b64encode(log.encode()).decode()}}

    def test_complete_archive_is_preserved_byte_for_byte(self):
        archive = gzip.compress(json.dumps({'schema_version': 1, 'records': []}).encode())
        log = '\n'.join('[00:00:01 +0ms] ' + line for line in [
            'KB_EVIDENCE_BEGIN', base64.b64encode(archive).decode(), 'KB_EVIDENCE_END'])
        self.assertEqual(module.extract(self.response(log)), archive)

    def test_truncated_log_is_rejected(self):
        with self.assertRaises(ValueError):
            module.extract(self.response('truncated\nKB_EVIDENCE_END'))

    def test_corrupt_archive_is_rejected(self):
        with self.assertRaises((OSError, EOFError)):
            module.extract(self.response('KB_EVIDENCE_BEGIN\nYWJj\nKB_EVIDENCE_END'))


if __name__ == '__main__':
    unittest.main()
