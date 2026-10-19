---
name: run-beamcraft-ui
description: Build and launch the native BeamCraft window (beamcraft-ui) headless in a Linux container with Xvfb and the Mesa lavapipe Vulkan driver, author a sample steel frame store, drive the UI with xdotool and take screenshots. Use when asked to run, screenshot or visually check the app.
---

# Run and screenshot beamcraft-ui (Linux container)

The UI is egui + wgpu and needs a Vulkan driver and an X display. In a container there is no GPU
and no window manager, so use Xvfb with the lavapipe software driver and give the window keyboard
focus explicitly. Everything below is wrapped in `run.sh` next to this file.

```sh
R=.claude/skills/run-beamcraft-ui/run.sh
S=/absolute/scratch/dir            # e.g. the session scratchpad; never inside the repo
export DISPLAY=:99                 # run.sh sets it itself; the raw xdotool calls below need it
$R setup                           # apt: mesa-vulkan-drivers, libxkbcommon-x11-0, xvfb, xdotool, xclip, imagemagick
$R build                           # cargo build --release -p beamcraft-cli -p beamcraft-ui (~several min cold)
$R store $S/store                  # 1 material, 2 profiles, 6 columns, 5 beams, 6 base plates with holes
$R start $S/store                  # Xvfb :99 1600x1000, UI window at 0,0, focused; waits for the
                                   # model (an empty store says "model loaded: no" after ~20 s)
$R shot  $S/01-initial.png         # then Read the PNG and look at it
```

The UI looks for the `beamcraft` CLI next to its own executable, so build both in the same profile.
Give `store` a fresh, empty absolute directory each time.

## Driving the window

Coordinates are screen pixels at 1600x1000 with the window at 0,0. Left panel layout (Plate tab):

| Control             | x, y     |
|---------------------|----------|
| Plate / Member tabs | 27,339 / 83,339 |
| Width / Length / Thickness / Name fields | 90,380 / 98,403 / 118,426 / 120,449 |
| Preview / Commit / Cancel | 36,511 / 100,511 / 160,511 |

The panel moves with its content, so take a screenshot before clicking anywhere new: it widens on
the Member tab and again when a long profile name is selected, and on a store with no revision yet
(no model loaded) the Create form sits 73 px higher (Width at 90,306, Preview/Commit at y 438).
Select-all before retyping a field: `$R click X Y; xdotool key ctrl+a; $R type NEW`.

```sh
$R click 90 380; $R type 800      # Width
$R click 98 403; $R type 1200     # Length
$R click 118 426; $R type 20      # Thickness
$R click 36 511                   # Preview: shows the exact create-plate argument list
$R click 100 511; sleep 4         # Commit: runs the CLI, reloads; Elements goes up by one
xdotool mousemove 900 500; $R key f   # fit (keys only reach the viewport when no field has focus)
```

Viewport: drag with button 1 to orbit (`xdotool mousedown 1`, several `mousemove_relative`,
`mouseup 1`), button 3 to pan, `xdotool click 4/5` to zoom. Small drags only: about 100 px
horizontally already swings the camera far.

`$R stop` ends the UI and Xvfb.

## Gotchas

- Text fields stay empty if the window has no X keyboard focus; `start` retries `windowfocus --sync`
  (X can answer BadMatch just after the window maps).
  Re-run `xdotool windowfocus --sync $(xdotool search --name '^BeamCraft$')` if typing stops working.
- A press and release in the same frame are missed by egui; `click` keeps them apart.
- Non-ASCII text (Cyrillic names): export `LANG=C.UTF-8` for xdotool and use `$R paste TEXT`;
  fast per-key typing drops and reorders remapped letters.
- Authoring sample stores: an element over 1 m3 is currently refused with `LIMIT_EXCEEDED: stage
  budget exceeded` (the layer engine caps stored numbers at 1e9 and volume is in mm3), so keep each
  plate and member at or under 1 m3.
- The info panel reports `llvmpipe ... / Cpu / Vulkan`; 35-50 FPS is normal for software rendering.
- A blank or black window means the driver did not load: check the UI log written by `start`.
- Without `assets/fonts/inter` the window runs without text (by design).
- For numbers without a window, `beamcraft-ui --store DIR --metrics FRAMES` renders offscreen and
  prints JSON (it still needs the Vulkan driver, not Xvfb).
