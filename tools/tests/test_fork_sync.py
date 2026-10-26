# SPDX-License-Identifier: MPL-2.0
"""Local-only sync regression tests: python3 tools/tests/test_fork_sync.py."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(os.environ.get('FORK_SYNC_SCRIPT', Path(__file__).resolve().parents[1] / 'fork-sync.sh'))


class ForkSyncTest(unittest.TestCase):
    def run_cmd(self, *args, cwd=None, check=True):
        return subprocess.run(args, cwd=cwd or self.fork, env=self.env,
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                              check=check)

    def git(self, *args, cwd=None):
        return self.run_cmd('git', *args, cwd=cwd).stdout.strip()

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.fork = self.root / 'fork'
        self.fork.mkdir()
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1',
                        GIT_AUTHOR_NAME='Test', GIT_AUTHOR_EMAIL='test@example.invalid',
                        GIT_COMMITTER_NAME='Test', GIT_COMMITTER_EMAIL='test@example.invalid',
                        GIT_MERGE_AUTOEDIT='no')
        self.git('init', '-b', 'master')
        (self.fork / 'tools').mkdir()
        shutil.copyfile(SCRIPT, self.fork / 'tools/fork-sync.sh')
        (self.fork / 'tools/upstream-mirror.sh').write_text('FORK_ONLY="tools/fork-sync.sh tools/upstream-mirror.sh"\n')
        (self.fork / 'shared').write_text('base\n')
        self.commit('base')
        self.origin = self.root / 'origin.git'
        self.upstream = self.root / 'upstream'
        self.git('clone', '--bare', str(self.fork), str(self.origin))
        self.git('clone', str(self.fork), str(self.upstream))
        self.git('remote', 'add', 'origin', str(self.origin))
        self.git('remote', 'add', 'upstream', str(self.upstream))
        self.git('config', 'branch.master.remote', 'origin')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.trace = self.root / 'cargo.log'
        self.env.update(PATH=str(self.bin) + os.pathsep + self.env['PATH'],
                        SYNC_TRACE=str(self.trace), SYNC_ORIGIN=str(self.origin))
        cargo = self.bin / 'cargo'
        cargo.write_text('''#!/usr/bin/env bash
set -eu
# The remote must remain unchanged until every gate has run.
[ "$(git --git-dir="$SYNC_ORIGIN" rev-parse master)" = "$SYNC_BEFORE" ]
printf '%s\n' "$*" >> "$SYNC_TRACE"
[ "${SYNC_FAIL:-}" != "$1" ]
''')
        cargo.chmod(0o755)

    def commit(self, message, cwd=None):
        self.git('add', '.', cwd=cwd)
        self.git('commit', '-m', message, cwd=cwd)

    def prepare(self, conflict=None, delete=False):
        path = conflict or 'shared'
        if delete:
            (self.fork / path).unlink()
        else:
            (self.fork / path).write_text('fork\n')
        self.commit('fork')
        self.git('push', 'origin', 'master')
        (self.upstream / (conflict or 'upstream-only')).write_text('upstream\n')
        (self.upstream / 'clean-file').write_text('clean\n')
        self.commit('upstream', cwd=self.upstream)
        self.before = self.git('rev-parse', 'HEAD')
        self.env['SYNC_BEFORE'] = self.before

    def sync(self, code=0, dry=False):
        args = ['bash', 'tools/fork-sync.sh', 'sync', '--yes']
        if dry:
            args.append('--dry-run')
        result = self.run_cmd(*args, check=False)
        self.assertEqual(result.returncode, code, result.stdout)
        self.assertEqual(self.git('status', '--porcelain'), '')
        self.assertFalse((self.fork / '.git/MERGE_HEAD').exists())
        return result.stdout

    def assert_published(self):
        self.assertEqual(self.git('rev-parse', 'HEAD'),
                         self.git('--git-dir=' + str(self.origin), 'rev-parse', 'master'))
        self.assertEqual((self.fork / 'clean-file').read_text(), 'clean\n')
        self.assertEqual(self.trace.read_text().splitlines(),
                         ['fmt --check', 'clippy --workspace --all-targets -- -D warnings', 'test --workspace'])

    def test_clean_merge(self):
        self.prepare()
        self.sync()
        self.assert_published()
        self.assertEqual((self.fork / 'shared').read_text(), 'fork\n')
        self.assertEqual((self.fork / 'upstream-only').read_text(), 'upstream\n')

    def test_conflict_keeps_fork_and_merges_other_files(self):
        self.prepare('shared')
        output = self.sync()
        self.assertEqual((self.fork / 'shared').read_text(), 'fork\n')
        self.assertIn('skipped upstream changes', output)
        self.assert_published()

    def test_fork_only_conflict(self):
        self.prepare('tools/upstream-mirror.sh')
        # Preserve a valid FORK_ONLY list while conflicting on its contents.
        (self.fork / 'tools/upstream-mirror.sh').write_text('FORK_ONLY="tools/upstream-mirror.sh"\n')
        self.commit('valid list')
        self.git('push', 'origin', 'master')
        self.env['SYNC_BEFORE'] = self.git('rev-parse', 'HEAD')
        self.sync()
        self.assertEqual((self.fork / 'tools/upstream-mirror.sh').read_text(),
                         'FORK_ONLY="tools/upstream-mirror.sh"\n')
        self.assert_published()

    def test_fork_deletion(self):
        self.prepare('shared', delete=True)
        self.sync()
        self.assertFalse((self.fork / 'shared').exists())
        self.assert_published()

    def test_failed_gate_rolls_back(self):
        self.prepare('shared')
        self.env['SYNC_FAIL'] = 'clippy'
        self.sync(code=3)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.before)
        self.assertEqual(self.git('--git-dir=' + str(self.origin), 'rev-parse', 'master'), self.before)
        self.assertEqual(len(self.trace.read_text().splitlines()), 2)

    def test_dry_run(self):
        self.prepare('shared')
        output = self.sync(dry=True)
        self.assertIn('shared', output)
        self.assertIn('keeps the fork versions', output)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.before)
        self.assertFalse(self.trace.exists())

    def test_merge_error_without_conflicts(self):
        self.prepare()
        self.git('config', 'merge.verifySignatures', 'true')
        self.sync(code=4)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.before)
        self.assertFalse(self.trace.exists())


if __name__ == '__main__':
    unittest.main()
