"""验证写入失败不会破坏缓存或任意自定义 target。"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'cnb-probe-cargo-target.sh'


class CargoTargetProbeTests(unittest.TestCase):
    """用临时目录与故障注入覆盖实际 shell 行为。"""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / 'custom target'
        self.target.mkdir()
        self.marker = self.target / 'unrelated-source.txt'
        self.marker.write_text('preserve me')
        self.existing = self.target / '.cnb-write-probe'
        self.existing.write_text('existing probe')
        self.before_mode = self.target.stat().st_mode
        self.env = dict(os.environ, CARGO_TARGET_DIR=str(self.target))

    def run_probe(self):
        result = subprocess.run(['sh', str(SCRIPT)], env=self.env,
                                cwd=self.root, capture_output=True, text=True)
        self.assertEqual(self.marker.read_text(), 'preserve me')
        self.assertEqual(self.existing.read_text(), 'existing probe')
        self.assertEqual(self.target.stat().st_mode, self.before_mode)
        self.assertEqual(sorted(p.name for p in self.target.iterdir()),
                         ['.cnb-write-probe', 'unrelated-source.txt'])
        self.assertEqual(list(self.root.glob('.cnb-write-probe.*')), [])
        return result

    def mock_mktemp(self, body):
        mock_bin = self.root / 'bin'
        mock_bin.mkdir()
        command = mock_bin / 'mktemp'
        command.write_text('#!/bin/sh\n' + body)
        command.chmod(0o755)
        self.env['PATH'] = str(mock_bin) + os.pathsep + os.environ['PATH']

    def test_writable_target_preserves_existing_probe(self):
        self.assertEqual(self.run_probe().returncode, 0)

    def test_target_failure_preserves_custom_directory(self):
        self.mock_mktemp('exit 1\n')
        self.assertEqual(self.run_probe().returncode, 1)

    def test_parent_failure_cleans_only_owned_probe(self):
        self.env['PROBE_TEST_REAL_MKTEMP'] = shutil.which('mktemp')
        self.mock_mktemp('case "$1" in "$CARGO_TARGET_DIR"/*) exec "$PROBE_TEST_REAL_MKTEMP" "$@";; *) exit 1;; esac\n')
        self.assertEqual(self.run_probe().returncode, 1)

    def test_symlink_target_preserves_destination(self):
        link = self.root / 'target-link'
        link.symlink_to(self.target, target_is_directory=True)
        self.env['CARGO_TARGET_DIR'] = str(link)
        self.assertEqual(self.run_probe().returncode, 0)
        self.assertEqual(link.is_symlink(), True)

    def test_option_like_relative_path(self):
        self.target.rename(self.root / '-target')
        self.target = self.root / '-target'
        self.marker = self.target / self.marker.name
        self.existing = self.target / self.existing.name
        self.env['CARGO_TARGET_DIR'] = '-target'
        self.assertEqual(self.run_probe().returncode, 0)


if __name__ == '__main__':
    unittest.main()
