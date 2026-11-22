#!/usr/bin/env bash
# Q02: build one unsigned release package. Run from the repository root after
#
#   cargo build --profile dist --locked --target <target> -p beamcraft-cli -p beamcraft-ui
#
# Usage: packaging/package.sh <target-triple> <win|linux|macos> <tag>
#
# Prints the archive path. Every package carries the two executables, the admitted
# Inter font with its OFL-1.1 licence, the shipped libraries, and the project
# documents. Signing and notarization are out of scope (owner decision).

set -eu

target="${1:?target triple required}"
kind="${2:?kind required: win, linux or macos}"
tag="${3:?tag required}"

case "$kind" in
    win | linux | macos) ;;
    *)
        echo "unknown kind: $kind (expected win, linux or macos)" >&2
        exit 2
        ;;
esac

name="beamcraft-${tag}-${target}"
release="target/${target}/dist"
ext=""
if [ "$kind" = win ]; then
    ext=".exe"
fi

# Documents next to the application in every layout. Drop any archive from an earlier
# run with the same tag: 7z updates an existing zip instead of replacing it.
rm -rf "$name"
rm -f "$name.zip"
mkdir -p "$name/docs/brand"
cp README.md LICENSE NOTICE ATTRIBUTION.md "$name/"
cp docs/brand/LICENSE-brand.txt "$name/docs/brand/"

if [ "$kind" = macos ]; then
    app="$name/BeamCraft.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/assets/fonts/inter" \
        "$app/Contents/Resources/assets/libraries" "$app/Contents/Resources/licenses"
    sed "s/@VERSION@/${tag#v}/" packaging/Info.plist > "$app/Contents/Info.plist"
    cp assets/icons/BeamCraft.icns "$app/Contents/Resources/"
    cp assets/icons/beamcraft-icon-macos.png "$app/Contents/Resources/beamcraft-icon.png"
    mkdir -p "$app/Contents/Resources/assets/icons"
    cp assets/icons/beamcraft-1024.png "$app/Contents/Resources/assets/icons/"
    cp "$release/beamcraft" "$release/beamcraft-ui" "$app/Contents/MacOS/"
    cp assets/fonts/inter/Inter-Regular.ttf assets/fonts/inter/Inter-Bold.ttf \
        "$app/Contents/Resources/assets/fonts/inter/"
    cp licenses/OFL-1.1-Inter.txt licenses/TOST-1.0-openusd.txt \
        "$app/Contents/Resources/licenses/"
    cp -R environments "$app/Contents/Resources/"
    cp -R assets/libraries "$app/Contents/Resources/assets/"
    # The arm64 kernel refuses unsigned executables; ad-hoc signing keeps the
    # bundle runnable on a fresh install. No developer identity is involved.
    # Progress goes to stderr: stdout carries the archive path only.
    codesign --force --deep --sign - "$app" 1>&2
else
    mkdir -p "$name/assets/fonts/inter" "$name/assets/icons" "$name/assets/libraries" \
        "$name/licenses"
    cp assets/icons/beamcraft-1024.png assets/icons/beamcraft-icon.png "$name/assets/icons/"
    cp "$release/beamcraft$ext" "$release/beamcraft-ui$ext" "$name/"
    cp assets/fonts/inter/Inter-Regular.ttf assets/fonts/inter/Inter-Bold.ttf \
        "$name/assets/fonts/inter/"
    cp licenses/OFL-1.1-Inter.txt licenses/TOST-1.0-openusd.txt "$name/licenses/"
    cp -R environments "$name/"
    cp -R assets/libraries "$name/assets/"
    if [ "$kind" = win ]; then
        cp assets/icons/beamcraft-windows.ico "$name/BeamCraft.ico"
    else
        cp assets/icons/beamcraft-linux-256.png "$name/assets/icons/"
        cp packaging/install-linux.sh "$name/"
        chmod +x "$name/install-linux.sh"
    fi
fi

# Self-check (Q02 finding F1): every file the shipped NOTICE points to must be
# in the package, so a future provenance pointer fails packaging here instead of
# dangling on a clean machine. third-party/ names describe files inside the
# foreign crate, not package members; the macOS bundle keeps licences under
# Contents/Resources instead of the package root.
roots="$name"
if [ "$kind" = macos ]; then
    roots="$roots $app/Contents/Resources"
fi
while IFS= read -r rel; do
    case "$rel" in third-party/*) continue ;; esac
    shipped=""
    for root in $roots; do
        if [ -f "$root/$rel" ]; then
            shipped=1
            break
        fi
    done
    if [ -z "$shipped" ]; then
        echo "NOTICE references a file the package does not ship: $rel" >&2
        exit 2
    fi
done <<EOF
$(grep -oE '[A-Za-z0-9][A-Za-z0-9/._-]*\.(md|txt)' "$name/NOTICE" | sort -u)
EOF

if [ "$kind" = win ]; then
    7z a "$name.zip" "$name" >/dev/null
    printf '%s\n' "$name.zip"
else
    tar czf "$name.tar.gz" "$name"
    printf '%s\n' "$name.tar.gz"
fi
