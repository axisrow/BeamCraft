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
  `beamcraft export-view --store ROOT --out - --with-definitions true --with-grids true` writes the pinned view
  snapshot as one bundle, framed after the store's definitions and grid display data from the same verified inspection,
  and `beamcraft_engine::ViewSnapshot::from_bundle` rebuilds and re-validates it here. One refresh
  is one process and one inspection; only when the export fails is the definitions listing asked on
  its own. The UI never opens USD and never depends on the CLI crate (F08:
  an app may not depend on an app). The CLI is looked up next to the UI executable unless `--cli`
  names it.
- Each distinct mesh is uploaded once and drawn with one instance per element placement.
  Attached read-only references in the [R01 translation-only display profile](../../docs/design/r01-readonly-reference.md#translation-only-viewport-path-issue-137)
  use the same path and contribute to camera fit; unsupported foreign transforms are explicit
  export-view refusals, not invisible or incorrectly placed geometry.
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
  toward the cursor. `Trackpad (macOS)` preset (Details window, from the status bar): primary drag orbits, Shift +
  primary drag or secondary drag pans, the wheel or a two-finger scroll or pinch zooms toward
  the cursor (the "Invert zoom" toggle flips the scroll direction for macOS natural scrolling;
  pinch and keys keep theirs). Both presets: `+` and `-` zoom from the keyboard, F fits, R
  reloads (`+`, `-`, F and R are ignored while a text field has the keyboard). A click selects,
  Esc clears the selection.
- Render modes (experimental V16 slice, see [the design note](../../docs/design/v16-render-modes.md)):
  the View tab's DISPLAY group and `Ctrl+1` to `Ctrl+5` switch the drawing of the one loaded
  scene between wireframe, transparent, grey, shaded (the default) and selected only; switching
  never reloads the scene. The bindings are remappable in `shortcuts.json` in the user's
  configuration directory (`BEAMCRAFT_CONFIG_DIR`, or `--config-dir` with `--store`); a missing
  file is the defaults, a damaged one keeps them and says so in the Details window.
- Live refresh: the window watches the store's committed revisions and reloads by itself when
  another process (the CLI, an agent) commits to the store it shows; `R` stays for a manual
  reload. A reload whose framing changed fits the camera, so new content comes into view.
- Layout (the BeamCraft look of `src/theme.rs`): a brand header with the project, its
  save state, the environment and author, and Publish; a ribbon of tools (Select; Plate, Beam,
  Column; Move, Copy, Delete, Undo, Redo; Shaded); an unobstructed 3D view; an
  horizontally resizable inspector on the right that shows what the chosen tool needs (properties, a create form or an
  edit form); a status bar with the selection and a Details window (frame rate, adapter,
  revision, navigation). Tools that do not exist yet (Bolts, Weld, Wire, the Plan view,
  snapping) are shown disabled. Model parts are drawn in the logo's colours on a blueprint
  ground that follows the light or dark theme.
- Command navigation: the three-horizontal-line button immediately left of Select slides the
  workspace panel in from the left edge below the ribbon. Its grouped file, exchange, project,
  support and session commands are disabled placeholders. Close it with the same toggle, its
  close button or Esc. The docked panel shares the layout with the viewport; the rest of the window
  stays interactive, without a popup frame or a backdrop.
  The ribbon has Model, Modify, View, Drawings and Project tabs. Existing modelling and editing
  commands retain their behaviour; new command placeholders are disabled with an availability
  tooltip. Horizontal scrolling keeps the tabs and tools reachable in narrow windows.
  Tabs use text and an active underline rather than button frames. The selection tool is an
  icon with a tooltip, without repeated Select captions. The viewport has no floating
  3D/Plan/Grid or Fit/zoom controls; the wheel and keyboard navigation remain available.
  Orbit dragging moves the model's near surface in the pointer's direction on both presets.
  See [the navigation note](../../docs/design/v10-command-navigation.md).
- V05.1 Create panel (experimental, now the inspector of Plate, Beam and Column): a plate (width, length, thickness, optional name and material)
  or a straight beam/column (start, end, profile and material chosen from the store's definitions,
  optional name). Preview stages the exact `create-plate` / `create-member` argument list (one per line, not a shell
  command line; no geometry preview), Commit runs it in a separate process and reloads the accepted revision,
  Cancel drops it and publishes nothing; a CLI refusal is shown as a message without a technical code prefix. The
  parameter mapping and reply parsing are `tools.rs` (pure, unit-tested); the process is
  `command.rs`. See [the design note](../../docs/design/v05-1-ui-create-tools.md).
- `--metrics` renders offscreen without a window and prints one JSON object (adapter, scene size,
  upload, per-frame median and p95). `--synthetic` measures synthetic kernel plates for scale.
- Grid insertion (experimental T01.6 slice): Model → Grid opens regular-family coordinates,
  labels, line limits and frame inputs. Create grid runs the existing `create-grid`; Cancel runs
  nothing. Guides and labels are read with the pinned meshes in one `export-view --with-grids true`
  process and drawn in the viewport. Grid visibility belongs to view properties, with no switch in
  the grid form. Levels are separate.
  Click a grid line or endpoint label to select the whole grid and edit its properties in the
  inspector. Apply changes invokes `update-grid` as one undo unit; Cancel discards unapplied input
  and selection. A changed grid makes a dirty form stale until selected again. Drawn-family editing
  is not offered in this panel.
  Coordinate edits extend regular lines across the full grid with 2000 mm extensions; explicit
  line-limit edits take precedence. Start distances are positive in the form and negated for the
  CLI. Rotation displays at most three decimals without changing unedited stored precision.
  Delete outside text fields, or Delete on the ribbon, deletes the selected grid through the CLI.
  See [the insertion note](../../docs/design/t01-6-grid-insertion.md) for prerequisite PRs and limits.
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
