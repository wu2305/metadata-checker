"""拒绝截断、限长和服务端报错的问答流，防止把部分回答计作完成。"""
import importlib.util
import json
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('kb_evaluate', Path(__file__).resolve().parents[1] / 'kb-evaluate.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def event(content='', finish=None):
    """构造服务端流事件。"""
    return 'data: ' + json.dumps({'model': 'test-model', 'choices': [
        {'delta': {'content': content}, 'finish_reason': finish}]}) + '\n'


class CompletionTests(unittest.TestCase):
    """完成标识与回答内容须同时存在。"""

    def test_complete_stream_collects_full_answer_and_model(self):
        self.assertEqual(module.decode_completion(event('一', '') + event('二', 'stop') + 'data: [DONE]\n'),
                         ('一二', ['test-model']))

    def test_truncated_stream_is_rejected(self):
        for stream in [event('partial'), event('partial', 'stop'), 'data: [DONE]\n',
                       event('partial', 'length') + 'data: [DONE]\n',
                       'data: {"error":{"message":"failed"}}\n']:
            with self.subTest(stream=stream), self.assertRaises(ValueError):
                module.decode_completion(stream)


if __name__ == '__main__':
    unittest.main()
