# SPDX-License-Identifier: MPL-2.0
"""Check the macOS icon and archive wiring without needing a Rust build or macOS."""
from io import BytesIO
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
    # Pixel size per icns element type. 'info' (bplist metadata) and 'toc ' (table of
    # contents) carry no pixels; newer iconutil emits 'info' and encodes the smallest
    # two sizes (ic04/ic05) as raw ARGB instead of PNG.
    ICNS_TYPES = {
        b'icp4': 16, b'icp5': 32, b'icp6': 64, b'ic07': 128, b'ic08': 256,
        b'ic09': 512, b'ic10': 1024, b'ic11': 32, b'ic12': 64, b'ic13': 256,
        b'ic14': 512, b'ic04': 16, b'ic05': 32,
    }

    def test_icns_sizes(self):
        data = (ROOT / 'assets/icons/BeamCraft.icns').read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        sizes = set()
        offset = 8
        while offset < len(data):
            kind = data[offset:offset + 4]
            length = struct.unpack('>I', data[offset + 4:offset + 8])[0]
            self.assertGreaterEqual(length, 8)
            self.assertLessEqual(offset + length, len(data))
            payload = data[offset + 8:offset + length]
            if kind not in (b'info', b'toc '):
                self.assertIn(kind, self.ICNS_TYPES)
                size = self.ICNS_TYPES[kind]
                sizes.add(size)
                if payload[:8] == b'\x89PNG\r\n\x1a\n':
                    width, height = struct.unpack('>II', payload[16:24])
                    self.assertEqual((width, height), (size, size))
                else:
                    self.assertIn(kind, (b'ic04', b'ic05'))
            offset += length
        self.assertEqual(offset, len(data))
        self.assertEqual(sizes, {16, 32, 64, 128, 256, 512, 1024})

    def test_macos_icon_has_transparent_margins(self):
        from PIL import Image
        image = Image.open(ROOT / 'assets/icons/beamcraft-macos-1024.png')
        self.assertEqual(image.size, (1024, 1024))
        self.assertEqual(image.mode, 'RGBA')
        alpha = image.getchannel('A')
        self.assertEqual(alpha.getpixel((0, 0)), 0)
        self.assertEqual(alpha.getpixel((512, 512)), 255)
        left, top, right, bottom = alpha.getbbox()
        self.assertGreater(left, 40)
        self.assertGreater(top, 40)
        self.assertLess(right, 984)
        self.assertLess(bottom, 1000)
        # The largest ICNS image must match the running-process PNG exactly.
        data = (ROOT / 'assets/icons/BeamCraft.icns').read_bytes()
        offset = 8
        while offset < len(data):
            length = struct.unpack('>I', data[offset + 4:offset + 8])[0]
            if data[offset:offset + 4] == b'ic10':
                bundled = Image.open(BytesIO(data[offset + 8:offset + length]))
                self.assertEqual(bundled.tobytes(), image.tobytes())
                return
            offset += length
        self.fail('Missing 1024px ICNS representation')

    def test_archive_contains_icon_before_signing(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for path in ('packaging/package.sh', 'packaging/Info.plist',
                         'assets/icons/BeamCraft.icns',
                         'assets/icons/beamcraft-macos-1024.png'):
                dest = root / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(ROOT / path, dest)
            # Synthetic build inputs; no claim of testing native executables/signing.
            for path in ('README.md', 'LICENSE', 'NOTICE', 'docs/brand/LICENSE-brand.txt',
                         'assets/fonts/inter/Inter-Regular.ttf',
                         'assets/fonts/inter/Inter-Bold.ttf', 'licenses/OFL-1.1-Inter.txt',
                         'target/aarch64-apple-darwin/dist/beamcraft',
                         'target/aarch64-apple-darwin/dist/beamcraft-ui'):
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
                with archive.extractfile(prefix + 'Resources/beamcraft-icon.png') as entry:
                    self.assertEqual(entry.read(), (ROOT / 'assets/icons/beamcraft-macos-1024.png').read_bytes())


if __name__ == '__main__':
    unittest.main()
