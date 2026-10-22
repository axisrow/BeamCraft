# macOS application icon

The original `beamcraft-1024.png` and everything under `docs/brand/` remain unchanged.
The macOS composition places the official SVG's I-beam group, without changing its
paths or colours, on a contributor-original continuous-corner enamel tile. The
824-unit tile has 100-unit canvas insets, a subtle rim, directional lighting and
soft shadows. No grid competes with the mark at small Dock sizes.

Rebuild from any directory with Python 3.12, CairoSVG 2.8.2 (system Cairo required)
and Pillow 12.3.0:

```sh
python3 -m pip install CairoSVG==2.8.2 Pillow==12.3.0
./packaging/generate-icon.sh
python3 -m unittest discover -s packaging -p 'test_*.py'
```

The SVG tile and the compositor are the editable sources. The generator reads
`docs/brand/beamcraft-logo.svg` without writing it. It supersamples at 2048px and
writes `beamcraft-macos-1024.png` plus PNG-backed ICNS representations for all
standard/Retina sizes, including 16, 32, 64, 128, 256, 512 and 1024px. Pinned
renderer versions in the same environment produce identical bytes; Cairo/zlib
versions can affect encoding across operating systems.

`packaging/package.sh` installs the ICNS for Finder/Spotlight and the same
composition as `Contents/Resources/beamcraft-icon.png` for eframe's runtime Dock
icon. Windows/Linux and the unbundled fallback are unchanged. Attribution and
retained trademark terms are in `ATTRIBUTION.md`.

## Native acceptance still required

On macOS, build the distribution binaries and run the packaging command described
in `packaging/package.sh`. Extract the archive, open BeamCraft.app and check the
running Dock icon alongside stock apps at normal and magnified sizes. Quit and
check the bundle icon in Finder and Spotlight. Capture actual screenshots of all
three surfaces, including the running Dock; generated previews do not establish
native acceptance. Verify transparent corners, comparable tile scale, clean small
sizes and matching running/closed icons. Do not mark issue #59 complete before
this check and the required workspace checks pass.
