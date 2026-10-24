# Application icons

The official BeamCraft logo is the application icon everywhere: the macOS Dock,
Finder and Spotlight, the Windows Explorer and taskbar, the Ubuntu launcher and
the Linux window managers, and — as the raster behind the start window, the
start page and the runtime window icon — inside the application itself. The
master artwork is `assets/icons/beamcraft-1024.png`, the unmodified official
raster attached to the brand issue; `docs/brand/beamcraft-logo.svg` is its
vector source. Neither file is ever modified.

## macOS

`BeamCraft.icns` is resampled from the official raster with the tools included
with macOS:

```sh
./packaging/generate-icon.sh
```

`packaging/package.sh` installs the ICNS for Finder/Spotlight and copies the
official raster into the bundle as `Contents/Resources/beamcraft-icon.png` for
eframe's runtime Dock icon. Release smoke checks decode the packaged ICNS with
`iconutil`.

## Windows and Linux

`beamcraft-windows.ico` contains 16, 24, 32, 48, 64, 128 and 256 pixel
representations, each a resize of the official raster; `beamcraft-linux-256.png`
is the hicolor launcher size. Windows/Linux packages ship the official raster
as `assets/icons/beamcraft-icon.png`, which the runtime window icon loads. The
ICO container is packed as a one-off from the resized rasters (PNG payloads,
no script shipped); provenance is recorded in `ATTRIBUTION.md`.

## Native acceptance still required

On macOS, build the distribution binaries and run the packaging command
described in `packaging/package.sh`. Extract the archive, open BeamCraft.app
and check the running Dock icon alongside stock apps at normal and magnified
sizes. Quit and check the bundle icon in Finder and Spotlight. Capture actual
screenshots of all three surfaces, including the running Dock; generated
previews do not establish native acceptance. Verify clean small sizes and
matching running/closed icons. On Windows and Linux, capture the desktop,
taskbar and launcher through the screenshot workflow.

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
