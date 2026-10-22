# Visual bug evidence, 2026-10-23

Original captures of unmodified BeamCraft commit `113adc77e8057e3d680e84f84a050c23e3e1e022`.
Capture time: 2026-10-22 UTC / 2026-10-23 Asia/Shanghai.
Environment: Linux x86_64, X11/Xvfb, native window 1400 x 900 px, 100% UI scale,
llvmpipe (LLVM 19.1.7, 256 bits) / CPU / Vulkan, 1x MSAA, development build.
Both application binaries were built with `cargo build --workspace --locked`.

These files support three individual bug issues. This branch stores evidence only;
it contains no application fix and is not intended for merging into master.
All model dimensions and profile parameters are invented synthetic test values,
not standards data, a manufacturer catalogue, or a client project.

## Evidence

- `selection-rectangle.png`: a click selects the rectangular control member, with a yellow outline.
- `selection-i-beam.png`: clicks on the visible I-section member leave Nothing selected and disabled edit actions.
- `selection-holed-plate.png`: clicks on solid areas of the holed plate also leave Nothing selected.
- `selection.mp4`: an 18-second recording comparing selection of the three visible objects; original screen recording, no annotations or compositing.
- `publish-stale.png`: Create confirms draft sequence 2, but Publish still reports zero unsent revisions and draft sequence 1, even after R.
- `publish-after-restart.png`: restarting the same application with the same store reports one unsent revision and draft sequence 2.
- `properties-axis-clipped.png`: the Properties Axis row is cut off by the fixed-width sidebar; most end coordinates cannot be read.

The PNGs are unmodified native-window captures. They contain no third-party software
screenshots, personal projects, or vendor data. Inter remains under its existing
OFL-1.1 attribution in the root ATTRIBUTION.md. Original captures and synthetic
reproducer are provided by BeamCraft contributors under MPL-2.0.

## Reproduce

Build both applications at the audited commit, then run one of:

```sh
python3 docs/visual-bug-evidence-2026-10-23/reproduce.py selection
python3 docs/visual-bug-evidence-2026-10-23/reproduce.py publish
python3 docs/visual-bug-evidence-2026-10-23/reproduce.py clipping
```

Each invocation creates a fresh temporary published store and personal workspace.
Use `--bin-dir /absolute/path/to/binaries` for another build directory, or
`--prepare-only` to generate the model and print the UI argument vector without
opening a window. The selection case pre-creates the I-beam; the captured session
created that same I-beam with Create > Member > Preview > Commit.

## Integrity

SHA-256 of the raw evidence files:

```text
e59240178d30d9bcd1a55065e5dad6ed3b8d79306bac49a91ec9710ed295a7cc  selection-rectangle.png
4090064e9e56a4b177fd952d790e2ad07a8c9aada843311d446ff7b20fd0c638  selection-i-beam.png
33c238f7dfaac27c4c125446a98d05ce813d2fe534a11b1db287f70e5407978e  selection-holed-plate.png
cce4ab90c59c6def759592b91063d8b6fd68ec17e6d3fb2da1d746773000dbd5  selection.mp4
9ce85a8c6d9ce86fb0453d69c7d1cfb97e8ff2b9b6e44c8a774c8c0fa578016e  publish-stale.png
da963bc2964449ac51a6d2e6a96a86d8451cf3b3073011675bb7f7f3736eb1b8  publish-after-restart.png
f1975772a2f8db50f9f36a871ded2944f0082480b90281a30a961b2ead69d31c  properties-axis-clipped.png
```
