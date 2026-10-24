# Application icons

The original `beamcraft-1024.png` and everything under `docs/brand/` remain unchanged.
The macOS composition places the official SVG's I-beam group, without changing its
paths or colours, on a contributor-original continuous-corner enamel tile. The
824-unit tile has 100-unit canvas insets, a subtle rim, directional lighting and
soft shadows. No grid competes with the mark at small Dock sizes.

`macos-tile.svg` is the editable design source of the composition;
`beamcraft-macos-1024.png` is its checked-in 1024px raster. Rebuild the ICNS from
the raster with the tools included with macOS:

```sh
./packaging/generate-icon.sh
```

## Windows and Linux icons

The Windows ICO and Linux PNG use the same composition. The ICO contains 16, 24,
32, 48, 64, 128 and 256 pixel representations. Windows/Linux runtime window icons
load the packaged PNG from `assets/icons/beamcraft-icon.png`; the Windows archive
also includes `BeamCraft.ico`.

Rebuild the Windows and Linux artwork from any directory with Python 3.12,
CairoSVG 2.8.2 (system Cairo required) and Pillow 12.3.0:

```sh
python3 -m pip install CairoSVG==2.8.2 Pillow==12.3.0
python3 packaging/generate-icon.py
```

The SVG tile and compositor are the editable sources. The generator reads
`docs/brand/beamcraft-logo.svg` without writing it. It supersamples at 2048px and
writes the Linux PNGs and the Windows ICO. Pinned renderer versions in the same
environment produce identical bytes; Cairo/zlib versions can affect encoding
across operating systems.

`packaging/package.sh` installs the ICNS for Finder/Spotlight and the same
composition as `Contents/Resources/beamcraft-icon.png` for eframe's runtime Dock
icon. Attribution and retained trademark terms are in `ATTRIBUTION.md`.

## Native acceptance still required

On macOS, build the distribution binaries and run the packaging command described
in `packaging/package.sh`. Extract the archive, open BeamCraft.app and check the
running Dock icon alongside stock apps at normal and magnified sizes. Quit and
check the bundle icon in Finder and Spotlight. Capture actual screenshots of all
three surfaces, including the running Dock; generated previews do not establish
native acceptance. Verify transparent corners, comparable tile scale, clean small
sizes and matching running/closed icons.

## Ubuntu application menu and Dock

After extracting the Linux release, run `bash install-linux.sh` from its folder.
It installs BeamCraft under `${XDG_DATA_HOME:-$HOME/.local/share}/beamcraft`,
registers `com.apibim.BeamCraft.desktop` in the user applications directory and
installs the application icon in the user's `hicolor` icon theme. No sudo is needed.
Open Ubuntu's Show Applications, search for BeamCraft and launch it. Right-click
its Dock icon and choose Pin to Dash to keep the launcher there. The Linux window's
application ID and X11 window class match the desktop entry, so the running window
groups with that launcher.
Run the installer again from a newer extracted release to update it.

The Linux screenshot workflow launches the installed desktop entry in Ubuntu's
GNOME Shell session and captures both the Dock and Show Applications.
