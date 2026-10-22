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
import zlib

ROOT = Path(__file__).resolve().parents[1]


def decode_png_rgba(data):
    """Decode a non-interlaced 8-bit RGBA PNG to raw scanlines, stdlib only:
    CI runs these checks without installing any Python dependencies."""
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise ValueError('not a PNG file')
    width = height = None
    compressed = bytearray()
    offset = 8
    while offset < len(data):
        length = struct.unpack('>I', data[offset:offset + 4])[0]
        kind = data[offset + 4:offset + 8]
        chunk = data[offset + 8:offset + 8 + length]
        if kind == b'IHDR':
            width, height, depth, color_type = struct.unpack('>IIBB', chunk[:10])
            if (depth, color_type) != (8, 6):
                raise ValueError('expected an 8-bit RGBA PNG')
            if chunk[12]:
                raise ValueError('interlaced PNG is not supported')
        elif kind == b'IDAT':
            compressed += chunk
        offset += 12 + length
    if width is None or height is None:
        raise ValueError('PNG is missing an IHDR chunk')
    stride = width * 4
    raw = zlib.decompress(bytes(compressed))
    pixels = bytearray()
    previous = bytes(stride)
    pos = 0
    for _ in range(height):
        filter_kind = raw[pos]
        row = bytearray(raw[pos + 1:pos + 1 + stride])
        pos += 1 + stride
        if filter_kind == 1:  # Sub
            for i in range(4, stride):
                row[i] = (row[i] + row[i - 4]) & 0xff
        elif filter_kind == 2:  # Up
            for i in range(stride):
                row[i] = (row[i] + previous[i]) & 0xff
        elif filter_kind == 3:  # Average
            for i in range(stride):
                left = row[i - 4] if i >= 4 else 0
                row[i] = (row[i] + (left + previous[i]) // 2) & 0xff
        elif filter_kind == 4:  # Paeth
            for i in range(stride):
                a = row[i - 4] if i >= 4 else 0
                b = previous[i]
                c = previous[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                if pa <= pb and pa <= pc:
                    predictor = a
                elif pb <= pc:
                    predictor = b
                else:
                    predictor = c
                row[i] = (row[i] + predictor) & 0xff
        elif filter_kind != 0:
            raise ValueError('unknown PNG filter %d' % filter_kind)
        pixels += row
        previous = row
    return width, height, bytes(pixels)


def alpha_bbox(width, height, pixels):
    """Bounding box of pixels with alpha > 0, as (left, top, right, bottom)."""
    stride = width * 4
    left, top, right, bottom = width, height, 0, 0
    for y in range(height):
        alpha = pixels[y * stride + 3:(y + 1) * stride:4]
        if alpha.count(0) == len(alpha):
            continue
        top = min(top, y)
        bottom = max(bottom, y + 1)
        left = min(left, len(alpha) - len(alpha.lstrip(b'\x00')))
        right = max(right, len(alpha.rstrip(b'\x00')))
    return left, top, right, bottom


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
        width, height, pixels = decode_png_rgba(
            (ROOT / 'assets/icons/beamcraft-macos-1024.png').read_bytes())
        self.assertEqual((width, height), (1024, 1024))
        self.assertEqual(pixels[3], 0)
        self.assertEqual(pixels[(512 * width + 512) * 4 + 3], 255)
        left, top, right, bottom = alpha_bbox(width, height, pixels)
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
                bundled = decode_png_rgba(data[offset + 8:offset + length])
                self.assertEqual(bundled, (width, height, pixels))
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
