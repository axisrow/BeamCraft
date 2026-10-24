#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Compose the unchanged official mark with our original tile; emit platform icons."""
from copy import deepcopy
from io import BytesIO
from pathlib import Path
import struct
import xml.etree.ElementTree as ET

import cairosvg
from PIL import Image, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
NS = '{http://www.w3.org/2000/svg}'
SCALE = 2  # Supersampling keeps the rim and diagonal edges clean at Dock sizes.


def render(svg):
    return Image.open(BytesIO(cairosvg.svg2png(
        bytestring=ET.tostring(svg), output_width=1024 * SCALE,
        output_height=1024 * SCALE))).convert('RGBA')


def shadow(layer, radius, offset, opacity):
    alpha = layer.getchannel('A').filter(ImageFilter.GaussianBlur(radius * SCALE))
    alpha = alpha.point(lambda value: round(value * opacity))
    result = Image.new('RGBA', layer.size)
    result.paste((0, 28, 52, 255), (0, offset * SCALE), alpha)
    return result


def main():
    tile = render(ET.parse(ROOT / 'assets/icons/macos-tile.svg').getroot())
    brand = ET.parse(ROOT / 'docs/brand/beamcraft-logo.svg').getroot()
    # Only place the official I-beam group. Never rewrite its paths, colours or strokes.
    mark_svg = ET.Element(NS + 'svg', width='1024', height='1024', viewBox='0 0 1024 1024')
    placement = ET.SubElement(mark_svg, NS + 'g', transform='translate(45 45) scale(0.73)')
    placement.append(deepcopy(brand.findall(NS + 'g')[-1]))
    mark = render(mark_svg)
    canvas = shadow(tile, 10, 12, 0.30)
    canvas.alpha_composite(tile)
    canvas.alpha_composite(shadow(mark, 8, 9, 0.28))
    canvas.alpha_composite(mark)
    # Guarantee fully transparent outer margins, including the blurred shadow tails.
    canvas = canvas.resize((1024, 1024), Image.Resampling.LANCZOS)
    canvas.save(ROOT / 'assets/icons/beamcraft-macos-1024.png')
    # Windows and Linux use the same composition as the macOS icon.
    canvas.save(ROOT / 'assets/icons/beamcraft-windows.ico', format='ICO',
                sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64),
                       (128, 128), (256, 256)])
    canvas.save(ROOT / 'assets/icons/beamcraft-linux-1024.png')
    canvas.resize((256, 256), Image.Resampling.LANCZOS).save(
        ROOT / 'assets/icons/beamcraft-linux-256.png')
    chunks = []
    # PNG-backed ICNS representations, including explicit Retina slots.
    for kind, size in ((b'icp4', 16), (b'icp5', 32), (b'icp6', 64),
                       (b'ic07', 128), (b'ic08', 256), (b'ic09', 512),
                       (b'ic10', 1024), (b'ic11', 32), (b'ic12', 64),
                       (b'ic13', 256), (b'ic14', 512)):
        png = BytesIO()
        canvas.resize((size, size), Image.Resampling.LANCZOS).save(png, format='PNG')
        payload = png.getvalue()
        chunks.append(kind + struct.pack('>I', len(payload) + 8) + payload)
    body = b''.join(chunks)
    (ROOT / 'assets/icons/BeamCraft.icns').write_bytes(b'icns' + struct.pack('>I', len(body) + 8) + body)


if __name__ == '__main__':
    main()
