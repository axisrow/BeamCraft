# SPDX-License-Identifier: MPL-2.0
"""Check the macOS icon and archive wiring without needing a Rust build or macOS."""
import os
from pathlib import Path
import plistlib
import shutil
import struct
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class MacIcon(unittest.TestCase):
    def test_icns_sizes(self):
        data = (ROOT / 'assets/icons/BeamCraft.icns').read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        sizes = set()
        offset = 8
        while offset < len(data):
            length = struct.unpack('>I', data[offset + 4:offset + 8])[0]
            self.assertGreaterEqual(length, 32)
            self.assertLessEqual(offset + length, len(data))
            png = data[offset + 8:offset + length]
            self.assertEqual(png[:8], b'\x89PNG\r\n\x1a\n')
            width, height = struct.unpack('>II', png[16:24])
            self.assertEqual(width, height)
            sizes.add(width)
            offset += length
        self.assertEqual(offset, len(data))
        self.assertEqual(sizes, {16, 32, 64, 128, 256, 512, 1024})

    def test_archive_contains_icon_before_signing(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for path in ('packaging/package.sh', 'packaging/Info.plist',
                         'assets/icons/BeamCraft.icns'):
                dest = root / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, dest)
            # Synthetic build inputs; no claim of testing native executables/signing.
            for path in ('README.md', 'LICENSE', 'NOTICE', 'docs/brand/LICENSE-brand.txt',
                         'assets/fonts/inter/Inter-Regular.ttf',
                         'assets/fonts/inter/Inter-Bold.ttf', 'licenses/OFL-1.1-Inter.txt',
                         'target/aarch64-apple-darwin/release/beamcraft',
                         'target/aarch64-apple-darwin/release/beamcraft-ui',
                         'target/aarch64-apple-darwin/release/beamcraft-usd-worker'):
                dest = root / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text('synthetic packaging input\n')
            tools = root / 'tools'
            tools.mkdir()
            signer = tools / 'codesign'
            signer.write_text('#!/bin/bash\nset -eu\napp="${!#}"\n'
                              'test -s "$app/Contents/Resources/BeamCraft.icns"\n')
            signer.chmod(0o755)
            env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ['PATH'])
            result = subprocess.run(['bash', 'packaging/package.sh',
                                     'aarch64-apple-darwin', 'macos', 'v0.0.1'],
                                    cwd=root, env=env, check=True, capture_output=True, text=True)
            with tarfile.open(root / result.stdout.strip()) as archive:
                prefix = 'beamcraft-v0.0.1-aarch64-apple-darwin/BeamCraft.app/Contents/'
                with archive.extractfile(prefix + 'Info.plist') as entry:
                    info = plistlib.load(entry)
                self.assertEqual(info['CFBundleShortVersionString'], '0.0.1')
                self.assertEqual(info['CFBundleIconFile'], 'BeamCraft.icns')
                with archive.extractfile(prefix + 'Resources/' + info['CFBundleIconFile']) as entry:
                    self.assertEqual(entry.read(), (ROOT / 'assets/icons/BeamCraft.icns').read_bytes())


if __name__ == '__main__':
    unittest.main()
