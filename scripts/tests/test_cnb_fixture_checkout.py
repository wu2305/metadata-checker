"""执行真实 YAML checkout stage，验证错误不会被后续输出覆盖。"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class FixtureCheckoutTests(unittest.TestCase):
    """用临时命令桩隔离 git 与凭据写入，不访问 fixture 仓库。"""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        config = (REPO / '.cnb.yml').read_text()
        section = config.split('.full_criterion_checkout_stage:', 1)[1].split('\n  exports:', 1)[0]
        body = section.split('  script: |\n', 1)[1]
        self.script = '\n'.join(line[4:] for line in body.splitlines())
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith('REAL_PROJECT_FIXTURE_') and key != 'METADATA_CHECKER_REAL_PROJECT_DIR'}
        # 只替换固定的副作用路径，保留分支、命令和退出状态逻辑。
        self.script = self.script.replace('/tmp/metadata-checker-real-fixtures', str(self.root / 'fixture'))
        self.script = self.script.replace('$HOME/.git-credentials', str(self.root / 'credentials'))
        git = self.root / 'git'
        git.write_text('#!/bin/sh\ncase "$1" in clone) exit "${TEST_CLONE_EXIT:-0}";; -C) exit "${TEST_CHECKOUT_EXIT:-0}";; config) exit 0;; *) exit 99;; esac\n')
        git.chmod(0o755)
        self.env['PATH'] = str(self.root) + os.pathsep + os.environ['PATH']

    def run_stage(self):
        return subprocess.run(['sh', '-c', self.script], env=self.env, capture_output=True, text=True)

    def configured(self):
        self.env.update(REAL_PROJECT_FIXTURE_DEPLOY_TOKEN='dummy-test-token',
                        REAL_PROJECT_FIXTURE_REPO_SLUG='test/fixture')

    def test_missing_token_fails(self):
        result = self.run_stage()
        self.assertEqual(result.returncode, 1)
        self.assertEqual('set-output' in result.stdout, False)

    def test_missing_repository_fails(self):
        self.env['REAL_PROJECT_FIXTURE_DEPLOY_TOKEN'] = 'dummy-test-token'
        self.assertEqual(self.run_stage().returncode, 1)

    def test_preset_directory_does_not_require_token(self):
        self.env['METADATA_CHECKER_REAL_PROJECT_DIR'] = str(self.root)
        result = self.run_stage()
        self.assertEqual(result.returncode, 0)
        self.assertEqual('set-output real_project_dir=' in result.stdout, True)

    def test_clone_failure_does_not_emit_success_output(self):
        self.configured()
        self.env['TEST_CLONE_EXIT'] = '128'
        result = self.run_stage()
        self.assertEqual(result.returncode, 1)
        self.assertEqual('set-output' in result.stdout, False)

    def test_checkout_failure_does_not_emit_success_output(self):
        self.configured()
        self.env.update(REAL_PROJECT_FIXTURE_REF='a' * 40, TEST_CHECKOUT_EXIT='128')
        result = self.run_stage()
        self.assertEqual(result.returncode, 1)
        self.assertEqual('set-output' in result.stdout, False)

    def test_success_emits_fixture_path(self):
        self.configured()
        result = self.run_stage()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(f'##[set-output real_project_dir={self.root}/fixture/xiaoshouyi]\n', result.stdout)


if __name__ == '__main__':
    unittest.main()
