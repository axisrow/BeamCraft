#!/usr/bin/env bash
# SPDX-License-Identifier: MPL-2.0
# Rebuild the macOS app icon using tools included with macOS; run from any directory.
set -eu

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
iconset="$work/BeamCraft.iconset"
mkdir -p "$iconset"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$root/assets/icons/beamcraft-1024.png" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$root/assets/icons/beamcraft-1024.png" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns --output "$root/assets/icons/BeamCraft.icns" "$iconset"
