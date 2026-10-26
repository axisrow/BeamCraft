# Application icons

BeamCraft has one application icon on every platform: the official logo
`docs/brand/beamcraft-logo.svg`, rendered unchanged and clipped to a rounded square (corner
radius 22.5 % of the side). The macOS Dock, Finder and Spotlight, the Windows taskbar and
Explorer, the Ubuntu launcher and the running window all show this same picture. Only the margin
differs: Windows and Linux draw icons edge to edge, so there the tile fills the canvas; on macOS
it sits on Apple's 1024-unit grid, an 824-unit tile with a 100-unit transparent margin. A margin
on Windows would leave the tile about 19 px wide in a 24 px taskbar slot.

| File | Use |
| --- | --- |
| `beamcraft-icon.png` | 1024 px, edge to edge; the runtime window icon on Windows and Linux |
| `beamcraft-icon-macos.png` | 1024 px on Apple's grid; the macOS runtime Dock icon |
| `BeamCraft.icns` | macOS bundle icon, 16 to 1024 px including the Retina slots |
| `beamcraft-windows.ico` | Windows icon, 16 to 256 px including 20, 24 and 40; shipped as `BeamCraft.ico` |
| `beamcraft-windows.res` | The ICO as a compiled resource; `build.rs` links it into `beamcraft-ui.exe` |
| `beamcraft-linux-256.png` | Linux hicolor launcher icon |

All of them are derived from the official SVG and committed, so building and packaging need no
image tools. To redo them after a change to the logo: render the SVG at the tile size (1024 px,
or 824 px for macOS), clip it to the rounded square, place it on a transparent 1024 px canvas,
and resample each platform size from that raster with a Lanczos filter. The ICNS holds PNG
representations (`icp4` to `ic14`); `beamcraft-windows.res` holds the ICO's images as `RT_ICON`
resources and their directory as `RT_GROUP_ICON` 1, which the window and Explorer load.

`beamcraft-1024.png` is the unmodified official raster. It is not an application icon: it is the
logo on the start window and start page. It and everything under `docs/brand/` are never
modified.

## Packaging

`packaging/package.sh` installs the ICNS for Finder/Spotlight and copies
`beamcraft-icon-macos.png` into the bundle as `Contents/Resources/beamcraft-icon.png`, which the
UI applies at run time to the Dock (eframe shows the process icon while the app runs).
Source-tree macOS launches also select `beamcraft-icon-macos.png`, so the edge-to-edge
Windows/Linux raster cannot override the macOS margin in development builds.
On Windows the executable's icon resource serves Explorer and shortcuts, and the patched eframe
(`Apibim-com/eframe`) sets it as the window icon too, so the title bar and the taskbar draw the
exact ICO size; `beamcraft-icon.png` is only the fallback there. Windows and Linux archives carry
`assets/icons/beamcraft-icon.png` for the window icon; the Windows archive also carries
`BeamCraft.ico`, the Linux archive `install-linux.sh` and the launcher PNG.

## Ubuntu application menu and Dock

After extracting the Linux release, run `bash install-linux.sh` from its folder. It installs
BeamCraft under `${XDG_DATA_HOME:-$HOME/.local/share}/beamcraft`, registers
`com.apibim.BeamCraft.desktop` in the user applications directory and installs the icon in the
user's `hicolor` icon theme; no sudo is needed. The window's Wayland app ID and X11 window class
match the desktop entry, so the running window groups with the launcher. Run the installer again
from a newer release to update it.

## Native acceptance still required

On macOS, package the app, open BeamCraft.app and compare the running Dock icon with stock apps
at normal and magnified sizes; quit and check Finder and Spotlight. On Windows, check the
taskbar; on Ubuntu, the Dock and Show Applications after `install-linux.sh`. Generated previews
do not establish native acceptance.
