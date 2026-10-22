# BeamCraft UI

V03: the native window and viewport. It shows the accepted (published) revision of a store.

```text
beamcraft-ui
beamcraft-ui --store ROOT [--cli PATH]
beamcraft-ui --workspace DIR [--published ROOT] [--cli PATH]
beamcraft-ui --store ROOT --metrics FRAMES [--width W] [--height H] [--samples 1|4] [--cli PATH]
beamcraft-ui --synthetic COUNT --metrics FRAMES [--width W] [--height H] [--samples 1|4]
beamcraft-ui --store ROOT --perf-run PLAN [--samples 1|4] [--cli PATH]
```

- With no arguments the window opens on a per-user default workspace under the system data
  directory (`~/Library/Application Support/BeamCraft`, `%APPDATA%\BeamCraft`, or
  `XDG_DATA_HOME`/`~/.local/share`): the published store and the workspace fork are created
  on demand, so a double-clicked app bundle launches with undo, redo and Publish.

- The model comes from the engine, through the headless CLI in a separate process:
  `beamcraft view-export --store ROOT --out - --with-definitions true` writes the pinned view
  snapshot as one bundle, framed after the store's definitions from the same verified inspection,
  and `beamcraft_engine::ViewSnapshot::from_bundle` rebuilds and re-validates it here. One refresh
  is one process and one inspection; only when the export fails is the definitions listing asked on
  its own. The UI never opens USD and never depends on the CLI crate (F08:
  an app may not depend on an app). The CLI is looked up next to the UI executable unless `--cli`
  names it.
- Each distinct mesh is uploaded once and drawn with one instance per element placement.
  Coordinates are millimetres, re-centred on the engineering bounds in f64 before f32 conversion.
  Once a scene is on the device the window keeps only its origin, framing and counts, not the
  expanded vertex buffers. A refresh of an unchanged snapshot builds, uploads and draws nothing,
  and meshes the device already holds (same digest) are not expanded or sent again; uploads never
  wait for the device on the UI thread.
- The window draws the 3D image every frame. A resize rebuilds the targets, never the shader or
  the pipeline. MSAA is 4x on a hardware adapter that supports it and 1x on a software (CPU)
  adapter; `--samples 1|4` overrides it.
- Controls, `Mouse` preset (the default): the left button is reserved for selection and never
  moves the camera, middle-button drag pans, Ctrl + middle-button drag orbits, the wheel zooms
  toward the cursor. `Trackpad (macOS)` preset (side panel): primary drag orbits, Shift +
  primary drag or secondary drag pans, the wheel or a two-finger scroll or pinch zooms toward
  the cursor (the "Invert zoom" toggle flips the scroll direction for macOS natural scrolling;
  pinch and keys keep theirs). Both presets: `+` and `-` zoom from the keyboard, F fits, R
  reloads (`+`, `-`, F and R are ignored while a text field has the keyboard). A click selects,
  Esc clears the selection.
- V05.1 Create panel (experimental): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown with its code and message. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- `--perf-run PLAN` opens the window, runs a scripted measurement in it and prints one JSON report:
  `PLAN` is a comma-separated list of `load`, `idle:S`, `ui:K` (frames without a viewport change),
  `orbit:K`, `reload:K`, `select:K` (clicks onto box elements), `cancel:K` (a click, Esc while its
  lookup runs, another click), `edit:K` (creates K plates in the store through the Create panel's
  command path) and `resize:K`. Each phase reports frames, 3D draws, submits, uploads and bytes,
  pipeline and target builds, CLI processes (started, killed) and their time, selection errors,
  refresh timings and, on Linux, the CPU time of the process and of its CLI children and the
  resident set size; the exit status is 1 when a phase failed. See `src/perf.rs` and
  [the issue 26 design note](../../docs/design/issue-26-viewport-performance.md).
- The UI font is the admitted Inter, read at run time from `assets/fonts/inter`; without it the
  window runs without text. `default_fonts` is never enabled.

Linux needs a Vulkan driver and the X11 or Wayland client libraries (including
`libxkbcommon-x11`) at run time; a missing window-system library is reported, not a crash.
